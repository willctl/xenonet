//! Envelopes: sealed per recipient, the head bound by the seal and checked
//! against the signed entry, live presence only from its author's own
//! connection, and copies refused.

use super::*;
use crate::roster::{Member, Settings};
use serde_json::json;
use std::collections::BTreeMap;
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::feed::{Feed, Kind, MemoryStore, Offer, Writer};

const NET: &str = "net_0123456789abcdef0123456789abcdef";
const T: u64 = 1_792_512_000_000;

/// A machine: its transport key and its machine signing key.
struct Machine {
    id: &'static str,
    transport: HostKey,
    signer: SeedSigner,
}

impl Machine {
    fn new(id: &'static str, n: u8) -> Machine {
        Machine { id, transport: HostKey::from_secrets(&[n; 32], [n + 1; 32]).unwrap(), signer: SeedSigner::from_seed(&[n + 2; 32], KeyHome::File).unwrap() }
    }

    fn key_id(&self) -> String {
        self.signer.public().id()
    }

    fn me(&self) -> (String, &HostKey) {
        (self.key_id(), &self.transport)
    }

    fn member(&self) -> Member {
        Member {
            id: self.id.into(),
            keys: *self.transport.public(),
            window: false,
            device: None,
            machine: self.signer.public(),
            settings: Settings::default(),
            settings_by: None,
            since: 1,
        }
    }
}

struct Net {
    studio: Machine,
    laptop: Machine,
    desk: Machine,
    /// Never in the network.
    stranger: Machine,
}

fn net() -> Net {
    Net { studio: Machine::new("studio", 10), laptop: Machine::new("laptop", 20), desk: Machine::new("desk", 30), stranger: Machine::new("stranger", 40) }
}

impl Net {
    /// The network's chain as a receiver holds it: studio, laptop, desk.
    fn view(&self) -> View {
        let members: BTreeMap<String, Member> = [&self.studio, &self.laptop, &self.desk].into_iter().map(|m| (m.id.to_string(), m.member())).collect();
        View { network: NET.into(), founder: "studio".into(), members, ..View::default() }
    }
}

/// studio's feed: a claim (kept) and a message (loose).
fn entries(by: &Machine) -> (FeedEntry, FeedEntry) {
    let mut w = Writer::start(by.key_id(), NET, T);
    let claim = w.write(&by.signer, Kind::Claim, T + 1, json!({ "project": "prj_atlas", "path": "src/api.rs" })).unwrap();
    let message = w.write(&by.signer, Kind::Message, T + 2, json!({ "to": "laptop", "text": "hi" })).unwrap();
    (claim, message)
}

fn sealed_for(e: &FeedEntry, to: &Machine) -> Envelope {
    Envelope::seal_entry(e, &to.key_id(), to.transport.public(), None).unwrap()
}

fn open_as(env: &Envelope, m: &Machine, view: &View) -> R<Verified> {
    let (id, key) = m.me();
    env.open_entry(&Me { machine: &id, key }, view)
}

#[test]
fn a_sealed_entry_opens_for_its_recipient_through_its_json() {
    let n = net();
    let view = n.view();
    let (claim, message) = entries(&n.studio);
    for e in [&claim, &message] {
        let env = sealed_for(e, &n.laptop);
        assert_eq!(env.class, if e.kind == Kind::Claim { Class::Kept } else { Class::Loose });
        assert!(env.sealing_head().starts_with("xenonet-net/2\n"));
        assert_eq!(env.head().lines().count(), 11);
        let back = Envelope::decode(&env.encode()).unwrap();
        assert_eq!(back, env);
        let v = open_as(&back, &n.laptop, &view).unwrap();
        assert_eq!(v.entry(), e);
    }
    // The receiver's feed takes it.
    let mut f = Feed::new(MemoryStore::new());
    let v = open_as(&sealed_for(&claim, &n.laptop), &n.laptop, &view).unwrap();
    assert_eq!(f.offer(v).unwrap(), Offer::New(vec![claim].into()));
}

#[test]
fn a_tampered_head_or_body_doesnt_open() {
    let n = net();
    let view = n.view();
    let (claim, _) = entries(&n.studio);
    let env = sealed_for(&claim, &n.laptop);
    type Change<'a> = Box<dyn Fn(&mut Envelope) + 'a>;
    let changes: Vec<(&str, Change<'_>)> = vec![
        ("id", Box::new(|e| e.id = hash_b64(b"another"))),
        ("kind", Box::new(|e| e.kind = "intent".into())),
        ("author", Box::new(|e| e.author = n.desk.key_id())),
        ("pos", Box::new(|e| e.pos = Some(Pos::new(T, 2)))),
        ("at", Box::new(|e| e.at += 1)),
        ("body", Box::new(|e| *e.body.last_mut().unwrap() ^= 1)),
    ];
    for (what, change) in &changes {
        let mut g = env.clone();
        change(&mut g);
        assert_ne!(g.head(), env.head(), "{what}");
        let back = Envelope::decode(&g.encode()).unwrap();
        assert_eq!(open_as(&back, &n.laptop, &view).unwrap_err().code, "unsealed", "{what}: the seal binds it");
    }
    // A class or seal that doesn't fit the kind doesn't even read.
    for v in [json!({ "class": "loose" }), json!({ "class": "live" }), json!({ "sealed": false }), json!({ "to": "*" }), json!({ "pos": null })] {
        let mut j = env.to_json();
        for (k, x) in v.as_object().unwrap() {
            j[k] = x.clone();
        }
        assert!(Envelope::parse(&j).is_err(), "{v}");
    }
    // Another network: no member of this one's chain.
    let mut g = env.clone();
    g.network = "net_ffffffffffffffffffffffffffffffff".into();
    assert_eq!(open_as(&g, &n.laptop, &view).unwrap_err().code, "not-member");
}

/// An envelope whose head is sealed rightly but says other than its entry:
/// what a lying sender, mailbox or holder could make, since the head has
/// no signature of its own.
fn lying(e: &FeedEntry, to: &Machine, change: impl Fn(&mut Envelope)) -> Envelope {
    let mut env = sealed_for(e, to);
    change(&mut env);
    env.body = seal_with([9; 32], to.transport.public(), &env.sealing_head(), &e.encode()).unwrap();
    env
}

#[test]
fn a_head_that_disagrees_with_the_entry_inside_is_dropped() {
    let n = net();
    let view = n.view();
    let (claim, _) = entries(&n.studio);
    let (desk_claim, _) = entries(&n.desk);
    let cases: Vec<(&str, Envelope)> = vec![
        ("id", lying(&claim, &n.laptop, |e| e.id = desk_claim.id())),
        ("kind", lying(&claim, &n.laptop, |e| e.kind = "intent".into())),
        ("author", lying(&claim, &n.laptop, |e| e.author = n.desk.key_id())),
        ("pos", lying(&claim, &n.laptop, |e| e.pos = Some(Pos::new(T, 7)))),
        ("time", lying(&claim, &n.laptop, |e| e.at = T + 99)),
    ];
    for (what, env) in cases {
        let err = open_as(&env, &n.laptop, &view).unwrap_err();
        assert_eq!((err.code, err.message.contains(what)), ("envelope-mismatch", true), "{what}: {}", err.message);
    }
    // Head and entry agree, but the entry isn't the author's: signed by a
    // key that isn't studio's machine key.
    let mut forged = claim.clone();
    forged.sign(&n.stranger.signer).unwrap_err();
    forged.sig = n.stranger.signer.sign(xenonet_id::sign::Purpose::FEED, b"x").unwrap();
    let env = lying(&forged, &n.laptop, |e| e.id = forged.id());
    assert_eq!(open_as(&env, &n.laptop, &view).unwrap_err().code, "refused");
    // An author the chain doesn't have.
    let (stranger_claim, _) = entries(&n.stranger);
    assert_eq!(open_as(&sealed_for(&stranger_claim, &n.laptop), &n.laptop, &view).unwrap_err().code, "not-member");
}

#[test]
fn an_envelope_for_another_machine_is_refused() {
    let n = net();
    let view = n.view();
    let (claim, _) = entries(&n.studio);
    let env = sealed_for(&claim, &n.laptop);
    assert_eq!(open_as(&env, &n.desk, &view).unwrap_err().code, "wrong-recipient");
    // Named for desk but sealed to laptop's key: desk can't open it.
    let mut g = env.clone();
    g.to = To::Machine(n.desk.key_id());
    assert_eq!(open_as(&g, &n.desk, &view).unwrap_err().code, "unsealed");
    // Named for laptop, opened with desk's key in laptop's name.
    let (id, _) = n.laptop.me();
    assert_eq!(env.open_entry(&Me { machine: &id, key: &n.desk.transport }, &view).unwrap_err().code, "unsealed");
}

#[test]
fn a_replayed_envelope_is_refused() {
    let n = net();
    let view = n.view();
    let (claim, message) = entries(&n.studio);
    let mut seen = Seen::default();
    for e in [&claim, &message] {
        let env = sealed_for(e, &n.laptop);
        open_as(&env, &n.laptop, &view).unwrap();
        seen.take(&env).unwrap();
        let again = Envelope::decode(&env.encode()).unwrap();
        assert_eq!(seen.take(&again).unwrap_err().code, "replayed");
        // Sealed again (a resend, or a holder re-sealing it): the same
        // entry, so the same id, refused too.
        assert_eq!(seen.take(&sealed_for(e, &n.laptop)).unwrap_err().code, "replayed");
    }
    // Past the window, the feed still takes an entry once.
    let mut small = Seen::new(1);
    let (a, b) = (sealed_for(&claim, &n.laptop), sealed_for(&message, &n.laptop));
    small.take(&a).unwrap();
    small.take(&b).unwrap();
    small.take(&a).unwrap();
    let mut f = Feed::new(MemoryStore::new());
    f.offer(open_as(&a, &n.laptop, &view).unwrap()).unwrap();
    assert_eq!(f.offer(open_as(&a, &n.laptop, &view).unwrap()).unwrap(), Offer::Duplicate);
}

#[test]
fn live_presence_is_unsealed_and_taken_only_from_its_authors_connection() {
    let n = net();
    let view = n.view();
    let body = json!({ "op": "delta", "sessions": [] });
    let env = Envelope::live(NET, &n.studio.key_id(), "presence", T, &body).unwrap();
    assert!(!env.sealed && env.to == To::All && env.pos.is_none());
    let back = Envelope::decode(&env.encode()).unwrap();
    assert_eq!(back.open_live(&n.studio.transport.public().sign, &view).unwrap(), body);
    // From another member's connection: forwarded, or forged.
    assert_eq!(back.open_live(&n.desk.transport.public().sign, &view).unwrap_err().code, "not-forwarded");
    // From someone outside the chain.
    let stranger = Envelope::live(NET, &n.stranger.key_id(), "presence", T, &body).unwrap();
    assert_eq!(stranger.open_live(&n.stranger.transport.public().sign, &view).unwrap_err().code, "not-member");
    // A changed body no longer matches its id.
    let mut j = env.to_json();
    j["body"] = json!(b64(br#"{"op":"delta","sessions":[1]}"#));
    assert!(Envelope::parse(&j).is_err());
    // Only presence travels live, and a live envelope carries no entry.
    assert!(Envelope::live(NET, &n.studio.key_id(), "message", T, &body).is_err());
    let (id, key) = n.laptop.me();
    assert!(env.open_entry(&Me { machine: &id, key }, &view).is_err());
    let (claim, _) = entries(&n.studio);
    assert!(sealed_for(&claim, &n.laptop).open_live(&n.studio.transport.public().sign, &view).is_err());
}

#[test]
fn envelopes_are_checked_for_their_shape() {
    let n = net();
    let (claim, _) = entries(&n.studio);
    let good = sealed_for(&claim, &n.laptop).to_json();
    assert!(Envelope::parse(&good).is_ok());
    for (k, v) in [
        ("v", json!(1)),
        ("network", json!("net_x")),
        ("id", json!("short")),
        ("kind", json!("hack")),
        ("author", json!("xnk1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
        ("to", json!("laptop")),
        ("pos", json!({ "inc": 0, "seq": 1 })),
        ("pos", json!({ "inc": 1 })),
        ("at", json!(-1)),
        ("class", json!("held")),
        ("sealed", json!("yes")),
        ("body", json!("a+b")),
    ] {
        let mut bad = good.clone();
        bad[k] = v.clone();
        assert!(Envelope::parse(&bad).is_err(), "{k}: {v}");
    }
    assert!(Envelope::decode(&vec![b' '; MAX_ENVELOPE + 1]).is_err());
    assert!(Envelope::seal_entry(&claim, "*", n.laptop.transport.public(), None).is_err(), "sealed for everyone");
}

#[test]
fn a_spaces_entry_goes_unsealed_with_its_text_beside_it_and_a_networks_never_does() {
    const SPACE: &str = "spc_abcdefghijklmnopqrstuvwxyz234567";
    let n = net();
    let mut w = Writer::start(n.studio.key_id(), SPACE, T);
    let page = w.write(&n.studio.signer, Kind::CtxPage, T + 1, json!({ "slug": "db" })).unwrap();
    let env = Envelope::plain_entry(&page, &n.laptop.key_id(), Some(b"# db")).unwrap();
    assert!(!env.sealed);
    let back = Envelope::decode(&env.encode()).unwrap();
    assert_eq!(back, env, "through its JSON");
    let (v, text) = back.open_plain_by(&n.laptop.key_id(), &n.studio.signer.public()).unwrap();
    assert_eq!((v.entry(), text.as_deref()), (&page, Some(&b"# db"[..])));
    assert!(back.open_plain_by(&n.studio.key_id(), &n.studio.signer.public()).is_err(), "for another machine");
    assert!(back.open_plain_by(&n.laptop.key_id(), &n.laptop.signer.public()).is_err(), "not its author's key");
    let mut lying = back.clone();
    lying.at += 1;
    assert!(lying.open_plain_by(&n.laptop.key_id(), &n.studio.signer.public()).is_err(), "a head that disagrees");
    // Without a text, as a holder serves an entry whose text was forgotten.
    let bare = Envelope::plain_entry(&page, &n.laptop.key_id(), None).unwrap();
    assert!(!bare.to_json().as_object().unwrap().contains_key("attach"));
    assert_eq!(bare.id, env.id, "the same entry");
    // A network's entries are never plain, and a sealed one carries no text.
    let (claim, _) = entries(&n.studio);
    assert!(Envelope::plain_entry(&claim, &n.laptop.key_id(), None).is_err());
    let mut v = sealed_for(&claim, &n.laptop).to_json();
    v["sealed"] = json!(false);
    assert!(Envelope::parse(&v).is_err(), "a network's envelope unsealed");
    let mut v = sealed_for(&claim, &n.laptop).to_json();
    v["attach"] = json!("YQ");
    assert!(Envelope::parse(&v).is_err(), "a sealed envelope with a text beside it");
    assert!(sealed_for(&claim, &n.laptop).open_plain_by(&n.laptop.key_id(), &n.studio.signer.public()).is_err());
}

#[test]
fn an_acts_authority_opens_only_for_its_recipient_beside_its_own_entry() {
    let n = net();
    let view = n.view();
    let (claim, message) = entries(&n.studio);
    let authority = json!({ "presence": { "what": "answer", "sig": "c2ln" } });
    let env = Envelope::seal_act(&message, &n.laptop.key_id(), n.laptop.transport.public(), None, Some(&authority)).unwrap();
    // It travels in the envelope's JSON, sealed: a mailbox reads its head
    // and keeps it whole, but sees nothing of what the act rests on.
    let back = Envelope::decode(&env.encode()).unwrap();
    assert_eq!(back, env);
    assert!(!String::from_utf8_lossy(&env.encode()).contains("answer"));
    let (id, key) = n.laptop.me();
    let me = Me { machine: &id, key };
    open_as(&back, &n.laptop, &view).unwrap();
    assert_eq!(back.open_authority(&me).unwrap(), authority);
    // None reads as an empty object, and carries no field.
    let bare = sealed_for(&message, &n.laptop);
    assert_eq!(bare.open_authority(&me).unwrap(), json!({}));
    assert!(bare.to_json().get("authority").is_none());
    // Moved onto another envelope for the same recipient, it doesn't open:
    // the seal binds it to its own head.
    let mut stolen = sealed_for(&entries(&n.desk).1, &n.laptop);
    stolen.authority = env.authority.clone();
    assert_eq!(stolen.open_authority(&me).unwrap_err().code, "unsealed", "another entry's authority");
    // Another machine can't open it, and only a message carries one.
    let (did, dkey) = n.desk.me();
    assert_eq!(env.open_authority(&Me { machine: &did, key: dkey }).unwrap_err().code, "wrong-recipient");
    assert!(Envelope::seal_act(&claim, &n.laptop.key_id(), n.laptop.transport.public(), None, Some(&authority)).is_err(), "a kept entry rests on no authority");
    let mut j = sealed_for(&claim, &n.laptop).to_json();
    j["authority"] = env.to_json()["authority"].clone();
    assert!(Envelope::parse(&j).is_err(), "on a kept entry");
    let mut j = bare.to_json();
    j["authority"] = json!("!!");
    assert!(Envelope::parse(&j).is_err(), "not base64url");
    // An empty authority is none.
    let empty = Envelope::seal_act(&message, &n.laptop.key_id(), n.laptop.transport.public(), None, Some(&json!({}))).unwrap();
    assert!(empty.authority.is_none());
}
