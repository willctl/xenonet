//! Waking an actor when a message arrives for it, so it needn't keep asking
//! its inbox. Three ways, by what the actor can do:
//!
//!   listening  a program of its own holds a stream open, and hears of a
//!              message at once
//!   webhook    XenoNet calls a URL the owner gave it, which starts the
//!              actor; it then reads its inbox
//!   notify     XenoNet tells you, for an actor nothing can wake, once a
//!              message has waited two minutes, and at most hourly
//!
//! A wake says only that messages wait, and how many: never what they say,
//! who sent them, or a token.
//!
//! A webhook's URL and key are the owner's to set, from XenoNet's window,
//! and are kept in the computer's credential store, never in XenoNet's
//! files, its logs or the page. Its request follows Standard Webhooks. It
//! goes to https:// only (http:// on this computer alone), follows no
//! redirect, and gives up after ten seconds.
//!
//! Messages in a burst make one wake. An actor is woken at most every 30
//! seconds and 20 times an hour, and not again while the messages it was
//! woken for wait unread, for two minutes; still unread then, it's woken
//! once more. A webhook that fails waits longer each time before the next
//! try, up to half an hour. Before each
//! call the URL is checked again, and its host must be the one set in
//! XenoNet's window.
//!
//! What's kept (the wakes you set, in the store's meta; the tokens; the
//! unread messages) is read through `Store`. The rest is the world's,
//! through `Host`: the clock, the log, your notifications, URL parsing, DNS,
//! the calls themselves, the timers by actor, the streams' sinks by number,
//! the credential store, and the registry (whether an actor reports itself,
//! and its name).

use crate::re;
use crate::secret_store::SECRET;
use crate::store::{self, Tx};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_os::sync::Locked;
use xenonet_text::json::{number, stringify};
use xenonet_text::text::display;
use xenonet_text::text::truthy;
use xenonet_text::utf16::{base64, base64_bytes};

pub const WAKE_KINDS: [&str; 3] = ["webhook", "notify", "none"];
const META: &str = "wakes";
pub const COALESCE_MS: f64 = 1500.0;
pub const MIN_GAP_MS: f64 = 30e3;
pub const TIMEOUT_MS: f64 = 10e3;
pub const BACKOFF_MS: [f64; 4] = [30e3, 120e3, 600e3, 1800e3];
pub const NOTIFY_AFTER_MS: f64 = 120e3;
pub const NOTIFY_EVERY_MS: f64 = 3600e3;
pub const STREAMS_PER_ACTOR: usize = 4;
/// One actor's wakes: 20 an hour, 5 at once.
pub const PER_ACTOR_RATE: f64 = 20.0 / 3600.0;
pub const PER_ACTOR_BURST: f64 = 5.0;
/// Woken, and the messages it was woken for still unread: it's at them, or
/// can't be, so it isn't woken again for this long.
pub const UNREAD_HOLD_MS: f64 = 2.0 * 60e3;
const TEST_GAP_MS: f64 = 5e3;
/// All wakes together, whatever the actors: 120 an hour, 30 at once.
const ALL_RATE: f64 = 120.0 / 3600.0;
const ALL_BURST: f64 = 30.0;
const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// How a call failed: a WakeError (an HTTP status, a code, a sentence), or
/// what a collaborator or the store failed with.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Wake { status: f64, code: &'static str, message: String },
    Error(store::Error),
}

impl From<store::Error> for Fail {
    fn from(e: store::Error) -> Self {
        Fail::Error(e)
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn fail<T>(status: f64, code: &'static str, message: impl Into<String>) -> Result<T> {
    Err(Fail::Wake { status, code, message: message.into() })
}

/// A URL as WHATWG's parser reads it, its hash taken off.
#[derive(Clone, Debug, PartialEq)]
pub struct Url {
    pub username: String,
    pub password: String,
    /// With its colon: "https:".
    pub protocol: String,
    pub hostname: String,
    /// The host name with its port, when it has one.
    pub host: String,
    pub href: String,
}

/// The request a wake makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// How a call went: the status it answered, or the name of what failed
/// (TimeoutError, TypeError…), when there is one.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetched {
    Status(f64),
    Failed(Option<String>),
}

/// What the wakes ask of the world.
pub trait Host {
    fn now(&mut self) -> f64;
    /// A line for the log, under `code`.
    fn log(&mut self, code: &str, detail: &str);
    /// A notification to you ({ actor, name, unread }).
    fn notify(&mut self, mail: Value);
    /// A URL as WHATWG's parser reads it, or None for what isn't one.
    fn parse_url(&mut self, s: &str) -> Option<Url>;
    /// The addresses a host name has.
    fn resolve(&mut self, host: &str) -> Vec<String>;
    /// One request, no redirect followed, TIMEOUT_MS at most.
    fn fetch(&mut self, url: &str, request: &Request) -> Fetched;
    /// An actor's wake or notify timer ("wake" or "notify"), set for `ms`
    /// from now, in place of the one it had: whether it's set.
    fn set_timer(&mut self, kind: &str, id: &str, ms: f64) -> bool;
    /// An actor's timers, cleared.
    fn clear_timers(&mut self, id: &str);
    /// An event to a stream's sink.
    fn send(&mut self, sink: u64, event: &str, data: Value);
    /// A stream's sink ended.
    fn end(&mut self, sink: u64);
    /// Whether the credential store can be reached.
    fn secrets_available(&mut self) -> bool;
    /// What the credential store is, for a refusal.
    fn secrets_kind(&mut self) -> Option<String>;
    /// A credential, or None for one that isn't there or can't be read.
    fn secret_get(&mut self, target: &str) -> Option<String>;
    fn secret_set(&mut self, target: &str, value: &str, comment: &str) -> store::Result<()>;
    /// A credential taken away; one that isn't there is no error.
    fn secret_remove(&mut self, target: &str);
    /// Whether the registry has a registered actor by this id.
    fn is_registered(&mut self, id: &str) -> bool;
    /// An actor as registering sees it, or null.
    fn info(&mut self, id: &str) -> Value;
}

/// What the wakes read of the engine's database.
pub trait Store: Tx + Send + Sync {
    /// A key of the store's meta, or null.
    fn meta(&self, key: &str) -> store::Result<Value>;
    fn set_meta(&self, key: &str, value: &str) -> store::Result<()>;
    /// Whether an actor has a gateway token.
    fn has_gateway_token(&self, slug: &str) -> store::Result<bool>;
    /// The ids of the messages an actor hasn't seen.
    fn unseen_ids(&self, actor: &str) -> store::Result<Vec<Value>>;
    /// Each actor's token times ({ slug, hasToken, hasGateway, … }).
    fn actor_token_times(&self) -> store::Result<Vec<Value>>;
}

/// The item in the credential store for an actor's webhook URL or key.
pub fn wake_target(slug: &str, part: &str) -> String {
    format!("XenoNet/actor wake/{slug}/{part}")
}

/// An address on the local link (169.254.0.0/16, fe80::/10, and IPv4's as
/// IPv6 maps it), where a computer's metadata services answer.
pub fn link_local(ip: &str) -> bool {
    re!(r"^169\.254\.").is_match(ip) || re!(r"(?i)^fe[89ab][0-9a-f]:").is_match(ip) || re!(r"(?i)^::ffff:(169\.254\.|a9fe:)").is_match(ip)
}

fn secret_ok(s: &str) -> bool {
    re!(SECRET).is_match(s)
}

/// `.replace(/^\[|\]$/g, "")`.
fn unbracket(h: &str) -> &str {
    let h = h.strip_prefix('[').unwrap_or(h);
    h.strip_suffix(']').unwrap_or(h)
}

/// A number as JS writes it.
fn num_text(n: f64) -> String {
    display(&number(n))
}

/// A webhook URL as XenoNet takes it (`raw`: null or absent for none):
/// https://, or http:// on this computer; no user name or password in it
/// (the key goes apart), not link-local, at most 2,048 characters. (url,
/// host), or a 400.
pub fn check_wake_url(host: &mut dyn Host, raw: &Value) -> Result<(String, String)> {
    let s = if raw.is_null() { String::new() } else { display(raw) };
    let s = xenonet_text::text::trim(&s);
    if s.encode_utf16().count() > 2048 {
        return fail(400.0, "bad-request", "that URL is over 2,048 characters");
    }
    let Some(u) = host.parse_url(s) else {
        return fail(400.0, "bad-request", "that isn't a URL");
    };
    if !u.username.is_empty() || !u.password.is_empty() {
        return fail(400.0, "bad-request", "a URL with a user name or password in it isn't taken; give the key on its own");
    }
    if u.protocol != "https:" && !(u.protocol == "http:" && LOOPBACK.contains(&u.hostname.as_str())) {
        return fail(400.0, "bad-request", "a wake URL starts https://, or http:// for this computer alone");
    }
    if link_local(unbracket(&u.hostname)) {
        return fail(400.0, "bad-request", "that address is link-local, which a wake never goes to");
    }
    Ok((u.href, u.host))
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |x: u8| k.iter().map(|b| b ^ x).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

/// Standard Webhooks' signature: v1, then the base64 HMAC-SHA256 of
/// "`<id>.<timestamp>.<body>`" with the secret's bytes (a whsec_ secret is
/// base64 after its prefix).
pub fn wake_signature(secret: &str, id: &str, timestamp: &str, body: &str) -> String {
    let key = match secret.strip_prefix("whsec_") {
        Some(rest) => base64_bytes(rest),
        None => secret.as_bytes().to_vec(),
    };
    format!("v1,{}", base64(&hmac_sha256(&key, format!("{id}.{timestamp}.{body}").as_bytes())))
}

fn random_hex(n: usize) -> store::Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| store::Error::new(format!("no random bytes: {e}")))?;
    Ok(crate::tokens::hex(&b))
}

/// The request a wake makes: its body says which actor and how many
/// messages wait, nothing more. `id` None for a new one.
pub fn wake_request(key: &str, actor: &str, unread: f64, version: &str, now: f64, id: Option<String>) -> store::Result<Request> {
    let id = match id {
        Some(id) => id,
        None => format!("msg_{}", random_hex(16)?),
    };
    let timestamp = (now / 1000.0).floor();
    if !now.is_finite() || now.abs() > 8.64e15 {
        return Err(store::Error::new("Invalid time value"));
    }
    let body = stringify(&json!({ "type": "xenonet.inbox", "timestamp": xenonet_text::time::iso(now), "data": { "actor": actor, "unread": number(unread) } }), 0);
    let stamp = num_text(timestamp);
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("user-agent".to_string(), format!("XenoNet/{version}")),
        ("webhook-id".to_string(), id.clone()),
        ("webhook-timestamp".to_string(), stamp.clone()),
    ];
    if key.starts_with("whsec_") {
        headers.push(("webhook-signature".into(), wake_signature(key, &id, &stamp, &body)));
    } else {
        headers.push(("authorization".into(), format!("Bearer {key}")));
    }
    Ok(Request { method: "POST".into(), headers, body })
}

/// How a call went: { ok, status?, why?, reason? }.
fn out(ok: bool, status: Option<f64>, why: Option<String>, reason: Option<String>) -> Value {
    let mut f = Map::new();
    f.insert("ok".into(), Value::Bool(ok));
    if let Some(s) = status {
        f.insert("status".into(), number(s));
    }
    if let Some(w) = why {
        f.insert("why".into(), Value::String(w));
    }
    if let Some(r) = reason {
        f.insert("reason".into(), Value::String(r));
    }
    Value::Object(f)
}

/// Why a failed call failed: `why` in words, with the host, for you; and
/// `reason`, without it, for what anyone on this computer may read. Never
/// the URL's path or query, or the key. `code`: the error's name, when
/// it has one.
fn failed(code: Option<&str>, host: &str) -> Value {
    let secs = num_text(TIMEOUT_MS / 1000.0);
    if matches!(code, Some("TimeoutError" | "ABORT_ERR" | "UND_ERR_CONNECT_TIMEOUT")) {
        return out(false, None, Some(format!("{host} didn't answer within {secs} seconds")), Some(format!("no answer within {secs} seconds")));
    }
    let said = match code {
        Some(c) if !c.is_empty() => format!(" ({c})"),
        _ => String::new(),
    };
    out(false, None, Some(format!("XenoNet couldn't reach {host}{said}")), Some(format!("couldn't connect{said}")))
}

fn answered(status: f64, host: &str) -> Value {
    let s = num_text(status);
    if (200.0..300.0).contains(&status) {
        out(true, Some(status), None, None)
    } else if (300.0..400.0).contains(&status) {
        out(false, Some(status), Some(format!("{host} answered with a redirect (HTTP {s}), which XenoNet doesn't follow")), Some(format!("a redirect (HTTP {s})")))
    } else {
        out(false, Some(status), Some(format!("{host} answered HTTP {s}")), Some(format!("HTTP {s}")))
    }
}

/// How one actor's wakes went.
#[derive(Clone, Debug)]
struct State {
    timer: bool,
    notify_timer: bool,
    in_flight: bool,
    again: bool,
    testing: bool,
    last_at: Option<f64>,
    last: Option<Value>,
    failures: f64,
    not_before: f64,
    notified_at: f64,
    tested_at: f64,
    woke: Option<Vec<Value>>,
    /// The next wake is the second look, two minutes after one: its last.
    rechecking: bool,
    /// When the wake timer set is due.
    timer_at: f64,
    tokens: f64,
    tokens_at: f64,
}

// One message's id as another's: numbers by value.
fn same_id(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

// An actor's state before its first wake, made at `t`.
fn new_state(t: f64) -> State {
    State {
        timer: false,
        notify_timer: false,
        in_flight: false,
        again: false,
        testing: false,
        last_at: None,
        last: None,
        failures: 0.0,
        not_before: 0.0,
        notified_at: 0.0,
        tested_at: 0.0,
        woke: None,
        rechecking: false,
        timer_at: 0.0,
        tokens: PER_ACTOR_BURST,
        tokens_at: t,
    }
}

#[derive(Default)]
struct Inner {
    /// By actor id, in the order they came.
    state: Vec<(String, State)>,
    /// The streams held open: each set of sinks by its number, and the set
    /// each actor has now.
    sets: HashMap<u64, Vec<u64>>,
    streams: Vec<(String, u64)>,
    next_set: u64,
    all_tokens: f64,
    all_at: f64,
}

pub struct Wakes {
    store: Arc<dyn Store>,
    version: String,
    inner: Mutex<Inner>,
}

fn slug_of(id: &str) -> String {
    id.strip_prefix("actor:").unwrap_or(id).to_string()
}

/// An object's keys in the order JS gives them: array indexes first, in
/// order, then the rest as they came.
fn js_keys(m: &Map<String, Value>) -> Vec<String> {
    let index = |k: &str| k.parse::<u32>().ok().filter(|n| *n != u32::MAX && n.to_string() == k);
    let mut ints: Vec<(u32, String)> = m.keys().filter_map(|k| index(k).map(|n| (n, k.clone()))).collect();
    ints.sort();
    let mut out: Vec<String> = ints.into_iter().map(|(_, k)| k).collect();
    out.extend(m.keys().filter(|k| index(k).is_none()).cloned());
    out
}

fn ordered(m: &Map<String, Value>) -> Map<String, Value> {
    js_keys(m).into_iter().map(|k| (k.clone(), m[&k].clone())).collect()
}

/// A field of an object, or null.
fn col(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

impl Wakes {
    pub fn new(store: Arc<dyn Store>, version: &str, now: f64) -> Self {
        Wakes { store, version: version.into(), inner: Mutex::new(Inner { all_tokens: ALL_BURST, all_at: now, ..Inner::default() }) }
    }

    fn log(h: &mut dyn Host, detail: &str) {
        h.log("actor.wake", detail);
    }

    /// The actor's state, made when it has none.
    fn st(&self, h: &mut dyn Host, id: &str) {
        if self.inner.locked().state.iter().any(|(k, _)| k == id) {
            return;
        }
        let s = new_state(h.now());
        let mut inner = self.inner.locked();
        if !inner.state.iter().any(|(k, _)| k == id) {
            inner.state.push((id.to_string(), s));
        }
    }

    fn with<T>(&self, id: &str, f: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.inner.locked().state.iter_mut().find(|(k, _)| k == id).map(|(_, s)| f(s))
    }

    fn configs(&self) -> Result<Value> {
        let meta = self.store.meta(META)?;
        let text = match meta {
            Value::Null => "{}".to_string(),
            Value::String(s) => s,
            other => display(&other),
        };
        Ok(match serde_json::from_str::<Value>(&text) {
            Ok(Value::Null) | Err(_) => json!({}),
            Ok(v) => v,
        })
    }

    fn config_of(&self, slug: &str) -> Result<Option<Value>> {
        Ok(match self.configs()? {
            Value::Object(m) => m.get(slug).cloned(),
            _ => None,
        })
    }

    fn save_configs(&self, c: &Value) -> Result<()> {
        let c = match c {
            Value::Object(m) => Value::Object(ordered(m)),
            other => other.clone(),
        };
        self.store.set_meta(META, &stringify(&c, 0))?;
        Ok(())
    }

    /// How an actor is woken: what you set, else Notify me for one that
    /// reaches XenoNet through the gateway, which nothing else wakes.
    fn kind_of(&self, slug: &str) -> Result<&'static str> {
        let config = self.config_of(slug)?;
        if let Some(k) = config.as_ref().and_then(|c| c.get("kind")).and_then(Value::as_str).and_then(|k| WAKE_KINDS.iter().find(|w| **w == k)) {
            return Ok(k);
        }
        Ok(if self.store.has_gateway_token(slug)? { "notify" } else { "none" })
    }

    fn unseen(&self, id: &str) -> Result<Vec<Value>> {
        Ok(self.store.unseen_ids(id)?)
    }

    fn unread(&self, id: &str) -> Result<usize> {
        Ok(self.unseen(id)?.len())
    }

    fn take_all(&self, h: &mut dyn Host) -> bool {
        let t = h.now();
        let mut i = self.inner.locked();
        i.all_tokens = ALL_BURST.min(i.all_tokens + ((t - i.all_at) / 1000.0) * ALL_RATE);
        i.all_at = t;
        if i.all_tokens < 1.0 || i.all_tokens.is_nan() {
            return false;
        }
        i.all_tokens -= 1.0;
        true
    }

    fn set_of(&self, id: &str) -> Vec<u64> {
        let i = self.inner.locked();
        i.streams.iter().find(|(k, _)| k == id).and_then(|(_, n)| i.sets.get(n)).cloned().unwrap_or_default()
    }

    fn in_set(&self, id: &str, sink: u64) -> bool {
        self.set_of(id).contains(&sink)
    }

    // ---------- a message arrives ----------

    pub fn arrived(&self, h: &mut dyn Host, id: &str) -> Result<()> {
        if !id.starts_with("actor:") || !h.is_registered(id) {
            return Ok(());
        }
        let n = self.unread(id)?;
        let live = self.set_of(id);
        if !live.is_empty() {
            for sink in live {
                if self.in_set(id, sink) {
                    h.send(sink, "inbox", json!({ "unread": number(n as f64) }));
                }
            }
            return Ok(());
        }
        match self.kind_of(&slug_of(id))? {
            "webhook" => self.schedule(h, id),
            "notify" => {
                self.schedule_notify(h, id);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// When its next wake may go: messages in a burst together, 30 seconds
    /// after the last, after a failure's wait, and not while the messages
    /// it was last woken for wait unread, for two minutes.
    fn next_at(&self, h: &mut dyn Host, id: &str) -> Result<f64> {
        self.st(h, id);
        let unseen = self.unseen(id)?;
        // Forgotten since st() made it sure: as st() would make it again.
        let s = match self.with(id, |s| s.clone()) {
            Some(s) => s,
            None => new_state(h.now()),
        };
        let held = match &s.woke {
            Some(w) if !w.is_empty() && w.iter().all(|m| unseen.iter().any(|u| same_id(u, m))) => s.last_at.unwrap_or(0.0) + UNREAD_HOLD_MS,
            _ => 0.0,
        };
        let refill = if s.tokens >= 1.0 { 0.0 } else { s.tokens_at + ((1.0 - s.tokens) / PER_ACTOR_RATE) * 1000.0 };
        let t = h.now();
        let all = [t + COALESCE_MS, s.last_at.unwrap_or(0.0) + MIN_GAP_MS, s.not_before, held, refill];
        Ok(if all.iter().any(|x| x.is_nan()) { f64::NAN } else { all.iter().copied().fold(f64::NEG_INFINITY, f64::max) })
    }

    /// One timer an actor: a sooner wake (its messages read, and a new one
    /// come) takes the place of a later one (the second look).
    fn schedule(&self, h: &mut dyn Host, id: &str) -> Result<()> {
        self.st(h, id);
        let (in_flight, timer, timer_at) = self.with(id, |s| (s.in_flight, s.timer, s.timer_at)).unwrap_or((false, false, 0.0));
        if in_flight {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        let at = self.next_at(h, id)?;
        if timer && timer_at <= at {
            return Ok(());
        }
        let ms = at - h.now();
        let set = h.set_timer("wake", id, ms);
        self.with(id, |s| {
            s.timer = set;
            s.timer_at = at;
        });
        Ok(())
    }

    fn schedule_notify(&self, h: &mut dyn Host, id: &str) {
        self.st(h, id);
        if self.with(id, |s| s.notify_timer).unwrap_or(false) {
            return;
        }
        let set = h.set_timer("notify", id, NOTIFY_AFTER_MS);
        self.with(id, |s| s.notify_timer = set);
    }

    /// A wake's timer came: whether to fire now.
    pub fn wake_due(&self, id: &str) -> bool {
        self.with(id, |s| {
            s.timer = false;
            if s.in_flight {
                s.again = true;
                return false;
            }
            true
        })
        .unwrap_or(true)
    }

    /// A notification's timer came.
    pub fn notify_due(&self, h: &mut dyn Host, id: &str) -> Result<()> {
        self.with(id, |s| s.notify_timer = false);
        let n = self.unread(id)?;
        let t = h.now();
        if n == 0 || !h.is_registered(id) || t - self.with(id, |s| s.notified_at).unwrap_or(0.0) < NOTIFY_EVERY_MS {
            return Ok(());
        }
        self.with(id, |s| s.notified_at = t);
        let info = h.info(id);
        let name = match info.get("name") {
            None | Some(Value::Null) => Value::String(slug_of(id)),
            Some(name) => name.clone(),
        };
        h.notify(json!({ "actor": id, "name": name, "unread": number(n as f64) }));
        Ok(())
    }

    /// One of the actor's own wakes.
    fn take_one(&self, h: &mut dyn Host, id: &str) -> bool {
        let t = h.now();
        self.with(id, |s| {
            s.tokens = PER_ACTOR_BURST.min(s.tokens + ((t - s.tokens_at) / 1000.0) * PER_ACTOR_RATE);
            s.tokens_at = t;
            if s.tokens < 1.0 || s.tokens.is_nan() {
                return false;
            }
            s.tokens -= 1.0;
            true
        })
        .unwrap_or(false)
    }

    /// Calls the webhook kept for `slug`, saying `n` messages wait. Its URL
    /// is checked again first, as it's read back from the store, and its
    /// host must be the one set in the window; an address that turns out to
    /// be link-local is refused.
    fn call(&self, h: &mut dyn Host, id: &str, slug: &str, n: usize, t: f64) -> Result<Value> {
        let url = h.secret_get(&wake_target(slug, "url"));
        let key = h.secret_get(&wake_target(slug, "key"));
        // configs()[slug]?.host
        let set_host = |w: &Self| -> Result<Option<Value>> { Ok(w.config_of(slug)?.and_then(|c| c.get("host").cloned())) };
        let host = match set_host(self)? {
            None | Some(Value::Null) => "its webhook".to_string(),
            Some(v) => display(&v),
        };
        let (Some(url), Some(key)) = (url.filter(|u| !u.is_empty()), key.filter(|k| !k.is_empty())) else {
            return Ok(out(
                false,
                None,
                Some("its webhook isn't in your credential store any more; set its wake again".into()),
                Some("its webhook is missing from the credential store".into()),
            ));
        };
        let checked = match check_wake_url(h, &Value::String(url)) {
            Ok(v) => Some(v),
            Err(Fail::Wake { .. }) => None,
            Err(e) => return Err(e),
        };
        let wanted = set_host(self)?;
        let checked_url = match checked {
            Some((url, hn)) if wanted.as_ref().and_then(Value::as_str) == Some(hn.as_str()) && secret_ok(&key) => url,
            _ => {
                return Ok(out(
                    false,
                    None,
                    Some("the webhook in your credential store isn't the one set in XenoNet's window; set its wake again".into()),
                    Some("its webhook changed outside XenoNet".into()),
                ))
            }
        };
        let bare = match h.parse_url(&checked_url) {
            Some(u) => unbracket(&u.hostname).to_string(),
            None => return Err(store::Error::new("Invalid URL").into()),
        };
        if !re!(r"^[0-9.]+$").is_match(&bare) && !bare.contains(':') && h.resolve(&bare).iter().any(|a| link_local(a)) {
            return Ok(out(false, None, Some(format!("{host} is a link-local address, which a wake never goes to")), Some("a link-local address".into())));
        }
        // A time that can't be written fails as a RangeError does, inside
        // the call.
        let Ok(request) = wake_request(&key, id, n as f64, &self.version, t, None) else {
            return Ok(failed(Some("RangeError"), &host));
        };
        Ok(match h.fetch(&checked_url, &request) {
            Fetched::Failed(code) => failed(code.as_deref(), &host),
            Fetched::Status(status) => answered(status, &host),
        })
    }

    /// How a wake went. Your test says so too, but neither holds the next
    /// wake back nor counts as a failure to wait out.
    fn record(&self, h: &mut dyn Host, id: &str, t: f64, out: Value, test: bool) -> Value {
        let last = json!({
            "ok": col(&out, "ok"),
            "at": number(t),
            "status": col(&out, "status"),
            "why": col(&out, "why"),
            "reason": col(&out, "reason"),
            "test": test,
        });
        self.with(id, |s| s.last = Some(last));
        if test {
            return out;
        }
        let ok = truthy(out.get("ok"));
        self.with(id, |s| {
            s.last_at = Some(t);
            if ok {
                s.failures = 0.0;
                s.not_before = 0.0;
            } else {
                s.failures += 1.0;
                let i = (s.failures.min(BACKOFF_MS.len() as f64) - 1.0) as usize;
                s.not_before = t + BACKOFF_MS[i];
            }
        });
        if !ok {
            Self::log(h, &format!("{id}: {}", xenonet_text::text::template(out.get("why"))));
        }
        out
    }

    /// A wake for the messages waiting, when its time comes: one at a time.
    pub fn fire(&self, h: &mut dyn Host, id: &str) -> Result<()> {
        let slug = slug_of(id);
        self.st(h, id);
        let busy = self.with(id, |s| {
            if s.in_flight {
                s.again = true;
                return true;
            }
            s.in_flight = true;
            s.again = false;
            false
        });
        if busy == Some(true) {
            return Ok(());
        }
        let r = self.fire_inner(h, id, &slug);
        self.with(id, |s| s.in_flight = false);
        let again = self.with(id, |s| s.again).unwrap_or(false);
        if again && self.unread(id)? > 0 {
            self.schedule(h, id)?;
        }
        r
    }

    fn fire_inner(&self, h: &mut dyn Host, id: &str, slug: &str) -> Result<()> {
        let unseen = self.unseen(id)?;
        // The second look, two minutes after a wake whose messages all wait
        // unread: its last.
        let recheck = self
            .with(id, |s| {
                let woke_unread = s.woke.as_ref().is_some_and(|w| !w.is_empty() && w.iter().all(|m| unseen.iter().any(|u| same_id(u, m))));
                std::mem::take(&mut s.rechecking) && woke_unread
            })
            .unwrap_or(false);
        if unseen.is_empty() || self.kind_of(slug)? != "webhook" || !h.is_registered(id) {
            return Ok(());
        }
        // Its time may have moved since it was set: the actor's own limit,
        // or the messages it was last woken for still unread.
        let at = self.next_at(h, id)?;
        if at > h.now() + COALESCE_MS {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        if !self.take_one(h, id) {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        if !self.take_all(h) {
            let t = h.now();
            self.record(h, id, t, out(false, None, Some("XenoNet sends at most 120 wakes an hour".into()), Some("XenoNet's limit of 120 wakes an hour".into())), false);
            return Ok(());
        }
        let t = h.now();
        let went = self.call(h, id, slug, unseen.len(), t)?;
        let went = self.record(h, id, t, went, false);
        if truthy(went.get("ok")) {
            // Still unread when the hold is up: woken once more, then not
            // again for these.
            self.with(id, |s| {
                s.woke = Some(unseen);
                if !recheck {
                    s.rechecking = true;
                    s.again = true;
                }
            });
        }
        Ok(())
    }

    /// Your test: called now, even with nothing waiting. It doesn't hold
    /// the next wake back, nor stop one on its way.
    fn fire_test(&self, h: &mut dyn Host, id: &str) -> Result<Value> {
        self.st(h, id);
        if self.with(id, |s| s.testing).unwrap_or(false) {
            return fail(429.0, "too-many", "a test is on its way");
        }
        self.with(id, |s| s.testing = true);
        let r = (|| -> Result<Value> {
            if !self.take_all(h) {
                let t = h.now();
                return Ok(self.record(
                    h,
                    id,
                    t,
                    out(false, None, Some("XenoNet sends at most 120 wakes an hour".into()), Some("XenoNet's limit of 120 wakes an hour".into())),
                    true,
                ));
            }
            let t = h.now();
            let n = self.unread(id)?;
            let went = self.call(h, id, &slug_of(id), n, t)?;
            Ok(self.record(h, id, t, went, true))
        })();
        self.with(id, |s| s.testing = false);
        r
    }

    // ---------- what's set ----------

    pub fn view(&self, id: &str) -> Result<Value> {
        let slug = slug_of(id);
        let config = self.config_of(&slug)?;
        let c = config.filter(|c| !c.is_null()).unwrap_or(json!({}));
        let s = self.with(id, |s| s.clone());
        let kind = self.kind_of(&slug)?;
        let set = self.config_of(&slug)?.is_some_and(|c| truthy(Some(&c)));
        let webhook = kind == "webhook";
        let host = if webhook { col(&c, "host") } else { Value::Null };
        let signed = if webhook { Value::Bool(c.get("signed").is_some_and(|v| truthy(Some(v)))) } else { Value::Null };
        let listening = self.set_of(id).len();
        let last = s.as_ref().and_then(|s| s.last.clone());
        Ok(json!({
            "actor": id,
            "kind": kind,
            "set": set,
            "host": host,
            "signed": signed,
            "listening": number(listening as f64),
            "lastAt": last.as_ref().map_or(Value::Null, |l| col(l, "at")),
            "last": last.unwrap_or(Value::Null),
            "failures": number(s.map_or(0.0, |s| s.failures)),
        }))
    }

    fn forget_state(&self, h: &mut dyn Host, id: &str) {
        let had = self.inner.locked().state.iter().any(|(k, _)| k == id);
        if !had {
            return;
        }
        h.clear_timers(id);
        self.inner.locked().state.retain(|(k, _)| k != id);
    }

    /// Each actor that reports itself, and how it's woken.
    pub fn list(&self, h: &mut dyn Host) -> Result<Value> {
        let mut ids: Vec<String> = Vec::new();
        let mut add = |id: String| {
            if !ids.contains(&id) {
                ids.push(id);
            }
        };
        for r in self.store.actor_token_times()? {
            if truthy(r.get("hasToken")) || truthy(r.get("hasGateway")) {
                add(format!("actor:{}", display(r.get("slug").unwrap_or(&Value::Null))));
            }
        }
        if let Value::Object(m) = self.configs()? {
            for k in js_keys(&m) {
                add(format!("actor:{k}"));
            }
        }
        let (streams, state): (Vec<String>, Vec<String>) = {
            let i = self.inner.locked();
            (i.streams.iter().map(|(k, _)| k.clone()).collect(), i.state.iter().map(|(k, _)| k.clone()).collect())
        };
        streams.into_iter().chain(state).for_each(&mut add);
        let mut out = Vec::new();
        for id in ids {
            if h.is_registered(&id) {
                out.push(self.view(&id)?);
            }
        }
        Ok(Value::Array(out))
    }

    /// { kind, url, key }: a webhook takes both, kept in the credential
    /// store; notify and none forget them.
    pub fn set(&self, h: &mut dyn Host, slug: &str, ask: &Value) -> Result<Value> {
        let id = format!("actor:{slug}");
        if !h.is_registered(&id) {
            return fail(404.0, "not-found", "XenoNet has no actor by that slug that reports itself");
        }
        let Some(kind) = ask.get("kind").and_then(Value::as_str).filter(|k| WAKE_KINDS.contains(k)).map(str::to_string) else {
            return fail(400.0, "bad-request", format!("a wake is {}", WAKE_KINDS.join(", ")));
        };
        let mut configs = self.configs()?;
        let config = if kind == "webhook" {
            let (url, host) = check_wake_url(h, ask.get("url").unwrap_or(&Value::Null))?;
            if !secret_ok(&url) {
                return fail(400.0, "bad-request", "that URL has a character XenoNet can't keep, such as a quote or a backslash");
            }
            let key = match ask.get("key") {
                None | Some(Value::Null) => String::new(),
                Some(k) => display(k),
            };
            let key = xenonet_text::text::trim(&key).to_string();
            if !secret_ok(&key) {
                return fail(400.0, "bad-request", "a webhook's key is 8 to 1,024 printable characters, without spaces, quotes or backslashes");
            }
            if !h.secrets_available() {
                let store = h.secrets_kind().unwrap_or_else(|| "a credential store".to_string());
                return fail(501.0, "no-store", format!("XenoNet can't reach {store} on this computer, so it can't keep a webhook's key"));
            }
            h.secret_set(&wake_target(slug, "url"), &url, "The webhook XenoNet calls to wake an actor")?;
            h.secret_set(&wake_target(slug, "key"), &key, "The key XenoNet sends with an actor's wake")?;
            let t = h.now();
            json!({ "kind": kind, "host": host, "signed": key.starts_with("whsec_"), "at": number(t) })
        } else {
            h.secret_remove(&wake_target(slug, "url"));
            h.secret_remove(&wake_target(slug, "key"));
            let t = h.now();
            json!({ "kind": kind, "at": number(t) })
        };
        if let Value::Object(m) = &mut configs {
            m.insert(slug.to_string(), config);
        }
        self.save_configs(&configs)?;
        self.forget_state(h, &id);
        self.view(&id)
    }

    /// A wake now, for the owner to see the webhook work.
    pub fn test(&self, h: &mut dyn Host, slug: &str) -> Result<Value> {
        let id = format!("actor:{slug}");
        if !h.is_registered(&id) {
            return fail(404.0, "not-found", "XenoNet has no actor by that slug that reports itself");
        }
        if self.kind_of(slug)? != "webhook" {
            return fail(409.0, "no-webhook", "it has no webhook to call");
        }
        self.st(h, &id);
        let t = h.now();
        if t - self.with(&id, |s| s.tested_at).unwrap_or(0.0) < TEST_GAP_MS {
            return fail(429.0, "too-many", "one test every five seconds");
        }
        let t = h.now();
        self.with(&id, |s| s.tested_at = t);
        let went = self.fire_test(h, &id)?;
        let mut fields = match went {
            Value::Object(f) => f,
            _ => Map::new(),
        };
        fields.shift_remove("wake");
        fields.insert("wake".into(), self.view(&id)?);
        Ok(Value::Object(fields))
    }

    /// The actor left or was removed: its wake and its streams go.
    pub fn forget(&self, h: &mut dyn Host, slug: &str) -> Result<()> {
        let id = format!("actor:{slug}");
        let mut configs = self.configs()?;
        if let Value::Object(m) = &mut configs {
            if m.get(slug).is_some_and(|v| truthy(Some(v))) {
                m.shift_remove(slug);
                self.save_configs(&configs)?;
            }
        }
        h.secret_remove(&wake_target(slug, "url"));
        h.secret_remove(&wake_target(slug, "key"));
        self.forget_state(h, &id);
        for sink in self.set_of(&id) {
            h.end(sink);
        }
        self.inner.locked().streams.retain(|(k, _)| *k != id);
        Ok(())
    }

    /// A program of the actor's holding a stream open, as sink `sink`.
    /// What it's told first is how many messages wait. The set it joined,
    /// for unsubscribe.
    pub fn subscribe(&self, h: &mut dyn Host, id: &str, sink: u64) -> Result<u64> {
        let set = {
            let mut i = self.inner.locked();
            let existing = i.streams.iter().find(|(k, _)| *k == id).map(|(_, n)| *n);
            let set = match existing {
                Some(n) => n,
                None => {
                    i.next_set += 1;
                    let n = i.next_set;
                    i.sets.insert(n, Vec::new());
                    i.streams.push((id.to_string(), n));
                    n
                }
            };
            if i.sets.get(&set).map_or(0, Vec::len) >= STREAMS_PER_ACTOR {
                drop(i);
                return fail(429.0, "too-many", format!("an actor holds at most {STREAMS_PER_ACTOR} streams open"));
            }
            let members = i.sets.entry(set).or_default();
            if !members.contains(&sink) {
                members.push(sink);
            }
            set
        };
        let n = self.unread(id)?;
        if n > 0 {
            h.send(sink, "inbox", json!({ "unread": number(n as f64) }));
        }
        Ok(set)
    }

    /// A stream closed: out of the set it joined, and the actor's streams
    /// gone with the last of them.
    pub fn unsubscribe(&self, id: &str, set: u64, sink: u64) {
        let mut i = self.inner.locked();
        let empty = match i.sets.get_mut(&set) {
            Some(m) => {
                m.retain(|s| *s != sink);
                m.is_empty()
            }
            None => true,
        };
        if empty {
            i.streams.retain(|(k, _)| *k != id);
            let live: Vec<u64> = i.streams.iter().map(|(_, n)| *n).collect();
            i.sets.retain(|n, m| live.contains(n) || !m.is_empty());
        }
    }

    pub fn stop(&self, h: &mut dyn Host) {
        let ids: Vec<String> = self.inner.locked().state.iter().map(|(k, _)| k.clone()).collect();
        for id in ids {
            self.forget_state(h, &id);
        }
        let sinks: Vec<u64> = {
            let i = self.inner.locked();
            i.streams.iter().flat_map(|(_, n)| i.sets.get(n).cloned().unwrap_or_default()).collect()
        };
        for sink in sinks {
            h.end(sink);
        }
        self.inner.locked().streams.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_match_standard_webhooks_own_vector() {
        assert_eq!(
            wake_signature("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw", "msg_p5jXN8AQM9LWM0D4loKWxJek", "1614265330", "{\"test\": 2432232314}"),
            "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
        );
    }

    #[test]
    fn base64_reads_as_node_reads_it() {
        for (s, hex) in [("YQ==", "61"), ("YQ==YQ==", "61"), ("Y Q", "61"), ("Y$Q", "61"), ("-_8", "fbff"), ("+/8", "fbff"), ("YWJjZA", "61626364"), ("Y", ""), ("YW=Jj", "61")] {
            let got = crate::tokens::hex(&base64_bytes(s));
            assert_eq!(got, hex, "{s}");
        }
        assert_eq!(base64(b"abcd"), "YWJjZA==");
    }

    #[test]
    fn link_local_addresses_are_known() {
        let got: Vec<bool> = ["169.254.1.1", "fe80::1", "::ffff:a9fe:a9fe", "10.0.0.1", "FE9F::1"].iter().map(|a| link_local(a)).collect();
        assert_eq!(got, [true, true, true, false, true]);
    }

    #[test]
    fn keys_come_in_js_order() {
        let m: Map<String, Value> = serde_json::from_str(r#"{"b":1,"2":2,"a":3,"1":4}"#).unwrap();
        assert_eq!(js_keys(&m), ["1", "2", "b", "a"]);
    }
}
