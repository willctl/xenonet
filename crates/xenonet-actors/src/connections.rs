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
//! disk, the actors' grants, the clock, the adapters, the map's places and
//! the identities for a pairing, and whoever listens for events. A
//! connection's credential is its actor's grant, the same kind every actor
//! holds (`xenonet_actor_`, with the right to read), issued with you there.
//! The engine keeps only its revocation id, and what it may read beside
//! the right (its projects) in the grant's scope row under that id. A
//! rotation, a revocation or the end of a grant takes its token back for
//! good; a change of scope changes the row and moves the revision on.

use crate::account_links::{self, LinkFail, Links};
use crate::adapters::TRANSPORTS;
use crate::detail::looks_like_key;
use crate::names::{check_name, slug_from, slug_refusal, suggest_slug};
use crate::re;
use crate::store;
use crate::tokens::is_token;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use xenonet_adapter::products::{product_of, Product};
use xenonet_model::ids::YOU;
use xenonet_os::sync::Locked;
use xenonet_text::json::{number, stringify};
use xenonet_text::text::{display, truthy};

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
    fn mark_disabled(&mut self, at: f64) -> Result<()>;
    fn clear_disabled(&mut self) -> Result<()>;
    // The actors' grants (xenonet-authz's Biscuits, minted from this
    // machine's token root), by the actor's slug.
    /// You, there, for issuing `slug` a grant with these rights and scopes
    /// or widening one to them: Ok, or why not.
    fn present(&mut self, slug: &str, rights: &[String], scopes: &Value, reason: &str) -> Result<()>;
    /// A grant for `slug` with these rights, until `until` (ms): its text,
    /// and the revocation id XenoNet keeps of it.
    fn mint_token(&mut self, slug: &str, rights: &[String], until: f64) -> Result<(String, String)>;
    /// Whether `token` is `slug`'s grant with the right to `action` now:
    /// its revocation id, or `revoked`, `no-right` or `invalid`.
    fn check_token(&mut self, slug: &str, token: &str, action: &str) -> std::result::Result<String, &'static str>;
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
    /// A token taken back for good, by its revocation id: `{ id, actor,
    /// reason, at }`, where the first reason stands.
    fn revoke_token(&self, row: &Value) -> store::Result<()>;
    /// A grant's scope row, made or changed: `{ token, actor, projects?,
    /// at, until? }`, where a field left out stays.
    fn put_scope(&self, row: &Value) -> store::Result<()>;
    /// A grant's scope row, by its token's revocation id.
    fn scope(&self, token: &str) -> store::Result<Option<Value>>;
    fn drop_scope(&self, token: &str) -> store::Result<()>;
    fn add_audit(&self, row: &Value) -> store::Result<()>;
    /// The project a place on the map gives rights in (the store's
    /// `prj_…`, the id every other surface names it by), or null for a
    /// place that isn't a confirmed checkout of one.
    fn project_of(&self, place: &Value) -> store::Result<Value>;
    /// The places a project is read through here: its confirmed
    /// checkouts not gone, by path.
    fn places_of_project(&self, project_id: &Value) -> store::Result<Vec<Value>>;
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
    display(&col(r, k))
}

fn is_null(r: &Value, k: &str) -> bool {
    r.get(k).is_none_or(Value::is_null)
}

/// Number(column), as a column compares with a number.
fn num(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(_) => xenonet_text::text::to_number(Some(v)),
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
        other => display(other),
    };
    serde_json::from_str(&text).unwrap_or(fallback)
}

/// A time column as an ISO date, or null.
fn iso(c: &Value) -> Value {
    match c {
        Value::Null => Value::Null,
        c => Value::String(xenonet_text::time::iso(num(c))),
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

fn is_grant_id(v: &str) -> bool {
    re!(r"^gr_[0-9a-f]{24}$").is_match(v)
}

/// What a connection's grant may do: read, and nothing else.
pub const RIGHTS: [&str; 1] = ["read"];

fn rights() -> Vec<String> {
    RIGHTS.iter().map(ToString::to_string).collect()
}

/// What you approve when a grant is issued or widened: the places it may
/// read, as you picked them, and whether names and titles go too.
fn scopes(places: &[Value], labels: bool) -> Value {
    json!({ "projects": places, "labels": labels })
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
    let line = xenonet_text::text::trim(&line);
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
        args.push(Value::String(display(&number(port))));
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
        let id = display(connection);
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

    /// A grant's token, as its row names it, taken back for good with its
    /// scope: the ground refuses it from then on, whatever the grant's row
    /// says.
    fn take_back(&self, g: &Value, reason: &str, t: f64) -> Result<()> {
        let token = text(g, "token_id");
        self.store.revoke_token(&json!({ "id": token, "actor": self.actor_of(g)?, "reason": reason, "at": number(t) }))?;
        self.store.drop_scope(&token)?;
        Ok(())
    }

    /// The slug of a grant's actor, by its connection.
    fn slug_of(&self, g: &Value) -> Result<Option<String>> {
        Ok(self.store.connection(&text(g, "connection_id"))?.and_then(|c| c.get("actor_slug").and_then(Value::as_str).map(str::to_string)))
    }

    /// A grant's actor's id, as its token and scope name it.
    fn actor_of(&self, g: &Value) -> Result<String> {
        Ok(format!("actor:{}", self.slug_of(g)?.unwrap_or_else(|| "?".into())))
    }

    /// A grant's connection's name, for what you're asked to approve.
    fn label_of(&self, g: &Value) -> Result<String> {
        Ok(self.store.connection(&text(g, "connection_id"))?.map(|c| text(&c, "label")).unwrap_or_default())
    }

    /// The project ids a grant may read, as its scope row has them.
    fn projects_of(&self, g: &Value) -> Result<Vec<Value>> {
        let row = self.store.scope(&text(g, "token_id"))?;
        Ok(row.map_or_else(Vec::new, |r| items(parse(&col(&r, "projects"), json!([])))))
    }

    /// The places these project ids are read through, those still here.
    fn places_of(&self, ids: &[Value]) -> Result<Vec<Value>> {
        let mut places = Vec::new();
        for id in ids {
            places.extend(self.store.places_of_project(id)?);
        }
        Ok(places)
    }

    /// The projects these places give rights in, each once, or a refusal
    /// for a place that gives rights in none: a folder that isn't a
    /// checkout, or a candidate the person hasn't confirmed.
    fn projects_for(&self, places: &[Value]) -> Result<Vec<Value>> {
        let mut ids: Vec<Value> = Vec::new();
        for place in places {
            let id = self.store.project_of(place)?;
            if !truthy(Some(&id)) {
                return fail(400.0, "bad-request", "pick projects you've confirmed on the map");
            }
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    fn revoke_row(&self, h: &mut dyn Host, g: &Value, reason: &str, t: f64, by: &str) -> Result<bool> {
        let grant = text(g, "grant_id");
        let connection = text(g, "connection_id");
        h.mark_revoked(&grant, t)?;
        h.delete_credential(&grant)?;
        let changed = self.transaction(|| -> Result<bool> {
            let r = self.store.revoke_grant(&grant, reason, t)?;
            self.take_back(g, reason, t)?;
            self.store.end_connection(&connection, t)?;
            let slug = self.store.connection(&connection)?.map_or("?".to_string(), |r| match r.get("actor_slug") {
                None | Some(Value::Null) => "?".to_string(),
                Some(c) => display(c),
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
            let id = display(given);
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
                return fail(400.0, "bad-request", format!("{} isn't {wanted}", xenonet_text::text::template(info.get("name"))));
            }
            return Ok(Plan { id, create: None });
        }
        if Self::remote_actors(h) >= REMOTE_ACTORS_MAX {
            return fail(409.0, "full", format!("{REMOTE_ACTORS_MAX} remote actors are paired already; revoke or remove one first"));
        }
        let made = adapter.product.as_deref().and_then(product_of);
        let (slug, name) = if let Some(p) = made {
            // A product with an actor per pairing (OpenAI's dots) takes its
            // slug, then the slug with a suffix.
            let slug = if p.per_pair {
                let mut slug = p.slug.to_string();
                let mut i = 0;
                while Self::is_taken(h, &slug) {
                    if i > 50 {
                        return fail(409.0, "taken", "XenoNet couldn't find a free slug for another dot");
                    }
                    slug = format!("{}-{}", p.slug, random_hex(2)?);
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
            let slug = display(&slug);
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
            if !h.places_has(&display(p)) {
                return fail(400.0, "bad-request", "pick projects from the map as it is now");
            }
        }
        let projects = self.projects_for(&chosen)?;
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
        if asserted && !h.identities_has(&xenonet_text::text::template(identity)) {
            return fail(400.0, "bad-request", "pick one of the identities XenoNet knows");
        }
        if self.active_grants(h, t)? >= CONNECTIONS_MAX {
            return fail(409.0, "full", format!("{CONNECTIONS_MAX} connections are active already; revoke one first"));
        }
        let plan = Self::plan_actor(h, ask, &adapter)?;

        let dev = self.device(h)?;
        let connection_id = new_id("cn")?;
        let grant_id = new_id("gr")?;
        let expires = t + days * DAY_MS;
        // The actor's grant, issued with you there, before anything is made.
        let slug: String = plan.id.chars().skip("actor:".len()).collect();
        let labels = ask.get("labels") == Some(&Value::Bool(true));
        h.present(&slug, &rights(), &scopes(&chosen, labels), &format!("Pair {label}"))?;
        let (made_token, made_id) = h.mint_token(&slug, &rights(), expires)?;
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
                "field_policy": stringify(&json!({ "labels": labels }), 0),
                "data_mode": if verification.as_str() == Some("owner_verified") { "real" } else { "fixture" },
                "lookback_ms": number(lookback * DAY_MS),
                "token_id": made_id,
                "created_at": number(t),
                "expires_at": number(expires),
            });
            self.store.add_grant(&grant)?;
            self.store
                .put_scope(&json!({ "token": made_id, "actor": plan.id, "projects": stringify(&Value::Array(projects.clone()), 0), "at": number(t), "until": number(expires) }))?;
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
            // The grant minted for it is taken back too: nothing names it.
            h.delete_credential(&grant_id)?;
            self.store.revoke_token(&json!({ "id": made_id, "actor": plan.id, "reason": "pair-failed", "at": number(t) }))?;
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
        let had = self.projects_of(&g)?;
        let had_labels = parse(&col(&g, "field_policy"), json!({})).get("labels") == Some(&Value::Bool(true));
        let mut change = Map::new();
        let mut projects: Option<(Vec<Value>, Vec<Value>)> = None;
        if let Some(asked) = change_to.get("projects") {
            let chosen = unique(Some(asked));
            if chosen.len() > PROJECTS_MAX {
                return fail(400.0, "bad-request", format!("a grant names at most {PROJECTS_MAX} projects"));
            }
            // A place of a project the grant names already may stay, on
            // the map or not.
            for p in &chosen {
                if !h.places_has(&display(p)) {
                    let id = self.store.project_of(p)?;
                    if !truthy(Some(&id)) || !had.contains(&id) {
                        return fail(400.0, "bad-request", "pick projects from the map as it is now");
                    }
                }
            }
            let ids = self.projects_for(&chosen)?;
            projects = Some((chosen, ids));
        }
        let mut labels = had_labels;
        if let Some(asked) = change_to.get("labels") {
            let mut policy = Map::new();
            match parse(&col(&g, "field_policy"), json!({})) {
                Value::Object(o) => policy.extend(o),
                Value::Array(a) => policy.extend(a.into_iter().enumerate().map(|(i, v)| (i.to_string(), v))),
                Value::String(s) => policy.extend(s.chars().enumerate().map(|(i, ch)| (i.to_string(), Value::String(ch.to_string())))),
                _ => {}
            }
            labels = *asked == Value::Bool(true);
            policy.insert("labels".into(), Value::Bool(labels));
            change.insert("field_policy".into(), Value::String(stringify(&Value::Object(policy), 0)));
        }
        if change.is_empty() && projects.is_none() {
            return self.view_of(h, Some(&g), t);
        }
        // Taking projects or names away needs nothing more; adding any is a
        // new approval, with you there.
        let added = projects.as_ref().is_some_and(|(_, ids)| ids.iter().any(|id| !had.contains(id)));
        if added || (labels && !had_labels) {
            let places = match &projects {
                Some((chosen, _)) => chosen.clone(),
                None => self.places_of(&had)?,
            };
            let slug = self.slug_of(&g)?.unwrap_or_default();
            h.present(&slug, &rights(), &scopes(&places, labels), &format!("Let {} read more", self.label_of(&g)?))?;
        }
        let revision = num(&col(&g, "revision")) + 1.0;
        let change = Value::Object(change);
        self.transaction(|| -> Result<()> {
            self.store.change_grant(grant_id, &change)?;
            if let Some((_, ids)) = &projects {
                self.store.put_scope(&json!({ "token": col(&g, "token_id"), "actor": self.actor_of(&g)?, "projects": stringify(&json!(ids), 0), "at": number(t) }))?;
            }
            let audit = json!({
                "at": number(t),
                "kind": "owner",
                "grant_id": grant_id,
                "grant_revision": number(revision),
                "connection_id": col(&g, "connection_id"),
                "operation": "change",
                "result": "ok",
                "projects": projects.as_ref().map_or(Value::Null, |(_, ids)| number(ids.len() as f64)),
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
    /// it are let go. It's a grant issued again, so you're asked to be
    /// there; its scope goes with it.
    pub fn rotate(&self, h: &mut dyn Host, grant_id: &str) -> Result<Value> {
        let t = h.now();
        let g = self.active(h, grant_id, t)?;
        let slug = self.slug_of(&g)?.unwrap_or_default();
        let had = self.projects_of(&g)?;
        let labels = parse(&col(&g, "field_policy"), json!({})).get("labels") == Some(&Value::Bool(true));
        h.present(&slug, &rights(), &scopes(&self.places_of(&had)?, labels), &format!("Give {} a new credential", self.label_of(&g)?))?;
        let (token, made_id) = h.mint_token(&slug, &rights(), num(&col(&g, "expires_at")))?;
        h.write_credential(grant_id, &Value::String(token), &col(&g, "device_id"), &col(&g, "expires_at"))?;
        let revision = num(&col(&g, "revision")) + 1.0;
        self.transaction(|| -> Result<()> {
            self.store.change_grant(grant_id, &json!({ "token_id": made_id }))?;
            let scope = json!({ "token": made_id, "actor": self.actor_of(&g)?, "projects": stringify(&json!(had), 0), "at": number(t), "until": col(&g, "expires_at") });
            self.store.put_scope(&scope)?;
            self.take_back(&g, "rotated", t)?;
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
        let grant = display(&col(ask, "grant"));
        let token = ask.get("token").and_then(Value::as_str).filter(|t| is_token(t));
        let (true, Some(token)) = (is_grant_id(&grant), token) else {
            return Ok(Standing::no("NOT_AUTHORIZED", "malformed"));
        };
        let Some(g) = self.store.grant(&grant)? else {
            return Ok(Standing::no("NOT_AUTHORIZED", "unknown-grant"));
        };
        let Some(slug) = self.slug_of(&g)? else {
            return Ok(Standing::no("NOT_AUTHORIZED", "wrong-token"));
        };
        match h.check_token(&slug, token, "read") {
            // The grant's token as its row has it now.
            Ok(id) if Some(id.as_str()) == g.get("token_id").and_then(Value::as_str) => self.standing(h, Some(&g), None),
            // A token of the grant's, taken back: a grant that no longer
            // stands says why, and one that does was given a newer token.
            Ok(_) | Err("revoked") => match self.standing(h, Some(&g), None)? {
                Standing::Ok { .. } => Ok(Standing::no("NOT_AUTHORIZED", "wrong-token")),
                no @ Standing::No { .. } => Ok(no),
            },
            Err(_) => Ok(Standing::no("NOT_AUTHORIZED", "wrong-token")),
        }
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
        let info = h.info(&display(&actor_id));
        let parse_col = |v: &Value, fallback: Value| -> Value {
            match v {
                Value::Null => fallback,
                v => serde_json::from_str(&display(v)).unwrap_or(fallback),
            }
        };
        let projects = Value::Array(self.projects_of(&g)?);
        let mut places = Vec::new();
        for p in items(projects.clone()) {
            for place in self.store.places_of_project(&p)? {
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
        for p in self.projects_of(g)? {
            let place = self.store.places_of_project(&p)?.into_iter().next().unwrap_or(Value::Null);
            let name = if truthy(Some(&place)) { h.describe(&display(&place)) } else { Value::Null };
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
            self.take_back(&g, "expired", t)?;
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
