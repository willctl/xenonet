//! `xenolith/join/1`: one Xenolith joins the spaces another curates, with an invite and a code, in
//! place of 1.x's pairing. It is xenonet-net's generic join (`xenonet_net::join`) with Xenolith's
//! `JoinTarget`, so the ticket, the proofs and their order are XenoNet's join/2's (review revision
//! 20 says what changes):
//!
//! - **The invite** (`xlj1_…`) is not secret. It carries the ticket id (128 random bits), the
//!   inviter's endpoint id, the inviter's relay if it uses one, and, as the target's id, the
//!   SHA-256 of the offer: the owner's key id, the curator's context key id, and each offered
//!   space's id and genesis hash. The proofs cover that hash, so a joiner whose invite was swapped
//!   for another's gets nothing it would take for the real one.
//! - **The code** is 26 characters of Crockford's base32 (130 bits; 1.x's pair code was 128), told
//!   the other way, typed at a prompt or given on stdin, never as an argument. A pasted invite can
//!   be a fake inviter's, and the fake then holds the joiner's proof, which it could try every code
//!   of a short one against offline; at 130 bits it can't. It works once, for ten minutes, and five
//!   wrong codes retire it (the door's). The inviter keeps only its hash.
//! - **The proofs.** The joiner proves first, over the ticket, the offer's hash, both endpoint ids
//!   (the inviter's being the one it dialled, which QUIC authenticated) and the `add` it asks for
//!   (its context key and its endpoint id). The inviter answers a wrong code with a refusal that
//!   holds no proof of its own, and a right one with its proof, the offer and the chains, each with
//!   the Share it appended for the joiner (`Inviting`).
//! - **The joiner checks** the answer before it follows anything (`accept`): the offer is the one
//!   the invite names, each chain checks from its genesis, is the space offered, with the owner and
//!   curator offered, and shares the space with this Xenolith's key at this endpoint. It follows
//!   what the person chose (all offered by default), each in a folder of its own beside any of the
//!   same name.
//! - **The owner key.** `xenolith invite` and `xenolith join` go through the service with the owner
//!   key, as the window's owner acts do, and a Share is signed by the standalone owner key. That's
//!   weaker than presence: an unsandboxed agent running as you can read both files.

use crate::{Config, hex, unhex};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use xenolith_core::chains::{self, Level};
use xenolith_core::keys::Signer;
use xenolith_core::spaces::SpaceId;
use xenolith_core::sync::{self as cs, Follow};
use xenonet_id::codec::{b64, sha256, unb64_n};
use xenonet_log::space::Space;
use xenonet_net::dial::{Alpn, Fut, PeerId};
use xenonet_net::join::{Admit, Admitted, Invite, JoinTarget, Ticket, TicketId};
use xenonet_net::{NetError, R};

pub const ALPN: &[u8] = b"xenolith/join/1";
/// How long an invite works.
pub const TICKET_MS: i64 = 10 * 60 * 1000;
/// A code's characters: 26 of Crockford's base32, 130 bits.
pub const CODE_LEN: usize = 26;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// The most spaces one invite offers.
pub const MAX_OFFERED: usize = 64;

/// Xenolith's join: its own ALPN, invites, proofs and codes, never XenoNet's.
pub const TARGET: JoinTarget = JoinTarget {
    alpn: Alpn::App(ALPN),
    prefix: "xlj1_",
    label: "xenolith-join/1",
    schema: "xenolith/join/v1",
    how: "xenolith invite",
    target_len: 32,
    target_bytes: |id| unb64_n::<32>(id).map(|b| b.to_vec()),
    target_of: |b| <[u8; 32]>::try_from(b).ok().map(|b| b64(&b)),
    relay_ok: |u| u.starts_with("https://") && u.len() <= 256,
    code: normalize,
    code_shape: "a code is 26 letters and digits, as `xenolith invite` showed it",
};

/// A new code: 26 characters, each uniform (256 is a multiple of 32).
pub fn make_code() -> Result<String, String> {
    let bytes = xenonet_id::codec::random::<CODE_LEN>().map_err(|e| e.message)?;
    Ok(bytes.iter().map(|b| char::from(ALPHABET[usize::from(b & 31)])).collect())
}

/// As people see and type it: groups of four.
pub fn display(code: &str) -> String {
    code.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

/// A code as it was typed, to its 26 characters: any case, dashes and spaces dropped, O for 0 and
/// I or L for 1 forgiven.
pub fn normalize(typed: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_LEN);
    for c in typed.trim().chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == CODE_LEN).then_some(out)
}

// ---------------------------------------------------------------- the offer

/// A space an invite offers: its id and its genesis hash, and, for the person, its address on the
/// inviter (not covered by the hash: the chain names the space).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offered {
    pub space: String,
    pub genesis: String,
    pub address: String,
}

/// What an invite offers: who owns the spaces, who curates them, which.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// `key:xns1_…` (the standalone owner key) or `person:prs_…`.
    pub owner: String,
    /// The curator's context key id.
    pub curator: String,
    pub spaces: Vec<Offered>,
}

impl Offer {
    /// What the invite carries of it: the SHA-256 of its lines, the spaces in id order.
    pub fn digest(&self) -> String {
        let mut spaces: Vec<&Offered> = self.spaces.iter().collect();
        spaces.sort_by(|a, b| a.space.cmp(&b.space));
        let mut text = format!("xenolith-offer/1\n{}\n{}\n", self.owner, self.curator);
        for s in spaces {
            text.push_str(&format!("{} {}\n", s.space, s.genesis));
        }
        b64(&sha256(text.as_bytes()))
    }

    pub fn to_json(&self) -> Value {
        json!({
            "owner": self.owner,
            "curator": self.curator,
            "spaces": self.spaces.iter().map(|s| json!({ "space": s.space, "genesis": s.genesis, "address": s.address })).collect::<Vec<_>>(),
        })
    }

    pub fn parse(v: &Value) -> Option<Offer> {
        let s = |k: &str| v[k].as_str().map(String::from);
        let list = v["spaces"].as_array().filter(|l| !l.is_empty() && l.len() <= MAX_OFFERED)?;
        let mut spaces = vec![];
        for x in list {
            let space = x["space"].as_str().filter(|i| xenonet_model::ids::SpaceId::parse(i).is_ok())?.to_string();
            let genesis = x["genesis"].as_str()?.to_string();
            let address = x["address"].as_str().unwrap_or("").to_string();
            spaces.push(Offered { space, genesis, address });
        }
        Some(Offer { owner: s("owner")?, curator: s("curator").filter(|k| xenonet_model::ids::is_signing_key_id(k))?, spaces })
    }
}

/// The offer for `addresses`, spaces this Xenolith holds and curates, all with one owner.
pub fn offer_of(cfg: &Config, addresses: &[SpaceId]) -> Result<(Offer, Vec<(SpaceId, Space)>), String> {
    let me = cfg.keys.context.public().id();
    if addresses.is_empty() {
        return Err("name the spaces to invite into, as project:atlas".into());
    }
    if addresses.len() > MAX_OFFERED {
        return Err(format!("one invite offers {MAX_OFFERED} spaces at most"));
    }
    let mut spaces = vec![];
    let mut owner: Option<String> = None;
    let mut held = vec![];
    for a in addresses {
        let (id, space) = chains::of_address(&cfg.paths, a).ok_or_else(|| format!("{a} has no chain here (xenolith upgrade founds one)"))?;
        if space.view().curator.id() != me {
            return Err(format!("{a} is curated by another Xenolith ({}): invite from there", space.view().curator.id()));
        }
        let o = chains::describe(&space)["owner"].as_str().unwrap_or("").to_string();
        if owner.as_ref().is_some_and(|x| *x != o) {
            return Err(format!("{a} has another owner than the other spaces named: invite to each owner's spaces apart"));
        }
        owner = Some(o);
        spaces.push(Offered { space: id, genesis: space.entries()[0].hash(), address: a.to_string() });
        held.push((a.clone(), space));
    }
    Ok((Offer { owner: owner.unwrap_or_default(), curator: me, spaces }, held))
}

// ---------------------------------------------------------------- the inviter

/// An invite waiting for its joiner: what it offers, at which level, and the owner key the request
/// that made it presented, which the Shares are signed with once a code checks.
#[derive(Clone)]
pub(crate) struct Pending {
    pub offer: Offer,
    pub addresses: Vec<SpaceId>,
    pub level: Level,
    pub owner_key: String,
}

pub(crate) type Pendings = Arc<Mutex<BTreeMap<TicketId, Pending>>>;

/// What the worker does once a joiner's code checked: appends a Share to each offered space's
/// chain for the joiner's context key at the endpoint that dialled, signed as the owner and the
/// curator, and answers with the offer and the chains. Run on the worker thread (it writes files).
pub(crate) fn admit_on_worker(cfg: &Config, p: &Pending, key: &str, endpoint: &str) -> Result<Value, String> {
    let real = xenolith_core::owner::read(&cfg.paths).ok_or("the owner key is gone: make a new invite")?;
    let owner = cfg.keys.owner.signer(&real, &p.owner_key).map_err(|_| "the service restarted since the invite was made: make a new invite".to_string())?;
    let ctx: &dyn Signer = &cfg.keys.context;
    let mut chains_out = serde_json::Map::new();
    for a in &p.addresses {
        // A Xenolith joining again (its copy lost, say) at the level it has: nothing to append.
        let (_, before) = chains::of_address(&cfg.paths, a).ok_or_else(|| format!("{a} has no chain here"))?;
        if chains::members(&before).iter().any(|m| m.key == key && m.endpoint == endpoint && m.level == p.level) {
            chains_out.insert(before.id().to_string(), before.to_json());
            continue;
        }
        chains::share(&cfg.paths, &cfg.root, a, key, endpoint, p.level, owner, ctx, xenolith_core::text::now_ms())?;
        let (id, space) = chains::of_address(&cfg.paths, a).ok_or_else(|| format!("{a} has no chain here"))?;
        chains_out.insert(id, space.to_json());
    }
    Ok(json!({ "offer": p.offer.to_json(), "level": p.level.as_str(), "chains": chains_out }))
}

/// The inviter's side of a join, on the endpoint's thread: checks what the joiner asked for and
/// hands the rest to the worker.
pub(crate) struct Inviting {
    pub pendings: Pendings,
    pub jobs: Mutex<std::sync::mpsc::Sender<crate::engine::Job>>,
}

impl Admit for Inviting {
    fn admit<'a>(&'a self, ticket: &'a Ticket, joiner: PeerId, add: &'a Value) -> Fut<'a, R<Admitted>> {
        Box::pin(async move {
            let bad = |w: &str| NetError::new(400, "bad-ask", w);
            let key = add["key"].as_str().filter(|k| xenonet_model::ids::is_signing_key_id(k)).ok_or_else(|| bad("the joiner named no context key"))?;
            // The Share names the endpoint that dialled, which QUIC authenticated, and nothing else.
            if add["endpoint"].as_str() != Some(hex(joiner.0).as_str()) {
                return Err(bad("the joiner named another endpoint than the one it dialled from"));
            }
            let pending = self.pendings.lock().ok().and_then(|p| p.get(&ticket.id).cloned()).ok_or_else(|| NetError::new(404, "no-ticket", "that invite was used or retired"))?;
            let (tx, rx) = tokio::sync::oneshot::channel();
            let job = crate::engine::Job::Admit { pending: Box::new(pending), key: key.to_string(), endpoint: hex(joiner.0), reply: tx };
            self.jobs.lock().map_err(|_| NetError::internal("the worker is gone"))?.send(job).map_err(|_| NetError::internal("the worker is gone"))?;
            let answer = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
                .await
                .map_err(|_| NetError::new(504, "timeout", "the inviter took too long to share the spaces"))?
                .map_err(|_| NetError::internal("the worker is gone"))?;
            match answer {
                Ok(v) => {
                    if let Ok(mut p) = self.pendings.lock() {
                        p.remove(&ticket.id);
                    }
                    Ok(Admitted::Joined(v))
                }
                Err(e) => Err(NetError::new(409, "refused", e)),
            }
        })
    }
}

// ---------------------------------------------------------------- the joiner

/// What the joiner asks to be added as: its context key and its endpoint id.
pub fn add_for(cfg: &Config, endpoint: &str) -> String {
    json!({ "key": cfg.keys.context.public().id(), "endpoint": endpoint }).to_string()
}

/// What the joiner wants of an invite: where the inviter is, which spaces (none named: all
/// offered), and whether to follow what the curator shares with it later too.
#[derive(Clone, Debug, Default)]
pub struct Choice {
    pub at: Vec<std::net::SocketAddr>,
    pub only: Vec<String>,
    pub all: bool,
}

/// Checks the inviter's answer against the invite and this Xenolith (see the module's notes), and
/// follows the chosen spaces: a follow record each (the curator's key, its endpoint, where it's
/// reached) and the chain, in a folder of its own. What the person sees of it.
pub fn accept(cfg: &Config, invite: &Invite, endpoint: &str, answer: &Value, choice: &Choice) -> Result<Value, String> {
    let wrong = |w: &str| format!("the inviter's answer was refused: {w}");
    let offer = Offer::parse(&answer["offer"]).ok_or_else(|| wrong("it names no offer"))?;
    if offer.digest() != invite.network {
        return Err(wrong("its offer isn't the one the invite names"));
    }
    let me = cfg.keys.context.public().id();
    let mut checked = vec![];
    for o in &offer.spaces {
        let chain = &answer["chains"][&o.space];
        let space = Space::from_json(xenolith_core::xenonet::space_policy(&cfg.paths), chain).map_err(|e| wrong(&format!("{}'s chain doesn't check: {}", o.space, e.message)))?;
        let v = space.view();
        if space.id() != o.space || space.entries()[0].hash() != o.genesis {
            return Err(wrong(&format!("the chain sent for {} is another space's", o.space)));
        }
        if chains::describe(&space)["owner"].as_str() != Some(offer.owner.as_str()) || v.curator.id() != offer.curator {
            return Err(wrong(&format!("{}'s chain names another owner or curator than the offer", o.space)));
        }
        if !chains::members(&space).iter().any(|m| m.key == me && m.endpoint == endpoint) {
            return Err(wrong(&format!("{}'s chain doesn't share it with this Xenolith", o.space)));
        }
        checked.push((o.clone(), chain.clone(), v.name.clone()));
    }
    let wanted = |o: &Offered, name: &str| choice.only.is_empty() || choice.only.iter().any(|w| *w == o.space || *w == o.address || *w == name);
    let addrs: Vec<String> = choice.at.iter().map(ToString::to_string).collect();
    let mut followed = vec![];
    let mut skipped = vec![];
    for (o, chain, name) in checked {
        if !wanted(&o, &name) {
            skipped.push(json!({ "space": o.space, "genesis": o.genesis, "offeredAs": o.address }));
            continue;
        }
        cs::follow(&cfg.paths, Follow { space: o.space.clone(), curator: offer.curator.clone(), endpoint: hex(invite.inviter.0), addrs: addrs.clone(), all: choice.all })?;
        let address = chains::receive(&cfg.paths, &cfg.root, &chain, &cfg.keys.context)?;
        followed.push(json!({ "space": o.space, "genesis": o.genesis, "offeredAs": o.address, "address": address.to_string() }));
    }
    Ok(json!({ "owner": offer.owner, "curator": offer.curator, "level": answer["level"], "followed": followed, "skipped": skipped }))
}

/// The inviter's endpoint id as an invite names it, for the person.
pub fn inviter_of(invite: &Invite) -> String {
    hex(invite.inviter.0)
}

/// An endpoint id from its hex, for a follow record.
pub fn peer_of(endpoint: &str) -> Option<PeerId> {
    unhex(endpoint).map(PeerId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_26_characters_of_130_bits_and_forgives_how_it_was_typed() {
        let c = make_code().unwrap();
        assert_eq!(c.len(), CODE_LEN);
        const { assert!(CODE_LEN * 5 >= 128, "never weaker than 1.x's pair code") };
        assert_eq!(normalize(&display(&c)).as_deref(), Some(c.as_str()));
        assert_eq!(normalize(&display(&c).to_lowercase()).as_deref(), Some(c.as_str()));
        assert_eq!(normalize("KX7Q-49MZ-T2"), None, "a join/2 code is too short");
        assert_eq!(normalize(&format!("{c}U")), None);
        assert_ne!(make_code().unwrap(), c);
    }

    #[test]
    fn an_offer_is_named_by_a_hash_of_its_owner_curator_and_spaces_in_any_order() {
        let s = |n: char| Offered { space: format!("spc_{}", n.to_string().repeat(32)), genesis: format!("g{n}"), address: format!("project:{n}") };
        let a = Offer { owner: "key:xns1_o".into(), curator: "xns1_c".into(), spaces: vec![s('a'), s('b')] };
        let b = Offer { spaces: vec![s('b'), s('a')], ..a.clone() };
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), Offer { owner: "key:xns1_x".into(), ..a.clone() }.digest());
        let mut g = a.clone();
        g.spaces[0].genesis = "other".into();
        assert_ne!(a.digest(), g.digest(), "another genesis is another offer");
        let mut addr = a.clone();
        addr.spaces[0].address = "project:renamed".into();
        assert_eq!(a.digest(), addr.digest(), "the address is for the person only");
        assert_eq!((TARGET.target_bytes)(&a.digest()).map(|b| b.len()), Some(32));
    }
}
