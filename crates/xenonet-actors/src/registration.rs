//! Registering actors (core/actors/registration.js;
//! docs/design/actors/interface.md, section 2). Every registered actor
//! exists because you allowed it:
//!
//!   - a program asks (register), and XenoNet answers at once with an id and
//!     a token that works only once you click Allow in XenoNet's window
//!     (decide); until then it opens only the actor's own GET
//!     /api/actors/me;
//!   - Deny, or ten minutes without an answer, revokes the token, and the
//!     same program asking again with the same name takes its old request's
//!     place;
//!   - adding an actor from the window (add) allows it at once, and a new
//!     token (rotate) is made there only;
//!   - a session may ask for an actor's gateway token (ask_gateway), but the
//!     answer never holds one: the ask waits as a card, and the token is
//!     made, and shown masked, only when you click Allow in the window;
//!   - removing an actor, or its leaving, revokes its token, releases its
//!     claims and takes its record; a product's actor that XenoNet also
//!     observes stays, as observed, and one XenoNet only observes can't be
//!     removed.
//!
//! Tokens are kept as their SHA-256 only. A token unused for 30 days is
//! revoked. Limits: 16 registered actors, 4 registrations waiting and 20
//! an hour.
//!
//! The actors and the clock are called through `Calls` ("actors", "now"),
//! as the JS called them; the store is the engine's own.

use crate::detail::{clip, status_line};
use crate::names::{check_name, product_word_refusal, slug_from, slug_refusal, suggest_slug};
use crate::registry::value_arg;
use crate::tokens::{hash_token, is_gateway_token, is_token, new_gateway_token, new_token};
use serde_json::{json, Map, Value};
use xenonet_base::ids::YOU;
use xenonet_base::paths::tilde;
use xenonet_engine::js::truthy;
use xenonet_engine::peer::{Calls, Thrown};
use xenonet_engine::products::{product_of, PRODUCTS, WHY_NOT};
use xenonet_engine::sql::{Arg, Row};
use xenonet_engine::store::Shared;

pub const REQUEST_MS: f64 = 10.0 * 60e3;
/// A token unused for 30 days stops working.
pub const TOKEN_UNUSED_MS: f64 = 30.0 * 24.0 * 3600e3;
pub const REGISTERED_MAX: usize = 16;
pub const WAITING_MAX: usize = 4;
pub const REQUESTS_PER_HOUR: usize = 20;
const HOUR_MS: f64 = 3600e3;
// When a token was last used is written at most this often, after its
// first use, which is written at once.
const TOUCH_MS: f64 = HOUR_MS;
const SWEEP_MS: f64 = 60e3;
const HEARTBEAT_MIN: f64 = 5000.0;
const HEARTBEAT_MAX: f64 = 3600e3;
const HEARTBEAT_FALLBACK: f64 = 60e3;
pub const CAPABILITIES: [&str; 8] = ["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents", "schedule", "browse", "use-computer"];

/// What a 401 revoked says, by its reason.
pub fn revoked_words(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "rotated" => "that token was replaced by a newer one",
        "denied" => "that registration was denied in XenoNet's window",
        "expired" => "that token expired: a registration lapses after ten minutes without an answer, and a token after 30 days without a call",
        "left" => "that actor left XenoNet",
        "removed" => "that actor was removed in XenoNet",
        _ => return None,
    })
}

pub const REVOKED_REASONS: [&str; 5] = ["rotated", "denied", "expired", "left", "removed"];

/// Whether a token last used (or made) `at` has gone unused for 30 days by
/// `t`: from that very millisecond, wherever it's asked.
pub fn token_expired(at: Option<f64>, t: f64) -> bool {
    at.is_none_or(|at| t - at >= TOKEN_UNUSED_MS)
}

/// An error a program can test: an HTTP status, a code from the
/// interface's table, a sentence for a person, and extra fields.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorError {
    pub status: f64,
    pub code: &'static str,
    pub message: String,
    pub extra: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Actor(ActorError),
    Thrown(Thrown),
}

impl From<Thrown> for Fail {
    fn from(t: Thrown) -> Self {
        Fail::Thrown(t)
    }
}

impl From<xenonet_engine::sql::SqlError> for Fail {
    fn from(e: xenonet_engine::sql::SqlError) -> Self {
        Fail::Thrown(e.into())
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn fail<T>(status: f64, code: &'static str, message: impl Into<String>, extra: Value) -> Result<T> {
    Err(Fail::Actor(ActorError { status, code, message: message.into(), extra }))
}

fn bad<T>(message: impl Into<String>) -> Result<T> {
    fail(400.0, "bad-request", message, json!({}))
}

fn id_of(slug: &str) -> String {
    format!("actor:{slug}")
}

fn parse(s: Option<&str>) -> Value {
    s.and_then(|s| xenonet_base::json::parse(s).ok()).unwrap_or(Value::Null)
}

fn text(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

// Number(v), as JS compares a value with a number.
fn to_number(v: &Arg) -> f64 {
    match v {
        Arg::Num(n) => *n,
        Arg::Big(n) => *n as f64,
        Arg::Bool(b) => f64::from(u8::from(*b)),
        Arg::Null => 0.0,
        Arg::Undef | Arg::Other | Arg::Obj(_) => f64::NAN,
        other => xenonet_base::text::js_number(Some(&other.to_json())),
    }
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=chunk.len() {
            out.push(B64URL[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn random_id(n: usize) -> Result<String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| Fail::Thrown(Thrown::new(format!("no random bytes: {e}"))))?;
    Ok(base64url(&b))
}

fn token() -> Result<(String, String)> {
    new_token().map_err(|e| Fail::Thrown(Thrown::new(format!("no random bytes: {e}"))))
}

/// A registration as asked, checked.
#[derive(Clone, Debug)]
pub struct Ask {
    pub name: String,
    pub slug: String,
    pub product: Value,
    pub capabilities: Vec<String>,
    pub ignored: Vec<String>,
    pub heartbeat_ms: Arg,
    pub pid: Arg,
}

impl Ask {
    fn json(&self) -> Value {
        json!({ "name": self.name, "slug": self.slug, "product": self.product, "capabilities": self.capabilities, "ignored": self.ignored, "heartbeatMs": self.heartbeat_ms.to_json(), "pid": self.pid.to_json() })
    }
    fn product_id(&self) -> Option<String> {
        self.product.get("id").filter(|v| !v.is_null()).map(|v| xenonet_engine::js::template(Some(v)))
    }
}

#[derive(Clone, Debug)]
struct GatewayAsk {
    id: String,
    slug: String,
    asker: Value,
    at: f64,
    expires_at: f64,
}

pub struct Registration {
    store: Shared,
    home: String,
    gateway_asks: Vec<GatewayAsk>,
    ask_times: Vec<f64>,
    last_sweep: f64,
    emits: Vec<(&'static str, Value)>,
}

fn product_id_of(info: &Value) -> Option<String> {
    info.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| xenonet_engine::js::template(Some(v)))
}

fn via_has(info: &Value, v: &str) -> bool {
    info.get("via").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(v)))
}

impl Registration {
    pub fn new(store: Shared, home: &str) -> Self {
        Registration { store, home: home.into(), gateway_asks: Vec::new(), ask_times: Vec::new(), last_sweep: f64::NEG_INFINITY, emits: Vec::new() }
    }

    /// The events told since this was last asked: ("requests" | "asked" |
    /// "asked-gateway", what it says).
    pub fn take_emits(&mut self) -> Vec<(&'static str, Value)> {
        std::mem::take(&mut self.emits)
    }

    fn now(c: &mut dyn Calls) -> Result<f64> {
        Ok(c.call("", "now", vec![])?.as_f64().unwrap_or(f64::NAN))
    }

    fn info(c: &mut dyn Calls, id: &str) -> Result<Value> {
        Ok(c.call("actors", "info", vec![Arg::text(id)])?.to_json())
    }

    fn is_registered(c: &mut dyn Calls, id: &str) -> Result<bool> {
        Ok(c.call("actors", "isRegistered", vec![Arg::text(id)])?.truthy())
    }

    fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let store = self.store.clone();
        store.transaction(|| f(self))
    }

    // ---------- what's asked ----------

    // A product's version or vendor, as the actor's own word: one line, no
    // key, 64 characters at most, nothing that closes a quote.
    fn word_of(value: &Arg, what: &str) -> Result<Value> {
        if value.is_nullish() {
            return Ok(Value::Null);
        }
        let Ok(line) = status_line(Some(&value.to_json())) else {
            return bad(format!("the product's {what} looks like it holds a key or a token"));
        };
        if line.as_deref().is_some_and(|l| l.chars().any(|c| matches!(c, '"' | '`' | '\\' | '\u{201c}' | '\u{201d}'))) {
            return bad(format!("the product's {what} can't hold quotation marks or backslashes, since it's shown quoted"));
        }
        Ok(line.map_or(Value::Null, |l| Value::String(clip(&l, 64))))
    }

    /// What a registration asks, checked as the interface's table has it.
    pub fn read_ask(body: &Arg, window: bool) -> Result<Ask> {
        let product_arg = body.field("product");
        let product_id = def_null(product_arg.field("id"));
        let known = product_id.as_str().and_then(product_of);
        if let Some(k) = known {
            if !k.registers {
                let why = WHY_NOT.iter().find(|(p, _)| *p == k.id).map(|(_, w)| *w).unwrap_or_default();
                return fail(403.0, "not-allowed", why, json!({}));
            }
        }
        let product = if let Some(k) = known {
            json!({ "id": k.id, "vendor": k.vendor, "version": Self::word_of(&product_arg.field("version"), "version")? })
        } else if product_id.truthy() {
            json!({ "id": product_id.to_json(), "vendor": Self::word_of(&product_arg.field("vendor"), "vendor")?, "version": Self::word_of(&product_arg.field("version"), "version")? })
        } else {
            Value::Null
        };
        if !product.is_null() && known.is_none() {
            let id_text = xenonet_engine::js::template(product.get("id"));
            let vendor = product.get("vendor").and_then(Value::as_str).filter(|v| !v.is_empty());
            let why = product_word_refusal(&id_text, "id").or_else(|| vendor.and_then(|v| product_word_refusal(v, "vendor")));
            if let Some(why) = why {
                return bad(why);
            }
        }
        let name = body.field("name");
        let name = check_name(name.as_str(), known.map(|k| k.id)).or_else(bad)?;
        let slug_arg = body.field("slug");
        let slug_given = def_null(slug_arg);
        let slug = if slug_given.is_nullish() { Arg::text(known.map_or_else(|| slug_from(&name), |k| k.slug.to_string())) } else { slug_given };
        if let Some(why) = slug_refusal(slug.as_str(), known.map(|k| k.id)) {
            return bad(why);
        }
        let slug = slug.as_str().unwrap().to_string();
        let mut asked: Vec<String> = Vec::new();
        for c in xenonet_engine::peer::list(def_null(body.field("capabilities")).or(Arg::List(vec![]))) {
            let c = text(&c);
            if !asked.contains(&c) {
                asked.push(c);
            }
        }
        let heartbeat = def_null(body.field("heartbeatMs")).or(Arg::Num(HEARTBEAT_FALLBACK));
        let h = to_number(&heartbeat);
        if h < HEARTBEAT_MIN || h > HEARTBEAT_MAX {
            return bad("heartbeatMs is 5,000 to 3,600,000 milliseconds");
        }
        let printable = |c: &str| c.chars().map(|ch| if (' '..='~').contains(&ch) { ch.to_string() } else { "?".repeat(ch.len_utf16()) }).collect::<String>();
        Ok(Ask {
            name,
            slug,
            product,
            capabilities: asked.iter().filter(|c| CAPABILITIES.contains(&c.as_str())).cloned().collect(),
            // Listed back to the program, and never shown on the card.
            ignored: asked.iter().filter(|c| !CAPABILITIES.contains(&c.as_str())).take(16).map(|c| clip(&printable(c), 40)).collect(),
            heartbeat_ms: heartbeat,
            pid: if window { Arg::Null } else { def_null(body.field("pid")) },
        })
    }

    // Whether a registration that is `product` joins the actor already
    // there rather than finding its slug taken: only a product XenoNet
    // knows joins its own actor.
    fn joins(there: &Value, product: Option<&str>) -> bool {
        truthy(Some(there)) && product.is_some_and(|p| product_of(p).is_some()) && product_id_of(there).as_deref() == product
    }

    // Whether a slug is someone else's: an actor's (unless this joins it),
    // or a registration waiting, other than `except`.
    fn taken_by(&self, c: &mut dyn Calls, slug: &str, product: Option<&str>, except: Option<&str>, t: Option<f64>) -> Result<bool> {
        let there = Self::info(c, &id_of(slug))?;
        if truthy(Some(&there)) && !Self::joins(&there, product) {
            return Ok(true);
        }
        let t = match t {
            Some(t) => t,
            None => Self::now(c)?,
        };
        Ok(self.store.with(|s| s.waiting_actor_requests(Arg::Num(t)))?.iter().any(|r| r.text("slug") == Some(slug) && r.text("id") != except))
    }

    fn taken<T>(&self, c: &mut dyn Calls, slug: &str, product: Option<&str>) -> Result<T> {
        let registered = Self::is_registered(c, &id_of(slug))?;
        let mut failed = None;
        let suggest = suggest_slug(slug, product, &mut |s| match self.taken_by(c, s, None, None, None) {
            Ok(b) => Ok(b),
            Err(e) => {
                failed = Some(e.clone());
                Err(e)
            }
        });
        if let Some(e) = failed {
            return Err(e);
        }
        let suggest = suggest?;
        fail(409.0, "taken", format!("actor:{slug} is taken{}", if registered { "; to give it a new token, use New token in XenoNet's Actors panel" } else { "" }), json!({ "suggest": suggest }))
    }

    fn full(c: &mut dyn Calls, id: &str) -> Result<bool> {
        Ok(!Self::is_registered(c, id)? && c.call("actors", "registeredCount", vec![])?.as_f64().unwrap_or(0.0) >= REGISTERED_MAX as f64)
    }

    fn full_error<T>() -> Result<T> {
        fail(409.0, "full", format!("{REGISTERED_MAX} actors are registered already; remove one in XenoNet's Actors panel first"), json!({}))
    }

    // What the card shows of who asked: the program, its parents up to the
    // first app or terminal, and its folder, with ~ for the home folder.
    fn asker_of(&self, chain: &[Value], cwd: &Value) -> Value {
        let file = |p: &Value| p.get("exe").filter(|e| !e.is_null()).or(p.get("program")).map(|v| xenonet_engine::js::template(Some(v))).unwrap_or_else(|| "undefined".into());
        let mut parents = Vec::new();
        for p in chain.iter().skip(1) {
            let f = file(p);
            parents.push(tilde(&f, &self.home));
            if f.contains(".app/") || parents.len() >= 8 {
                break;
            }
        }
        json!({ "program": tilde(&file(&chain[0]), &self.home), "parents": parents, "folder": if truthy(Some(cwd)) { Value::String(tilde(&xenonet_engine::js::template(Some(cwd)), &self.home)) } else { Value::Null } })
    }

    fn asker(&self, checked: &Arg, chain: &Arg, cwd: &Arg) -> Value {
        let chain: Vec<Value> = match chain {
            Arg::List(l) => l.iter().map(Arg::to_json).collect(),
            _ => Vec::new(),
        };
        if checked.truthy() && !chain.is_empty() {
            self.asker_of(&chain, &cwd.to_json())
        } else {
            Value::Null
        }
    }

    // ---------- tokens ----------

    fn revoke(&self, hash: &Arg, id: &str, reason: &str, t: f64) -> Result<()> {
        let row = Arg::Obj(vec![("token_hash".into(), hash.clone()), ("actor".into(), Arg::text(id)), ("reason".into(), Arg::text(reason)), ("at".into(), Arg::Num(t))]);
        Ok(self.store.with(|s| s.revoke_actor_token(&row))?)
    }

    // A registration that waited too long: its token is revoked.
    fn lapse(&self, r: &Row, t: f64) -> Result<()> {
        if self.store.with(|s| s.decide_actor_request(r.get("id").arg(), Arg::text("expired"), Arg::Num(t)))? {
            self.revoke(&r.get("token_hash").arg(), &id_of(r.text("slug").unwrap_or("")), "expired", t)?;
        }
        Ok(())
    }

    fn expire_lapsed(&mut self, t: f64) -> Result<()> {
        let before = self.gateway_asks.len();
        self.gateway_asks.retain(|g| g.expires_at > t);
        let gone = before - self.gateway_asks.len();
        let lapsed = self.store.with(|s| s.lapsed_actor_requests(Arg::Num(t)))?;
        if !lapsed.is_empty() {
            self.transaction(|me| {
                for r in &lapsed {
                    me.lapse(r, t)?;
                }
                Ok(())
            })?;
        }
        if !lapsed.is_empty() || gone > 0 {
            self.changed(t)?;
        }
        Ok(())
    }

    // The registrations waiting changed: the window hears how many.
    fn changed(&mut self, t: f64) -> Result<()> {
        let waiting = self.store.with(|s| s.waiting_actor_requests(Arg::Num(t)))?.len() + self.gateway_asks.len();
        self.emits.push(("requests", json!({ "at": num(t), "waiting": waiting })));
        Ok(())
    }

    // A registration, waiting from now: its credentials, and the request's
    // id.
    fn open(&mut self, c: &mut dyn Calls, ask: &Ask, asker: &Value, t: f64) -> Result<(String, Value)> {
        self.transaction(|me| {
            let waiting = me.store.with(|s| s.waiting_actor_requests(Arg::Num(t)))?;
            // The same program asking again with the same name takes its old
            // request's place, and the old token stops working.
            let program_of = |who: &Value| if truthy(Some(who)) { Some(xenonet_base::json::stringify(&json!([who.get("program"), who.get("parents")]), 0)) } else { None };
            let mine = program_of(asker);
            let same = waiting.iter().position(|r| r.text("slug") == Some(ask.slug.as_str()) && parse(r.text("ask")).get("name").and_then(Value::as_str) == Some(ask.name.as_str()) && program_of(&parse(r.text("asker"))) == mine);
            let product = ask.product_id();
            if me.taken_by(c, &ask.slug, product.as_deref(), same.and_then(|i| waiting[i].text("id")), Some(t))? {
                return me.taken(c, &ask.slug, product.as_deref());
            }
            if Self::full(c, &id_of(&ask.slug))? {
                return Self::full_error();
            }
            if waiting.len() - usize::from(same.is_some()) >= WAITING_MAX {
                return fail(409.0, "full", format!("{WAITING_MAX} registrations are waiting for an answer already; try again once they're answered or have lapsed"), json!({}));
            }
            // The hour before now, so a retry after Retry-After is taken.
            let hour = me.store.with(|s| s.actor_request_times(Arg::Num(t - HOUR_MS + 1.0)))?;
            if hour.len() >= REQUESTS_PER_HOUR {
                let first = hour[0].as_f64().unwrap_or(f64::NAN);
                return fail(429.0, "too-many", format!("XenoNet takes at most {REQUESTS_PER_HOUR} registrations an hour"), json!({ "retryAfterMs": num(first + HOUR_MS - t) }));
            }
            if let Some(i) = same {
                let r = &waiting[i];
                me.store.with(|s| s.decide_actor_request(r.get("id").arg(), Arg::text("replaced"), Arg::Num(t)))?;
                me.revoke(&r.get("token_hash").arg(), &id_of(r.text("slug").unwrap_or("")), "rotated", t)?;
            }
            let (token, hash) = token()?;
            let id = random_id(9)?;
            let row = Arg::Obj(vec![
                ("id".into(), Arg::text(&id)),
                ("slug".into(), Arg::text(&ask.slug)),
                ("ask".into(), Arg::text(xenonet_base::json::stringify(&ask.json(), 0))),
                ("token_hash".into(), Arg::text(hash)),
                ("asker".into(), if asker.is_null() { Arg::Null } else { Arg::text(xenonet_base::json::stringify(asker, 0)) }),
                ("created_at".into(), Arg::Num(t)),
                ("expires_at".into(), Arg::Num(t + REQUEST_MS)),
            ]);
            me.store.with(|s| s.add_actor_request(&row))?;
            Ok((id, json!({ "schema": "actor/credentials/v1", "id": id_of(&ask.slug), "token": token, "state": "waiting", "expiresAt": num(t + REQUEST_MS), "ignored": ask.ignored })))
        })
    }

    /// How many wait for you now, registrations and gateway token asks.
    pub fn waiting(&mut self, t: f64) -> Result<usize> {
        self.expire_lapsed(t)?;
        Ok(self.store.with(|s| s.waiting_actor_requests(Arg::Num(t)))?.len() + self.gateway_asks.len())
    }

    /// A program asks to be an actor: the credentials, waiting. `checked`,
    /// `chain` and `cwd`: what the process check saw.
    pub fn register(&mut self, c: &mut dyn Calls, body: &Arg, checked: &Arg, chain: &Arg, cwd: &Arg) -> Result<Value> {
        let t = Self::now(c)?;
        let ask = Self::read_ask(body, false)?;
        self.expire_lapsed(t)?;
        let asker = self.asker(checked, chain, cwd);
        let (id, credentials) = self.open(c, &ask, &asker, t)?;
        self.changed(t)?;
        self.emits.push(("asked", json!({ "id": id, "name": ask.name })));
        Ok(credentials)
    }

    /// The registrations waiting for you, oldest first, as an
    /// actor/requests/v1.
    pub fn requests(&mut self, c: &mut dyn Calls) -> Result<Value> {
        let t = Self::now(c)?;
        self.expire_lapsed(t)?;
        let mut requests = Vec::new();
        for r in self.store.with(|s| s.waiting_actor_requests(Arg::Num(t)))? {
            let ask = parse(r.text("ask"));
            let ask = if ask.is_object() { ask } else { json!({}) };
            let slug = r.text("slug").unwrap_or("").to_string();
            let there = Self::info(c, &id_of(&slug))?;
            let pid = ask.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| xenonet_engine::js::template(Some(v)));
            let mut o = Map::new();
            o.insert("id".into(), r.get("id").to_json());
            o.insert("actor".into(), json!(id_of(&slug)));
            if let Some(n) = ask.get("name") {
                o.insert("name".into(), n.clone());
            }
            o.insert("product".into(), ask.get("product").cloned().unwrap_or(Value::Null));
            o.insert("capabilities".into(), ask.get("capabilities").filter(|v| !v.is_null()).cloned().unwrap_or(json!([])));
            o.insert("heartbeatMs".into(), ask.get("heartbeatMs").cloned().unwrap_or(Value::Null));
            o.insert("pid".into(), ask.get("pid").cloned().unwrap_or(Value::Null));
            o.insert("asker".into(), parse(r.text("asker")));
            o.insert("joins".into(), json!(Self::joins(&there, pid.as_deref())));
            o.insert("at".into(), r.get("created_at").to_json());
            o.insert("expiresAt".into(), r.get("expires_at").to_json());
            requests.push(Value::Object(o));
        }
        let mut asks = Vec::new();
        for g in self.gateway_asks.clone() {
            let there = Self::info(c, &id_of(&g.slug))?;
            let name = there.get("name").filter(|n| !n.is_null()).cloned().unwrap_or(json!(g.slug));
            let replaces = self.store.with(|s| s.gateway_token(Arg::text(&g.slug)))?.is_some();
            asks.push(json!({ "id": g.id, "actor": id_of(&g.slug), "name": name, "replaces": replaces, "asker": g.asker, "at": num(g.at), "expiresAt": num(g.expires_at) }));
        }
        Ok(json!({ "schema": "actor/requests/v1", "at": num(t), "requests": requests, "gatewayAsks": asks }))
    }

    /// Your answer to a registration, from the window.
    pub fn decide(&mut self, c: &mut dyn Calls, request_id: &Arg, decision: &Arg) -> Result<Value> {
        let decision = match decision.as_str() {
            Some(d @ ("allow" | "deny")) => d.to_string(),
            _ => return bad("the decision is \"allow\" or \"deny\""),
        };
        let t = Self::now(c)?;
        self.expire_lapsed(t)?;
        let rid = text(request_id);
        // A session's ask for a gateway token: Allow makes it now, and the
        // answer, for the window alone, holds it once. Deny drops the ask.
        if let Some(gate) = self.gateway_asks.iter().find(|g| g.id == rid).cloned() {
            if decision == "deny" {
                self.gateway_asks.retain(|g| g.slug != gate.slug);
                self.changed(t)?;
                return Ok(json!({ "request": gate.id, "decision": decision, "actor": id_of(&gate.slug) }));
            }
            let credentials = self.gateway(c, &Arg::text(&gate.slug), &Arg::Obj(vec![]))?;
            return Ok(json!({ "request": gate.id, "decision": decision, "actor": id_of(&gate.slug), "credentials": credentials }));
        }
        let out = self.transaction(|me| {
            let Some(r) = me.store.with(|s| s.actor_request(Arg::text(&rid)))?.filter(|r| r.get("decision").is_null()) else {
                return fail(404.0, "not-found", "no registration with that id is waiting; it may have been answered or have lapsed", json!({}));
            };
            let slug = r.text("slug").unwrap_or("").to_string();
            let id = id_of(&slug);
            if decision == "deny" {
                me.store.with(|s| s.decide_actor_request(r.get("id").arg(), Arg::text("deny"), Arg::Num(t)))?;
                me.revoke(&r.get("token_hash").arg(), &id, "denied", t)?;
                return Ok(json!({ "request": r.get("id").to_json(), "decision": decision, "actor": id }));
            }
            let ask = parse(r.text("ask"));
            let ask = if ask.is_object() { ask } else { json!({}) };
            let product = ask.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| xenonet_engine::js::template(Some(v)));
            let there = Self::info(c, &id)?;
            if truthy(Some(&there)) && !Self::joins(&there, product.as_deref()) {
                return me.taken(c, &slug, product.as_deref());
            }
            if Self::full(c, &id)? {
                return Self::full_error();
            }
            if let Some(old) = me.store.with(|s| s.actor_token(Arg::text(&slug)))? {
                me.revoke(&old.get("token_hash").arg(), &id, "rotated", t)?;
            }
            let there_product = there.get("product").filter(|v| !v.is_null()).cloned();
            let def = Arg::Obj(vec![
                ("id".into(), Arg::text(&id)),
                ("name".into(), ask.get("name").map_or(Arg::Undef, value_arg)),
                ("product".into(), value_arg(&there_product.or_else(|| ask.get("product").filter(|v| !v.is_null()).cloned()).unwrap_or(Value::Null))),
                ("via".into(), value_arg(&json!(with_registered(&there)))),
                ("heartbeatMs".into(), ask.get("heartbeatMs").map_or(Arg::Undef, value_arg)),
                ("capabilities".into(), value_arg(ask.get("capabilities").filter(|v| !v.is_null()).unwrap_or(&json!([])))),
                ("identity".into(), value_arg(there.get("identity").unwrap_or(&Value::Null))),
                ("allowedAt".into(), Arg::Num(t)),
                ("at".into(), Arg::Num(t)),
            ]);
            c.call("actors", "upsert", vec![def])?;
            me.store.with(|s| s.set_actor_token(Arg::text(&slug), r.get("token_hash").arg(), Arg::Num(t)))?;
            me.store.with(|s| s.decide_actor_request(r.get("id").arg(), Arg::text("allow"), Arg::Num(t)))?;
            if let Some(pid) = ask.get("pid").filter(|p| truthy(Some(p))) {
                c.call("actors", "watchPid", vec![Arg::text(&id), value_arg(pid)])?;
            }
            let snapshot = c.call("actors", "snapshot", vec![Arg::text(&id)])?.to_json();
            Ok(json!({ "request": r.get("id").to_json(), "decision": decision, "actor": snapshot }))
        })?;
        self.changed(t)?;
        Ok(out)
    }

    /// An actor added from the window: allowed at once, with its token,
    /// once.
    pub fn add(&mut self, c: &mut dyn Calls, body: &Arg) -> Result<Value> {
        let t = Self::now(c)?;
        let ask = Self::read_ask(body, true)?;
        self.expire_lapsed(t)?;
        self.transaction(|me| {
            let id = id_of(&ask.slug);
            let product = ask.product_id();
            if me.taken_by(c, &ask.slug, product.as_deref(), None, Some(t))? {
                return me.taken(c, &ask.slug, product.as_deref());
            }
            if Self::full(c, &id)? {
                return Self::full_error();
            }
            let there = Self::info(c, &id)?;
            if let Some(old) = me.store.with(|s| s.actor_token(Arg::text(&ask.slug)))? {
                me.revoke(&old.get("token_hash").arg(), &id, "rotated", t)?;
            }
            let product = there.get("product").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| ask.product.clone());
            let def = Arg::Obj(vec![
                ("id".into(), Arg::text(&id)),
                ("name".into(), Arg::text(&ask.name)),
                ("product".into(), value_arg(&product)),
                ("via".into(), value_arg(&json!(with_registered(&there)))),
                ("heartbeatMs".into(), ask.heartbeat_ms.clone()),
                ("capabilities".into(), value_arg(&json!(ask.capabilities))),
                ("identity".into(), value_arg(there.get("identity").unwrap_or(&Value::Null))),
                ("allowedAt".into(), Arg::Num(t)),
                ("at".into(), Arg::Num(t)),
            ]);
            c.call("actors", "upsert", vec![def])?;
            let (token, hash) = token()?;
            me.store.with(|s| s.set_actor_token(Arg::text(&ask.slug), Arg::text(hash), Arg::Num(t)))?;
            Ok(json!({ "schema": "actor/credentials/v1", "id": id, "token": token, "state": "allowed", "ignored": ask.ignored }))
        })
    }

    /// A new token for a registered actor, from the window: the old one
    /// stops working at once.
    pub fn rotate(&mut self, c: &mut dyn Calls, slug: &Arg) -> Result<Value> {
        let slug = text(slug);
        let id = id_of(&slug);
        let there = Self::info(c, &id)?;
        if id == YOU || !via_has(&there, "registered") {
            return fail(404.0, "not-found", "XenoNet has no registered actor by that slug", json!({}));
        }
        let t = Self::now(c)?;
        self.transaction(|me| {
            if let Some(old) = me.store.with(|s| s.actor_token(Arg::text(&slug)))? {
                me.revoke(&old.get("token_hash").arg(), &id, "rotated", t)?;
            }
            let (token, hash) = token()?;
            me.store.with(|s| s.set_actor_token(Arg::text(&slug), Arg::text(hash), Arg::Num(t)))?;
            c.call("actors", "allow", vec![Arg::text(&id), Arg::Num(t)])?;
            Ok(json!({ "schema": "actor/credentials/v1", "id": id, "token": token, "state": "allowed", "ignored": [] }))
        })
    }

    /// An actor removed (yours) or gone of its own accord (`reason` left).
    pub fn remove(&mut self, c: &mut dyn Calls, slug: &Arg, reason: &str) -> Result<Value> {
        let slug = text(slug);
        let id = id_of(&slug);
        if id == YOU {
            return bad("you can't be removed; turn off Show me on the map in the Actors panel instead");
        }
        if !c.call("actors", "has", vec![Arg::text(&id)])?.truthy() {
            return fail(404.0, "not-found", "XenoNet has no such actor", json!({}));
        }
        if c.call("actors", "onlyObserved", vec![Arg::text(&id)])?.truthy() {
            let a = Self::info(c, &id)?;
            let product = product_id_of(&a).and_then(|p| product_of(&p));
            let how = match product {
                Some(p) => format!("the {} switch under observers in XenoNet's Actors panel, or xenonet actor observe {} off", p.name, p.id),
                None => "its switch under observers in XenoNet's Actors panel".into(),
            };
            let name = xenonet_engine::js::template(a.get("name"));
            return fail(409.0, "observed", format!("XenoNet only observes {name}, so it can't be removed; turn off its observer instead, with {how}"), product.map_or(json!({}), |p| json!({ "product": p.id })));
        }
        let t = Self::now(c)?;
        let registered = Self::is_registered(c, &id)?;
        let before = self.gateway_asks.len();
        self.gateway_asks.retain(|g| g.slug != slug);
        if before != self.gateway_asks.len() {
            self.changed(t)?;
        }
        self.transaction(|me| {
            if let Some(old) = me.store.with(|s| s.actor_token(Arg::text(&slug)))? {
                me.revoke(&old.get("token_hash").arg(), &id, reason, t)?;
            }
            me.store.with(|s| s.drop_actor_token(Arg::text(&slug)))?;
            if let Some(gate) = me.store.with(|s| s.gateway_token(Arg::text(&slug)))? {
                me.revoke(&gate.get("gateway_hash").arg(), &id, reason, t)?;
            }
            me.store.with(|s| s.drop_gateway_token(Arg::text(&slug)))?;
            me.store.with(|s| s.release(Arg::text(&id), &Arg::Null))?;
            me.store.with(|s| s.set_focus(Arg::text(&id), Arg::Null, Arg::Num(t)))?;
            // Removed by you, its remote connections end with it; leaving on
            // its own, it keeps them.
            if registered {
                c.call("actors", "unregister", vec![Arg::text(&id), Arg::Obj(vec![("keepRemote".into(), Arg::Bool(reason != "removed"))])])?;
            } else {
                c.call("actors", "remove", vec![Arg::text(&id)])?;
            }
            Ok(())
        })?;
        let observed = c.call("actors", "has", vec![Arg::text(&id)])?.truthy();
        Ok(if observed { json!({ "removed": id, "observed": true }) } else { json!({ "removed": id }) })
    }

    /// The actor a token belongs to, or the 401 it gets.
    pub fn authenticate(&mut self, c: &mut dyn Calls, token: &Arg) -> Result<Value> {
        let unknown = json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that token" });
        let Some(token) = token.as_str().filter(|t| is_token(t)) else { return Ok(unknown) };
        let hash = hash_token(token);
        let t = Self::now(c)?;
        if let Some(held) = self.store.with(|s| s.actor_by_token(Arg::text(&hash)))? {
            let slug = held.text("slug").unwrap_or("").to_string();
            if c.call("actors", "has", vec![Arg::text(id_of(&slug))])?.truthy() {
                let at = held.num("token_at");
                if token_expired(at, t) {
                    self.transaction(|me| {
                        me.revoke(&Arg::text(&hash), &id_of(&slug), "expired", t)?;
                        Ok(me.store.with(|s| s.drop_actor_token(Arg::text(&slug)))?)
                    })?;
                } else {
                    // Its first use is written exactly; after that, at most
                    // once an hour.
                    let made = Self::info(c, &id_of(&slug))?.get("allowedAt").and_then(Value::as_f64);
                    let at = at.unwrap_or(f64::NAN);
                    if made.is_some_and(|m| at <= m) || t - at >= TOUCH_MS {
                        self.store.with(|s| s.touch_actor_token(Arg::text(&slug), Arg::Num(t)))?;
                    }
                    return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "allowed" }));
                }
            }
        }
        if let Some(revoked) = self.store.with(|s| s.revoked_actor_token(Arg::text(&hash)))? {
            let reason = revoked.text("reason").unwrap_or("").to_string();
            return Ok(json!({ "ok": false, "status": 401, "code": "revoked", "reason": revoked.get("reason").to_json(), "error": revoked_words(&reason).unwrap_or("that token was revoked") }));
        }
        if let Some(r) = self.store.with(|s| s.actor_request_by_token(Arg::text(&hash)))?.filter(|r| !truthy(Some(&r.get("decision").to_json()))) {
            let slug = r.text("slug").unwrap_or("").to_string();
            if r.num("expires_at").is_some_and(|e| e > t) {
                return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "waiting", "request": r.to_json() }));
            }
            self.lapse(&r, t)?;
            self.changed(t)?;
            return Ok(json!({ "ok": false, "status": 401, "code": "revoked", "reason": "expired", "error": revoked_words("expired") }));
        }
        Ok(unknown)
    }

    // ---------- the actor gateway ----------

    /// A gateway token for an actor, from XenoNet's window only, shown once.
    pub fn gateway(&mut self, c: &mut dyn Calls, slug: &Arg, body: &Arg) -> Result<Value> {
        let slug = text(slug);
        let id = id_of(&slug);
        if id == YOU {
            return bad("you reach XenoNet through its window, not the gateway");
        }
        let t = Self::now(c)?;
        self.expire_lapsed(t)?;
        let out = self.transaction(|me| {
            let there = Self::info(c, &id)?;
            if !via_has(&there, "registered") {
                let product = product_id_of(&there).and_then(|p| product_of(&p));
                let Some(product) = product.filter(|p| truthy(Some(&there)) && p.registers) else {
                    return fail(404.0, "not-found", "XenoNet has no registered actor by that slug; add or allow it first", json!({}));
                };
                if Self::full(c, &id)? {
                    return Self::full_error();
                }
                let caps = match body.field("capabilities") {
                    Arg::List(l) => Arg::List(l),
                    _ => Arg::List(vec![Arg::text("message-agents")]),
                };
                let hb = def_null(body.field("heartbeatMs")).or(Arg::Num(HEARTBEAT_MAX));
                let ask_body = Arg::Obj(vec![("name".into(), there.get("name").map_or(Arg::Undef, value_arg)), ("product".into(), Arg::Obj(vec![("id".into(), Arg::text(product.id))])), ("capabilities".into(), caps), ("heartbeatMs".into(), hb)]);
                let ask = Self::read_ask(&ask_body, true)?;
                let def = Arg::Obj(vec![
                    ("id".into(), Arg::text(&id)),
                    ("name".into(), Arg::text(&ask.name)),
                    ("product".into(), value_arg(there.get("product").unwrap_or(&Value::Null))),
                    ("via".into(), value_arg(&json!(with_registered(&there)))),
                    ("heartbeatMs".into(), ask.heartbeat_ms.clone()),
                    ("capabilities".into(), value_arg(&json!(ask.capabilities))),
                    ("identity".into(), value_arg(there.get("identity").filter(|v| !v.is_null()).unwrap_or(&Value::Null))),
                    ("allowedAt".into(), Arg::Num(t)),
                    ("at".into(), Arg::Num(t)),
                ]);
                c.call("actors", "upsert", vec![def])?;
            }
            if let Some(old) = me.store.with(|s| s.gateway_token(Arg::text(&slug)))? {
                me.revoke(&old.get("gateway_hash").arg(), &id, "rotated", t)?;
            }
            let (token, hash) = new_gateway_token().map_err(|e| Fail::Thrown(Thrown::new(format!("no random bytes: {e}"))))?;
            me.store.with(|s| s.set_gateway_token(Arg::text(&slug), Arg::text(hash), Arg::Num(t)))?;
            // Any session's ask for this token is answered by it.
            me.gateway_asks.retain(|g| g.slug != slug);
            me.changed(t)?;
            Ok(json!({ "schema": "actor/gateway-credentials/v1", "id": id, "token": token }))
        })?;
        Ok(out)
    }

    /// A session's ask for an actor's gateway token: it makes no token and
    /// answers with none; it only leaves a card in XenoNet's window.
    pub fn ask_gateway(&mut self, c: &mut dyn Calls, slug: &Arg, checked: &Arg, chain: &Arg, cwd: &Arg) -> Result<Value> {
        let slug = text(slug);
        let id = id_of(&slug);
        if id == YOU {
            return bad("you reach XenoNet through its window, not the gateway");
        }
        let t = Self::now(c)?;
        self.expire_lapsed(t)?;
        let there = Self::info(c, &id)?;
        if !via_has(&there, "registered") {
            let product = product_id_of(&there).and_then(|p| product_of(&p));
            if !truthy(Some(&there)) || !product.is_some_and(|p| p.registers) {
                return fail(404.0, "not-found", "XenoNet has no registered actor by that slug; add or allow it first", json!({}));
            }
            if Self::full(c, &id)? {
                return Self::full_error();
            }
        }
        if self.gateway_asks.iter().filter(|g| g.slug != slug).count() >= WAITING_MAX {
            return fail(409.0, "full", format!("{WAITING_MAX} gateway tokens are asked for already; try again once they're answered or have lapsed"), json!({}));
        }
        self.ask_times.retain(|at| *at > t - HOUR_MS);
        if self.ask_times.len() >= REQUESTS_PER_HOUR {
            let first = self.ask_times[0];
            return fail(429.0, "too-many", format!("XenoNet takes at most {REQUESTS_PER_HOUR} asks for a gateway token an hour"), json!({ "retryAfterMs": num(first + HOUR_MS - t) }));
        }
        self.ask_times.push(t);
        let ask = GatewayAsk { id: format!("gw-{}", random_id(9)?), slug: slug.clone(), asker: self.asker(checked, chain, cwd), at: t, expires_at: t + REQUEST_MS };
        let (ask_id, expires_at) = (ask.id.clone(), ask.expires_at);
        match self.gateway_asks.iter_mut().find(|g| g.slug == slug) {
            Some(slot) => *slot = ask,
            None => self.gateway_asks.push(ask),
        }
        self.changed(t)?;
        let name = there.get("name").filter(|n| !n.is_null()).cloned().unwrap_or(json!(slug));
        self.emits.push(("asked-gateway", json!({ "id": ask_id, "actor": id, "name": name })));
        let replaces = self.store.with(|s| s.gateway_token(Arg::text(&slug)))?.is_some();
        Ok(json!({ "schema": "actor/gateway-ask/v1", "id": id, "state": "waiting", "request": ask_id, "expiresAt": num(expires_at), "replaces": replaces }))
    }

    /// An actor's gateway token taken away: its own token, if any, stands.
    pub fn drop_gateway(&mut self, c: &mut dyn Calls, slug: &Arg) -> Result<Value> {
        let slug = text(slug);
        let id = id_of(&slug);
        let t = Self::now(c)?;
        self.transaction(|me| {
            let Some(old) = me.store.with(|s| s.gateway_token(Arg::text(&slug)))? else {
                return fail(404.0, "not-found", "that actor has no gateway token", json!({}));
            };
            me.revoke(&old.get("gateway_hash").arg(), &id, "removed", t)?;
            me.store.with(|s| s.drop_gateway_token(Arg::text(&slug)))?;
            Ok(json!({ "actor": id, "gateway": false }))
        })
    }

    /// The actor a gateway token belongs to, or a 401. An actor's own
    /// token is refused here by name.
    pub fn authenticate_gateway(&mut self, c: &mut dyn Calls, token: &Arg) -> Result<Value> {
        let unknown = json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that gateway token" });
        let token_text = token.as_str();
        if token_text.is_some_and(is_token) {
            return Ok(json!({ "ok": false, "status": 401, "code": "wrong-token", "error": "that's an actor's own token, which works only on this machine; the gateway takes a gateway token, made in XenoNet's window" }));
        }
        let Some(token) = token_text.filter(|t| is_gateway_token(t)) else { return Ok(unknown) };
        let hash = hash_token(token);
        let t = Self::now(c)?;
        if let Some(held) = self.store.with(|s| s.actor_by_gateway_token(Arg::text(&hash)))? {
            let slug = held.text("slug").unwrap_or("").to_string();
            if Self::is_registered(c, &id_of(&slug))? {
                let at = held.num("gateway_at");
                if token_expired(at, t) {
                    self.transaction(|me| {
                        me.revoke(&Arg::text(&hash), &id_of(&slug), "expired", t)?;
                        Ok(me.store.with(|s| s.drop_gateway_token(Arg::text(&slug)))?)
                    })?;
                } else {
                    if t - at.unwrap_or(f64::NAN) >= TOUCH_MS {
                        self.store.with(|s| s.touch_gateway_token(Arg::text(&slug), Arg::Num(t)))?;
                    }
                    return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "allowed" }));
                }
            }
        }
        if let Some(revoked) = self.store.with(|s| s.revoked_actor_token(Arg::text(&hash)))? {
            let reason = revoked.text("reason").unwrap_or("").to_string();
            return Ok(json!({ "ok": false, "status": 401, "code": "revoked", "reason": revoked.get("reason").to_json(), "actor": revoked.get("actor").to_json(), "error": revoked_words(&reason).unwrap_or("that token was revoked") }));
        }
        Ok(unknown)
    }

    /// The product an authenticated actor is (its id), for the process
    /// check.
    pub fn product_of(c: &mut dyn Calls, auth: &Arg) -> Result<Value> {
        if auth.field("state").as_str() == Some("waiting") {
            let ask = parse(auth.field("request").field("ask").as_str());
            return Ok(ask.get("product").and_then(|p| p.get("id")).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null));
        }
        let info = Self::info(c, &text(&auth.field("id")))?;
        Ok(info.get("product").and_then(|p| p.get("id")).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null))
    }

    /// An actor as it sees itself (actor/me/v1).
    pub fn me(&self, c: &mut dyn Calls, auth: &Arg, extra: &Arg) -> Result<Value> {
        let id = auth.field("id").to_json();
        let mut o = if auth.field("state").as_str() == Some("waiting") {
            let ask = parse(auth.field("request").field("ask").as_str());
            json!({ "schema": "actor/me/v1", "id": id, "state": "waiting", "presence": "unknown", "heartbeatMs": ask.get("heartbeatMs").cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null), "unread": 0 })
        } else {
            let a = Self::info(c, &text(&auth.field("id")))?;
            let unread = self.store.with(|s| s.unseen(auth.field("id")))?.len();
            json!({ "schema": "actor/me/v1", "id": id, "state": "allowed", "presence": a.get("presence").filter(|v| !v.is_null()).cloned().unwrap_or(json!("unknown")), "heartbeatMs": a.get("heartbeatMs").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null), "unread": unread })
        };
        if let (Value::Object(m), Value::Object(e)) = (&mut o, extra.to_json()) {
            for (k, v) in e {
                m.insert(k, v);
            }
        }
        Ok(o)
    }

    /// On the clock: registrations that lapsed, and once a minute, tokens
    /// unused for 30 days.
    pub fn tick(&mut self, t: f64) -> Result<()> {
        self.expire_lapsed(t)?;
        if t - self.last_sweep < SWEEP_MS {
            return Ok(());
        }
        self.last_sweep = t;
        let stale = self.store.with(|s| s.stale_actor_tokens(Arg::Num(t - TOKEN_UNUSED_MS + 1.0)))?;
        if !stale.is_empty() {
            self.transaction(|me| {
                for row in &stale {
                    let slug = row.text("slug").unwrap_or("");
                    me.revoke(&row.get("token_hash").arg(), &id_of(slug), "expired", t)?;
                    me.store.with(|s| s.drop_actor_token(Arg::text(slug)))?;
                }
                Ok(())
            })?;
        }
        let gates = self.store.with(|s| s.stale_gateway_tokens(Arg::Num(t - TOKEN_UNUSED_MS + 1.0)))?;
        if !gates.is_empty() {
            self.transaction(|me| {
                for row in &gates {
                    let slug = row.text("slug").unwrap_or("");
                    me.revoke(&row.get("gateway_hash").arg(), &id_of(slug), "expired", t)?;
                    me.store.with(|s| s.drop_gateway_token(Arg::text(slug)))?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    // ---------- observers ----------

    /// Whether a product's observer is on: unless you turned it off.
    pub fn observing(c: &mut dyn Calls, product: &Arg) -> Result<bool> {
        let ids = xenonet_engine::peer::list(c.call("actors", "ofProduct", vec![product.clone()])?);
        for id in &ids {
            if !c.call("actors", "enabled", vec![id.clone()])?.truthy() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The products XenoNet observes, each with whether its observer is on
    /// and the actors it knows of it.
    pub fn observers(c: &mut dyn Calls) -> Result<Value> {
        let at = Self::now(c)?;
        let mut out = Vec::new();
        for p in PRODUCTS.iter().filter(|p| p.observed) {
            let on = c.call("", "observing", vec![Arg::text(p.id)])?.truthy();
            let actors = c.call("actors", "ofProduct", vec![Arg::text(p.id)])?.to_json();
            out.push(json!({ "product": p.id, "name": p.name, "on": on, "actors": actors }));
        }
        Ok(json!({ "at": num(at), "observers": out }))
    }

    /// Turns a product's observer on or off (yours).
    pub fn observe(c: &mut dyn Calls, product: &Arg, on: &Arg) -> Result<Value> {
        let Some(p) = product.as_str().and_then(product_of).filter(|p| p.observed) else {
            return fail(404.0, "not-found", "XenoNet has no observer for that product", json!({}));
        };
        let Arg::Bool(on) = on else { return bad("on is true or false") };
        let mut ids = c.call("actors", "ofProduct", vec![product.clone()])?;
        if xenonet_engine::peer::list(ids.clone()).is_empty() {
            let def = Arg::Obj(vec![
                ("id".into(), Arg::text(id_of(p.slug))),
                ("name".into(), Arg::text(p.short)),
                ("product".into(), value_arg(&json!({ "id": p.id, "vendor": p.vendor, "version": null }))),
                ("via".into(), Arg::List(vec![Arg::text("observed")])),
                ("enabled".into(), Arg::Bool(*on)),
            ]);
            c.call("actors", "upsert", vec![def])?;
            ids = Arg::List(vec![Arg::text(id_of(p.slug))]);
        } else {
            for id in xenonet_engine::peer::list(ids.clone()) {
                c.call("actors", "setEnabled", vec![id, Arg::Bool(*on)])?;
            }
        }
        Ok(json!({ "product": product.to_json(), "on": on, "actors": ids.to_json() }))
    }

    // ---------- XenoNet Dev ----------

    /// actor-request, in the demo and test instances only: a made-up
    /// program asks to register, so the request card can be built against
    /// it.
    pub fn simulate(&mut self, c: &mut dyn Calls, kind: &Arg, a: &Arg) -> Result<Value> {
        if kind.as_str() != Some("actor-request") || !c.get("actors.demo")?.truthy() {
            return Ok(Value::Null);
        }
        let t = Self::now(c)?;
        let made = (|| -> Result<Value> {
            let raw = a.as_str().filter(|s| !xenonet_base::text::js_trim(s).is_empty()).unwrap_or("Heron");
            let name = check_name(Some(raw), None).or_else(bad)?;
            let base = slug_from(&name);
            let slug = if slug_refusal(Some(&base), None).is_none() && !self.taken_by(c, &base, None, None, Some(t))? {
                Some(base.clone())
            } else {
                let mut failed = None;
                let s = suggest_slug(&base, None, &mut |s| match self.taken_by(c, s, None, None, Some(t)) {
                    Ok(b) => Ok(b),
                    Err(e) => {
                        failed = Some(e.clone());
                        Err(e)
                    }
                });
                if let Some(e) = failed {
                    return Err(e);
                }
                s?
            };
            let slug_text = slug.clone().unwrap_or_else(|| "null".into());
            let ask = Ask {
                name: name.clone(),
                slug: slug_text.clone(),
                product: Value::Null,
                capabilities: vec!["run-commands".into(), "edit-repos".into(), "message-agents".into()],
                ignored: vec![],
                heartbeat_ms: Arg::Num(60e3),
                pid: Arg::Null,
            };
            let asker = json!({ "program": format!("~/bin/{slug_text}"), "parents": ["/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], "folder": format!("~/Projects/{slug_text}") });
            self.expire_lapsed(t)?;
            let (id, credentials) = self.open(c, &ask, &asker, t)?;
            self.changed(t)?;
            Ok(json!({ "ok": true, "kind": kind.to_json(), "a": credentials["id"], "request": id, "token": credentials["token"], "expiresAt": credentials["expiresAt"] }))
        })();
        match made {
            Err(Fail::Actor(e)) => Ok(json!({ "ok": false, "kind": kind.to_json(), "code": e.code, "error": e.message })),
            other => other,
        }
    }
}

fn with_registered(there: &Value) -> Vec<Value> {
    let mut via: Vec<Value> = Vec::new();
    for v in there.get("via").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().chain(std::iter::once(json!("registered"))) {
        if !via.contains(&v) {
            via.push(v);
        }
    }
    via
}

fn def_null(a: Arg) -> Arg {
    if a == Arg::Undef {
        Arg::Null
    } else {
        a
    }
}

fn num(n: f64) -> Value {
    xenonet_base::json::number(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_read_as_the_table_says() {
        let body = Arg::Obj(vec![("name".into(), Arg::text("Wren")), ("capabilities".into(), Arg::List(vec![Arg::text("browse"), Arg::text("fly\u{1F600}")]))]);
        let ask = Registration::read_ask(&body, false).unwrap();
        assert_eq!((ask.slug.as_str(), ask.capabilities.clone(), ask.ignored.clone()), ("wren", vec!["browse".to_string()], vec!["fly??".to_string()]));
        let slow = Arg::Obj(vec![("name".into(), Arg::text("Wren")), ("heartbeatMs".into(), Arg::Num(10.0))]);
        assert!(matches!(Registration::read_ask(&slow, false), Err(Fail::Actor(ActorError { status, .. })) if status == 400.0));
        let dots = Arg::Obj(vec![("name".into(), Arg::text("Dot")), ("product".into(), Arg::Obj(vec![("id".into(), Arg::text("dots"))]))]);
        assert!(matches!(Registration::read_ask(&dots, false), Err(Fail::Actor(ActorError { code: "not-allowed", .. }))));
        assert_eq!(base64url(&[0, 0, 0, 0, 0, 0, 0, 0, 0]).len(), 12);
    }
}
