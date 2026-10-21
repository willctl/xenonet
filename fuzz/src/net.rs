//! What the network's targets share (docs/design/network.md): a made-up
//! network whose hosts have real keys from fixed seeds, so an input can be
//! a well-formed frame, roster entry or request, or a change to one; a
//! machine in it with its own files; and a reader that takes the input's
//! first bytes as choices.
//!
//! studio founded the network and is its gateway; desk is a window host;
//! laptop isn't; gone was in it and was removed; spare never joined.

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_base::network::{Incoming, Sink};
use xenonet_net::codec::b64;
use xenonet_net::files::{Dir, NetFile, Pin, GATEWAY_DB, KEY};
use xenonet_net::frame::{gateway_frame, host_frame, Draft, Frame};
use xenonet_net::gateway::Gateway;
use xenonet_net::gwstore::{Caps, GwStore};
use xenonet_net::keys::{HostKey, PublicKeys};
use xenonet_net::node::{Node, Options, Shared, Timing};
use xenonet_net::roster::{found, HostRef, Op, Roster};
use xenonet_net::sink::SinkSlot;

pub const NET: &str = "net_0123456789abcdef0123456789abcdef";
/// When the network was made. The machines' clock starts a day later.
pub const T0: i64 = 1_792_512_000_000;
pub const HOSTS: [&str; 5] = ["studio", "desk", "laptop", "gone", "spare"];
pub const STUDIO: usize = 0;
pub const DESK: usize = 1;
pub const LAPTOP: usize = 2;
pub const GONE: usize = 3;
pub const SPARE: usize = 4;
/// The ticket the gateway keeps for spare, when a target adds it.
pub const TICKET: &str = "KX7Q49MZT2";

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

/// The roster: studio founds, adds desk (a window host), laptop and gone,
/// then removes gone.
pub fn roster() -> Roster {
    static ROSTER: OnceLock<Roster> = OnceLock::new();
    ROSTER
        .get_or_init(|| {
            let mut r = Roster::new();
            r.offer(found(NET, "studio", key(STUDIO), None, T0)).expect("entry 0");
            for (i, window) in [(DESK, true), (LAPTOP, false), (GONE, false)] {
                let mut e = r.draft(Op::Add, HostRef { id: HOSTS[i].into(), keys: public(i), window }, None, T0 + r.len() as i64);
                e.sign_by("studio", key(STUDIO));
                r.offer(e).expect("an add");
            }
            let mut e = r.draft(Op::Remove, HostRef { id: "gone".into(), keys: public(GONE), window: false }, None, T0 + r.len() as i64);
            e.sign_by("studio", key(STUDIO));
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

/// A frame id that no earlier run used, so a machine doesn't drop a frame
/// as a repeat of one from before.
pub fn fresh_frame_id(salt: u16) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    format!("frm_{:08x}-{salt:04x}-4000-8000-{:012x}", (n >> 32) as u32, n & 0xffff_ffff_ffff)
}

/// A frame from `from`, signed with `signer`'s key, and for a host frame
/// sealed for `seal_for`: what the network would carry, before anything
/// is changed.
#[expect(clippy::too_many_arguments, reason = "each is a part of the frame a target picks")]
pub fn frame(id: String, kind: &str, from: &str, to: &str, at: i64, body: &Value, signer: usize, seal_for: usize, eph: u8) -> Frame {
    let d = Draft { network: NET, id, kind, from, to, at };
    if xenonet_net::frame::GATEWAY_KINDS.contains(&kind) {
        gateway_frame(&d, key(signer), body)
    } else {
        host_frame(&d, key(signer), key(seal_for).public(), body, Some(secret(eph | 0x80))).expect("sealing with fixed keys")
    }
}

/// Whether two frames say the same thing, whatever their `seq`, which the
/// gateway sets and nothing signs.
pub fn same_frame(a: &Frame, b: &Frame) -> bool {
    a.id == b.id && a.kind == b.kind && a.from == b.from && a.to == b.to && a.at == b.at && a.sealed == b.sealed && a.body == b.body && a.sig == b.sig
}

/// Whether what sealing binds (the head's first lines and the body)
/// differs.
pub fn sealing_differs(a: &Frame, b: &Frame) -> bool {
    a.id != b.id || a.kind != b.kind || a.from != b.from || a.to != b.to || a.at != b.at || a.sealed != b.sealed || a.body != b.body
}

/// Changes a frame's JSON: one field set to `v`, taken out, or one more
/// field. Which, by `pick`.
pub fn change_field(f: &Frame, pick: u8, v: Value) -> String {
    const FIELDS: [&str; 10] = ["v", "id", "seq", "kind", "from", "to", "at", "sealed", "body", "sig"];
    let mut j = f.to_json();
    let field = FIELDS[usize::from(pick) % FIELDS.len()];
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

/// What the machine's network passed on to its engine.
#[derive(Default)]
pub struct Caught {
    pub incoming: Mutex<Vec<Incoming>>,
    pub removed: Mutex<Vec<String>>,
    pub gaps: AtomicU64,
    pub linked: AtomicU64,
}

impl Sink for Caught {
    fn receive(&self, incoming: Incoming) {
        if let Ok(mut l) = self.incoming.lock() {
            l.push(incoming);
        }
    }
    fn linked(&self, _up: bool) {
        self.linked.fetch_add(1, Ordering::Relaxed);
    }
    fn removed(&self, host: &str) {
        if let Ok(mut l) = self.removed.lock() {
            l.push(host.to_string());
        }
    }
    fn gap(&self) {
        self.gaps.fetch_add(1, Ordering::Relaxed);
    }
}

impl Caught {
    pub fn take(&self) -> Vec<Incoming> {
        self.incoming.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default()
    }
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

/// Short waits, so a stream or a join that waits ends within a run.
pub fn timing() -> Timing {
    Timing { ping_ms: 5, stream_ms: 20, idle_ms: 20, poll_ms: 20, backoff_min_ms: 1.0, backoff_max_ms: 5.0, handover_ms: 10 }
}

/// One host of the network on this machine, with its own folder: its
/// node, what it passed on, and, for studio, the gateway.
pub struct Machine {
    pub dir: PathBuf,
    pub node: Arc<Node>,
    pub caught: Arc<Caught>,
    pub gateway: Option<Arc<Gateway>>,
}

impl Machine {
    pub fn new(host: usize) -> Machine {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("xenonet-fuzz-net-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        let files = Dir::new(&dir);
        let (sign, seal) = secrets(host);
        std::fs::create_dir_all(&dir).expect("a folder for the machine");
        let k = json!({ "v": 1, "sign": b64(&sign), "seal": b64(&seal), "keyId": key(host).key_id() });
        std::fs::write(files.file(KEY), format!("{k}\n")).expect("the host key");
        let gw = public(STUDIO);
        let net = NetFile {
            id: NET.into(),
            name: "home".into(),
            host: HOSTS[host].into(),
            founder: "studio".into(),
            gateway: Pin { host: "studio".into(), key_id: gw.key_id(), sign: gw.sign_b64(), seal: gw.seal_b64(), address: Some("gw.example.net".into()) },
            lan: None,
            titles_only: false,
            joined: T0,
        };
        files.write_network(&net).expect("network.json");
        files.write_roster(&roster()).expect("roster.json");
        let caught = Arc::new(Caught::default());
        let mut o = Options::new(&dir, "0.0.0", "linux", Arc::new(|_: &str, _: &str| {}));
        let c = clock().clone();
        o.clock = Arc::new(move || c.load(Ordering::Relaxed) as f64);
        o.timing = timing();
        o.caps = Caps { frames: 64, bytes: 1 << 20, keep_ms: xenonet_net::gwstore::WEEK_MS };
        o.outbox = o.caps;
        let node = Node::open(o, Arc::new(SinkSlot::new(caught.clone()))).expect("the node");
        let gateway =
            (host == STUDIO).then(|| Gateway::new(Arc::downgrade(node.shared()), GwStore::open(&files.file(GATEWAY_DB)).expect("the gateway's store")).expect("the gateway"));
        Machine { dir, node, caught, gateway }
    }

    pub fn shared(&self) -> &Arc<Shared> {
        self.node.shared()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        if let Some(g) = &self.gateway {
            g.close();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A machine kept between runs, made again when a run changed what it
/// keeps (the roster, being removed) or after `every` runs, so its files
/// don't grow.
pub struct Kept {
    host: usize,
    every: u64,
    runs: u64,
    machine: Option<Machine>,
}

impl Kept {
    pub const fn new(host: usize, every: u64) -> Kept {
        Kept { host, every, runs: 0, machine: None }
    }

    pub fn get(&mut self) -> &Machine {
        self.runs += 1;
        if self.runs > self.every {
            self.runs = 1;
            self.machine = None;
        }
        self.machine.get_or_insert_with(|| Machine::new(self.host))
    }

    /// The machine changed in a way the next run shouldn't see.
    pub fn spoilt(&mut self) {
        self.machine = None;
    }

    /// Whether the machine still stands as it was made: the roster, its
    /// link, and its gateway where it was.
    pub fn changed(m: &Machine) -> bool {
        let s = m.shared();
        s.is_removed() || s.roster().len() != roster().len() || s.roster().view().gateway != "studio"
    }
}

/// A body for a frame of `kind`, as its schema has it, with the input's
/// fields over it.
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
        "message" => json!({
            "schema": "network/message/v1",
            "mid": format!("msg_{:08x}", c.u16()),
            "sender": "claude:aaaaaaaa-0001",
            "senderKind": *c.pick(&["agent", "actor", "you"]),
            "auth": { "method": "session", "strength": *c.pick(&["strong", "weak"]) },
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
            "schema": "network/operator/v1",
            "action": *c.pick(&["message", "ticket", "remove", "replace", "shutdown"]),
            "args": {},
            "checked": *c.pick(&["window", "person", "session"]),
            "strength": *c.pick(&["strong", "weak"]),
            "nonce": format!("n{:04x}{:04x}", c.u16(), fresh_counter()),
            "at": now + i64::from(c.i16()) * 4,
        }),
        "hello" => {
            json!({ "epoch": "ZXBvY2gtb25l", "nonce": "bm9uY2Utb2YtdGhlLWxpbms", "now": now, "network": NET, "gateway": "studio", "keyId": public(STUDIO).key_id(), "rosterN": c.below(8), "rosterHead": roster().head(), "hosts": { "desk": { "platform": "darwin", "version": "0.6.1" } } })
        }
        "ping" => json!({ "now": now + i64::from(c.i16()) * 1000, "nonce": "bm9uY2Utb2YtdGhlLWxpbms", "linked": ["desk", "laptop"] }),
        "gap" => json!({ "lost": c.u8(), "at": now }),
        "roster" => json!({ "entries": [], "proposals": [], "asks": [] }),
        "removed" => json!({ "network": NET, "host": "laptop", "by": ["studio"], "at": now, "n": c.below(8) }),
        _ => json!({}),
    };
    let over = c.value();
    overlay(base, over, c)
}

fn fresh_counter() -> u16 {
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed) as u16
}
