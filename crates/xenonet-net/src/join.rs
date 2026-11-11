//! Joining a network over `xenonet/join/2` (docs/design/network.md; phase
//! 5's join): an invite, a code told apart from it, and proofs both ways.
//!
//! **The invite** (`xni2_…`) is not secret: it gets pasted into chats. It
//! names the network, the inviter's endpoint id, at most one relay URL
//! (one the network's chain lists), and a random 128-bit ticket id. It
//! carries no direct address, since IPs go stale and say where a machine
//! is: mDNS on the LAN, the relay, or an address the person gives finds
//! the inviter.
//!
//! **The code** is the secret, told to the joiner some other way: today's
//! ten characters of Crockford's base32 (`ticket`), 50 bits, which work
//! once, for ten minutes. The inviter keeps only its hash.
//!
//! **The proofs.** On one stream, the joiner sends the ticket id, the `add`
//! entry it asks for (its transport keys, its machine signing key, its
//! device), and an HMAC keyed by the code's hash over the ticket id, the
//! network id, both endpoint ids and the hash of that `add`. QUIC has
//! authenticated both endpoint ids already, so the proof can't be moved to
//! another connection or another `add`. Only once that proof checks does
//! the inviter send its own, over the same fields with its own label, and
//! the chain. An inviter that answered first would hand anyone with the
//! invite something to try all 2^50 codes against offline; this way a
//! guess costs a connection, and five wrong ones retire the ticket.
//!
//! An attempt names its ticket id, so only someone holding the invite can
//! burn a ticket, and the console says when one was. join/2 is answered
//! only while a ticket is open (`Mesh::open_ticket`), and dialled only to
//! the endpoint id an invite names (`Mesh::begin_join`).
//!
//! **After joining**, the joiner checks the chain it was sent from its
//! genesis (`verify`): every entry from the `found` one, the network the
//! invite named, the inviter in it, and itself added with the keys it
//! asked for.

use crate::codec::{b64, sha256, unb64, unb64_n};
use crate::dial::{Alpn, Fut, Link, Mesh, PeerId, Reader, Writer};
use crate::keys::PublicKeys;
use crate::roster::{NetworkPolicy, Op, Roster};
use crate::{ticket, NetError, R};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenonet_id::sign::VerifyingKey;
use xenonet_os::sync::Locked;

/// What an invite starts with: `xni` and the join protocol's version.
pub const INVITE_PREFIX: &str = "xni2_";
/// Wrong codes aimed at a ticket before it is retired.
pub const MAX_WRONG: u32 = 5;
/// How long either side waits for the other's record.
pub const STEP: Duration = Duration::from_secs(30);
const SCHEMA: &str = "network/join/v2";
const ASK_MAX: usize = 64 << 10;
// The chain and the owner's person chain.
const ANSWER_MAX: usize = 8 << 20;

/// A ticket's id: 128 random bits, which the invite carries and every
/// attempt names.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TicketId(pub [u8; 16]);

impl TicketId {
    pub fn new() -> R<TicketId> {
        Ok(TicketId(crate::codec::random::<16>()?))
    }

    pub fn parse(text: &str) -> Option<TicketId> {
        unb64_n::<16>(text).map(TicketId)
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&b64(&self.0))
    }
}

impl fmt::Debug for TicketId {
    // Only its start, as the log names it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TicketId({}…)", &b64(&self.0)[..6])
    }
}

/// An invite to a network, as `xenonet net invite` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub network: String,
    pub inviter: PeerId,
    pub relay: Option<String>,
    pub ticket: TicketId,
}

fn hex_bytes(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

impl Invite {
    /// `xni2_` and, in base64url: the ticket id (16 bytes), the inviter's
    /// endpoint id (32), the network id's 16 bytes, and the relay URL's
    /// bytes, if there is one.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(64 + self.relay.as_ref().map_or(0, String::len));
        bytes.extend_from_slice(&self.ticket.0);
        bytes.extend_from_slice(&self.inviter.0);
        bytes.extend_from_slice(&hex_bytes(self.network.strip_prefix("net_").unwrap_or("")).unwrap_or_default());
        if let Some(r) = &self.relay {
            bytes.extend_from_slice(r.as_bytes());
        }
        format!("{INVITE_PREFIX}{}", b64(&bytes))
    }

    /// An invite as it was pasted: surrounding spaces forgiven.
    pub fn parse(text: &str) -> R<Invite> {
        let bad = || NetError::bad("that isn't an invite: it starts with xni2_, as xenonet net invite shows it");
        let bytes = text.trim().strip_prefix(INVITE_PREFIX).and_then(unb64).ok_or_else(bad)?;
        if bytes.len() < 64 || bytes.len() > 64 + 300 {
            return Err(bad());
        }
        let mut ticket = [0u8; 16];
        ticket.copy_from_slice(&bytes[..16]);
        let mut inviter = [0u8; 32];
        inviter.copy_from_slice(&bytes[16..48]);
        let network = format!("net_{}", xenonet_id::codec::hex(&bytes[48..64]));
        let relay = match &bytes[64..] {
            [] => None,
            r => Some(String::from_utf8(r.to_vec()).ok().filter(|u| crate::roster::is_relay_url(u)).ok_or_else(bad)?),
        };
        Ok(Invite { network, inviter: PeerId(inviter), relay, ticket: TicketId(ticket) })
    }
}

/// The fields both proofs cover.
pub struct Bound<'a> {
    pub ticket: &'a TicketId,
    pub network: &'a str,
    pub inviter: &'a PeerId,
    pub joiner: &'a PeerId,
    /// The SHA-256 of the `add` the joiner asks for, as it sent it.
    pub add: &'a [u8; 32],
}

impl Bound<'_> {
    fn text(&self, role: &str) -> String {
        format!("xenonet-join/2\n{role}\n{}\n{}\n{}\n{}\n{}", self.ticket, self.network, b64(&self.inviter.0), b64(&self.joiner.0), b64(self.add))
    }
}

/// The joiner's proof that it holds the code: keyed by the code's hash.
pub fn joiner_proof(hash: &[u8; 32], b: &Bound<'_>) -> String {
    ticket::mac(hash, &b.text("joiner"))
}

pub fn check_joiner(hash: &[u8; 32], b: &Bound<'_>, proof: &str) -> bool {
    ticket::check(hash, &b.text("joiner"), proof)
}

/// The inviter's proof that it knew the code too, sent only after the
/// joiner's checked.
pub fn inviter_proof(hash: &[u8; 32], b: &Bound<'_>) -> String {
    ticket::mac(hash, &b.text("inviter"))
}

pub fn check_inviter(hash: &[u8; 32], b: &Bound<'_>, proof: &str) -> bool {
    ticket::check(hash, &b.text("inviter"), proof)
}

/// An open ticket, at the inviter.
#[derive(Clone, Debug)]
pub struct Ticket {
    pub id: TicketId,
    pub network: String,
    /// The machine's id in the network, as the inviter named it.
    pub host: String,
    pub window: bool,
    pub expires: i64,
    /// Wrong codes aimed at it so far.
    pub wrong: u32,
    hash: [u8; 32],
}

/// A ticket that closed without being used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Burned {
    pub host: String,
    pub at: i64,
    /// Retired after five wrong codes; false when it ran out of time.
    pub retired: bool,
}

/// What became of one join/2 attempt, at the inviter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Served {
    /// The machine is in the network: the ticket is used.
    Joined { host: String },
    /// The code was right, and the add waits for an approval.
    Waiting { host: String },
    /// Refused before any proof was sent back, with the code the joiner
    /// was told.
    Refused(&'static str),
}

/// What the inviter's network says to a joiner whose code checked.
pub enum Admitted {
    /// The answer: the chain, the owner's person chain, the network.
    Joined(Value),
    /// The add waits (for a second window host's approval), and why.
    Waiting(String),
}

/// Signs a joiner in once its code checked: the inviter's node.
pub trait Admit: Send + Sync {
    /// `add` is what the joiner asked to be added as; `joiner` its endpoint
    /// id, as QUIC authenticated it.
    fn admit<'a>(&'a self, ticket: &'a Ticket, joiner: PeerId, add: &'a Value) -> Fut<'a, R<Admitted>>;
}

#[derive(Default)]
struct Book {
    open: BTreeMap<TicketId, Ticket>,
    burned: Vec<Burned>,
}

/// The inviter's tickets, kept in step with the `Mesh`: join/2 is offered
/// while any is open.
pub struct Door {
    mesh: Arc<Mesh>,
    book: Mutex<Book>,
}

impl Door {
    pub fn new(mesh: Arc<Mesh>) -> Door {
        Door { mesh, book: Mutex::new(Book::default()) }
    }

    pub fn mesh(&self) -> &Arc<Mesh> {
        &self.mesh
    }

    /// A ticket for `host`, with `code`, until `expires`. One for the same
    /// host before it closes: a machine is invited by one ticket at a time.
    pub fn open(&self, network: &str, host: &str, window: bool, code: &str, expires: i64) -> R<TicketId> {
        let id = TicketId::new()?;
        let t = Ticket { id, network: network.into(), host: host.into(), window, expires, wrong: 0, hash: ticket::hash(code) };
        let replaced = {
            let mut b = self.book.locked();
            let before = b.open.len();
            b.open.retain(|_, t| t.host != host);
            let replaced = before - b.open.len();
            b.open.insert(id, t);
            replaced
        };
        self.mesh.open_ticket();
        for _ in 0..replaced {
            self.mesh.close_ticket();
        }
        Ok(id)
    }

    /// The open ticket `id`, unless it's past its time.
    pub fn get(&self, id: &TicketId, now: i64) -> Option<Ticket> {
        self.book.locked().open.get(id).filter(|t| t.expires > now).cloned()
    }

    /// Closes `id`: it was used. Whether it was open.
    pub fn close(&self, id: &TicketId) -> bool {
        let gone = self.book.locked().open.remove(id).is_some();
        if gone {
            self.mesh.close_ticket();
        }
        gone
    }

    /// A wrong code aimed at `id`: how many it has had, and whether that
    /// retired it.
    fn wrong(&self, id: &TicketId, now: i64) -> (u32, bool) {
        let mut b = self.book.locked();
        // Retired or used meanwhile, by another attempt.
        let Some(t) = b.open.get_mut(id) else { return (MAX_WRONG, true) };
        t.wrong += 1;
        let tries = t.wrong;
        if tries < MAX_WRONG {
            return (tries, false);
        }
        let host = t.host.clone();
        b.open.remove(id);
        b.burned.push(Burned { host, at: now, retired: true });
        drop(b);
        self.mesh.close_ticket();
        (tries, true)
    }

    /// Closes the tickets past their time, and returns them.
    pub fn expire(&self, now: i64) -> Vec<Ticket> {
        let gone: Vec<Ticket> = {
            let mut b = self.book.locked();
            let gone: Vec<Ticket> = b.open.values().filter(|t| t.expires <= now).cloned().collect();
            for t in &gone {
                b.open.remove(&t.id);
                b.burned.push(Burned { host: t.host.clone(), at: now, retired: false });
            }
            gone
        };
        for _ in &gone {
            self.mesh.close_ticket();
        }
        gone
    }

    /// When the next open ticket runs out.
    pub fn next_expiry(&self) -> Option<i64> {
        self.book.locked().open.values().map(|t| t.expires).min()
    }

    pub fn is_empty(&self) -> bool {
        self.book.locked().open.is_empty()
    }

    /// The open tickets, oldest to expire first.
    pub fn open_tickets(&self) -> Vec<Ticket> {
        let mut v: Vec<Ticket> = self.book.locked().open.values().cloned().collect();
        v.sort_by_key(|t| t.expires);
        v
    }

    /// The tickets that closed unused lately, newest last.
    pub fn burned(&self) -> Vec<Burned> {
        self.book.locked().burned.clone()
    }

    /// Closes every ticket.
    pub fn close_all(&self) {
        let n = std::mem::take(&mut self.book.locked().open).len();
        for _ in 0..n {
            self.mesh.close_ticket();
        }
    }

    /// Answers one join/2 connection: one attempt, on one stream.
    pub async fn serve(&self, link: &Link, admit: &dyn Admit, now: i64) -> R<Served> {
        let (mut w, mut r) = within(link.accept_bi()).await?;
        let ask = within(read(&mut r, ASK_MAX)).await?;
        let served = self.answer(link, &ask, admit, now, &mut w).await;
        let _ = w.shutdown().await;
        // Up to the joiner's end of the stream, which follows its ask: a
        // stream dropped with that still to come may be reset, and the
        // reset can reach the joiner ahead of the answer.
        let mut rest = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), r.read_to_end(&mut rest)).await;
        served
    }

    async fn answer(&self, link: &Link, ask: &[u8], admit: &dyn Admit, now: i64, w: &mut Writer) -> R<Served> {
        let ask: Value = serde_json::from_slice(ask).unwrap_or(Value::Null);
        let s = |k: &str| ask.get(k).and_then(Value::as_str).unwrap_or("");
        // Nothing is counted against a ticket the attempt doesn't name: only
        // someone holding the invite can burn one.
        let Some(id) = TicketId::parse(s("ticket")) else {
            return refuse(w, "no-ticket", "a join names its invite's ticket", Value::Null).await;
        };
        let Some(t) = self.get(&id, now) else {
            return refuse(w, "no-ticket", "that invite was used or retired, or it's past its ten minutes: ask for a new one", Value::Null).await;
        };
        let add_text = s("add");
        let add_hash = sha256(add_text.as_bytes());
        let me = self.mesh.me();
        let joiner = link.peer();
        let bound = Bound { ticket: &id, network: &t.network, inviter: &me, joiner: &joiner, add: &add_hash };
        if !check_joiner(&t.hash, &bound, s("proof")) {
            let (tries, retired) = self.wrong(&id, now);
            if retired {
                return refuse(w, "retired", "that code was wrong five times, so the invite was retired: ask for a new one", Value::Null).await;
            }
            let left = MAX_WRONG - tries;
            return refuse(w, "wrong-code", &format!("that code doesn't match the invite: {left} tries left"), json!({ "left": left })).await;
        }
        let proof = inviter_proof(&t.hash, &bound);
        let add: Value = match serde_json::from_str(add_text) {
            Ok(v @ Value::Object(_)) => v,
            _ => return refuse(w, "bad-ask", "the machine joining asked for no add entry", Value::Null).await,
        };
        match admit.admit(&t, joiner, &add).await {
            Ok(Admitted::Joined(answer)) => {
                write(w, &json!({ "schema": SCHEMA, "proof": proof, "answer": answer })).await?;
                self.close(&id);
                Ok(Served::Joined { host: t.host })
            }
            Ok(Admitted::Waiting(why)) => {
                write(w, &json!({ "schema": SCHEMA, "proof": proof, "waiting": why })).await?;
                Ok(Served::Waiting { host: t.host })
            }
            Err(e) => refuse(w, e.code, &e.message, json!({ "status": e.status })).await,
        }
    }
}

// A refusal, with no proof of the inviter's in it.
async fn refuse(w: &mut Writer, code: &'static str, words: &str, extra: Value) -> R<Served> {
    let mut v = json!({ "schema": SCHEMA, "refused": code, "error": words });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    write(w, &v).await?;
    Ok(Served::Refused(code))
}

/// What the inviter answered a join whose code checked.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Joined(Value),
    Waiting(String),
}

// An inviter's refusal as this machine says it: the codes are join/2's.
fn refusal(v: &Value) -> NetError {
    let words = v.get("error").and_then(Value::as_str).unwrap_or("the inviter refused the join").to_string();
    let status = v.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok());
    let code = v.get("refused").and_then(Value::as_str).unwrap_or("");
    let (s, c) = match code {
        "no-ticket" => (404, "no-ticket"),
        "wrong-code" => (403, "wrong-code"),
        "retired" => (410, "retired"),
        "bad-ask" => (400, "bad-ask"),
        "taken" => (409, "taken"),
        "no-person" => (409, "no-person"),
        _ => (status.unwrap_or(403), "refused"),
    };
    NetError::new(s, c, words)
}

/// Joins through `invite` with `code`, asking to be added as `add` (the
/// exact text sent, which the proofs cover). The dial goes to the endpoint
/// id the invite names alone.
pub async fn ask(mesh: &Mesh, invite: &Invite, code: &str, add: &str) -> R<Answer> {
    let code = ticket::normalize(code).ok_or_else(|| NetError::bad("a code is 10 letters and digits, as KX7Q-49MZ-T2"))?;
    mesh.begin_join(invite.inviter);
    let out = ask_once(mesh, invite, &code, add).await;
    mesh.end_join(&invite.inviter);
    out
}

async fn ask_once(mesh: &Mesh, invite: &Invite, code: &str, add: &str) -> R<Answer> {
    let link = mesh.dial(invite.inviter, Alpn::Join).await?;
    let out = async {
        let (mut w, mut r) = within(link.open_bi()).await?;
        let hash = ticket::hash(code);
        let add_hash = sha256(add.as_bytes());
        let me = mesh.me();
        let bound = Bound { ticket: &invite.ticket, network: &invite.network, inviter: &invite.inviter, joiner: &me, add: &add_hash };
        let proof = joiner_proof(&hash, &bound);
        write(&mut w, &json!({ "schema": SCHEMA, "ticket": invite.ticket.to_string(), "network": invite.network, "add": add, "proof": proof })).await?;
        let _ = w.shutdown().await;
        let reply = within(read(&mut r, ANSWER_MAX)).await?;
        let v: Value = serde_json::from_slice(&reply).map_err(|_| bad_answer("the inviter's answer isn't JSON"))?;
        if v.get("refused").is_some() {
            return Err(refusal(&v));
        }
        let ok = v.get("proof").and_then(Value::as_str).is_some_and(|p| check_inviter(&hash, &bound, p));
        if !ok {
            return Err(bad_answer("the inviter couldn't prove it knew the code"));
        }
        if let Some(why) = v.get("waiting").and_then(Value::as_str) {
            return Ok(Answer::Waiting(why.to_string()));
        }
        v.get("answer").filter(|a| a.is_object()).cloned().map(Answer::Joined).ok_or_else(|| bad_answer("the inviter's answer has no chain"))
    }
    .await;
    link.close(crate::dial::Close::Done);
    out
}

fn bad_answer(words: &str) -> NetError {
    NetError::new(502, "bad-answer", words)
}

/// The chain an inviter sent, checked from its genesis: every entry from
/// the `found` one under the network's rules (with the person chains
/// `policy` holds), for the network the invite named, with the inviter in
/// it and this machine added as `host` with the keys it asked for.
pub fn verify(policy: NetworkPolicy, chain: &Value, invite: &Invite, host: &str, keys: &PublicKeys, machine: &VerifyingKey) -> R<Roster> {
    let roster = Roster::from_json_with(policy, chain).map_err(|e| bad_answer(&format!("the network's chain didn't check: {}", e.message)))?;
    let genesis = roster.entries().first().ok_or_else(|| bad_answer("the network's chain is empty"))?;
    if genesis.op != Op::Found || genesis.network != invite.network {
        return Err(bad_answer("the network's chain doesn't start with the founding of the network the invite names"));
    }
    let view = roster.view();
    if view.network != invite.network {
        return Err(bad_answer("the network's chain is another network's"));
    }
    if !view.members.values().any(|m| PeerId(m.keys.sign) == invite.inviter) {
        return Err(bad_answer("the inviter isn't in the network's chain"));
    }
    if !view.member(host).is_some_and(|m| m.keys == *keys && m.machine == *machine) {
        return Err(bad_answer("the network's chain doesn't add this machine with its keys"));
    }
    Ok(roster)
}

async fn within<T>(f: impl std::future::Future<Output = R<T>>) -> R<T> {
    tokio::time::timeout(STEP, f).await.map_err(|_| NetError::new(504, "timeout", "the other machine stopped answering the join"))?
}

async fn write(w: &mut Writer, v: &Value) -> R<()> {
    let bytes = serde_json::to_vec(v).map_err(|e| NetError::internal(e.to_string()))?;
    let len = u32::try_from(bytes.len()).map_err(|_| NetError::internal("a join record is too long"))?;
    let io = |e: std::io::Error| NetError::new(502, "link-closed", format!("the join's stream: {e}"));
    w.write_all(&len.to_be_bytes()).await.map_err(io)?;
    w.write_all(&bytes).await.map_err(io)?;
    w.flush().await.map_err(io)
}

async fn read(r: &mut Reader, max: usize) -> R<Vec<u8>> {
    let io = |e: std::io::Error| NetError::new(502, "link-closed", format!("the join's stream: {e}"));
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await.map_err(io)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(NetError::bad("a join record is too long"));
    }
    let mut out = vec![0u8; len];
    r.read_exact(&mut out).await.map_err(io)?;
    Ok(out)
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;
