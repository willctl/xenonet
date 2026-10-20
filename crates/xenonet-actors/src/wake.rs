//! Waking an actor when a message arrives for it, so it needn't keep asking
//! its inbox: core/actors/wake.js. Three ways, by what the actor can do:
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
//! woken for wait unread, for ten minutes. A webhook that fails waits
//! longer each time before the next try, up to half an hour. Before each
//! call the URL is checked again, and its host must be the one set in
//! XenoNet's window.
//!
//! The store is the engine's own. The rest is called as the JS called it,
//! through `Calls`: "actors", "secrets" (the credential store), and the
//! twin's "now", "log", "notify", "parseUrl" (WHATWG URL parsing),
//! "resolve" (DNS), "fetch", "setTimer"/"clearTimers" (its timers, by
//! actor), and "send"/"end" (a stream's sink, by number).

use crate::registration::to_number;
use crate::secret_store::SECRET;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use xenonet_base::json::stringify;
use xenonet_base::sync::Locked;
use xenonet_engine::js::{base64, base64_bytes};
use xenonet_engine::peer::{list, Calls, Thrown};
use xenonet_engine::re;
use xenonet_engine::sql::{Arg, Cell, SqlError};
use xenonet_engine::store::{js_text, Shared};

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
pub const UNREAD_HOLD_MS: f64 = 10.0 * 60e3;
const TEST_GAP_MS: f64 = 5e3;
/// All wakes together, whatever the actors: 120 an hour, 30 at once.
const ALL_RATE: f64 = 120.0 / 3600.0;
const ALL_BURST: f64 = 30.0;
const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// How a call failed: a WakeError (an HTTP status, a code, a sentence), or
/// what a collaborator threw.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Wake { status: f64, code: &'static str, message: String },
    Thrown(Thrown),
}

impl From<Thrown> for Fail {
    fn from(t: Thrown) -> Self {
        Fail::Thrown(t)
    }
}

impl From<SqlError> for Fail {
    fn from(e: SqlError) -> Self {
        Fail::Thrown(e.into())
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn fail<T>(status: f64, code: &'static str, message: impl Into<String>) -> Result<T> {
    Err(Fail::Wake { status, code, message: message.into() })
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

/// A URL as WHATWG's parser reads it (the twin's "parseUrl"), hash and all
/// taken off: None for what isn't one.
struct Url {
    username: String,
    password: String,
    protocol: String,
    hostname: String,
    host: String,
    href: String,
}

fn parse_url(c: &mut dyn Calls, s: &str) -> Result<Option<Url>> {
    let u = c.call("", "parseUrl", vec![Arg::text(s)])?;
    if u.is_nullish() {
        return Ok(None);
    }
    let f = |k: &str| js_text(&u.field(k));
    Ok(Some(Url { username: f("username"), password: f("password"), protocol: f("protocol"), hostname: f("hostname"), host: f("host"), href: f("href") }))
}

/// A webhook URL as XenoNet takes it: https://, or http:// on this
/// computer; no user name or password in it (the key goes apart), not
/// link-local, at most 2,048 characters. (url, host), or a 400.
pub fn check_wake_url(c: &mut dyn Calls, raw: &Arg) -> Result<(String, String)> {
    let s = if raw.is_nullish() { String::new() } else { js_text(raw) };
    let s = xenonet_base::text::js_trim(&s);
    if s.encode_utf16().count() > 2048 {
        return fail(400.0, "bad-request", "that URL is over 2,048 characters");
    }
    let Some(u) = parse_url(c, s)? else {
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

fn random_hex(n: usize) -> std::result::Result<String, Thrown> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    Ok(crate::tokens::hex(&b))
}

/// A value for JSON.stringify: undefined left out of an object.
fn json_of(a: &Arg) -> Option<Value> {
    match a {
        Arg::Undef | Arg::Other => None,
        Arg::Num(n) => Some(xenonet_base::json::number(*n)),
        Arg::List(l) => Some(Value::Array(l.iter().map(|v| json_of(v).unwrap_or(Value::Null)).collect())),
        Arg::Obj(o) => Some(Value::Object(o.iter().filter_map(|(k, v)| json_of(v).map(|v| (k.clone(), v))).collect())),
        other => Some(other.to_json()),
    }
}

/// The request a wake makes: its body says which actor and how many
/// messages wait, nothing more. `id` None for a new one.
pub fn wake_request(key: &str, actor: &Arg, unread: &Arg, version: &Arg, now: f64, id: Option<String>) -> std::result::Result<Arg, Thrown> {
    let id = match id {
        Some(id) => id,
        None => format!("msg_{}", random_hex(16)?),
    };
    let timestamp = (now / 1000.0).floor();
    if !now.is_finite() || now.abs() > 8.64e15 {
        return Err(Thrown::new("Invalid time value"));
    }
    let mut data = Map::new();
    if let Some(a) = json_of(actor) {
        data.insert("actor".into(), a);
    }
    if let Some(u) = json_of(unread) {
        data.insert("unread".into(), u);
    }
    let body = stringify(&json!({ "type": "xenonet.inbox", "timestamp": xenonet_base::time::iso(now), "data": Value::Object(data) }), 0);
    let stamp = js_text(&Arg::num(timestamp));
    let mut headers = vec![
        ("content-type".to_string(), Arg::text("application/json")),
        ("user-agent".to_string(), Arg::text(format!("XenoNet/{}", js_text(version)))),
        ("webhook-id".to_string(), Arg::text(id.clone())),
        ("webhook-timestamp".to_string(), Arg::text(stamp.clone())),
    ];
    if key.starts_with("whsec_") {
        headers.push(("webhook-signature".into(), Arg::text(wake_signature(key, &id, &stamp, &body))));
    } else {
        headers.push(("authorization".into(), Arg::text(format!("Bearer {key}"))));
    }
    Ok(Arg::Obj(vec![("method".into(), Arg::text("POST")), ("headers".into(), Arg::Obj(headers)), ("body".into(), Arg::text(body)), ("redirect".into(), Arg::text("manual"))]))
}

/// How a call went: { ok, status?, why?, reason? }.
fn out(ok: bool, status: Option<Arg>, why: Option<String>, reason: Option<String>) -> Arg {
    let mut f = vec![("ok".to_string(), Arg::Bool(ok))];
    if let Some(s) = status {
        f.push(("status".into(), s));
    }
    if let Some(w) = why {
        f.push(("why".into(), Arg::text(w)));
    }
    if let Some(r) = reason {
        f.push(("reason".into(), Arg::text(r)));
    }
    Arg::Obj(f)
}

/// Why a failed call failed: `why` in words, with the host, for you; and
/// `reason`, without it, for what anyone on this computer may read. Never
/// the URL's path or query, or the key. `code`: the error's cause's code,
/// its code or its name, the first there is.
fn failed(code: &Arg, host: &str) -> Arg {
    let secs = js_text(&Arg::num(TIMEOUT_MS / 1000.0));
    if matches!(code.as_str(), Some("TimeoutError" | "ABORT_ERR" | "UND_ERR_CONNECT_TIMEOUT")) {
        return out(false, None, Some(format!("{host} didn't answer within {secs} seconds")), Some(format!("no answer within {secs} seconds")));
    }
    let said = if code.truthy() { format!(" ({})", js_text(code)) } else { String::new() };
    out(false, None, Some(format!("XenoNet couldn't reach {host}{said}")), Some(format!("couldn't connect{said}")))
}

fn answered(status: &Arg, host: &str) -> Arg {
    let n = to_number(status);
    let s = js_text(status);
    if (200.0..300.0).contains(&n) {
        out(true, Some(status.clone()), None, None)
    } else if (300.0..400.0).contains(&n) {
        out(false, Some(status.clone()), Some(format!("{host} answered with a redirect (HTTP {s}), which XenoNet doesn't follow")), Some(format!("a redirect (HTTP {s})")))
    } else {
        out(false, Some(status.clone()), Some(format!("{host} answered HTTP {s}")), Some(format!("HTTP {s}")))
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
    last: Option<Arg>,
    failures: f64,
    not_before: f64,
    notified_at: f64,
    tested_at: f64,
    woke: Option<Vec<Cell>>,
    tokens: f64,
    tokens_at: f64,
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
    store: Shared,
    version: Arg,
    inner: Mutex<Inner>,
}

fn slug_of(id: &str) -> String {
    id.strip_prefix("actor:").unwrap_or(id).to_string()
}

/// A JS object's keys in the order JS gives them: array indexes first,
/// in order, then the rest as they came.
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

impl Wakes {
    pub fn new(store: Shared, version: Arg, now: f64) -> Self {
        Wakes { store, version, inner: Mutex::new(Inner { all_tokens: ALL_BURST, all_at: now, ..Inner::default() }) }
    }

    fn now(c: &mut dyn Calls) -> Result<f64> {
        Ok(to_number(&c.call("", "now", vec![])?))
    }

    fn log(c: &mut dyn Calls, detail: String) -> Result<()> {
        c.call("", "log", vec![Arg::text("actor.wake"), Arg::text(detail)])?;
        Ok(())
    }

    /// The actor's state, made when it has none.
    fn st(&self, c: &mut dyn Calls, id: &str) -> Result<()> {
        if self.inner.locked().state.iter().any(|(k, _)| k == id) {
            return Ok(());
        }
        let s = new_state(Self::now(c)?);
        let mut inner = self.inner.locked();
        if !inner.state.iter().any(|(k, _)| k == id) {
            inner.state.push((id.to_string(), s));
        }
        Ok(())
    }

    fn with<T>(&self, id: &str, f: impl FnOnce(&mut State) -> T) -> Option<T> {
        self.inner.locked().state.iter_mut().find(|(k, _)| k == id).map(|(_, s)| f(s))
    }

    fn configs(&self) -> Result<Value> {
        let meta = self.store.with(|s| s.meta(Arg::text(META)))?;
        let text = match meta {
            Value::Null => "{}".to_string(),
            Value::String(s) => s,
            other => js_text(&json_arg(&other)),
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
        self.store.with(|s| s.set_meta(Arg::text(META), Arg::text(stringify(&c, 0))))?;
        Ok(())
    }

    /// How an actor is woken: what you set, else Notify me for one that
    /// reaches XenoNet through the gateway, which nothing else wakes.
    fn kind_of(&self, slug: &str) -> Result<&'static str> {
        let config = self.config_of(slug)?;
        if let Some(k) = config.as_ref().and_then(|c| c.get("kind")).and_then(Value::as_str).and_then(|k| WAKE_KINDS.iter().find(|w| **w == k)) {
            return Ok(k);
        }
        Ok(if self.store.with(|s| s.gateway_token(Arg::text(slug)))?.is_some() { "notify" } else { "none" })
    }

    fn unseen(&self, id: &str) -> Result<Vec<Cell>> {
        Ok(self.store.with(|s| s.unseen(Arg::text(id)))?.iter().map(|m| m.get("id").clone()).collect())
    }

    fn unread(&self, id: &str) -> Result<usize> {
        Ok(self.unseen(id)?.len())
    }

    fn take_all(&self, c: &mut dyn Calls) -> Result<bool> {
        let t = Self::now(c)?;
        let mut i = self.inner.locked();
        i.all_tokens = ALL_BURST.min(i.all_tokens + ((t - i.all_at) / 1000.0) * ALL_RATE);
        i.all_at = t;
        if i.all_tokens < 1.0 || i.all_tokens.is_nan() {
            return Ok(false);
        }
        i.all_tokens -= 1.0;
        Ok(true)
    }

    fn registered(c: &mut dyn Calls, id: &str) -> Result<bool> {
        Ok(c.call("actors", "isRegistered", vec![Arg::text(id)])?.truthy())
    }

    fn set_of(&self, id: &str) -> Vec<u64> {
        let i = self.inner.locked();
        i.streams.iter().find(|(k, _)| k == id).and_then(|(_, n)| i.sets.get(n)).cloned().unwrap_or_default()
    }

    fn in_set(&self, id: &str, sink: u64) -> bool {
        self.set_of(id).contains(&sink)
    }

    // ---------- a message arrives ----------

    pub fn arrived(&self, c: &mut dyn Calls, id: &Arg) -> Result<()> {
        let id = js_text(id);
        if !id.starts_with("actor:") || !Self::registered(c, &id)? {
            return Ok(());
        }
        let n = self.unread(&id)?;
        let live = self.set_of(&id);
        if !live.is_empty() {
            for sink in live {
                if self.in_set(&id, sink) {
                    c.call("", "send", vec![Arg::num(sink as f64), Arg::text("inbox"), Arg::Obj(vec![("unread".into(), Arg::num(n as f64))])])?;
                }
            }
            return Ok(());
        }
        match self.kind_of(&slug_of(&id))? {
            "webhook" => self.schedule(c, &id),
            "notify" => self.schedule_notify(c, &id),
            _ => Ok(()),
        }
    }

    /// When its next wake may go: messages in a burst together, 30 seconds
    /// after the last, after a failure's wait, and not while the messages
    /// it was last woken for wait unread, for ten minutes.
    fn next_at(&self, c: &mut dyn Calls, id: &str) -> Result<f64> {
        self.st(c, id)?;
        let unseen = self.unseen(id)?;
        // Forgotten since st() made it sure: as st() would make it again.
        let s = match self.with(id, |s| s.clone()) {
            Some(s) => s,
            None => new_state(Self::now(c)?),
        };
        let same = |a: &Cell, b: &Cell| match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y,
            _ => a == b,
        };
        let held = match &s.woke {
            Some(w) if !w.is_empty() && w.iter().all(|m| unseen.iter().any(|u| same(u, m))) => s.last_at.unwrap_or(0.0) + UNREAD_HOLD_MS,
            _ => 0.0,
        };
        let refill = if s.tokens >= 1.0 { 0.0 } else { s.tokens_at + ((1.0 - s.tokens) / PER_ACTOR_RATE) * 1000.0 };
        let t = Self::now(c)?;
        let all = [t + COALESCE_MS, s.last_at.unwrap_or(0.0) + MIN_GAP_MS, s.not_before, held, refill];
        Ok(if all.iter().any(|x| x.is_nan()) { f64::NAN } else { all.iter().copied().fold(f64::NEG_INFINITY, f64::max) })
    }

    fn schedule(&self, c: &mut dyn Calls, id: &str) -> Result<()> {
        self.st(c, id)?;
        let (in_flight, timer) = self.with(id, |s| (s.in_flight, s.timer)).unwrap_or((false, false));
        if in_flight {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        if timer {
            return Ok(());
        }
        let at = self.next_at(c, id)?;
        let ms = at - Self::now(c)?;
        let set = c.call("", "setTimer", vec![Arg::text("wake"), Arg::text(id), Arg::num(ms)])?.truthy();
        self.with(id, |s| s.timer = set);
        Ok(())
    }

    fn schedule_notify(&self, c: &mut dyn Calls, id: &str) -> Result<()> {
        self.st(c, id)?;
        if self.with(id, |s| s.notify_timer).unwrap_or(false) {
            return Ok(());
        }
        let set = c.call("", "setTimer", vec![Arg::text("notify"), Arg::text(id), Arg::num(NOTIFY_AFTER_MS)])?.truthy();
        self.with(id, |s| s.notify_timer = set);
        Ok(())
    }

    /// A wake's timer came: whether to fire now.
    pub fn wake_due(&self, id: &Arg) -> bool {
        let id = js_text(id);
        self.with(&id, |s| {
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
    pub fn notify_due(&self, c: &mut dyn Calls, id: &Arg) -> Result<()> {
        let id = js_text(id);
        self.with(&id, |s| s.notify_timer = false);
        let n = self.unread(&id)?;
        let t = Self::now(c)?;
        if n == 0 || !Self::registered(c, &id)? || t - self.with(&id, |s| s.notified_at).unwrap_or(0.0) < NOTIFY_EVERY_MS {
            return Ok(());
        }
        self.with(&id, |s| s.notified_at = t);
        let info = c.call("actors", "info", vec![Arg::text(id.clone())])?;
        let name = info.field("name");
        let name = if name.is_nullish() { Arg::text(slug_of(&id)) } else { name };
        c.call("", "notify", vec![Arg::Obj(vec![("actor".into(), Arg::text(id.clone())), ("name".into(), name), ("unread".into(), Arg::num(n as f64))])])?;
        Ok(())
    }

    /// One of the actor's own wakes.
    fn take_one(&self, c: &mut dyn Calls, id: &str) -> Result<bool> {
        let t = Self::now(c)?;
        Ok(self
            .with(id, |s| {
                s.tokens = PER_ACTOR_BURST.min(s.tokens + ((t - s.tokens_at) / 1000.0) * PER_ACTOR_RATE);
                s.tokens_at = t;
                if s.tokens < 1.0 || s.tokens.is_nan() {
                    return false;
                }
                s.tokens -= 1.0;
                true
            })
            .unwrap_or(false))
    }

    /// Calls the webhook kept for `slug`, saying `n` messages wait. Its URL
    /// is checked again first, as it's read back from the store, and its
    /// host must be the one set in the window; an address that turns out to
    /// be link-local is refused.
    fn call(&self, c: &mut dyn Calls, id: &str, slug: &str, n: usize, t: f64) -> Result<Arg> {
        let got = c.call_all(vec![("secrets", "get", vec![Arg::text(wake_target(slug, "url"))]), ("secrets", "get", vec![Arg::text(wake_target(slug, "key"))])]);
        let mut got = got.into_iter().map(|r| r.unwrap_or(Arg::Null));
        let (url, key) = (got.next().unwrap_or(Arg::Null), got.next().unwrap_or(Arg::Null));
        // configs()[slug]?.host
        let set_host = |w: &Self| -> Result<Arg> { Ok(w.config_of(slug)?.and_then(|c| c.get("host").map(json_arg)).unwrap_or(Arg::Undef)) };
        let host = set_host(self)?;
        let host = if host.is_nullish() { "its webhook".to_string() } else { js_text(&host) };
        if !url.truthy() || !key.truthy() {
            return Ok(out(
                false,
                None,
                Some("its webhook isn't in your credential store any more; set its wake again".into()),
                Some("its webhook is missing from the credential store".into()),
            ));
        }
        let checked = match check_wake_url(c, &url) {
            Ok(v) => Some(v),
            Err(Fail::Wake { .. }) => None,
            Err(e) => return Err(e),
        };
        let wanted = set_host(self)?;
        let key_text = js_text(&key);
        let checked_url = match checked {
            Some((url, h)) if wanted.as_str() == Some(h.as_str()) && secret_ok(&key_text) => url,
            _ => {
                return Ok(out(
                    false,
                    None,
                    Some("the webhook in your credential store isn't the one set in XenoNet's window; set its wake again".into()),
                    Some("its webhook changed outside XenoNet".into()),
                ))
            }
        };
        let bare = match parse_url(c, &checked_url)? {
            Some(u) => unbracket(&u.hostname).to_string(),
            None => return Err(Thrown::new("Invalid URL").into()),
        };
        if !re!(r"^[\d.]+$").is_match(&bare) && !bare.contains(':') {
            let addrs = c.call("", "resolve", vec![Arg::text(bare.clone())]).unwrap_or(Arg::List(vec![]));
            if list(addrs).iter().any(|a| link_local(&js_text(&a.field("address")))) {
                return Ok(out(false, None, Some(format!("{host} is a link-local address, which a wake never goes to")), Some("a link-local address".into())));
            }
        }
        // A time that can't be written fails as the JS's RangeError did,
        // inside the call.
        let Ok(init) = wake_request(&key_text, &Arg::text(id), &Arg::num(n as f64), &self.version, t, None) else {
            return Ok(failed(&Arg::text("RangeError"), &host));
        };
        let res = c.call("", "fetch", vec![Arg::text(checked_url), init])?;
        let threw = res.field("failed");
        if !threw.is_nullish() {
            let code = ["cause", "code", "name"].iter().map(|k| threw.field(k)).find(|v| !v.is_nullish()).unwrap_or(Arg::Undef);
            return Ok(failed(&code, &host));
        }
        Ok(answered(&res.field("status"), &host))
    }

    /// How a wake went. Your test says so too, but neither holds the next
    /// wake back nor counts as a failure to wait out.
    fn record(&self, c: &mut dyn Calls, id: &str, t: f64, out: Arg, test: bool) -> Result<Arg> {
        let or_null = |k: &str| out.field(k).or(Arg::Null);
        let last = Arg::Obj(vec![
            ("ok".into(), out.field("ok")),
            ("at".into(), Arg::num(t)),
            ("status".into(), or_null("status")),
            ("why".into(), or_null("why")),
            ("reason".into(), or_null("reason")),
            ("test".into(), Arg::Bool(test)),
        ]);
        self.with(id, |s| s.last = Some(last));
        if test {
            return Ok(out);
        }
        let ok = out.field("ok").truthy();
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
            Self::log(c, format!("{id}: {}", js_text(&out.field("why"))))?;
        }
        Ok(out)
    }

    /// A wake for the messages waiting, when its time comes: one at a time.
    pub fn fire(&self, c: &mut dyn Calls, id: &Arg) -> Result<()> {
        let id = js_text(id);
        let slug = slug_of(&id);
        self.st(c, &id)?;
        let busy = self.with(&id, |s| {
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
        let r = self.fire_inner(c, &id, &slug);
        self.with(&id, |s| s.in_flight = false);
        let again = self.with(&id, |s| s.again).unwrap_or(false);
        if again && self.unread(&id)? > 0 {
            self.schedule(c, &id)?;
        }
        r
    }

    fn fire_inner(&self, c: &mut dyn Calls, id: &str, slug: &str) -> Result<()> {
        let unseen = self.unseen(id)?;
        if unseen.is_empty() || self.kind_of(slug)? != "webhook" || !Self::registered(c, id)? {
            return Ok(());
        }
        // Its time may have moved since it was set: the actor's own limit,
        // or the messages it was last woken for still unread.
        let at = self.next_at(c, id)?;
        if at > Self::now(c)? + COALESCE_MS {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        if !self.take_one(c, id)? {
            self.with(id, |s| s.again = true);
            return Ok(());
        }
        if !self.take_all(c)? {
            let t = Self::now(c)?;
            self.record(c, id, t, out(false, None, Some("XenoNet sends at most 120 wakes an hour".into()), Some("XenoNet's limit of 120 wakes an hour".into())), false)?;
            return Ok(());
        }
        let t = Self::now(c)?;
        let went = self.call(c, id, slug, unseen.len(), t)?;
        let went = self.record(c, id, t, went, false)?;
        if went.field("ok").truthy() {
            self.with(id, |s| s.woke = Some(unseen));
        }
        Ok(())
    }

    /// Your test: called now, even with nothing waiting. It doesn't hold
    /// the next wake back, nor stop one on its way.
    fn fire_test(&self, c: &mut dyn Calls, id: &str) -> Result<Arg> {
        self.st(c, id)?;
        if self.with(id, |s| s.testing).unwrap_or(false) {
            return fail(429.0, "too-many", "a test is on its way");
        }
        self.with(id, |s| s.testing = true);
        let r = (|| -> Result<Arg> {
            if !self.take_all(c)? {
                let t = Self::now(c)?;
                return self.record(c, id, t, out(false, None, Some("XenoNet sends at most 120 wakes an hour".into()), Some("XenoNet's limit of 120 wakes an hour".into())), true);
            }
            let t = Self::now(c)?;
            let n = self.unread(id)?;
            let went = self.call(c, id, &slug_of(id), n, t)?;
            self.record(c, id, t, went, true)
        })();
        self.with(id, |s| s.testing = false);
        r
    }

    // ---------- what's set ----------

    pub fn view(&self, id: &Arg) -> Result<Arg> {
        let id = js_text(id);
        let slug = slug_of(&id);
        let config = self.config_of(&slug)?;
        let c = config.clone().filter(|c| !c.is_null()).unwrap_or(json!({}));
        let s = self.with(&id, |s| s.clone());
        let kind = self.kind_of(&slug)?;
        let set = self.config_of(&slug)?.is_some_and(|c| json_arg(&c).truthy());
        let webhook = kind == "webhook";
        let host = if webhook { c.get("host").map_or(Arg::Null, json_arg).or(Arg::Null) } else { Arg::Null };
        let signed = if webhook { Arg::Bool(c.get("signed").is_some_and(|v| json_arg(v).truthy())) } else { Arg::Null };
        let listening = self.set_of(&id).len();
        let last = s.as_ref().and_then(|s| s.last.clone());
        Ok(Arg::Obj(vec![
            ("actor".into(), Arg::text(id.clone())),
            ("kind".into(), Arg::text(kind)),
            ("set".into(), Arg::Bool(set)),
            ("host".into(), host),
            ("signed".into(), signed),
            ("listening".into(), Arg::num(listening as f64)),
            ("lastAt".into(), last.as_ref().map_or(Arg::Null, |l| l.field("at").or(Arg::Null))),
            ("last".into(), last.unwrap_or(Arg::Null)),
            ("failures".into(), Arg::num(s.map_or(0.0, |s| s.failures))),
        ]))
    }

    fn forget_state(&self, c: &mut dyn Calls, id: &str) -> Result<()> {
        let had = self.inner.locked().state.iter().any(|(k, _)| k == id);
        if !had {
            return Ok(());
        }
        c.call("", "clearTimers", vec![Arg::text(id)])?;
        self.inner.locked().state.retain(|(k, _)| k != id);
        Ok(())
    }

    /// Each actor that reports itself, and how it's woken.
    pub fn list(&self, c: &mut dyn Calls) -> Result<Arg> {
        let mut ids: Vec<String> = Vec::new();
        let mut add = |id: String| {
            if !ids.contains(&id) {
                ids.push(id);
            }
        };
        for r in self.store.with(|s| s.actor_token_times())? {
            if json_arg(r.get("hasToken").unwrap_or(&Value::Null)).truthy() || json_arg(r.get("hasGateway").unwrap_or(&Value::Null)).truthy() {
                add(format!("actor:{}", js_text(&json_arg(r.get("slug").unwrap_or(&Value::Null)))));
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
            if Self::registered(c, &id)? {
                out.push(self.view(&Arg::text(id))?);
            }
        }
        Ok(Arg::List(out))
    }

    /// { kind, url, key }: a webhook takes both, kept in the credential
    /// store; notify and none forget them.
    pub fn set(&self, c: &mut dyn Calls, slug: &Arg, ask: &Arg) -> Result<Arg> {
        let slug = js_text(slug);
        let id = format!("actor:{slug}");
        if !Self::registered(c, &id)? {
            return fail(404.0, "not-found", "XenoNet has no actor by that slug that reports itself");
        }
        let kind = ask.field("kind");
        let Some(kind) = kind.as_str().filter(|k| WAKE_KINDS.contains(k)).map(str::to_string) else {
            return fail(400.0, "bad-request", format!("a wake is {}", WAKE_KINDS.join(", ")));
        };
        let mut configs = self.configs()?;
        let config = if kind == "webhook" {
            let (url, host) = check_wake_url(c, &ask.field("url"))?;
            if !secret_ok(&url) {
                return fail(400.0, "bad-request", "that URL has a character XenoNet can't keep, such as a quote or a backslash");
            }
            let key = ask.field("key");
            let key = if key.is_nullish() { String::new() } else { js_text(&key) };
            let key = xenonet_base::text::js_trim(&key).to_string();
            if !secret_ok(&key) {
                return fail(400.0, "bad-request", "a webhook's key is 8 to 1,024 printable characters, without spaces, quotes or backslashes");
            }
            if !c.call("secrets", "available", vec![])?.truthy() {
                let store = c.get("secrets.kind")?;
                let store = if store.is_nullish() { "a credential store".to_string() } else { js_text(&store) };
                return fail(501.0, "no-store", format!("XenoNet can't reach {store} on this computer, so it can't keep a webhook's key"));
            }
            c.call(
                "secrets",
                "set",
                vec![Arg::text(wake_target(&slug, "url")), Arg::text(url), Arg::Obj(vec![("comment".into(), Arg::text("The webhook XenoNet calls to wake an actor"))])],
            )?;
            c.call(
                "secrets",
                "set",
                vec![Arg::text(wake_target(&slug, "key")), Arg::text(key.clone()), Arg::Obj(vec![("comment".into(), Arg::text("The key XenoNet sends with an actor's wake"))])],
            )?;
            let t = Self::now(c)?;
            json!({ "kind": kind, "host": host, "signed": key.starts_with("whsec_"), "at": xenonet_base::json::number(t) })
        } else {
            let _ = c.call_all(vec![("secrets", "remove", vec![Arg::text(wake_target(&slug, "url"))]), ("secrets", "remove", vec![Arg::text(wake_target(&slug, "key"))])]);
            let t = Self::now(c)?;
            json!({ "kind": kind, "at": xenonet_base::json::number(t) })
        };
        if let Value::Object(m) = &mut configs {
            m.insert(slug.clone(), config);
        }
        self.save_configs(&configs)?;
        self.forget_state(c, &id)?;
        self.view(&Arg::text(id))
    }

    /// A wake now, for the owner to see the webhook work.
    pub fn test(&self, c: &mut dyn Calls, slug: &Arg) -> Result<Arg> {
        let slug = js_text(slug);
        let id = format!("actor:{slug}");
        if !Self::registered(c, &id)? {
            return fail(404.0, "not-found", "XenoNet has no actor by that slug that reports itself");
        }
        if self.kind_of(&slug)? != "webhook" {
            return fail(409.0, "no-webhook", "it has no webhook to call");
        }
        self.st(c, &id)?;
        let t = Self::now(c)?;
        if t - self.with(&id, |s| s.tested_at).unwrap_or(0.0) < TEST_GAP_MS {
            return fail(429.0, "too-many", "one test every five seconds");
        }
        let t = Self::now(c)?;
        self.with(&id, |s| s.tested_at = t);
        let went = self.fire_test(c, &id)?;
        let mut fields = match went {
            Arg::Obj(f) => f,
            _ => Vec::new(),
        };
        fields.retain(|(k, _)| k != "wake");
        fields.push(("wake".into(), self.view(&Arg::text(id))?));
        Ok(Arg::Obj(fields))
    }

    /// The actor left or was removed: its wake and its streams go.
    pub fn forget(&self, c: &mut dyn Calls, slug: &Arg) -> Result<()> {
        let slug = js_text(slug);
        let id = format!("actor:{slug}");
        let mut configs = self.configs()?;
        if let Value::Object(m) = &mut configs {
            if m.get(&slug).is_some_and(|v| json_arg(v).truthy()) {
                m.shift_remove(&slug);
                self.save_configs(&configs)?;
            }
        }
        let _ = c.call_all(vec![("secrets", "remove", vec![Arg::text(wake_target(&slug, "url"))]), ("secrets", "remove", vec![Arg::text(wake_target(&slug, "key"))])]);
        self.forget_state(c, &id)?;
        for sink in self.set_of(&id) {
            c.call("", "end", vec![Arg::num(sink as f64)])?;
        }
        self.inner.locked().streams.retain(|(k, _)| *k != id);
        Ok(())
    }

    /// A program of the actor's holding a stream open, as sink `sink`.
    /// What it's told first is how many messages wait. The set it joined,
    /// for unsubscribe.
    pub fn subscribe(&self, c: &mut dyn Calls, id: &Arg, sink: u64) -> Result<u64> {
        let id = js_text(id);
        let set = {
            let mut i = self.inner.locked();
            let existing = i.streams.iter().find(|(k, _)| *k == id).map(|(_, n)| *n);
            let set = match existing {
                Some(n) => n,
                None => {
                    i.next_set += 1;
                    let n = i.next_set;
                    i.sets.insert(n, Vec::new());
                    i.streams.push((id.clone(), n));
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
        let n = self.unread(&id)?;
        if n > 0 {
            c.call("", "send", vec![Arg::num(sink as f64), Arg::text("inbox"), Arg::Obj(vec![("unread".into(), Arg::num(n as f64))])])?;
        }
        Ok(set)
    }

    /// A stream closed: out of the set it joined, and the actor's streams
    /// gone with the last of them.
    pub fn unsubscribe(&self, id: &Arg, set: u64, sink: u64) {
        let id = js_text(id);
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

    pub fn stop(&self, c: &mut dyn Calls) -> Result<()> {
        let ids: Vec<String> = self.inner.locked().state.iter().map(|(k, _)| k.clone()).collect();
        for id in ids {
            self.forget_state(c, &id)?;
        }
        let sinks: Vec<u64> = {
            let i = self.inner.locked();
            i.streams.iter().flat_map(|(_, n)| i.sets.get(n).cloned().unwrap_or_default()).collect()
        };
        for sink in sinks {
            c.call("", "end", vec![Arg::num(sink as f64)])?;
        }
        self.inner.locked().streams.clear();
        Ok(())
    }
}

fn json_arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(json_arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), json_arg(v))).collect()),
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
