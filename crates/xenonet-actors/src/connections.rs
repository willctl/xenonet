//! Remote connections (docs/design/cloud-assistants.md, sections 3 and 4):
//! how a remote actor, such as an OpenAI dot in the cloud, reaches XenoNet
//! to read what's going on, and only that. core/actors/connections.js.
//!
//! A connection is one way an actor reaches XenoNet (a tunnel, or a local
//! MCP client), and its grant what it may read through it: the projects
//! you picked (none unless you pick some), whether names and titles go too
//! (off unless you turn them on), how far back, and until when (30 days
//! unless you choose otherwise). It belongs to an actor in the actors'
//! table, never to a registry of its own. Revoking a connection leaves its
//! actor, its history, its other connections and any observer of it as
//! they were; a registered actor's own token is never touched.
//!
//! Everything here is yours, from XenoNet's window or your own terminal,
//! never from a connection itself: pair, update, rotate, revoke, disable.
//! Until you've said you verified who can call a connection's endpoint
//! (owner_verified), it reads made-up data only (fixture). A grant stops
//! at once if this state folder turns up on another machine or account:
//! the device's id changes and every grant must be made again.
//!
//! The store is the engine's own. The rest is called as the JS called it,
//! through `Calls`: "actors" (the registry), "disk" (the credentials'
//! files), and the twin's "now", "emit", "describe", "taken",
//! "adapterFor" with "picked" (the adapter it picked), "writeCredential"
//! (with the I/O it was given), "wall" (Date.now), and for a pairing
//! "places" and "identities".

use crate::account_links::{self, LinkFail};
use crate::adapters::TRANSPORTS;
use crate::detail::looks_like_key;
use crate::names::{check_name, slug_from, slug_refusal, suggest_slug};
use crate::products::{product_of, Product};
use crate::registration::to_number;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use xenonet_base::ids::YOU;
use xenonet_base::json::stringify;
use xenonet_base::sync::Locked;
use xenonet_engine::peer::{list, Calls, Thrown};
use xenonet_engine::re;
use xenonet_engine::sql::{Arg, Cell, Row, SqlError};
use xenonet_engine::store::{js_text, Shared};

const DAY_MS: f64 = 24.0 * 3600e3;
pub const GRANT_DAYS: [f64; 6] = [1.0, 7.0, 14.0, 30.0, 60.0, 90.0];
pub const DEFAULT_GRANT_DAYS: f64 = 30.0;
/// How far back a grant reads, at most; you can make it less.
pub const LOOKBACK_DAYS: f64 = 30.0;
/// Remote actors beside the 16 registered ones, counted once each however
/// many connections they have; connections that are active, and the
/// projects one grant names.
pub const REMOTE_ACTORS_MAX: usize = 8;
pub const CONNECTIONS_MAX: usize = 16;
pub const PROJECTS_MAX: usize = 64;
/// When a grant was last used is written at most this often.
const TOUCH_MS: f64 = 60e3;
pub const TRANSPORT_SCOPES: [&str; 3] = ["owner_endpoint", "authenticated_user", "other_approved_scope"];
pub const DEFAULT_PORT: f64 = 47840.0;

/// A refusal a program can test (ConnectionError): an HTTP status, a code,
/// a sentence for a person, and extra fields.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Connection { status: f64, code: &'static str, message: String, extra: Value },
    Link { code: &'static str, message: String },
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

impl From<LinkFail> for Fail {
    fn from(f: LinkFail) -> Self {
        match f {
            LinkFail::Link { code, message } => Fail::Link { code, message },
            LinkFail::Thrown(t) => Fail::Thrown(t),
        }
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn fail<T>(status: f64, code: &'static str, message: impl Into<String>) -> Result<T> {
    Err(Fail::Connection { status, code, message: message.into(), extra: json!({}) })
}

fn random_hex(n: usize) -> std::result::Result<String, Thrown> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    Ok(crate::tokens::hex(&b))
}

fn new_id(prefix: &str) -> std::result::Result<String, Thrown> {
    Ok(format!("{prefix}_{}", random_hex(12)?))
}

/// `<prefix>_` and crypto.randomUUID().
fn uuid_id(prefix: &str) -> std::result::Result<String, Thrown> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::tokens::hex(&b);
    Ok(format!("{prefix}_{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

/// A column as JS reads it.
fn val(c: &Cell) -> Arg {
    match c {
        Cell::Int(i) => Arg::Num(*i as f64),
        other => other.arg(),
    }
}

/// A row as JS reads it.
pub fn row_arg(r: &Row) -> Arg {
    Arg::Obj(r.0.iter().map(|(k, c)| (k.clone(), val(c))).collect())
}

/// Number(cell), as JS compares a column with a number.
fn num(c: &Cell) -> f64 {
    match c {
        Cell::Null => 0.0,
        Cell::Text(s) => xenonet_base::text::js_number(Some(&Value::String(s.clone()))),
        Cell::Blob(_) => f64::NAN,
        c => c.as_f64().unwrap_or(f64::NAN),
    }
}

/// `a === b` for two columns.
fn same_cell(a: &Cell, b: &Cell) -> bool {
    match (a, b) {
        (Cell::Text(x), Cell::Text(y)) => x == y,
        (Cell::Null, Cell::Null) => true,
        (x, y) => matches!((x.as_f64(), y.as_f64()), (Some(p), Some(q)) if p == q),
    }
}

/// JSON.parse(s), or `fallback` for null or what doesn't parse.
fn parse(c: &Cell, fallback: Value) -> Value {
    let text = match c {
        Cell::Null => return fallback,
        Cell::Text(s) => s.clone(),
        other => js_text(&val(other)),
    };
    serde_json::from_str(&text).unwrap_or(fallback)
}

fn iso(c: &Cell) -> Arg {
    match c {
        Cell::Null => Arg::Null,
        c => Arg::text(xenonet_base::time::iso(num(c))),
    }
}

/// Math.round.
fn js_round(x: f64) -> f64 {
    let r = x.floor();
    if x - r >= 0.5 {
        r + 1.0
    } else {
        r
    }
}

/// Node's Buffer.from(text, "hex"): pairs of hex digits until the first
/// that isn't one.
fn hex_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::new();
    for pair in b.chunks_exact(2) {
        match (digit(pair[0]), digit(pair[1])) {
            (Some(h), Some(l)) => out.push(h << 4 | l),
            _ => break,
        }
    }
    out
}

/// crypto.timingSafeEqual for two of the same length.
fn timing_safe_eq(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_grant_id(v: &Arg) -> bool {
    re!(r"^gr_[0-9a-f]{24}$").is_match(&js_text(v))
}

/// One line of your own words (a connection's name, a computer's, who can
/// call an endpoint), or None for none; never a key.
fn words(value: &Arg, what: &str, max: usize) -> Result<Option<String>> {
    let text = match value {
        Arg::Undef | Arg::Null => return Ok(None),
        Arg::Text(s) if s.is_empty() => return Ok(None),
        Arg::Text(s) => s,
        _ => return fail(400.0, "bad-request", format!("{what} is text")),
    };
    let blanked = re!(r"[\u0000-\u001f\u007f-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]").replace_all(text, " ");
    let line = re!(r"\s+").replace_all(&blanked, " ");
    let line = xenonet_base::text::js_trim(&line);
    if line.is_empty() {
        return Ok(None);
    }
    if line.chars().count() > max {
        return fail(400.0, "bad-request", format!("{what} is at most {max} characters"));
    }
    if looks_like_key(line) {
        return fail(400.0, "bad-request", format!("{what} looks like it holds a key or a token"));
    }
    Ok(Some(line.to_string()))
}

/// This installation as it is: its machine, account and state folder,
/// hashed. Never sent anywhere.
pub fn fingerprint_of(host: &Arg, user: &Arg, state_dir: &Arg) -> String {
    use sha2::{Digest, Sha256};
    let part = |a: &Arg| if a.is_nullish() { String::new() } else { js_text(a) };
    let joined = [part(host), part(user), part(state_dir)].join("\0");
    crate::tokens::hex(&Sha256::digest(joined.as_bytes()))
}

/// What the tunnel client runs for a grant: the adapter, holding no secret.
pub fn launch_for(grant: &Arg, port: &Arg) -> Arg {
    let mut args = vec![Arg::text("mcp"), Arg::text("assistant"), Arg::text("--grant"), grant.clone()];
    if !matches!(port, Arg::Num(p) if *p == DEFAULT_PORT) {
        args.push(Arg::text("--port"));
        args.push(Arg::text(js_text(port)));
    }
    Arg::Obj(vec![("command".into(), Arg::text("xenonet")), ("args".into(), Arg::List(args))])
}

fn obj(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn value_arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(value_arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), value_arg(v))).collect()),
    }
}

/// A parsed list's items (`for (const p of parse(...))`).
fn items(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        Value::String(s) => s.chars().map(|c| Value::String(c.to_string())).collect(),
        _ => Vec::new(),
    }
}

/// `[...new Set(list)]` of what a caller sent.
fn unique(a: &Arg) -> Vec<Arg> {
    let all = match a {
        Arg::List(l) => l.clone(),
        Arg::Text(s) => s.chars().map(|c| Arg::text(c.to_string())).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<Arg> = Vec::new();
    for v in all {
        let seen = out.iter().any(|o| match (o, &v) {
            (Arg::Num(x), Arg::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
            (Arg::Obj(_) | Arg::List(_), _) => false,
            (o, v) => o == v,
        });
        if !seen {
            out.push(v);
        }
    }
    out
}

/// A product by the id a value holds (`productOf(x)`).
fn product(v: &Arg) -> Option<&'static Product> {
    v.as_str().and_then(product_of)
}

/// What an adapter the twin picked comes to.
struct Adapter {
    id: Arg,
    product: Arg,
    provider: Arg,
    transports: Vec<Arg>,
}

/// What a pairing makes, before anything is changed: an actor already
/// here, or a new one.
struct Plan {
    id: String,
    create: Option<Arg>,
}

/// How a grant stands now, for a call through it.
pub enum Standing {
    No { code: &'static str, reason: Arg },
    Ok { grant: Row, connection: Row, actor_id: String, revision: Cell },
}

impl Standing {
    fn no(code: &'static str, reason: &str) -> Self {
        Standing::No { code, reason: Arg::text(reason) }
    }
    pub fn to_arg(&self) -> Arg {
        match self {
            Standing::No { code, reason } => obj(vec![("ok", Arg::Bool(false)), ("code", Arg::text(*code)), ("reason", reason.clone())]),
            Standing::Ok { grant, connection, actor_id, revision } => obj(vec![
                ("ok", Arg::Bool(true)),
                ("grant", row_arg(grant)),
                ("connection", row_arg(connection)),
                ("actorId", Arg::text(actor_id.clone())),
                ("revision", val(revision)),
            ]),
        }
    }
}

pub struct Connections {
    store: Shared,
    dir: Arg,
    fingerprint: Arg,
    port: Arg,
    /// Adapters connected now, by connection.
    bridges: Mutex<Vec<(String, f64)>>,
    touched: Mutex<HashMap<String, f64>>,
    device_id: Mutex<Option<Value>>,
    expired: Mutex<HashSet<String>>,
}

impl Connections {
    pub fn new(store: Shared, dir: Arg, fingerprint: Arg, port: Arg) -> Self {
        Connections { store, dir, fingerprint, port, bridges: Mutex::default(), touched: Mutex::default(), device_id: Mutex::default(), expired: Mutex::default() }
    }

    /// The first look: this device, the tombstones, and what each actor's
    /// connections come to.
    pub fn start(&self, c: &mut dyn Calls) -> Result<()> {
        self.device(c)?;
        self.apply_tombstones(c)?;
        let t = Self::now(c)?;
        self.sync(c, t)
    }

    fn now(c: &mut dyn Calls) -> Result<f64> {
        Ok(to_number(&c.call("", "now", vec![])?))
    }

    fn emit(c: &mut dyn Calls, name: &str, what: Arg) -> Result<()> {
        c.call("", "emit", vec![Arg::text(name), what])?;
        Ok(())
    }

    fn disk(&self, c: &mut dyn Calls, method: &str, mut args: Vec<Arg>) -> Result<Arg> {
        args.insert(0, self.dir.clone());
        Ok(c.call("disk", method, args)?)
    }

    fn bridged(&self, connection: &Cell) -> bool {
        let id = js_text(&val(connection));
        self.bridges.locked().iter().any(|(k, n)| *k == id && *n > 0.0)
    }

    // ---------- this device ----------

    pub fn device(&self, c: &mut dyn Calls) -> Result<Value> {
        if let Some(id) = self.device_id.locked().clone().filter(|id| value_arg(id).truthy()) {
            return Ok(id);
        }
        let had = self.store.with(|s| s.meta(Arg::text("device_id")))?;
        let had_some = value_arg(&had).truthy();
        if had_some && value_arg(&self.store.with(|s| s.meta(Arg::text("device_fingerprint")))?) == self.fingerprint {
            *self.device_id.locked() = Some(had.clone());
            return Ok(had);
        }
        let t = Self::now(c)?;
        let id = uuid_id("dev")?;
        let key = random_hex(32)?;
        self.store.transaction(|| -> Result<()> {
            self.store.with(|s| -> std::result::Result<(), SqlError> {
                s.set_meta(Arg::text("device_id"), Arg::text(id.clone()))?;
                s.set_meta(Arg::text("device_fingerprint"), self.fingerprint.clone())?;
                if !value_arg(&s.meta(Arg::text("id_key"))?).truthy() {
                    s.set_meta(Arg::text("id_key"), Arg::text(key.clone()))?;
                }
                Ok(())
            })?;
            Ok(())
        })?;
        *self.device_id.locked() = Some(Value::String(id.clone()));
        // Moved or copied: every grant made for the old one stops.
        if had_some {
            for g in self.store.with(|s| s.grants())? {
                if g.get("revoked_at").is_null() {
                    self.revoke_row(c, &g, &Arg::text("device-changed"), t, "system")?;
                }
            }
        }
        Ok(Value::String(id))
    }

    /// A key for ids that mustn't say what they're made of, local to this
    /// installation.
    pub fn id_key(&self, c: &mut dyn Calls) -> Result<Value> {
        self.device(c)?;
        Ok(self.store.with(|s| s.meta(Arg::text("id_key")))?)
    }

    // ---------- standing ----------

    pub fn disabled(&self, c: &mut dyn Calls) -> Result<bool> {
        Ok(self.disk(c, "isDisabled", vec![])?.truthy())
    }

    fn state_of(&self, c: &mut dyn Calls, g: &Row, t: f64) -> Result<&'static str> {
        if !g.get("revoked_at").is_null() || self.disk(c, "isMarkedRevoked", vec![val(g.get("grant_id"))])?.truthy() {
            return Ok("revoked");
        }
        if self.disabled(c)? {
            return Ok("disabled");
        }
        if num(g.get("expires_at")) <= t {
            return Ok("expired");
        }
        Ok("active")
    }

    /// What each actor's connections come to, for the map and the Actors
    /// panel (actors.setRemote).
    pub fn sync(&self, c: &mut dyn Calls, t: f64) -> Result<()> {
        let grants = self.store.with(|s| s.grants())?;
        // By actor slug: each connection, with its grant if it has one.
        type Entries = Vec<(Row, Option<Row>)>;
        let mut by_slug: Vec<(String, Entries)> = Vec::new();
        for conn in self.store.with(|s| s.connections())? {
            let g = grants.iter().find(|x| same_cell(x.get("connection_id"), conn.get("connection_id"))).cloned();
            let slug = js_text(&val(conn.get("actor_slug")));
            match by_slug.iter_mut().find(|(s, _)| *s == slug) {
                Some((_, l)) => l.push((conn, g)),
                None => by_slug.push((slug, vec![(conn, g)])),
            }
        }
        for id in list(c.call("actors", "ids", vec![])?) {
            let id_text = js_text(&id);
            let slug: String = id_text.encode_utf16().skip("actor:".len()).collect::<Vec<u16>>().iter().map(|u| char::from_u32(u32::from(*u)).unwrap_or('\u{fffd}')).collect();
            let Some((_, entries)) = by_slug.iter().find(|(s, _)| *s == slug) else {
                if c.call("actors", "remoteOf", vec![id.clone()])?.truthy() {
                    c.call("actors", "setRemote", vec![id.clone(), Arg::Null])?;
                }
                continue;
            };
            // Each active connection, with its grant.
            let mut active: Vec<(&Row, &Row)> = Vec::new();
            for (conn, g) in entries {
                if let Some(g) = g {
                    if self.state_of(c, g, t)? == "active" {
                        active.push((conn, g));
                    }
                }
            }
            let bridge = if active.iter().any(|(conn, _)| self.bridged(conn.get("connection_id"))) { "connected" } else { "not_connected" };
            let (link, _) = account_links::link_state(&self.store, &Arg::text(slug.clone()), t)?;
            let mut permissions: Vec<Value> = Vec::new();
            for (_, g) in &active {
                let parsed = parse(g.get("permissions"), json!([]));
                let each = match parsed {
                    Value::Array(a) => a,
                    other => vec![other],
                };
                for p in each {
                    let seen = !matches!(p, Value::Object(_) | Value::Array(_)) && permissions.contains(&p);
                    if !seen {
                        permissions.push(p);
                    }
                }
            }
            let remote = obj(vec![
                ("active", Arg::Bool(!active.is_empty())),
                ("connections", Arg::num(entries.len() as f64)),
                ("bridge", Arg::text(bridge)),
                ("accountLink", Arg::text(link)),
                ("permissions", Arg::List(permissions.iter().map(value_arg).collect())),
                ("transports", Arg::List(active.iter().map(|(conn, _)| val(conn.get("transport_kind"))).collect())),
            ]);
            c.call("actors", "setRemote", vec![id.clone(), remote])?;
        }
        Ok(())
    }

    /// A grant's revision as it is now, on disk too, where an adapter with
    /// no service to ask looks before it serves anything it read earlier.
    fn note_revision(&self, c: &mut dyn Calls, grant_id: &Arg) -> Result<()> {
        if let Some(g) = self.store.with(|s| s.grant(grant_id.clone()))? {
            self.disk(c, "markRevision", vec![grant_id.clone(), val(g.get("revision"))])?;
        }
        Ok(())
    }

    fn revoke_row(&self, c: &mut dyn Calls, g: &Row, reason: &Arg, t: f64, by: &str) -> Result<bool> {
        let grant = val(g.get("grant_id"));
        let connection = val(g.get("connection_id"));
        self.disk(c, "markRevoked", vec![grant.clone(), Arg::num(t)])?;
        self.disk(c, "deleteCredential", vec![grant.clone()])?;
        let changed = self.store.transaction(|| -> Result<bool> {
            let r = self.store.with(|s| s.revoke_grant(grant.clone(), reason.clone(), Arg::num(t)))?;
            self.store.with(|s| s.end_connection(connection.clone(), Arg::num(t)))?;
            let slug = self.store.with(|s| s.connection(connection.clone()))?.map_or("?".to_string(), |r| match r.get("actor_slug") {
                Cell::Null => "?".to_string(),
                c => js_text(&val(c)),
            });
            let audit = obj(vec![
                ("at", Arg::num(t)),
                ("kind", Arg::text(by)),
                ("grant_id", grant.clone()),
                ("grant_revision", val(g.get("revision"))),
                ("actor", Arg::text(format!("actor:{slug}"))),
                ("connection_id", connection.clone()),
                ("operation", Arg::text("revoke")),
                ("result", reason.clone()),
            ]);
            self.store.with(|s| s.add_audit(&audit))?;
            Ok(r)
        })?;
        self.note_revision(c, &grant)?;
        Self::emit(c, "revoked", obj(vec![("grant", grant), ("connection", connection), ("reason", reason.clone())]))?;
        Ok(changed)
    }

    /// A grant revoked while the service was stopped, or one a restored
    /// database has as active: the tombstone wins.
    fn apply_tombstones(&self, c: &mut dyn Calls) -> Result<()> {
        let t = Self::now(c)?;
        for id in list(self.disk(c, "markedRevoked", vec![])?) {
            if let Some(g) = self.store.with(|s| s.grant(id.clone()))? {
                if g.get("revoked_at").is_null() {
                    self.revoke_row(c, &g, &Arg::text("revoked-offline"), t, "owner")?;
                }
            }
        }
        Ok(())
    }

    // ---------- pairing ----------

    fn remote_actors(c: &mut dyn Calls) -> Result<usize> {
        let mut n = 0;
        for id in list(c.call("actors", "ids", vec![])?) {
            if c.call("actors", "isRemote", vec![id.clone()])?.truthy() && c.call("actors", "remoteOf", vec![id.clone()])?.field("active").truthy() {
                n += 1;
            }
        }
        Ok(n)
    }

    fn active_grants(&self, c: &mut dyn Calls, t: f64) -> Result<usize> {
        let mut n = 0;
        for g in self.store.with(|s| s.grants())? {
            if self.state_of(c, &g, t)? == "active" {
                n += 1;
            }
        }
        Ok(n)
    }

    fn is_taken(c: &mut dyn Calls, slug: &str) -> std::result::Result<bool, Thrown> {
        Ok(c.call("actors", "has", vec![Arg::text(format!("actor:{slug}"))])?.truthy() || c.call("", "taken", vec![Arg::text(slug)])?.truthy())
    }

    /// The actor a pairing is for, without changing anything yet: one
    /// already here, as you picked it, or a new one. A name a caller sends
    /// never picks one.
    fn plan_actor(c: &mut dyn Calls, ask: &Arg, adapter: &Adapter) -> Result<Plan> {
        let given = ask.field("actor");
        if !given.is_nullish() {
            let id = js_text(&given);
            let info = c.call("actors", "info", vec![Arg::text(id.clone())])?;
            let demo = list(info.field("via")).iter().any(|v| v.as_str() == Some("demo"));
            if !info.truthy() || id == YOU || demo {
                return fail(404.0, "not-found", "XenoNet has no such actor to pair");
            }
            let theirs = product(&info.field("product").field("id"));
            let matches = match (theirs, &adapter.product) {
                (Some(p), Arg::Text(a)) => p.id == a,
                (None, Arg::Null) => true,
                _ => false,
            };
            if !matches {
                let wanted = if adapter.product.truthy() { product(&adapter.product).map_or("undefined", |p| p.name).to_string() } else { "an actor of your own".to_string() };
                return fail(400.0, "bad-request", format!("{} isn't {wanted}", js_text(&info.field("name"))));
            }
            return Ok(Plan { id, create: None });
        }
        if Self::remote_actors(c)? >= REMOTE_ACTORS_MAX {
            return fail(409.0, "full", format!("{REMOTE_ACTORS_MAX} remote actors are paired already; revoke or remove one first"));
        }
        let made = product(&adapter.product);
        let (slug, name) = if let Some(p) = made {
            let slug = if p.id == "dots" {
                let mut slug = "dots".to_string();
                let mut i = 0;
                while Self::is_taken(c, &slug)? {
                    if i > 50 {
                        return fail(409.0, "taken", "XenoNet couldn't find a free slug for another dot");
                    }
                    slug = format!("dots-{}", random_hex(2)?);
                    i += 1;
                }
                slug
            } else {
                if Self::is_taken(c, p.slug)? {
                    return fail(409.0, "taken", format!("{} is beside the map already, as actor:{}: pair that actor instead", p.name, p.slug));
                }
                p.slug.to_string()
            };
            let raw = ask.field("name");
            let raw = if raw.is_nullish() { Some(p.short.to_string()) } else { raw.as_str().map(str::to_string) };
            let name = match check_name(raw.as_deref(), Some(p.id)) {
                Ok(n) => n,
                Err(reason) => return fail(400.0, "bad-request", reason),
            };
            (slug, name)
        } else {
            let name = match check_name(ask.field("name").as_str(), None) {
                Ok(n) => n,
                Err(reason) => return fail(400.0, "bad-request", reason),
            };
            let given = ask.field("slug");
            let slug = if given.is_nullish() { Arg::text(slug_from(&name)) } else { given };
            if let Some(why) = slug_refusal(slug.as_str(), None) {
                return fail(400.0, "bad-request", why);
            }
            let slug = js_text(&slug);
            if Self::is_taken(c, &slug)? {
                let suggest = suggest_slug(&slug, None, &mut |s| Self::is_taken(c, s))?;
                return Err(Fail::Connection { status: 409.0, code: "taken", message: format!("actor:{slug} is taken"), extra: json!({ "suggest": suggest }) });
            }
            (slug, name)
        };
        let product = made.map_or(Arg::Null, |p| obj(vec![("id", Arg::text(p.id)), ("vendor", Arg::text(p.vendor)), ("version", Arg::Null)]));
        Ok(Plan { id: format!("actor:{slug}"), create: Some(obj(vec![("kind", Arg::text("agent")), ("name", Arg::text(name)), ("product", product)])) })
    }

    fn adapter_for(c: &mut dyn Calls, product: Arg) -> Result<Option<Adapter>> {
        let a = c.call("", "adapterFor", vec![product])?;
        if !a.truthy() {
            return Ok(None);
        }
        Ok(Some(Adapter { id: a.field("id"), product: a.field("product"), provider: a.field("provider"), transports: list(a.field("transports")) }))
    }

    /// A new connection and its grant (the window's Pair). "places": the
    /// places you may pick, by node id; "identities": the identities
    /// XenoNet knows, for a link that's your word. Your view of it, with
    /// its launch command; the credential is only ever on disk.
    pub fn pair(&self, c: &mut dyn Calls, ask: &Arg) -> Result<Arg> {
        let t = Self::now(c)?;
        let Some(adapter) = Self::adapter_for(c, ask.field("product").or(Arg::Null))? else {
            return fail(400.0, "bad-request", "that product can't be paired as a remote actor");
        };
        let transport = ask.field("transport");
        let known = TRANSPORTS.iter().any(|x| transport.as_str() == Some(x.id));
        if !known || !adapter.transports.contains(&transport) {
            let who = if adapter.product.truthy() { product(&adapter.product).map_or("undefined", |p| p.name) } else { "An actor of your own" };
            let names: Vec<&str> = adapter.transports.iter().map(|x| TRANSPORTS.iter().find(|t| x.as_str() == Some(t.id)).map_or("undefined", |t| t.name)).collect();
            return fail(400.0, "bad-request", format!("{who} reaches XenoNet through {}", names.join(" or ")));
        }
        let Some(label) = words(&ask.field("label"), "a connection's name", 64)? else {
            return fail(400.0, "bad-request", "a connection needs a name");
        };
        let alias = words(&ask.field("deviceAlias"), "this computer's name", 64)?;
        let audience = ask.field("audience");
        let Some(description) = words(&audience.field("description"), "who can call it", 200)? else {
            return fail(400.0, "bad-request", "say who can call this connection's endpoint");
        };
        let verification = audience.field("verification");
        if !matches!(verification.as_str(), Some("owner_verified" | "unverified")) {
            return fail(400.0, "bad-request", "say whether you verified who can call it: owner_verified or unverified");
        }
        let scope = audience.field("scope");
        if !scope.as_str().is_some_and(|s| TRANSPORT_SCOPES.contains(&s)) {
            return fail(400.0, "bad-request", format!("its scope is {}", TRANSPORT_SCOPES.join(", ")));
        }
        let days = ask.field("days").or(Arg::num(DEFAULT_GRANT_DAYS));
        let Arg::Num(days) = days else {
            return fail(400.0, "bad-request", format!("a grant lasts {} days", GRANT_DAYS.map(|d| d.to_string()).join(", ")));
        };
        if !GRANT_DAYS.contains(&days) {
            return fail(400.0, "bad-request", format!("a grant lasts {} days", GRANT_DAYS.map(|d| d.to_string()).join(", ")));
        }
        let lookback = ask.field("lookbackDays").or(Arg::num(LOOKBACK_DAYS));
        let lookback = match lookback {
            Arg::Num(n) if n.is_finite() && n.fract() == 0.0 && (1.0..=LOOKBACK_DAYS).contains(&n) => n,
            _ => return fail(400.0, "bad-request", format!("it reads back 1 to {LOOKBACK_DAYS} days")),
        };
        let chosen = unique(&ask.field("projects").or(Arg::List(vec![])));
        if chosen.len() > PROJECTS_MAX {
            return fail(400.0, "bad-request", format!("a grant names at most {PROJECTS_MAX} projects"));
        }
        for p in &chosen {
            if !c.call("places", "has", vec![p.clone()])?.truthy() {
                return fail(400.0, "bad-request", "pick projects from the map as it is now");
            }
        }
        let account_link = ask.field("accountLink");
        let link = account_link.field("state").or(Arg::text("unverified"));
        if link.as_str() == Some("verified") {
            return fail(400.0, "bad-request", "only its provider can verify an account; this connection's can't yet, so choose unverified or your own word");
        }
        if !matches!(link.as_str(), Some("unverified" | "owner_asserted")) {
            return fail(400.0, "bad-request", "an account link is unverified or owner_asserted");
        }
        let asserted = link.as_str() == Some("owner_asserted");
        if asserted && !c.call("identities", "has", vec![account_link.field("identity")])?.truthy() {
            return fail(400.0, "bad-request", "pick one of the identities XenoNet knows");
        }
        if self.active_grants(c, t)? >= CONNECTIONS_MAX {
            return fail(409.0, "full", format!("{CONNECTIONS_MAX} connections are active already; revoke one first"));
        }
        let plan = Self::plan_actor(c, ask, &adapter)?;

        let dev = value_arg(&self.device(c)?);
        let connection_id = new_id("cn")?;
        let grant_id = new_id("gr")?;
        let made = c.call("disk", "newToken", vec![])?;
        let expires = t + days * DAY_MS;
        // The credential first: if it can't be kept private, nothing is made.
        let credential = obj(vec![("grant", Arg::text(grant_id.clone())), ("token", made.field("token")), ("device", dev.clone()), ("expiresAt", Arg::num(expires))]);
        c.call("", "writeCredential", vec![credential])?;
        let made_all = self.store.transaction(|| -> Result<()> {
            match &plan.create {
                Some(create) => {
                    let Arg::Obj(fields) = create else { unreachable!() };
                    let mut def = vec![("id".to_string(), Arg::text(plan.id.clone()))];
                    def.extend(fields.iter().cloned());
                    def.extend([
                        ("via".to_string(), Arg::List(vec![Arg::text("remote")])),
                        ("heartbeatMs".to_string(), Arg::Null),
                        ("capabilities".to_string(), Arg::List(vec![])),
                        ("identity".to_string(), Arg::Null),
                        ("at".to_string(), Arg::num(t)),
                    ]);
                    c.call("actors", "upsert", vec![Arg::Obj(def)])?;
                }
                None => {
                    c.call("actors", "addVia", vec![Arg::text(plan.id.clone()), Arg::text("remote")])?;
                }
            }
            let slug: String = plan.id.chars().skip("actor:".len()).collect();
            let mut projects = Vec::new();
            for place in &chosen {
                projects.push(self.store.with(|s| s.project_id_of(place.clone(), &mut || -> Result<Arg> { Ok(Arg::text(uuid_id("prj")?)) }, Arg::num(t)))?);
            }
            let connection = obj(vec![
                ("connection_id", Arg::text(connection_id.clone())),
                ("actor_slug", Arg::text(slug.clone())),
                ("adapter_id", adapter.id.clone()),
                ("provider", adapter.provider.clone()),
                ("transport_kind", transport.clone()),
                ("transport_binding", Arg::Null),
                ("label", Arg::text(label.clone())),
                ("device_alias", Arg::opt_text(alias.as_deref())),
                ("audience_description", Arg::text(description.clone())),
                ("audience_verification", verification.clone()),
                ("audience_scope", scope.clone()),
                ("created_at", Arg::num(t)),
            ]);
            self.store.with(|s| s.add_connection(&connection))?;
            let capabilities = c.call("picked", "capabilities", vec![])?;
            let grant = obj(vec![
                ("grant_id", Arg::text(grant_id.clone())),
                ("connection_id", Arg::text(connection_id.clone())),
                ("device_id", dev.clone()),
                ("permissions", Arg::text(stringify(&capabilities.to_json(), 0))),
                ("project_allowlist", Arg::text(stringify(&Value::Array(projects.clone()), 0))),
                ("field_policy", Arg::text(stringify(&json!({ "labels": ask.field("labels") == Arg::Bool(true) }), 0))),
                ("data_mode", Arg::text(if verification.as_str() == Some("owner_verified") { "real" } else { "fixture" })),
                ("lookback_ms", Arg::num(lookback * DAY_MS)),
                ("token_hash", made.field("hash")),
                ("created_at", Arg::num(t)),
                ("expires_at", Arg::num(expires)),
            ]);
            self.store.with(|s| s.add_grant(&grant))?;
            if asserted {
                let known = list(c.get("identities")?);
                account_links::assert_link(&self.store, &Arg::text(slug.clone()), &account_link.field("identity"), &adapter.provider, Some(&known), &Arg::num(t))?;
            }
            let audit = obj(vec![
                ("at", Arg::num(t)),
                ("kind", Arg::text("owner")),
                ("grant_id", Arg::text(grant_id.clone())),
                ("grant_revision", Arg::num(1.0)),
                ("actor", Arg::text(plan.id.clone())),
                ("connection_id", Arg::text(connection_id.clone())),
                ("operation", Arg::text("pair")),
                ("result", Arg::text("ok")),
                ("projects", Arg::num(projects.len() as f64)),
            ]);
            self.store.with(|s| s.add_audit(&audit))?;
            Ok(())
        });
        if let Err(e) = made_all {
            self.disk(c, "deleteCredential", vec![Arg::text(grant_id.clone())])?;
            return Err(e);
        }
        let grant = Arg::text(grant_id.clone());
        self.note_revision(c, &grant)?;
        self.sync(c, t)?;
        Self::emit(c, "changed", obj(vec![("grant", grant.clone())]))?;
        let g = self.store.with(|s| s.grant(grant))?;
        self.view_of(c, g.as_ref(), t)
    }

    // ---------- changing one ----------

    fn active(&self, c: &mut dyn Calls, grant_id: &Arg, t: f64) -> Result<Row> {
        let g = if is_grant_id(grant_id) { self.store.with(|s| s.grant(grant_id.clone()))? } else { None };
        let Some(g) = g else {
            return fail(404.0, "not-found", "XenoNet has no such connection");
        };
        if self.state_of(c, &g, t)? != "active" && self.state_of(c, &g, t)? != "disabled" {
            let state = self.state_of(c, &g, t)?;
            return fail(409.0, "not-active", format!("that connection is {state}"));
        }
        Ok(g)
    }

    /// New projects or fields (the window): what it reads changes now, and
    /// its revision moves on. Taking projects away needs nothing more;
    /// adding them is a new approval, which is why this is the window's.
    pub fn update(&self, c: &mut dyn Calls, grant_id: &Arg, change_to: &Arg) -> Result<Arg> {
        let t = Self::now(c)?;
        let g = self.active(c, grant_id, t)?;
        let mut change: Vec<(String, Arg)> = Vec::new();
        let projects = change_to.field("projects");
        let mut count = None;
        if !matches!(projects, Arg::Undef) {
            let chosen = unique(&projects);
            if chosen.len() > PROJECTS_MAX {
                return fail(400.0, "bad-request", format!("a grant names at most {PROJECTS_MAX} projects"));
            }
            // A place the grant names already may stay, on the map or not.
            let had = items(parse(g.get("project_allowlist"), json!([])));
            for p in &chosen {
                if !c.call("places", "has", vec![p.clone()])?.truthy() {
                    let id = self.store.with(|s| s.project_id_for(p.clone()))?;
                    let kept = had.iter().any(|h| match (h, &id) {
                        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
                        (Value::Object(_) | Value::Array(_), _) => false,
                        (a, b) => a == b,
                    });
                    if !kept {
                        return fail(400.0, "bad-request", "pick projects from the map as it is now");
                    }
                }
            }
            let mut ids = Vec::new();
            for place in &chosen {
                ids.push(self.store.with(|s| s.project_id_of(place.clone(), &mut || -> Result<Arg> { Ok(Arg::text(uuid_id("prj")?)) }, Arg::num(t)))?);
            }
            count = Some(ids.len());
            change.push(("project_allowlist".into(), Arg::text(stringify(&Value::Array(ids), 0))));
        }
        let labels = change_to.field("labels");
        if !matches!(labels, Arg::Undef) {
            let mut policy = Map::new();
            match parse(g.get("field_policy"), json!({})) {
                Value::Object(o) => policy.extend(o),
                Value::Array(a) => policy.extend(a.into_iter().enumerate().map(|(i, v)| (i.to_string(), v))),
                Value::String(s) => policy.extend(s.chars().enumerate().map(|(i, ch)| (i.to_string(), Value::String(ch.to_string())))),
                _ => {}
            }
            policy.insert("labels".into(), Value::Bool(labels == Arg::Bool(true)));
            change.push(("field_policy".into(), Arg::text(stringify(&Value::Object(policy), 0))));
        }
        if change.is_empty() {
            return self.view_of(c, Some(&g), t);
        }
        let revision = num(g.get("revision")) + 1.0;
        let change = Arg::Obj(change);
        self.store.transaction(|| -> Result<()> {
            self.store.with(|s| s.change_grant(grant_id.clone(), &change))?;
            let audit = obj(vec![
                ("at", Arg::num(t)),
                ("kind", Arg::text("owner")),
                ("grant_id", grant_id.clone()),
                ("grant_revision", Arg::num(revision)),
                ("connection_id", val(g.get("connection_id"))),
                ("operation", Arg::text("change")),
                ("result", Arg::text("ok")),
                ("projects", count.map_or(Arg::Null, |n| Arg::num(n as f64))),
            ]);
            self.store.with(|s| s.add_audit(&audit))?;
            Ok(())
        })?;
        self.note_revision(c, grant_id)?;
        Self::emit(c, "changed", obj(vec![("grant", grant_id.clone())]))?;
        let g = self.store.with(|s| s.grant(grant_id.clone()))?;
        self.view_of(c, g.as_ref(), t)
    }

    /// A new credential: the old one stops at once, and adapters holding
    /// it are let go.
    pub fn rotate(&self, c: &mut dyn Calls, grant_id: &Arg) -> Result<Arg> {
        let t = Self::now(c)?;
        let g = self.active(c, grant_id, t)?;
        let made = c.call("disk", "newToken", vec![])?;
        let credential = obj(vec![("grant", grant_id.clone()), ("token", made.field("token")), ("device", val(g.get("device_id"))), ("expiresAt", val(g.get("expires_at")))]);
        c.call("", "writeCredential", vec![credential])?;
        let revision = num(g.get("revision")) + 1.0;
        self.store.transaction(|| -> Result<()> {
            self.store.with(|s| s.change_grant(grant_id.clone(), &obj(vec![("token_hash", made.field("hash"))])))?;
            let audit = obj(vec![
                ("at", Arg::num(t)),
                ("kind", Arg::text("owner")),
                ("grant_id", grant_id.clone()),
                ("grant_revision", Arg::num(revision)),
                ("connection_id", val(g.get("connection_id"))),
                ("operation", Arg::text("rotate")),
                ("result", Arg::text("ok")),
            ]);
            self.store.with(|s| s.add_audit(&audit))?;
            Ok(())
        })?;
        self.note_revision(c, grant_id)?;
        Self::emit(c, "rotated", obj(vec![("grant", grant_id.clone()), ("connection", val(g.get("connection_id")))]))?;
        let g = self.store.with(|s| s.grant(grant_id.clone()))?;
        self.view_of(c, g.as_ref(), t)
    }

    pub fn revoke(&self, c: &mut dyn Calls, grant_id: &Arg, reason: &Arg) -> Result<Arg> {
        let g = if is_grant_id(grant_id) { self.store.with(|s| s.grant(grant_id.clone()))? } else { None };
        let Some(g) = g else {
            return fail(404.0, "not-found", "XenoNet has no such connection");
        };
        let t = Self::now(c)?;
        let changed = if g.get("revoked_at").is_null() { self.revoke_row(c, &g, reason, t, "owner")? } else { false };
        self.sync(c, t)?;
        Ok(obj(vec![("revoked", grant_id.clone()), ("already", Arg::Bool(!changed))]))
    }

    /// Every connection off, until you turn them on again; adapters are
    /// let go at once.
    pub fn disable(&self, c: &mut dyn Calls) -> Result<Arg> {
        let t = Self::now(c)?;
        self.disk(c, "markDisabled", vec![Arg::num(t)])?;
        self.store.with(|s| s.add_audit(&obj(vec![("at", Arg::num(t)), ("kind", Arg::text("owner")), ("operation", Arg::text("disable")), ("result", Arg::text("ok"))])))?;
        self.sync(c, t)?;
        Self::emit(c, "disabled", Arg::Obj(vec![]))?;
        Ok(obj(vec![("disabled", Arg::Bool(true))]))
    }

    pub fn enable(&self, c: &mut dyn Calls) -> Result<Arg> {
        let t = Self::now(c)?;
        self.disk(c, "clearDisabled", vec![])?;
        self.store.with(|s| s.add_audit(&obj(vec![("at", Arg::num(t)), ("kind", Arg::text("owner")), ("operation", Arg::text("enable")), ("result", Arg::text("ok"))])))?;
        self.sync(c, t)?;
        Self::emit(c, "enabled", Arg::Obj(vec![]))?;
        Ok(obj(vec![("disabled", Arg::Bool(false))]))
    }

    // ---------- an adapter's calls ----------

    /// How a grant stands now, for a call through it, where the code is for
    /// the caller and the reason for the log alone.
    pub fn standing(&self, c: &mut dyn Calls, g: Option<&Row>, t: Option<f64>) -> Result<Standing> {
        let t = match t {
            Some(t) => t,
            None => Self::now(c)?,
        };
        let Some(g) = g else {
            return Ok(Standing::no("NOT_AUTHORIZED", "unknown-grant"));
        };
        if self.disabled(c)? {
            return Ok(Standing::no("DISABLED", "disabled"));
        }
        if !g.get("revoked_at").is_null() || self.disk(c, "isMarkedRevoked", vec![val(g.get("grant_id"))])?.truthy() {
            let reason = match g.get("revoked_reason") {
                Cell::Null => Arg::text("revoked"),
                r => val(r),
            };
            return Ok(Standing::No { code: "GRANT_REVOKED", reason });
        }
        if num(g.get("expires_at")) <= t {
            return Ok(Standing::no("GRANT_EXPIRED", "expired"));
        }
        let device = self.device(c)?;
        if !matches!((g.get("device_id"), &device), (Cell::Text(a), Value::String(b)) if a == b) {
            return Ok(Standing::no("NOT_AUTHORIZED", "device-changed"));
        }
        let conn = self.store.with(|s| s.connection(val(g.get("connection_id"))))?;
        let Some(conn) = conn.filter(|r| r.text("state") == Some("active")) else {
            return Ok(Standing::no("GRANT_REVOKED", "connection-ended"));
        };
        let actor_id = format!("actor:{}", js_text(&val(conn.get("actor_slug"))));
        if !c.call("actors", "has", vec![Arg::text(actor_id.clone())])?.truthy() {
            return Ok(Standing::no("NOT_AUTHORIZED", "actor-gone"));
        }
        Ok(Standing::Ok { revision: g.get("revision").clone(), grant: g.clone(), connection: conn, actor_id })
    }

    /// An adapter saying which grant it's for, with the grant's credential.
    /// A wrong credential is refused the same way as an unknown grant; one
    /// that's right hears how its grant stands.
    pub fn authenticate(&self, c: &mut dyn Calls, ask: &Arg) -> Result<Standing> {
        let (grant, token) = (ask.field("grant"), ask.field("token"));
        if !is_grant_id(&grant) || !c.call("disk", "isToken", vec![token.clone()])?.truthy() {
            return Ok(Standing::no("NOT_AUTHORIZED", "malformed"));
        }
        let Some(g) = self.store.with(|s| s.grant(grant))? else {
            return Ok(Standing::no("NOT_AUTHORIZED", "unknown-grant"));
        };
        let theirs = hex_bytes(&js_text(&c.call("disk", "hashToken", vec![token])?));
        let ours = match g.get("token_hash") {
            Cell::Text(h) => hex_bytes(h),
            other => hex_bytes(&js_text(&val(other))),
        };
        if theirs.len() != ours.len() || !timing_safe_eq(&theirs, &ours) {
            return Ok(Standing::no("NOT_AUTHORIZED", "wrong-token"));
        }
        self.standing(c, Some(&g), None)
    }

    /// The same, again, by grant and the revision a read began under, just
    /// before its answer goes: changed is true when the revision moved on.
    pub fn recheck(&self, c: &mut dyn Calls, grant_id: &Arg, revision: &Arg) -> Result<Arg> {
        let g = self.store.with(|s| s.grant(grant_id.clone()))?;
        let s = self.standing(c, g.as_ref(), None)?;
        let changed = match &s {
            Standing::Ok { revision: r, .. } => Some(!matches!((val(r), revision), (Arg::Num(a), Arg::Num(b)) if a == *b)),
            Standing::No { .. } => None,
        };
        let mut out = s.to_arg();
        if let (Some(changed), Arg::Obj(fields)) = (changed, &mut out) {
            fields.push(("changed".into(), Arg::Bool(changed)));
        }
        Ok(out)
    }

    /// What a read through a grant may see. `places` comes back as
    /// [project, place] pairs.
    pub fn policy(&self, c: &mut dyn Calls, s: &Arg) -> Result<Arg> {
        let (g, conn) = (s.field("grant"), s.field("connection"));
        let actor_id = s.field("actorId");
        let info = c.call("actors", "info", vec![actor_id.clone()])?;
        let parse_arg = |v: &Arg, fallback: Value| -> Value {
            match v {
                Arg::Undef | Arg::Null => fallback,
                v => serde_json::from_str(&js_text(v)).unwrap_or(fallback),
            }
        };
        let projects = parse_arg(&g.field("project_allowlist"), json!([]));
        let mut places = Vec::new();
        for p in items(projects.clone()) {
            let place = self.store.with(|s| s.place_of_project(value_arg(&p)))?;
            if value_arg(&place).truthy() {
                places.push(Arg::List(vec![value_arg(&p), value_arg(&place)]));
            }
        }
        let wall = to_number(&c.call("", "wall", vec![])?);
        let (link, _) = account_links::link_state(&self.store, &conn.field("actor_slug"), wall)?;
        let labels = parse_arg(&g.field("field_policy"), json!({}));
        Ok(obj(vec![
            ("grantId", g.field("grant_id")),
            ("revision", g.field("revision")),
            ("actorId", actor_id),
            ("connectionId", conn.field("connection_id")),
            ("deviceId", g.field("device_id")),
            ("deviceAlias", conn.field("device_alias").or(Arg::Null)),
            ("productId", product(&info.field("product").field("id")).map_or(Arg::Null, |p| Arg::text(p.id))),
            ("accountLink", Arg::text(link)),
            ("projects", value_arg(&projects)),
            ("places", Arg::List(places)),
            ("labels", Arg::Bool(labels.get("labels") == Some(&Value::Bool(true)))),
            ("lookbackMs", g.field("lookback_ms")),
            ("dataMode", Arg::text(if g.field("data_mode").as_str() == Some("real") { "real" } else { "fixture" })),
            ("transportScope", conn.field("audience_scope")),
            ("audienceLabel", conn.field("label")),
            ("permissions", value_arg(&parse_arg(&g.field("permissions"), json!([])))),
        ]))
    }

    /// A read went out through a grant: when it was last used, written at
    /// most once a minute.
    pub fn used(&self, grant_id: &Arg, t: f64) -> Result<()> {
        let key = js_text(grant_id);
        {
            let mut touched = self.touched.locked();
            if t - touched.get(&key).copied().unwrap_or(f64::NEG_INFINITY) < TOUCH_MS {
                return Ok(());
            }
            touched.insert(key, t);
        }
        self.store.with(|s| s.touch_grant(grant_id.clone(), Arg::num(t)))?;
        Ok(())
    }

    /// An adapter connected through a connection, or went (`delta` +1 or -1).
    pub fn bridge(&self, c: &mut dyn Calls, connection_id: &Arg, delta: f64) -> Result<()> {
        let key = js_text(connection_id);
        {
            let mut bridges = self.bridges.locked();
            let had = bridges.iter().find(|(k, _)| *k == key).map_or(0.0, |(_, n)| *n);
            let n = (had + delta).max(0.0);
            let n = if (had + delta).is_nan() { f64::NAN } else { n };
            if n != 0.0 && !n.is_nan() {
                match bridges.iter_mut().find(|(k, _)| *k == key) {
                    Some(e) => e.1 = n,
                    None => bridges.push((key, n)),
                }
            } else {
                bridges.retain(|(k, _)| *k != key);
            }
        }
        let t = Self::now(c)?;
        self.sync(c, t)
    }

    // ---------- your view ----------

    fn view_of(&self, c: &mut dyn Calls, g: Option<&Row>, t: f64) -> Result<Arg> {
        let Some(g) = g else {
            return Err(Thrown::new("Cannot read properties of undefined (reading 'connection_id')").into());
        };
        let Some(conn) = self.store.with(|s| s.connection(val(g.get("connection_id"))))? else {
            return Err(Thrown::new("Cannot read properties of undefined (reading 'actor_slug')").into());
        };
        let (link, identity) = account_links::link_state(&self.store, &val(conn.get("actor_slug")), t)?;
        let mut projects = Vec::new();
        for p in items(parse(g.get("project_allowlist"), json!([]))) {
            let place = self.store.with(|s| s.place_of_project(value_arg(&p)))?;
            let name = if value_arg(&place).truthy() { c.call("", "describe", vec![value_arg(&place)])?.or(Arg::Null) } else { Arg::Null };
            projects.push(obj(vec![("project_id", value_arg(&p)), ("place", value_arg(&place)), ("name", name)]));
        }
        let state = self.state_of(c, g, t)?;
        let opt = |k: &str| match conn.get(k) {
            Cell::Null => Arg::Null,
            v => val(v),
        };
        let policy = parse(g.get("field_policy"), json!({}));
        Ok(obj(vec![
            ("schema", Arg::text("actor/connection/v1")),
            ("connection_id", val(conn.get("connection_id"))),
            ("grant_id", val(g.get("grant_id"))),
            ("actor_id", Arg::text(format!("actor:{}", js_text(&val(conn.get("actor_slug")))))),
            ("label", val(conn.get("label"))),
            ("adapter_id", val(conn.get("adapter_id"))),
            ("provider", val(conn.get("provider"))),
            ("transport_kind", val(conn.get("transport_kind"))),
            (
                "audience",
                obj(vec![
                    ("description", val(conn.get("audience_description"))),
                    ("verification", val(conn.get("audience_verification"))),
                    ("scope", val(conn.get("audience_scope"))),
                ]),
            ),
            ("account_link", obj(vec![("state", Arg::text(link)), ("identity", value_arg(&identity))])),
            ("device", obj(vec![("id", val(g.get("device_id"))), ("alias", opt("device_alias"))])),
            ("permissions", value_arg(&parse(g.get("permissions"), json!([])))),
            ("data_mode", Arg::text(if g.text("data_mode") == Some("real") { "real" } else { "fixture" })),
            ("projects", Arg::List(projects)),
            ("labels", Arg::Bool(policy.get("labels") == Some(&Value::Bool(true)))),
            ("lookback_days", Arg::num(js_round(num(g.get("lookback_ms")) / DAY_MS))),
            ("revision", val(g.get("revision"))),
            ("state", Arg::text(state)),
            ("bridge", Arg::text(if self.bridged(conn.get("connection_id")) { "connected" } else { "not_connected" })),
            ("created_at", iso(g.get("created_at"))),
            ("expires_at", iso(g.get("expires_at"))),
            ("revoked_at", iso(g.get("revoked_at"))),
            (
                "revoked_reason",
                match g.get("revoked_reason") {
                    Cell::Null => Arg::Null,
                    r => val(r),
                },
            ),
            ("last_used_at", iso(g.get("last_used_at"))),
            ("launch", launch_for(&val(g.get("grant_id")), &self.port)),
        ]))
    }

    /// Your view of one grant, or null.
    pub fn view(&self, c: &mut dyn Calls, grant_id: &Arg) -> Result<Arg> {
        match self.store.with(|s| s.grant(grant_id.clone()))? {
            Some(g) => {
                let t = Self::now(c)?;
                self.view_of(c, Some(&g), t)
            }
            None => Ok(Arg::Null),
        }
    }

    pub fn list(&self, c: &mut dyn Calls) -> Result<Arg> {
        let t = Self::now(c)?;
        let disabled = self.disabled(c)?;
        let device = self.device(c)?;
        let mut views = Vec::new();
        for g in self.store.with(|s| s.grants())? {
            views.push(self.view_of(c, Some(&g), t)?);
        }
        views.reverse();
        Ok(obj(vec![("at", Arg::num(t)), ("disabled", Arg::Bool(disabled)), ("device", obj(vec![("id", value_arg(&device))])), ("connections", Arg::List(views))]))
    }

    /// On the clock: grants that expired let their adapters go.
    pub fn tick(&self, c: &mut dyn Calls, t: f64) -> Result<()> {
        for g in self.store.with(|s| s.grants())? {
            let id = js_text(&val(g.get("grant_id")));
            if !g.get("revoked_at").is_null() || num(g.get("expires_at")) > t || self.expired.locked().contains(&id) {
                continue;
            }
            self.expired.locked().insert(id);
            Self::emit(c, "expired", obj(vec![("grant", val(g.get("grant_id"))), ("connection", val(g.get("connection_id")))]))?;
        }
        self.sync(c, t)
    }

    /// Removing an actor (yours) revokes its connections first, while its
    /// grants can still be named; the rows then go with the actor's.
    pub fn before_remove(&self, c: &mut dyn Calls, slug: &Arg) -> Result<()> {
        let t = Self::now(c)?;
        for conn in self.store.with(|s| s.connections_of(slug.clone()))? {
            let g = self.store.with(|s| s.grants())?.into_iter().find(|x| same_cell(x.get("connection_id"), conn.get("connection_id")));
            if let Some(g) = g {
                if g.get("revoked_at").is_null() {
                    self.revoke_row(c, &g, &Arg::text("actor-removed"), t, "owner")?;
                }
            }
        }
        Ok(())
    }

    /// Whether any grant could be used now.
    pub fn any_active(&self, c: &mut dyn Calls) -> Result<bool> {
        if self.disabled(c)? {
            return Ok(false);
        }
        for g in self.store.with(|s| s.grants())? {
            let t = Self::now(c)?;
            if self.state_of(c, &g, t)? == "active" {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_reads_as_node_reads_it() {
        assert_eq!(hex_bytes("0aff"), vec![0x0a, 0xff]);
        assert_eq!(hex_bytes("0az1"), vec![0x0a]);
        assert_eq!(hex_bytes("abc"), vec![0xab]);
    }

    #[test]
    fn words_are_one_line_of_text_and_never_a_key() {
        assert_eq!(words(&Arg::text("  My\n dot "), "a name", 64).unwrap(), Some("My dot".into()));
        assert_eq!(words(&Arg::text(""), "a name", 64).unwrap(), None);
        assert!(words(&Arg::num(1.0), "a name", 64).is_err());
        assert!(words(&Arg::text("x".repeat(65)), "a name", 64).is_err());
    }

    #[test]
    fn launch_commands_name_a_port_only_when_its_not_the_default() {
        let a = launch_for(&Arg::text("gr_1"), &Arg::num(47840.0));
        assert_eq!(list(a.field("args")).len(), 4);
        let b = launch_for(&Arg::text("gr_1"), &Arg::num(5000.0));
        assert_eq!(list(b.field("args"))[5], Arg::text("5000"));
    }

    #[test]
    fn math_round_rounds_halves_up() {
        assert_eq!(js_round(-0.5), 0.0);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(29.999), 30.0);
    }

    #[test]
    fn uuids_are_version_four() {
        let id = uuid_id("dev").unwrap();
        assert_eq!(id.len(), 4 + 36);
        assert_eq!(&id[4 + 14..4 + 15], "4");
    }
}
