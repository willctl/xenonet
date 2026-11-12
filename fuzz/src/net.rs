//! What the network's targets share (docs/design/network.md): a made-up
//! network whose hosts have real keys from fixed seeds, so an input can be
//! a well-formed envelope, feed entry or roster entry, or a change to one;
//! and a reader that takes the input's first bytes as choices.
//!
//! studio founded the network; desk is a window host;
//! laptop isn't; gone was in it and was removed; spare never joined. The
//! network's owner is one person, from fixed seeds, whose devices are
//! studio's and desk's: their presence blocks (fixed nonces) are what the
//! roster's admits carry.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer, VerifyingKey};
use xenonet_log::feed::{Class, FeedEntry, Kind as FeedKind, Link, Pos};
use xenonet_log::person::{self, DeviceKind, NewDevice, PersonChain};
use xenonet_net::codec::b64;
use xenonet_net::keys::{HostKey, PublicKeys};
use xenonet_net::roster::{found, HostRef, NetworkPolicy, Op, Roster};

pub const NET: &str = "net_0123456789abcdef0123456789abcdef";
/// When the network was made. The machines' clock starts a day later.
pub const T0: i64 = 1_792_512_000_000;
pub const HOSTS: [&str; 5] = ["studio", "desk", "laptop", "gone", "spare"];
pub const STUDIO: usize = 0;
pub const DESK: usize = 1;
pub const LAPTOP: usize = 2;
pub const GONE: usize = 3;
pub const SPARE: usize = 4;

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn secrets(i: usize) -> ([u8; 32], [u8; 32]) {
    (secret(2 * i as u8 + 1), secret(2 * i as u8 + 2))
}

/// Host `i`'s keys.
pub fn key(i: usize) -> &'static HostKey {
    static KEYS: OnceLock<Vec<HostKey>> = OnceLock::new();
    &KEYS.get_or_init(|| {
        (0..HOSTS.len())
            .map(|i| {
                let (sign, seal) = secrets(i);
                HostKey::from_secrets(&sign, seal).expect("a key from a fixed seed")
            })
            .collect()
    })[i]
}

pub fn public(i: usize) -> PublicKeys {
    *key(i).public()
}

pub fn index_of(host: &str) -> Option<usize> {
    HOSTS.iter().position(|h| *h == host)
}

/// Host `i`'s device key (studio's and desk's are the person's).
pub fn device(i: usize) -> &'static SeedSigner {
    static DEVICES: OnceLock<Vec<SeedSigner>> = OnceLock::new();
    &DEVICES.get_or_init(|| (0..HOSTS.len()).map(|i| SeedSigner::from_seed(&secret(0x30 + i as u8), KeyHome::File).expect("a device key")).collect())[i]
}

/// Host `i`'s machine signing key, which signs its feed.
pub fn machine_key(i: usize) -> SeedSigner {
    SeedSigner::from_seed(&secret(0x50 + i as u8), KeyHome::File).expect("a machine key")
}

pub fn machine(i: usize) -> VerifyingKey {
    machine_key(i).public()
}

/// The kinds an entry made here may be: every kind but a compact entry,
/// whose body the feed's writer makes.
pub const KINDS: [FeedKind; 7] = [FeedKind::Claim, FeedKind::Intent, FeedKind::Handoff, FeedKind::Assign, FeedKind::Report, FeedKind::Message, FeedKind::Receipt];

/// Entry `seq` of host `i`'s feed, of `kind`, saying `body`, signed by its
/// machine key: kept entries in one sequence from the network's start,
/// loose ones each its own. A kept entry after the first names a made-up
/// id for the one before it (`entry_after` names a real one).
pub fn entry(i: usize, kind: FeedKind, seq: u64, body: Value) -> FeedEntry {
    entry_after(i, kind, seq, None, body)
}

/// `entry`, naming `prev` as the kept entry before it.
pub fn entry_after(i: usize, kind: FeedKind, seq: u64, prev: Option<Link>, body: Value) -> FeedEntry {
    let kept = kind.class() == Class::Kept;
    let seq = seq.max(1);
    let before = if seq == 1 { Link::ORIGIN } else { Link::to(Pos::new(T0 as u64, seq - 1), [0; 32]) };
    let mut e = FeedEntry {
        author: machine(i).id(),
        group: NET.into(),
        inc: T0 as u64,
        seq,
        prev: kept.then(|| prev.unwrap_or(before)),
        kind,
        at: now() as u64,
        body,
        sig: Vec::new(),
    };
    e.sign(&machine_key(i)).expect("a signature");
    e
}

/// A presence block from host `i`'s device for `what` and `act`, with a
/// fixed nonce.
pub fn presence(i: usize, what: &str, act: [u8; 32], at: i64) -> PresenceBlock {
    let d = device(i);
    let mut b = PresenceBlock { device: d.public().id(), what: what.into(), act, nonce: [0x40 + i as u8; 16], at, sig: Vec::new() };
    b.sig = d.sign(Purpose::PRESENCE, &b.signed_bytes()).expect("a signature");
    b
}

/// The network's owner: a root key, with studio's and desk's devices.
pub fn person() -> PersonChain {
    static PERSON: OnceLock<PersonChain> = OnceLock::new();
    PERSON
        .get_or_init(|| {
            let root = SeedSigner::from_seed(&secret(0x2f), KeyHome::File).expect("a root key");
            let mut c = PersonChain::new();
            c.offer(person::found(&root, T0 - 10).expect("entry 0")).expect("entry 0");
            for i in [STUDIO, DESK] {
                let mut e = c.next(person::Body { op: person::Op::AddDevice(NewDevice { key: device(i).public(), kind: DeviceKind::Weaker, label: HOSTS[i].into() }), at: T0 - 5 });
                e.sign(&root.public().id(), &root, Some(presence(i, person::PAIR, e.digest(), T0 - 5))).expect("a signature");
                c.offer(e).expect("a device");
            }
            c
        })
        .clone()
}

/// The roster's rules, checking presence against the owner's chain.
pub fn policy() -> NetworkPolicy {
    NetworkPolicy::with(Arc::new(person()))
}

/// The entry about host `i`, as the roster names it.
pub fn host_ref(i: usize, window: bool) -> HostRef {
    let device = [STUDIO, DESK].contains(&i).then(|| device(i).public().id());
    HostRef { id: HOSTS[i].into(), keys: public(i), window, device, machine: machine(i) }
}

/// The roster: studio founds, adds desk (a window host), laptop and gone,
/// then removes gone.
pub fn roster() -> Roster {
    static ROSTER: OnceLock<Roster> = OnceLock::new();
    ROSTER
        .get_or_init(|| {
            let p = person();
            let mut r = Roster::with_policy(policy());
            r.offer(found(NET, host_ref(STUDIO, true), &machine_key(STUDIO), p.id().map(str::to_string), p.head(), None, T0).expect("entry 0")).expect("entry 0");
            for (i, window) in [(DESK, true), (LAPTOP, false), (GONE, false)] {
                let mut e = r.draft(Op::Add, host_ref(i, window), None, T0 + r.len() as i64);
                e.person_head = p.head();
                let block = presence(STUDIO, xenonet_net::roster::ADMIT, e.digest(), T0);
                e.sign_with("studio", &machine_key(STUDIO), Some(block)).expect("a signature");
                r.offer(e).expect("an add");
            }
            let mut e = r.draft(Op::Remove, host_ref(GONE, false), None, T0 + r.len() as i64);
            e.person_head = p.head();
            e.sign_by("studio", &machine_key(STUDIO)).expect("a signature");
            r.offer(e).expect("a remove");
            r
        })
        .clone()
}

/// The keys a roster has for a host now, or had when it was removed.
pub fn keys_in(r: &Roster, host: &str) -> Option<PublicKeys> {
    let v = r.view_ref()?;
    v.member(host).map(|m| m.keys).or_else(|| v.removed.get(host).map(|x| x.keys))
}

/// The input's first bytes as choices, the rest as data. Past the end,
/// every choice is 0 and every slice empty.
pub struct Choices<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Choices<'a> {
    pub fn new(data: &'a [u8]) -> Choices<'a> {
        // Each network target starts here: the adapters go in place first,
        // as in the service.
        crate::adapters();
        Choices { data, at: 0 }
    }

    pub fn u8(&mut self) -> u8 {
        let b = self.data.get(self.at).copied().unwrap_or(0);
        self.at = (self.at + 1).min(self.data.len());
        b
    }

    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }

    pub fn i16(&mut self) -> i16 {
        i16::from_le_bytes([self.u8(), self.u8()])
    }

    pub fn bool(&mut self) -> bool {
        self.u8() & 1 == 1
    }

    /// True `n` times in 256, and never once the input is used up, so a
    /// short input takes the usual way.
    pub fn rarely(&mut self, n: u8) -> bool {
        self.u8() > u8::MAX - n
    }

    /// One of `n`, 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            usize::from(self.u8()) % n
        }
    }

    pub fn pick<'b, T>(&mut self, list: &'b [T]) -> &'b T {
        &list[self.below(list.len())]
    }

    /// The next `n` bytes, or what's left.
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        let end = (self.at + n).min(self.data.len());
        let out = &self.data[self.at..end];
        self.at = end;
        out
    }

    /// A slice its first byte says the length of, or, when that's 255, the
    /// two after it: room for a whole roster or request body.
    pub fn chunk(&mut self) -> &'a [u8] {
        let n = match self.u8() {
            255 => usize::from(self.u16()),
            n => usize::from(n),
        };
        self.take(n)
    }

    /// Whether the input is used up.
    pub fn is_empty(&self) -> bool {
        self.at >= self.data.len()
    }

    pub fn rest(&mut self) -> &'a [u8] {
        self.take(self.data.len())
    }

    /// A chunk as JSON, or as a string when it isn't JSON; null when it's
    /// empty, which `overlay` leaves alone.
    pub fn value(&mut self) -> Value {
        let c = self.chunk();
        if c.is_empty() {
            return Value::Null;
        }
        crate::json(c).unwrap_or_else(|| Value::String(crate::text(c)))
    }
}

/// `base` with the fields of `over` put in, when `over` is an object, or
/// `over` itself in place of one field.
pub fn overlay(mut base: Value, over: Value, c: &mut Choices<'_>) -> Value {
    match over {
        Value::Object(o) => {
            for (k, v) in o {
                base[k] = v;
            }
        }
        Value::Null => {}
        v => {
            if let Some(m) = base.as_object_mut() {
                let keys: Vec<String> = m.keys().cloned().collect();
                if !keys.is_empty() {
                    let k = keys[c.below(keys.len())].clone();
                    m.insert(k, v);
                }
            }
        }
    }
    base
}

/// Changes an object's JSON: one field set to `v`, taken out, or one more
/// field, picking among its own fields (an envelope's, say) by `pick`.
pub fn change_json_field(j: &Value, pick: u8, v: Value) -> String {
    let fields: Vec<&str> = j.as_object().map(|m| m.keys().map(String::as_str).collect()).unwrap_or_default();
    if fields.is_empty() {
        return j.to_string();
    }
    change_json(j.clone(), &fields, pick, v)
}

fn change_json(mut j: Value, fields: &[&str], pick: u8, v: Value) -> String {
    let field = fields[usize::from(pick) % fields.len()];
    match pick / 16 % 4 {
        0 => {
            if let Some(m) = j.as_object_mut() {
                m.shift_remove(field);
            }
        }
        1 => j["extra"] = v,
        // A string field's text with the value's text after it.
        2 => {
            let s = j[field].as_str().map(|s| format!("{s}{}", v.as_str().map_or_else(|| v.to_string(), str::to_string)));
            j[field] = s.map_or(v, Value::String);
        }
        _ => j[field] = v,
    }
    j.to_string()
}

/// Changes a line's bytes: `edits` overwrites, inserts and cuts, at places
/// and with bytes the choices give.
pub fn change_bytes(line: &str, c: &mut Choices<'_>) -> Vec<u8> {
    let mut b = line.as_bytes().to_vec();
    for _ in 0..=c.below(4) {
        let at = if b.is_empty() { 0 } else { usize::from(c.u16()) % b.len() };
        let with = c.chunk();
        match c.below(3) {
            0 => {
                for (i, x) in with.iter().enumerate() {
                    if let Some(y) = b.get_mut(at + i) {
                        *y = *x;
                    }
                }
                if with.is_empty() {
                    if let Some(y) = b.get_mut(at) {
                        *y ^= 1;
                    }
                }
            }
            1 => {
                b.splice(at..at, with.iter().copied());
            }
            _ => {
                let end = (at + 1 + with.len()).min(b.len());
                b.drain(at..end);
            }
        }
    }
    b
}

/// The machines' clock, in ms, which a target moves on between runs so
/// nonces, rates and tickets lapse as they would.
pub fn clock() -> &'static Arc<AtomicI64> {
    static CLOCK: OnceLock<Arc<AtomicI64>> = OnceLock::new();
    CLOCK.get_or_init(|| Arc::new(AtomicI64::new(T0 + 86_400_000)))
}

pub fn now() -> i64 {
    clock().load(Ordering::Relaxed)
}

/// A body of `kind`, as its schema has it, with the input's fields over
/// it.
pub fn body_for(kind: &str, c: &mut Choices<'_>) -> Value {
    let now = now();
    let base = match kind {
        "presence" => json!({
            "schema": "network/presence/v1",
            "counter": c.u16(),
            "full": c.bool(),
            "agents": [{ "id": "claude:aaaaaaaa-0001", "app": "claude", "title": "Add partial refunds", "state": "working", "waitsOnYou": c.bool() }],
            "removed": [],
            "you": c.bool(),
            "youAt": now - i64::from(c.i16()) * 1000,
            "roster": roster().head(),
        }),
        "claims" => {
            json!({ "schema": "network/claims/v1", "full": c.bool(), "claims": [{ "repoKey": "github.com/acme/atlas", "path": "src/a.ts", "agent": "claude:aaaaaaaa-0001", "kind": "claim", "until": now + 60_000 }], "released": [] })
        }
        "send" => json!({
            "schema": "network/send/v1",
            "mid": format!("msg_{:08x}", c.u16()),
            "sender": "claude:aaaaaaaa-0001",
            "senderKind": *c.pick(&["agent", "actor", "person"]),
            "target": "claude:bbbbbbbb-0001",
            "body": "hi",
            "kind": "message",
            "hops": c.below(8),
            "at": now,
        }),
        "receipt" => {
            json!({ "schema": "network/receipt/v1", "mid": format!("msg_{:08x}", c.u16()), "agent": "claude:bbbbbbbb-0001", "state": *c.pick(&["stored", "delivered", "seen", "held", "refused"]), "at": now })
        }
        "handoff" => {
            json!({ "schema": "network/handoff/v1", "id": format!("hof_{:08x}", c.u16()), "op": *c.pick(&["offer", "accept", "taken", "lapsed"]), "from": "claude:aaaaaaaa-0001", "to": { "agent": null, "repoKey": "github.com/acme/atlas", "host": null }, "goal": "finish refunds", "at": now })
        }
        "operator" => json!({
            "schema": "network/operator/v2",
            "answer": { "ask": format!("ask_{:04x}{:04x}", c.u16(), fresh_counter()), "session": "claude:bbbbbbbb-0001", "decision": *c.pick(&["allow", "deny", "maybe"]) },
            "authority": { "person": { "person": person().id(), "device": device(STUDIO).public().id(), "sig": b64(&[2u8; 64]) } },
            "at": now + i64::from(c.i16()) * 4,
        }),
        _ => json!({}),
    };
    let over = c.value();
    overlay(base, over, c)
}

fn fresh_counter() -> u16 {
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed) as u16
}
