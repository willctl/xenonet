//! Remote connections (docs/design/cloud-assistants.md, sections 3 and 4):
//! how a remote actor, such as an OpenAI dot in the cloud, reaches XenoNet
//! to read what's going on, and only that.
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
//! The rows are the engine's, reached through `Store`. The rest is the
//! world's, through `Host`: the actors' registry, the credentials' files on
//! disk, the clock, the adapters, the map's places and the identities for
//! a pairing, and whoever listens for events.

use crate::account_links::{self, LinkFail, Links};
use crate::adapters::TRANSPORTS;
use crate::detail::looks_like_key;
use crate::names::{check_name, slug_from, slug_refusal, suggest_slug};
use crate::re;
use crate::store;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use xenonet_adapter::products::{product_of, Product};
use xenonet_base::ids::YOU;
use xenonet_base::json::{number, stringify};
use xenonet_base::sync::Locked;
use xenonet_base::text::{js_string, truthy};

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
/// a sentence for a person, and extra fields; a link's refusal; or what
/// the store or a collaborator failed with.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Connection { status: f64, code: &'static str, message: String, extra: Value },
    Link { code: &'static str, message: String },
    Error(store::Error),
}

impl From<store::Error> for Fail {
    fn from(e: store::Error) -> Self {
        Fail::Error(e)
    }
}

impl From<LinkFail> for Fail {
    fn from(f: LinkFail) -> Self {
        match f {
            LinkFail::Link { code, message } => Fail::Link { code, message },
            LinkFail::Error(e) => Fail::Error(e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn fail<T>(status: f64, code: &'static str, message: impl Into<String>) -> Result<T> {
    Err(Fail::Connection { status, code, message: message.into(), extra: json!({}) })
}

fn error<T>(message: impl Into<String>) -> Result<T> {
    Err(Fail::Error(store::Error::new(message)))
}

/// An adapter for a product: what the connections ask of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Adapter {
    pub id: String,
    /// The product it's for, None for an actor of your own.
    pub product: Option<String>,
    pub provider: String,
    /// The transports its product reaches XenoNet through.
    pub transports: Vec<String>,
}

/// What the connections ask of the world.
pub trait Host {
    /// The time, and the wall clock (the same, outside tests).
    fn now(&mut self) -> f64;
    fn wall(&mut self) -> f64;
    /// Something happened to a grant (changed, rotated, revoked, expired,
    /// disabled, enabled), for whoever listens.
    fn emit(&mut self, name: &str, what: Value);
    /// A place's name on the map, or null.
    fn describe(&mut self, place: &str) -> Value;
    /// Whether a registration is waiting under a slug.
    fn taken(&mut self, slug: &str) -> bool;
    /// The adapter for a product (None for an actor of your own), picked
    /// for the pairing under way; None when there's none to pair it with.
    fn adapter_for(&mut self, product: Option<&Value>) -> Option<Adapter>;
    /// The capabilities of the adapter picked, or an error when none was.
    fn picked_capabilities(&mut self) -> Result<Vec<String>>;
    /// A grant's credential written to disk, private; an error when it
    /// can't be kept private.
    fn write_credential(&mut self, grant: &str, token: &Value, device: &Value, expires_at: &Value) -> Result<()>;
    /// The places you may pick for a pairing, by node id.
    fn places_has(&mut self, id: &str) -> bool;
    /// The identities XenoNet knows, for a link that's your word.
    fn identities_has(&mut self, id: &str) -> bool;
    fn identities(&mut self) -> Vec<String>;
    // The credentials' files on disk.
    fn is_disabled(&mut self) -> bool;
    fn is_marked_revoked(&mut self, grant: &str) -> bool;
    fn mark_revision(&mut self, grant: &str, revision: &Value) -> Result<()>;
    fn mark_revoked(&mut self, grant: &str, at: f64) -> Result<()>;
    fn delete_credential(&mut self, grant: &str) -> Result<()>;
    fn marked_revoked(&mut self) -> Vec<String>;
    /// A new credential: (token, its hash).
    fn new_token(&mut self) -> (String, String);
    fn mark_disabled(&mut self, at: f64) -> Result<()>;
    fn clear_disabled(&mut self) -> Result<()>;
    fn is_token(&mut self, token: &Value) -> bool;
    fn hash_token(&mut self, token: &Value) -> String;
    // The actors' registry.
    fn ids(&mut self) -> Vec<String>;
    fn remote_of(&mut self, id: &str) -> Value;
    fn set_remote(&mut self, id: &str, remote: &Value) -> Result<()>;
    fn has(&mut self, id: &str) -> bool;
    fn info(&mut self, id: &str) -> Value;
    fn is_remote(&mut self, id: &str) -> bool;
    fn upsert(&mut self, def: &Value) -> Result<()>;
    fn add_via(&mut self, id: &str, via: &str) -> Result<()>;
}

/// The connections' rows in the engine's database, and the account links'.
/// A row is a JSON object of its columns.
pub trait Store: Links {
    fn meta(&self, key: &str) -> store::Result<Value>;
    fn set_meta(&self, key: &str, value: &str) -> store::Result<()>;
    /// Every grant, oldest first.
    fn grants(&self) -> store::Result<Vec<Value>>;
    fn grant(&self, id: &str) -> store::Result<Option<Value>>;
    fn connection(&self, id: &str) -> store::Result<Option<Value>>;
    fn connections(&self) -> store::Result<Vec<Value>>;
    fn connections_of(&self, slug: &str) -> store::Result<Vec<Value>>;
    fn add_connection(&self, row: &Value) -> store::Result<()>;
    fn add_grant(&self, row: &Value) -> store::Result<()>;
    /// Columns of a grant changed; whether the grant was there.
    fn change_grant(&self, id: &str, change: &Value) -> store::Result<bool>;
    fn revoke_grant(&self, id: &str, reason: &str, at: f64) -> store::Result<bool>;
    fn end_connection(&self, id: &str, at: f64) -> store::Result<()>;
    fn touch_grant(&self, id: &str, at: f64) -> store::Result<()>;
    fn add_audit(&self, row: &Value) -> store::Result<()>;
    /// A place's project id, made by `make` when it has none yet.
    fn project_id_of(&self, place: &Value, make: &mut dyn FnMut() -> store::Result<Value>, now: f64) -> store::Result<Value>;
    /// A place's project id, or null.
    fn project_id_for(&self, place: &Value) -> store::Result<Value>;
    /// A project id's place, or null.
    fn place_of_project(&self, project_id: &Value) -> store::Result<Value>;
}

fn random_hex(n: usize) -> store::Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| store::Error::new(format!("no random bytes: {e}")))?;
    Ok(crate::tokens::hex(&b))
}

fn new_id(prefix: &str) -> store::Result<String> {
    Ok(format!("{prefix}_{}", random_hex(12)?))
}

/// `<prefix>_` and a version 4 UUID.
fn uuid_id(prefix: &str) -> store::Result<String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| store::Error::new(format!("no random bytes: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = crate::tokens::hex(&b);
    Ok(format!("{prefix}_{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

/// A column of a row, or null.
fn col(r: &Value, k: &str) -> Value {
    r.get(k).cloned().unwrap_or(Value::Null)
}

/// A column of a row as text (`${column}`).
fn text(r: &Value, k: &str) -> String {
    js_string(&col(r, k))
}

fn is_null(r: &Value, k: &str) -> bool {
    r.get(k).is_none_or(Value::is_null)
}

/// Number(column), as a column compares with a number.
fn num(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(_) => xenonet_base::text::js_number(Some(v)),
        _ => f64::NAN,
    }
}

/// `a === b` for two columns.
fn same_col(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (x, y) => matches!((x.as_f64(), y.as_f64()), (Some(p), Some(q)) if p == q),
    }
}

/// JSON.parse(column), or `fallback` for null or what doesn't parse.
fn parse(c: &Value, fallback: Value) -> Value {
    let text = match c {
        Value::Null => return fallback,
        Value::String(s) => s.clone(),
        other => js_string(other),
    };
    serde_json::from_str(&text).unwrap_or(fallback)
}

/// A time column as an ISO date, or null.
fn iso(c: &Value) -> Value {
    match c {
        Value::Null => Value::Null,
        c => Value::String(xenonet_base::time::iso(num(c))),
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

fn is_grant_id(v: &str) -> bool {
    re!(r"^gr_[0-9a-f]{24}$").is_match(v)
}

/// One line of your own words (a connection's name, a computer's, who can
/// call an endpoint), or None for none; never a key.
fn words(value: Option<&Value>, what: &str, max: usize) -> Result<Option<String>> {
    let text = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) if s.is_empty() => return Ok(None),
        Some(Value::String(s)) => s,
        Some(_) => return fail(400.0, "bad-request", format!("{what} is text")),
    };
    let blanked = re!(r"[\u0000-\u001f\u007f-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]").replace_all(text, " ");
    let line = re!(r"[\s\x{FEFF}--\x{85}]+").replace_all(&blanked, " ");
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
pub fn fingerprint_of(host: &str, user: &str, state_dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let joined = [host, user, state_dir].join("\0");
    crate::tokens::hex(&Sha256::digest(joined.as_bytes()))
}

/// What the tunnel client runs for a grant: the adapter, holding no secret.
pub fn launch_for(grant: &str, port: f64) -> Value {
    let mut args = vec![json!("mcp"), json!("assistant"), json!("--grant"), json!(grant)];
    if port != DEFAULT_PORT {
        args.push(json!("--port"));
        args.push(Value::String(js_string(&number(port))));
    }
    json!({ "command": "xenonet", "args": args })
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
fn unique(a: Option<&Value>) -> Vec<Value> {
    let all: Vec<Value> = match a {
        Some(Value::Array(l)) => l.clone(),
        Some(Value::String(s)) => s.chars().map(|c| Value::String(c.to_string())).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<Value> = Vec::new();
    for v in all {
        let seen = out.iter().any(|o| match (o, &v) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Object(_) | Value::Array(_), _) => false,
            (o, v) => o == v,
        });
        if !seen {
            out.push(v);
        }
    }
    out
}

/// A product by the id a value holds (`productOf(x)`).
fn product(v: Option<&Value>) -> Option<&'static Product> {
    v.and_then(Value::as_str).and_then(product_of)
}

/// What a pairing makes, before anything is changed: an actor already
/// here, or a new one (kind, name, product).
struct Plan {
    id: String,
    create: Option<Value>,
}

/// How a grant stands now, for a call through it.
pub enum Standing {
    No { code: &'static str, reason: Value },
    Ok { grant: Value, connection: Value, actor_id: String, revision: Value },
}

impl Standing {
    fn no(code: &'static str, reason: &str) -> Self {
        Standing::No { code, reason: json!(reason) }
    }
    pub fn to_json(&self) -> Value {
        match self {
            Standing::No { code, reason } => json!({ "ok": false, "code": code, "reason": reason }),
            Standing::Ok { grant, connection, actor_id, revision } => json!({ "ok": true, "grant": grant, "connection": connection, "actorId": actor_id, "revision": revision }),
        }
    }
}

pub struct Connections {
    store: Arc<dyn Store>,
    fingerprint: String,
    port: f64,
    /// Adapters connected now, by connection.
    bridges: Mutex<Vec<(String, f64)>>,
    touched: Mutex<HashMap<String, f64>>,
    device_id: Mutex<Option<Value>>,
    expired: Mutex<HashSet<String>>,
}

impl Connections {
    pub fn new(store: Arc<dyn Store>, fingerprint: &str, port: f64) -> Self {
        Connections { store, fingerprint: fingerprint.into(), port, bridges: Mutex::default(), touched: Mutex::default(), device_id: Mutex::default(), expired: Mutex::default() }
    }

    fn links(&self) -> &dyn Links {
        self.store.as_ref()
    }

    /// `f` in a transaction of the store's, unless one is under way.
    fn transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        store::transaction(self.store.as_ref(), f)
    }

    /// The first look: this device, the tombstones, and what each actor's
    /// connections come to.
    pub fn start(&self, h: &mut dyn Host) -> Result<()> {
        self.device(h)?;
        self.apply_tombstones(h)?;
        let t = h.now();
        self.sync(h, t)
    }

    fn bridged(&self, connection: &Value) -> bool {
        let id = js_string(connection);
        self.bridges.locked().iter().any(|(k, n)| *k == id && *n > 0.0)
    }

    // ---------- this device ----------

    pub fn device(&self, h: &mut dyn Host) -> Result<Value> {
        if let Some(id) = self.device_id.locked().clone().filter(|id| truthy(Some(id))) {
            return Ok(id);
        }
        let had = self.store.meta("device_id")?;
        let had_some = truthy(Some(&had));
        if had_some && self.store.meta("device_fingerprint")?.as_str() == Some(self.fingerprint.as_str()) {
            *self.device_id.locked() = Some(had.clone());
            return Ok(had);
        }
        let t = h.now();
        let id = uuid_id("dev")?;
        let key = random_hex(32)?;
        self.transaction(|| -> Result<()> {
            self.store.set_meta("device_id", &id)?;
            self.store.set_meta("device_fingerprint", &self.fingerprint)?;
            if !truthy(Some(&self.store.meta("id_key")?)) {
                self.store.set_meta("id_key", &key)?;
            }
            Ok(())
        })?;
        *self.device_id.locked() = Some(Value::String(id.clone()));
        // Moved or copied: every grant made for the old one stops.
        if had_some {
            for g in self.store.grants()? {
                if is_null(&g, "revoked_at") {
                    self.revoke_row(h, &g, "device-changed", t, "system")?;
                }
            }
        }
        Ok(Value::String(id))
    }

    /// A key for ids that mustn't say what they're made of, local to this
    /// installation.
    pub fn id_key(&self, h: &mut dyn Host) -> Result<Value> {
        self.device(h)?;
        Ok(self.store.meta("id_key")?)
    }

    // ---------- standing ----------

    fn state_of(h: &mut dyn Host, g: &Value, t: f64) -> &'static str {
        if !is_null(g, "revoked_at") || h.is_marked_revoked(&text(g, "grant_id")) {
            return "revoked";
        }
        if h.is_disabled() {
            return "disabled";
        }
        if num(&col(g, "expires_at")) <= t {
            return "expired";
        }
        "active"
    }

    /// What each actor's connections come to, for the map and the Actors
    /// panel (the registry's remote).
    pub fn sync(&self, h: &mut dyn Host, t: f64) -> Result<()> {
        let grants = self.store.grants()?;
        // By actor slug: each connection, with its grant if it has one.
        type Entries = Vec<(Value, Option<Value>)>;
        let mut by_slug: Vec<(String, Entries)> = Vec::new();
        for conn in self.store.connections()? {
            let g = grants.iter().find(|x| same_col(&col(x, "connection_id"), &col(&conn, "connection_id"))).cloned();
            let slug = text(&conn, "actor_slug");
            match by_slug.iter_mut().find(|(s, _)| *s == slug) {
                Some((_, l)) => l.push((conn, g)),
                None => by_slug.push((slug, vec![(conn, g)])),
            }
        }
        for id in h.ids() {
            let slug: String = id.chars().skip("actor:".len()).collect();
            let Some((_, entries)) = by_slug.iter().find(|(s, _)| *s == slug) else {
                if truthy(Some(&h.remote_of(&id))) {
                    h.set_remote(&id, &Value::Null)?;
                }
                continue;
            };
            // Each active connection, with its grant.
            let mut active: Vec<(&Value, &Value)> = Vec::new();
            for (conn, g) in entries {
                if let Some(g) = g {
                    if Self::state_of(h, g, t) == "active" {
                        active.push((conn, g));
                    }
                }
            }
            let bridge = if active.iter().any(|(conn, _)| self.bridged(&col(conn, "connection_id"))) { "connected" } else { "not_connected" };
            let (link, _) = account_links::link_state(self.links(), &slug, t)?;
            let mut permissions: Vec<Value> = Vec::new();
            for (_, g) in &active {
                let parsed = parse(&col(g, "permissions"), json!([]));
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
            let remote = json!({
                "active": !active.is_empty(),
                "connections": number(entries.len() as f64),
                "bridge": bridge,
                "accountLink": link,
                "permissions": permissions,
                "transports": active.iter().map(|(conn, _)| col(conn, "transport_kind")).collect::<Vec<_>>(),
            });
            h.set_remote(&id, &remote)?;
        }
        Ok(())
    }

    /// A grant's revision as it is now, on disk too, where an adapter with
    /// no service to ask looks before it serves anything it read earlier.
    fn note_revision(&self, h: &mut dyn Host, grant_id: &str) -> Result<()> {
        if let Some(g) = self.store.grant(grant_id)? {
            h.mark_revision(grant_id, &col(&g, "revision"))?;
        }
        Ok(())
    }

    fn revoke_row(&self, h: &mut dyn Host, g: &Value, reason: &str, t: f64, by: &str) -> Result<bool> {
        let grant = text(g, "grant_id");
        let connection = text(g, "connection_id");
        h.mark_revoked(&grant, t)?;
        h.delete_credential(&grant)?;
        let changed = self.transaction(|| -> Result<bool> {
            let r = self.store.revoke_grant(&grant, reason, t)?;
            self.store.end_connection(&connection, t)?;
            let slug = self.store.connection(&connection)?.map_or("?".to_string(), |r| match r.get("actor_slug") {
                None | Some(Value::Null) => "?".to_string(),
                Some(c) => js_string(c),
            });
            let audit = json!({
                "at": number(t),
                "kind": by,
                "grant_id": col(g, "grant_id"),
                "grant_revision": col(g, "revision"),
                "actor": format!("actor:{slug}"),
                "connection_id": col(g, "connection_id"),
                "operation": "revoke",
                "result": reason,
            });
            self.store.add_audit(&audit)?;
            Ok(r)
        })?;
        self.note_revision(h, &grant)?;
        h.emit("revoked", json!({ "grant": col(g, "grant_id"), "connection": col(g, "connection_id"), "reason": reason }));
        Ok(changed)
    }

    /// A grant revoked while the service was stopped, or one a restored
    /// database has as active: the tombstone wins.
    fn apply_tombstones(&self, h: &mut dyn Host) -> Result<()> {
        let t = h.now();
        for id in h.marked_revoked() {
            if let Some(g) = self.store.grant(&id)? {
                if is_null(&g, "revoked_at") {
                    self.revoke_row(h, &g, "revoked-offline", t, "owner")?;
                }
            }
        }
        Ok(())
    }

    // ---------- pairing ----------

    fn remote_actors(h: &mut dyn Host) -> usize {
        let mut n = 0;
        for id in h.ids() {
            if h.is_remote(&id) && truthy(h.remote_of(&id).get("active")) {
                n += 1;
            }
        }
        n
    }

    fn active_grants(&self, h: &mut dyn Host, t: f64) -> Result<usize> {
        let mut n = 0;
        for g in self.store.grants()? {
            if Self::state_of(h, &g, t) == "active" {
                n += 1;
            }
        }
        Ok(n)
    }

    fn is_taken(h: &mut dyn Host, slug: &str) -> bool {
        h.has(&format!("actor:{slug}")) || h.taken(slug)
    }

    /// The actor a pairing is for, without changing anything yet: one
    /// already here, as you picked it, or a new one. A name a caller sends
    /// never picks one.
    fn plan_actor(h: &mut dyn Host, ask: &Value, adapter: &Adapter) -> Result<Plan> {
        if let Some(given) = ask.get("actor").filter(|v| !v.is_null()) {
            let id = js_string(given);
            let info = h.info(&id);
            let demo = info.get("via").and_then(Value::as_array).is_some_and(|v| v.iter().any(|x| x.as_str() == Some("demo")));
            if !truthy(Some(&info)) || id == YOU || demo {
                return fail(404.0, "not-found", "XenoNet has no such actor to pair");
            }
            let theirs = product(info.get("product").and_then(|p| p.get("id")));
            let matches = match (theirs, &adapter.product) {
                (Some(p), Some(a)) => p.id == a,
                (None, None) => true,
                _ => false,
            };
            if !matches {
                let wanted = match &adapter.product {
                    Some(p) => product_of(p).map_or("undefined", |p| p.name).to_string(),
                    None => "an actor of your own".to_string(),
                };
                return fail(400.0, "bad-request", format!("{} isn't {wanted}", xenonet_base::text::template(info.get("name"))));
            }
            return Ok(Plan { id, create: None });
        }
        if Self::remote_actors(h) >= REMOTE_ACTORS_MAX {
            return fail(409.0, "full", format!("{REMOTE_ACTORS_MAX} remote actors are paired already; revoke or remove one first"));
        }
        let made = adapter.product.as_deref().and_then(product_of);
        let (slug, name) = if let Some(p) = made {
            let slug = if p.id == "dots" {
                let mut slug = "dots".to_string();
                let mut i = 0;
                while Self::is_taken(h, &slug) {
                    if i > 50 {
                        return fail(409.0, "taken", "XenoNet couldn't find a free slug for another dot");
                    }
                    slug = format!("dots-{}", random_hex(2)?);
                    i += 1;
                }
                slug
            } else {
                if Self::is_taken(h, p.slug) {
                    return fail(409.0, "taken", format!("{} is beside the map already, as actor:{}: pair that actor instead", p.name, p.slug));
                }
                p.slug.to_string()
            };
            let raw = match ask.get("name").filter(|v| !v.is_null()) {
                None => Some(p.short.to_string()),
                Some(raw) => raw.as_str().map(str::to_string),
            };
            let name = match check_name(raw.as_deref(), Some(p.id)) {
                Ok(n) => n,
                Err(reason) => return fail(400.0, "bad-request", reason),
            };
            (slug, name)
        } else {
            let name = match check_name(ask.get("name").and_then(Value::as_str), None) {
                Ok(n) => n,
                Err(reason) => return fail(400.0, "bad-request", reason),
            };
            let slug = match ask.get("slug").filter(|v| !v.is_null()) {
                None => Value::String(slug_from(&name)),
                Some(given) => given.clone(),
            };
            if let Some(why) = slug_refusal(slug.as_str(), None) {
                return fail(400.0, "bad-request", why);
            }
            let slug = js_string(&slug);
            if Self::is_taken(h, &slug) {
                let suggest = suggest_slug(&slug, None, &mut |s| Ok::<bool, Fail>(Self::is_taken(&mut *h, s)))?;
                return Err(Fail::Connection { status: 409.0, code: "taken", message: format!("actor:{slug} is taken"), extra: json!({ "suggest": suggest }) });
            }
            (slug, name)
        };
        let product = made.map_or(Value::Null, |p| json!({ "id": p.id, "vendor": p.vendor, "version": null }));
        Ok(Plan { id: format!("actor:{slug}"), create: Some(json!({ "kind": "agent", "name": name, "product": product })) })
    }

    /// A new connection and its grant (the window's Pair). Your view of it,
    /// with its launch command; the credential is only ever on disk.
    pub fn pair(&self, h: &mut dyn Host, ask: &Value) -> Result<Value> {
        let t = h.now();
        let Some(adapter) = h.adapter_for(ask.get("product").filter(|p| !p.is_null())) else {
            return fail(400.0, "bad-request", "that product can't be paired as a remote actor");
        };
        let transport = ask.get("transport").and_then(Value::as_str);
        let known = TRANSPORTS.iter().any(|x| transport == Some(x.id));
        if !known || !transport.is_some_and(|t| adapter.transports.iter().any(|x| x == t)) {
            let who = match &adapter.product {
                Some(p) => product_of(p).map_or("undefined", |p| p.name),
                None => "An actor of your own",
            };
            let names: Vec<&str> = adapter.transports.iter().map(|x| TRANSPORTS.iter().find(|t| x == t.id).map_or("undefined", |t| t.name)).collect();
            return fail(400.0, "bad-request", format!("{who} reaches XenoNet through {}", names.join(" or ")));
        }
        let transport = Value::String(transport.unwrap_or_default().to_string());
        let Some(label) = words(ask.get("label"), "a connection's name", 64)? else {
            return fail(400.0, "bad-request", "a connection needs a name");
        };
        let alias = words(ask.get("deviceAlias"), "this computer's name", 64)?;
        let audience = ask.get("audience");
        let Some(description) = words(audience.and_then(|a| a.get("description")), "who can call it", 200)? else {
            return fail(400.0, "bad-request", "say who can call this connection's endpoint");
        };
        let verification = audience.and_then(|a| a.get("verification")).cloned().unwrap_or(Value::Null);
        if !matches!(verification.as_str(), Some("owner_verified" | "unverified")) {
            return fail(400.0, "bad-request", "say whether you verified who can call it: owner_verified or unverified");
        }
        let scope = audience.and_then(|a| a.get("scope")).cloned().unwrap_or(Value::Null);
        if !scope.as_str().is_some_and(|s| TRANSPORT_SCOPES.contains(&s)) {
            return fail(400.0, "bad-request", format!("its scope is {}", TRANSPORT_SCOPES.join(", ")));
        }
        let days = match ask.get("days").filter(|d| !d.is_null()) {
            None => DEFAULT_GRANT_DAYS,
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            Some(_) => return fail(400.0, "bad-request", format!("a grant lasts {} days", GRANT_DAYS.map(|d| d.to_string()).join(", "))),
        };
        if !GRANT_DAYS.contains(&days) {
            return fail(400.0, "bad-request", format!("a grant lasts {} days", GRANT_DAYS.map(|d| d.to_string()).join(", ")));
        }
        let lookback = match ask.get("lookbackDays").filter(|d| !d.is_null()) {
            None => LOOKBACK_DAYS,
            Some(Value::Number(n)) if n.as_f64().is_some_and(|n| n.is_finite() && n.fract() == 0.0 && (1.0..=LOOKBACK_DAYS).contains(&n)) => n.as_f64().unwrap_or(LOOKBACK_DAYS),
            Some(_) => return fail(400.0, "bad-request", format!("it reads back 1 to {LOOKBACK_DAYS} days")),
        };
        let chosen = unique(ask.get("projects").filter(|p| !p.is_null()));
        if chosen.len() > PROJECTS_MAX {
            return fail(400.0, "bad-request", format!("a grant names at most {PROJECTS_MAX} projects"));
        }
        for p in &chosen {
            if !h.places_has(&js_string(p)) {
                return fail(400.0, "bad-request", "pick projects from the map as it is now");
            }
        }
        let account_link = ask.get("accountLink");
        let link = account_link.and_then(|a| a.get("state")).filter(|s| !s.is_null()).cloned().unwrap_or_else(|| json!("unverified"));
        if link.as_str() == Some("verified") {
            return fail(400.0, "bad-request", "only its provider can verify an account; this connection's can't yet, so choose unverified or your own word");
        }
        if !matches!(link.as_str(), Some("unverified" | "owner_asserted")) {
            return fail(400.0, "bad-request", "an account link is unverified or owner_asserted");
        }
        let asserted = link.as_str() == Some("owner_asserted");
        let identity = account_link.and_then(|a| a.get("identity"));
        if asserted && !h.identities_has(&xenonet_base::text::template(identity)) {
            return fail(400.0, "bad-request", "pick one of the identities XenoNet knows");
        }
        if self.active_grants(h, t)? >= CONNECTIONS_MAX {
            return fail(409.0, "full", format!("{CONNECTIONS_MAX} connections are active already; revoke one first"));
        }
        let plan = Self::plan_actor(h, ask, &adapter)?;

        let dev = self.device(h)?;
        let connection_id = new_id("cn")?;
        let grant_id = new_id("gr")?;
        let (made_token, made_hash) = h.new_token();
        let expires = t + days * DAY_MS;
        // The credential first: if it can't be kept private, nothing is made.
        h.write_credential(&grant_id, &Value::String(made_token), &dev, &number(expires))?;
        let made_all = self.transaction(|| -> Result<()> {
            match &plan.create {
                Some(create) => {
                    let mut def = Map::new();
                    def.insert("id".into(), json!(plan.id));
                    if let Value::Object(fields) = create {
                        def.extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
                    }
                    def.insert("via".into(), json!(["remote"]));
                    def.insert("heartbeatMs".into(), Value::Null);
                    def.insert("capabilities".into(), json!([]));
                    def.insert("identity".into(), Value::Null);
                    def.insert("at".into(), number(t));
                    h.upsert(&Value::Object(def))?;
                }
                None => {
                    h.add_via(&plan.id, "remote")?;
                }
            }
            let slug: String = plan.id.chars().skip("actor:".len()).collect();
            let mut projects = Vec::new();
            for place in &chosen {
                projects.push(self.store.project_id_of(place, &mut || Ok(Value::String(uuid_id("prj")?)), t)?);
            }
            let connection = json!({
                "connection_id": connection_id,
                "actor_slug": slug,
                "adapter_id": adapter.id,
                "provider": adapter.provider,
                "transport_kind": transport,
                "transport_binding": null,
                "label": label,
                "device_alias": alias,
                "audience_description": description,
                "audience_verification": verification,
                "audience_scope": scope,
                "created_at": number(t),
            });
            self.store.add_connection(&connection)?;
            let capabilities = h.picked_capabilities()?;
            let grant = json!({
                "grant_id": grant_id,
                "connection_id": connection_id,
                "device_id": dev,
                "permissions": stringify(&json!(capabilities), 0),
                "project_allowlist": stringify(&Value::Array(projects.clone()), 0),
                "field_policy": stringify(&json!({ "labels": ask.get("labels") == Some(&Value::Bool(true)) }), 0),
                "data_mode": if verification.as_str() == Some("owner_verified") { "real" } else { "fixture" },
                "lookback_ms": number(lookback * DAY_MS),
                "token_hash": made_hash,
                "created_at": number(t),
                "expires_at": number(expires),
            });
            self.store.add_grant(&grant)?;
            if asserted {
                let known = h.identities();
                account_links::assert_link(self.links(), &slug, identity.unwrap_or(&Value::Null), &adapter.provider, Some(&known), t)?;
            }
            let audit = json!({
                "at": number(t),
                "kind": "owner",
                "grant_id": grant_id,
                "grant_revision": 1,
                "actor": plan.id,
                "connection_id": connection_id,
                "operation": "pair",
                "result": "ok",
                "projects": number(projects.len() as f64),
            });
            self.store.add_audit(&audit)?;
            Ok(())
        });
        if let Err(e) = made_all {
            h.delete_credential(&grant_id)?;
            return Err(e);
        }
        self.note_revision(h, &grant_id)?;
        self.sync(h, t)?;
        h.emit("changed", json!({ "grant": grant_id }));
        let g = self.store.grant(&grant_id)?;
        self.view_of(h, g.as_ref(), t)
    }

    // ---------- changing one ----------

    fn active(&self, h: &mut dyn Host, grant_id: &str, t: f64) -> Result<Value> {
        let g = if is_grant_id(grant_id) { self.store.grant(grant_id)? } else { None };
        let Some(g) = g else {
            return fail(404.0, "not-found", "XenoNet has no such connection");
        };
        if Self::state_of(h, &g, t) != "active" && Self::state_of(h, &g, t) != "disabled" {
            let state = Self::state_of(h, &g, t);
            return fail(409.0, "not-active", format!("that connection is {state}"));
        }
        Ok(g)
    }

    /// New projects or fields (the window): what it reads changes now, and
    /// its revision moves on. Taking projects away needs nothing more;
    /// adding them is a new approval, which is why this is the window's.
    pub fn update(&self, h: &mut dyn Host, grant_id: &str, change_to: &Value) -> Result<Value> {
        let t = h.now();
        let g = self.active(h, grant_id, t)?;
        let mut change = Map::new();
        let mut count = None;
        if let Some(projects) = change_to.get("projects") {
            let chosen = unique(Some(projects));
            if chosen.len() > PROJECTS_MAX {
                return fail(400.0, "bad-request", format!("a grant names at most {PROJECTS_MAX} projects"));
            }
            // A place the grant names already may stay, on the map or not.
            let had = items(parse(&col(&g, "project_allowlist"), json!([])));
            for p in &chosen {
                if !h.places_has(&js_string(p)) {
                    let id = self.store.project_id_for(p)?;
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
                ids.push(self.store.project_id_of(place, &mut || Ok(Value::String(uuid_id("prj")?)), t)?);
            }
            count = Some(ids.len());
            change.insert("project_allowlist".into(), Value::String(stringify(&Value::Array(ids), 0)));
        }
        if let Some(labels) = change_to.get("labels") {
            let mut policy = Map::new();
            match parse(&col(&g, "field_policy"), json!({})) {
                Value::Object(o) => policy.extend(o),
                Value::Array(a) => policy.extend(a.into_iter().enumerate().map(|(i, v)| (i.to_string(), v))),
                Value::String(s) => policy.extend(s.chars().enumerate().map(|(i, ch)| (i.to_string(), Value::String(ch.to_string())))),
                _ => {}
            }
            policy.insert("labels".into(), Value::Bool(*labels == Value::Bool(true)));
            change.insert("field_policy".into(), Value::String(stringify(&Value::Object(policy), 0)));
        }
        if change.is_empty() {
            return self.view_of(h, Some(&g), t);
        }
        let revision = num(&col(&g, "revision")) + 1.0;
        let change = Value::Object(change);
        self.transaction(|| -> Result<()> {
            self.store.change_grant(grant_id, &change)?;
            let audit = json!({
                "at": number(t),
                "kind": "owner",
                "grant_id": grant_id,
                "grant_revision": number(revision),
                "connection_id": col(&g, "connection_id"),
                "operation": "change",
                "result": "ok",
                "projects": count.map_or(Value::Null, |n| number(n as f64)),
            });
            self.store.add_audit(&audit)?;
            Ok(())
        })?;
        self.note_revision(h, grant_id)?;
        h.emit("changed", json!({ "grant": grant_id }));
        let g = self.store.grant(grant_id)?;
        self.view_of(h, g.as_ref(), t)
    }

    /// A new credential: the old one stops at once, and adapters holding
    /// it are let go.
    pub fn rotate(&self, h: &mut dyn Host, grant_id: &str) -> Result<Value> {
        let t = h.now();
        let g = self.active(h, grant_id, t)?;
        let (token, hash) = h.new_token();
        h.write_credential(grant_id, &Value::String(token), &col(&g, "device_id"), &col(&g, "expires_at"))?;
        let revision = num(&col(&g, "revision")) + 1.0;
        self.transaction(|| -> Result<()> {
            self.store.change_grant(grant_id, &json!({ "token_hash": hash }))?;
            let audit = json!({
                "at": number(t),
                "kind": "owner",
                "grant_id": grant_id,
                "grant_revision": number(revision),
                "connection_id": col(&g, "connection_id"),
                "operation": "rotate",
                "result": "ok",
            });
            self.store.add_audit(&audit)?;
            Ok(())
        })?;
        self.note_revision(h, grant_id)?;
        h.emit("rotated", json!({ "grant": grant_id, "connection": col(&g, "connection_id") }));
        let g = self.store.grant(grant_id)?;
        self.view_of(h, g.as_ref(), t)
    }

    pub fn revoke(&self, h: &mut dyn Host, grant_id: &str, reason: &str) -> Result<Value> {
        let g = if is_grant_id(grant_id) { self.store.grant(grant_id)? } else { None };
        let Some(g) = g else {
            return fail(404.0, "not-found", "XenoNet has no such connection");
        };
        let t = h.now();
        let changed = if is_null(&g, "revoked_at") { self.revoke_row(h, &g, reason, t, "owner")? } else { false };
        self.sync(h, t)?;
        Ok(json!({ "revoked": grant_id, "already": !changed }))
    }

    /// Every connection off, until you turn them on again; adapters are
    /// let go at once.
    pub fn disable(&self, h: &mut dyn Host) -> Result<Value> {
        let t = h.now();
        h.mark_disabled(t)?;
        self.store.add_audit(&json!({ "at": number(t), "kind": "owner", "operation": "disable", "result": "ok" }))?;
        self.sync(h, t)?;
        h.emit("disabled", json!({}));
        Ok(json!({ "disabled": true }))
    }

    pub fn enable(&self, h: &mut dyn Host) -> Result<Value> {
        let t = h.now();
        h.clear_disabled()?;
        self.store.add_audit(&json!({ "at": number(t), "kind": "owner", "operation": "enable", "result": "ok" }))?;
        self.sync(h, t)?;
        h.emit("enabled", json!({}));
        Ok(json!({ "disabled": false }))
    }

    // ---------- an adapter's calls ----------

    /// How a grant stands now, for a call through it, where the code is for
    /// the caller and the reason for the log alone.
    pub fn standing(&self, h: &mut dyn Host, g: Option<&Value>, t: Option<f64>) -> Result<Standing> {
        let t = match t {
            Some(t) => t,
            None => h.now(),
        };
        let Some(g) = g else {
            return Ok(Standing::no("NOT_AUTHORIZED", "unknown-grant"));
        };
        if h.is_disabled() {
            return Ok(Standing::no("DISABLED", "disabled"));
        }
        if !is_null(g, "revoked_at") || h.is_marked_revoked(&text(g, "grant_id")) {
            let reason = match g.get("revoked_reason") {
                None | Some(Value::Null) => json!("revoked"),
                Some(r) => r.clone(),
            };
            return Ok(Standing::No { code: "GRANT_REVOKED", reason });
        }
        if num(&col(g, "expires_at")) <= t {
            return Ok(Standing::no("GRANT_EXPIRED", "expired"));
        }
        let device = self.device(h)?;
        if !matches!((g.get("device_id"), &device), (Some(Value::String(a)), Value::String(b)) if a == b) {
            return Ok(Standing::no("NOT_AUTHORIZED", "device-changed"));
        }
        let conn = self.store.connection(&text(g, "connection_id"))?;
        let Some(conn) = conn.filter(|r| r.get("state").and_then(Value::as_str) == Some("active")) else {
            return Ok(Standing::no("GRANT_REVOKED", "connection-ended"));
        };
        let actor_id = format!("actor:{}", text(&conn, "actor_slug"));
        if !h.has(&actor_id) {
            return Ok(Standing::no("NOT_AUTHORIZED", "actor-gone"));
        }
        Ok(Standing::Ok { revision: col(g, "revision"), grant: g.clone(), connection: conn, actor_id })
    }

    /// An adapter saying which grant it's for, with the grant's credential.
    /// A wrong credential is refused the same way as an unknown grant; one
    /// that's right hears how its grant stands.
    pub fn authenticate(&self, h: &mut dyn Host, ask: &Value) -> Result<Standing> {
        let (grant, token) = (col(ask, "grant"), col(ask, "token"));
        if !is_grant_id(&js_string(&grant)) || !h.is_token(&token) {
            return Ok(Standing::no("NOT_AUTHORIZED", "malformed"));
        }
        let Some(g) = self.store.grant(&js_string(&grant))? else {
            return Ok(Standing::no("NOT_AUTHORIZED", "unknown-grant"));
        };
        let theirs = hex_bytes(&h.hash_token(&token));
        let ours = hex_bytes(&text(&g, "token_hash"));
        if theirs.len() != ours.len() || !timing_safe_eq(&theirs, &ours) {
            return Ok(Standing::no("NOT_AUTHORIZED", "wrong-token"));
        }
        self.standing(h, Some(&g), None)
    }

    /// The same, again, by grant and the revision a read began under, just
    /// before its answer goes: changed is true when the revision moved on.
    pub fn recheck(&self, h: &mut dyn Host, grant_id: &str, revision: &Value) -> Result<Value> {
        let g = self.store.grant(grant_id)?;
        let s = self.standing(h, g.as_ref(), None)?;
        let changed = match &s {
            Standing::Ok { revision: r, .. } => Some(!matches!((r.as_f64(), revision.as_f64()), (Some(a), Some(b)) if a == b)),
            Standing::No { .. } => None,
        };
        let mut out = s.to_json();
        if let (Some(changed), Value::Object(fields)) = (changed, &mut out) {
            fields.insert("changed".into(), Value::Bool(changed));
        }
        Ok(out)
    }

    /// What a read through a grant may see. `places` comes back as
    /// [project, place] pairs.
    pub fn policy(&self, h: &mut dyn Host, s: &Value) -> Result<Value> {
        let (g, conn) = (col(s, "grant"), col(s, "connection"));
        let actor_id = col(s, "actorId");
        let info = h.info(&js_string(&actor_id));
        let parse_col = |v: &Value, fallback: Value| -> Value {
            match v {
                Value::Null => fallback,
                v => serde_json::from_str(&js_string(v)).unwrap_or(fallback),
            }
        };
        let projects = parse_col(&col(&g, "project_allowlist"), json!([]));
        let mut places = Vec::new();
        for p in items(projects.clone()) {
            let place = self.store.place_of_project(&p)?;
            if truthy(Some(&place)) {
                places.push(json!([p, place]));
            }
        }
        let wall = h.wall();
        let (link, _) = account_links::link_state(self.links(), &text(&conn, "actor_slug"), wall)?;
        let labels = parse_col(&col(&g, "field_policy"), json!({}));
        Ok(json!({
            "grantId": col(&g, "grant_id"),
            "revision": col(&g, "revision"),
            "actorId": actor_id,
            "connectionId": col(&conn, "connection_id"),
            "deviceId": col(&g, "device_id"),
            "deviceAlias": col(&conn, "device_alias"),
            "productId": product(info.get("product").and_then(|p| p.get("id"))).map_or(Value::Null, |p| json!(p.id)),
            "accountLink": link,
            "projects": projects,
            "places": places,
            "labels": labels.get("labels") == Some(&Value::Bool(true)),
            "lookbackMs": col(&g, "lookback_ms"),
            "dataMode": if g.get("data_mode").and_then(Value::as_str) == Some("real") { "real" } else { "fixture" },
            "transportScope": col(&conn, "audience_scope"),
            "audienceLabel": col(&conn, "label"),
            "permissions": parse_col(&col(&g, "permissions"), json!([])),
        }))
    }

    /// A read went out through a grant: when it was last used, written at
    /// most once a minute.
    pub fn used(&self, grant_id: &str, t: f64) -> Result<()> {
        {
            let mut touched = self.touched.locked();
            if t - touched.get(grant_id).copied().unwrap_or(f64::NEG_INFINITY) < TOUCH_MS {
                return Ok(());
            }
            touched.insert(grant_id.to_string(), t);
        }
        self.store.touch_grant(grant_id, t)?;
        Ok(())
    }

    /// An adapter connected through a connection, or went (`delta` +1 or -1).
    pub fn bridge(&self, h: &mut dyn Host, connection_id: &str, delta: f64) -> Result<()> {
        {
            let mut bridges = self.bridges.locked();
            let had = bridges.iter().find(|(k, _)| k == connection_id).map_or(0.0, |(_, n)| *n);
            let n = (had + delta).max(0.0);
            let n = if (had + delta).is_nan() { f64::NAN } else { n };
            if n != 0.0 && !n.is_nan() {
                match bridges.iter_mut().find(|(k, _)| k == connection_id) {
                    Some(e) => e.1 = n,
                    None => bridges.push((connection_id.to_string(), n)),
                }
            } else {
                bridges.retain(|(k, _)| k != connection_id);
            }
        }
        let t = h.now();
        self.sync(h, t)
    }

    // ---------- your view ----------

    fn view_of(&self, h: &mut dyn Host, g: Option<&Value>, t: f64) -> Result<Value> {
        let Some(g) = g else {
            return error("Cannot read properties of undefined (reading 'connection_id')");
        };
        let Some(conn) = self.store.connection(&text(g, "connection_id"))? else {
            return error("Cannot read properties of undefined (reading 'actor_slug')");
        };
        let (link, identity) = account_links::link_state(self.links(), &text(&conn, "actor_slug"), t)?;
        let mut projects = Vec::new();
        for p in items(parse(&col(g, "project_allowlist"), json!([]))) {
            let place = self.store.place_of_project(&p)?;
            let name = if truthy(Some(&place)) { h.describe(&js_string(&place)) } else { Value::Null };
            projects.push(json!({ "project_id": p, "place": place, "name": name }));
        }
        let state = Self::state_of(h, g, t);
        let policy = parse(&col(g, "field_policy"), json!({}));
        Ok(json!({
            "schema": "actor/connection/v1",
            "connection_id": col(&conn, "connection_id"),
            "grant_id": col(g, "grant_id"),
            "actor_id": format!("actor:{}", text(&conn, "actor_slug")),
            "label": col(&conn, "label"),
            "adapter_id": col(&conn, "adapter_id"),
            "provider": col(&conn, "provider"),
            "transport_kind": col(&conn, "transport_kind"),
            "audience": {
                "description": col(&conn, "audience_description"),
                "verification": col(&conn, "audience_verification"),
                "scope": col(&conn, "audience_scope"),
            },
            "account_link": { "state": link, "identity": identity },
            "device": { "id": col(g, "device_id"), "alias": col(&conn, "device_alias") },
            "permissions": parse(&col(g, "permissions"), json!([])),
            "data_mode": if g.get("data_mode").and_then(Value::as_str) == Some("real") { "real" } else { "fixture" },
            "projects": projects,
            "labels": policy.get("labels") == Some(&Value::Bool(true)),
            "lookback_days": number(js_round(num(&col(g, "lookback_ms")) / DAY_MS)),
            "revision": col(g, "revision"),
            "state": state,
            "bridge": if self.bridged(&col(&conn, "connection_id")) { "connected" } else { "not_connected" },
            "created_at": iso(&col(g, "created_at")),
            "expires_at": iso(&col(g, "expires_at")),
            "revoked_at": iso(&col(g, "revoked_at")),
            "revoked_reason": col(g, "revoked_reason"),
            "last_used_at": iso(&col(g, "last_used_at")),
            "launch": launch_for(&text(g, "grant_id"), self.port),
        }))
    }

    /// Your view of one grant, or null.
    pub fn view(&self, h: &mut dyn Host, grant_id: &str) -> Result<Value> {
        match self.store.grant(grant_id)? {
            Some(g) => {
                let t = h.now();
                self.view_of(h, Some(&g), t)
            }
            None => Ok(Value::Null),
        }
    }

    pub fn list(&self, h: &mut dyn Host) -> Result<Value> {
        let t = h.now();
        let disabled = h.is_disabled();
        let device = self.device(h)?;
        let mut views = Vec::new();
        for g in self.store.grants()? {
            views.push(self.view_of(h, Some(&g), t)?);
        }
        views.reverse();
        Ok(json!({ "at": number(t), "disabled": disabled, "device": { "id": device }, "connections": views }))
    }

    /// On the clock: grants that expired let their adapters go.
    pub fn tick(&self, h: &mut dyn Host, t: f64) -> Result<()> {
        for g in self.store.grants()? {
            let id = text(&g, "grant_id");
            if !is_null(&g, "revoked_at") || num(&col(&g, "expires_at")) > t || self.expired.locked().contains(&id) {
                continue;
            }
            self.expired.locked().insert(id);
            h.emit("expired", json!({ "grant": col(&g, "grant_id"), "connection": col(&g, "connection_id") }));
        }
        self.sync(h, t)
    }

    /// Removing an actor (yours) revokes its connections first, while its
    /// grants can still be named; the rows then go with the actor's.
    pub fn before_remove(&self, h: &mut dyn Host, slug: &str) -> Result<()> {
        let t = h.now();
        for conn in self.store.connections_of(slug)? {
            let g = self.store.grants()?.into_iter().find(|x| same_col(&col(x, "connection_id"), &col(&conn, "connection_id")));
            if let Some(g) = g {
                if is_null(&g, "revoked_at") {
                    self.revoke_row(h, &g, "actor-removed", t, "owner")?;
                }
            }
        }
        Ok(())
    }

    /// Whether any grant could be used now.
    pub fn any_active(&self, h: &mut dyn Host) -> Result<bool> {
        if h.is_disabled() {
            return Ok(false);
        }
        for g in self.store.grants()? {
            let t = h.now();
            if Self::state_of(h, &g, t) == "active" {
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
        assert_eq!(words(Some(&json!("  My\n dot ")), "a name", 64).unwrap(), Some("My dot".into()));
        assert_eq!(words(Some(&json!("")), "a name", 64).unwrap(), None);
        assert!(words(Some(&json!(1.0)), "a name", 64).is_err());
        assert!(words(Some(&json!("x".repeat(65))), "a name", 64).is_err());
    }

    #[test]
    fn launch_commands_name_a_port_only_when_its_not_the_default() {
        let a = launch_for("gr_1", 47840.0);
        assert_eq!(a["args"].as_array().unwrap().len(), 4);
        let b = launch_for("gr_1", 5000.0);
        assert_eq!(b["args"][5], json!("5000"));
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
