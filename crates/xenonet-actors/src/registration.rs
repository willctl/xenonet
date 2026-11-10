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
//! What it asks of the actors and the clock goes through `Host`; its rows
//! (requests, tokens, revoked tokens) through `Store`. A body or a row is a
//! JSON object: a key left out is one the caller didn't say, a null one it
//! did.

use crate::detail::{clip, status_line};
use crate::names::{check_name, product_word_refusal, slug_from, slug_refusal, suggest_slug};
use crate::store::{self, Tx};
use crate::tokens::{hash_token, is_gateway_token, is_token, new_gateway_token, new_token};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use xenonet_adapter::products::{product_of, PRODUCTS, WHY_NOT};
use xenonet_model::ids::YOU;
use xenonet_os::paths::tilde;
use xenonet_text::text::display;
use xenonet_text::text::truthy;

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
pub use xenonet_adapter::observe::CAPABILITIES;

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

/// How a call failed: a refusal a program can test, or what the store or
/// a collaborator failed with.
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Actor(ActorError),
    Error(store::Error),
}

impl From<store::Error> for Fail {
    fn from(e: store::Error) -> Self {
        Fail::Error(e)
    }
}

/// What registering asks of the actors' registry and the clock.
pub trait Host {
    fn now(&mut self) -> f64;
    /// Whether this is the demo (its made-up request).
    fn demo(&mut self) -> bool;
    /// Whether a product's observer is on (`Registration::observing`).
    fn observing(&mut self, product: &str) -> Result<bool>;
    /// An actor as registering sees it (the registry's info), or null.
    fn info(&mut self, id: &str) -> Value;
    fn is_registered(&mut self, id: &str) -> bool;
    fn registered_count(&mut self) -> usize;
    fn upsert(&mut self, def: &Value) -> Result<()>;
    fn watch_pid(&mut self, id: &str, pid: &Value) -> Result<()>;
    /// The actor's snapshot, as the public view has it.
    fn snapshot(&mut self, id: &str) -> Result<Value>;
    fn allow(&mut self, id: &str, at: f64) -> Result<()>;
    fn has(&mut self, id: &str) -> bool;
    fn only_observed(&mut self, id: &str) -> bool;
    /// A registered actor that no longer reports itself; `keep_remote`
    /// leaves its remote connections.
    fn unregister(&mut self, id: &str, keep_remote: bool) -> Result<()>;
    fn remove(&mut self, id: &str) -> Result<()>;
    fn of_product(&mut self, product: &str) -> Vec<String>;
    fn enabled(&mut self, id: &str) -> bool;
    fn set_enabled(&mut self, id: &str, on: bool) -> Result<()>;
}

/// What registering keeps in the engine's database: requests, tokens,
/// gateway tokens and revoked tokens, each row a JSON object of its
/// columns.
pub trait Store: Tx + Send + Sync {
    fn waiting_actor_requests(&self, now: f64) -> store::Result<Vec<Value>>;
    fn lapsed_actor_requests(&self, now: f64) -> store::Result<Vec<Value>>;
    fn actor_request(&self, id: &str) -> store::Result<Option<Value>>;
    fn actor_request_by_token(&self, hash: &str) -> store::Result<Option<Value>>;
    fn add_actor_request(&self, row: &Value) -> store::Result<()>;
    /// Whether the request was still undecided.
    fn decide_actor_request(&self, id: &Value, decision: &str, at: f64) -> store::Result<bool>;
    /// When each request since `since` was made, oldest first.
    fn actor_request_times(&self, since: f64) -> store::Result<Vec<Value>>;
    fn revoke_actor_token(&self, row: &Value) -> store::Result<()>;
    fn set_actor_token(&self, slug: &str, hash: &Value, at: f64) -> store::Result<()>;
    fn drop_actor_token(&self, slug: &str) -> store::Result<()>;
    fn touch_actor_token(&self, slug: &str, at: f64) -> store::Result<()>;
    fn actor_by_token(&self, hash: &str) -> store::Result<Option<Value>>;
    fn actor_token(&self, slug: &str) -> store::Result<Option<Value>>;
    fn stale_actor_tokens(&self, before: f64) -> store::Result<Vec<Value>>;
    fn revoked_actor_token(&self, hash: &str) -> store::Result<Option<Value>>;
    fn set_gateway_token(&self, slug: &str, hash: &str, at: f64) -> store::Result<()>;
    fn drop_gateway_token(&self, slug: &str) -> store::Result<()>;
    fn touch_gateway_token(&self, slug: &str, at: f64) -> store::Result<()>;
    fn actor_by_gateway_token(&self, hash: &str) -> store::Result<Option<Value>>;
    fn gateway_token(&self, slug: &str) -> store::Result<Option<Value>>;
    fn stale_gateway_tokens(&self, before: f64) -> store::Result<Vec<Value>>;
    /// An actor's claims released and its focus cleared, as it goes.
    fn release_claims(&self, agent: &str) -> store::Result<()>;
    fn clear_focus(&self, agent: &str, at: f64) -> store::Result<()>;
    /// How many messages wait unread for an actor.
    fn unseen_count(&self, agent: &str) -> store::Result<usize>;
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
    s.and_then(|s| xenonet_text::json::parse(s).ok()).unwrap_or(Value::Null)
}

/// A column of a row, as text.
fn text<'a>(r: &'a Value, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
}

/// A column of a row, as a number.
fn num_of(r: &Value, k: &str) -> Option<f64> {
    r.get(k).and_then(Value::as_f64)
}

/// A column of a row, or null.
fn col(r: &Value, k: &str) -> Value {
    r.get(k).cloned().unwrap_or(Value::Null)
}

/// `${v}` of a field, undefined when it isn't there.
fn template(v: Option<&Value>) -> String {
    xenonet_text::text::template(v)
}

// Number(v), as a value compares with a number.
pub fn to_number(v: &Value) -> f64 {
    xenonet_text::text::to_number(Some(v))
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
    getrandom::fill(&mut b).map_err(|e| Fail::Error(store::Error::new(format!("no random bytes: {e}"))))?;
    Ok(base64url(&b))
}

fn token() -> Result<(String, String)> {
    new_token().map_err(|e| Fail::Error(store::Error::new(format!("no random bytes: {e}"))))
}

/// A registration as asked, checked.
#[derive(Clone, Debug)]
pub struct Ask {
    pub name: String,
    pub slug: String,
    pub product: Value,
    pub capabilities: Vec<String>,
    pub ignored: Vec<String>,
    pub heartbeat_ms: Value,
    pub pid: Value,
}

impl Ask {
    fn json(&self) -> Value {
        json!({ "name": self.name, "slug": self.slug, "product": self.product, "capabilities": self.capabilities, "ignored": self.ignored, "heartbeatMs": self.heartbeat_ms, "pid": self.pid })
    }
    fn product_id(&self) -> Option<String> {
        self.product.get("id").filter(|v| !v.is_null()).map(|v| template(Some(v)))
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
    store: Arc<dyn Store>,
    home: String,
    gateway_asks: Vec<GatewayAsk>,
    ask_times: Vec<f64>,
    last_sweep: f64,
    emits: Vec<(&'static str, Value)>,
}

fn product_id_of(info: &Value) -> Option<String> {
    info.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| template(Some(v)))
}

fn via_has(info: &Value, v: &str) -> bool {
    info.get("via").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(v)))
}

impl Registration {
    pub fn new(store: Arc<dyn Store>, home: &str) -> Self {
        Registration { store, home: home.into(), gateway_asks: Vec::new(), ask_times: Vec::new(), last_sweep: f64::NEG_INFINITY, emits: Vec::new() }
    }

    /// The events told since this was last asked: ("requests" | "asked" |
    /// "asked-gateway", what it says).
    pub fn take_emits(&mut self) -> Vec<(&'static str, Value)> {
        std::mem::take(&mut self.emits)
    }

    fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let store = self.store.clone();
        store::transaction(store.as_ref(), || f(self))
    }

    // ---------- what's asked ----------

    // A product's version or vendor, as the actor's own word: one line, no
    // key, 64 characters at most, nothing that closes a quote.
    fn word_of(value: Option<&Value>, what: &str) -> Result<Value> {
        let Some(value) = value.filter(|v| !v.is_null()) else { return Ok(Value::Null) };
        let Ok(line) = status_line(Some(value)) else {
            return bad(format!("the product's {what} looks like it holds a key or a token"));
        };
        if line.as_deref().is_some_and(|l| l.chars().any(|c| matches!(c, '"' | '`' | '\\' | '\u{201c}' | '\u{201d}'))) {
            return bad(format!("the product's {what} can't hold quotation marks or backslashes, since it's shown quoted"));
        }
        Ok(line.map_or(Value::Null, |l| Value::String(clip(&l, 64))))
    }

    /// What a registration asks, checked as the interface's table has it.
    pub fn read_ask(body: &Value, window: bool) -> Result<Ask> {
        let product_given = body.get("product");
        let product_id = product_given.and_then(|p| p.get("id")).cloned().unwrap_or(Value::Null);
        let known = product_id.as_str().and_then(product_of);
        if let Some(k) = known {
            if !k.registers {
                let why = WHY_NOT.iter().find(|(p, _)| *p == k.id).map(|(_, w)| *w).unwrap_or_default();
                return fail(403.0, "not-allowed", why, json!({}));
            }
        }
        let field = |k: &str| product_given.and_then(|p| p.get(k));
        let product = if let Some(k) = known {
            json!({ "id": k.id, "vendor": k.vendor, "version": Self::word_of(field("version"), "version")? })
        } else if truthy(Some(&product_id)) {
            json!({ "id": product_id, "vendor": Self::word_of(field("vendor"), "vendor")?, "version": Self::word_of(field("version"), "version")? })
        } else {
            Value::Null
        };
        if !product.is_null() && known.is_none() {
            let id_text = template(product.get("id"));
            let vendor = product.get("vendor").and_then(Value::as_str).filter(|v| !v.is_empty());
            let why = product_word_refusal(&id_text, "id").or_else(|| vendor.and_then(|v| product_word_refusal(v, "vendor")));
            if let Some(why) = why {
                return bad(why);
            }
        }
        let name = check_name(body.get("name").and_then(Value::as_str), known.map(|k| k.id)).or_else(bad)?;
        let slug_given = body.get("slug").filter(|v| !v.is_null()).cloned();
        let slug = slug_given.unwrap_or_else(|| Value::String(known.map_or_else(|| slug_from(&name), |k| k.slug.to_string())));
        if let Some(why) = slug_refusal(slug.as_str(), known.map(|k| k.id)) {
            return bad(why);
        }
        let slug = slug.as_str().expect("slug_refusal refuses anything but a text").to_string();
        let mut asked: Vec<String> = Vec::new();
        for c in body.get("capabilities").and_then(Value::as_array).cloned().unwrap_or_default() {
            let c = display(&c);
            if !asked.contains(&c) {
                asked.push(c);
            }
        }
        let heartbeat = body.get("heartbeatMs").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| num(HEARTBEAT_FALLBACK));
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
            pid: if window { Value::Null } else { col(body, "pid") },
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
    fn taken_by(&self, c: &mut dyn Host, slug: &str, product: Option<&str>, except: Option<&str>, t: Option<f64>) -> Result<bool> {
        let there = c.info(&id_of(slug));
        if truthy(Some(&there)) && !Self::joins(&there, product) {
            return Ok(true);
        }
        let t = t.unwrap_or_else(|| c.now());
        Ok(self.store.waiting_actor_requests(t)?.iter().any(|r| text(r, "slug") == Some(slug) && text(r, "id") != except))
    }

    fn taken<T>(&self, c: &mut dyn Host, slug: &str, product: Option<&str>) -> Result<T> {
        let registered = c.is_registered(&id_of(slug));
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
        fail(
            409.0,
            "taken",
            format!("actor:{slug} is taken{}", if registered { "; to give it a new token, use New token in XenoNet's Actors panel" } else { "" }),
            json!({ "suggest": suggest }),
        )
    }

    fn full(c: &mut dyn Host, id: &str) -> bool {
        !c.is_registered(id) && c.registered_count() >= REGISTERED_MAX
    }

    fn full_error<T>() -> Result<T> {
        fail(409.0, "full", format!("{REGISTERED_MAX} actors are registered already; remove one in XenoNet's Actors panel first"), json!({}))
    }

    // What the card shows of who asked: the program, its parents up to the
    // first app or terminal, and its folder, with ~ for the home folder.
    fn asker_of(&self, chain: &[Value], cwd: &Value) -> Value {
        let file = |p: &Value| p.get("exe").filter(|e| !e.is_null()).or(p.get("program")).map_or_else(|| "undefined".into(), |v| template(Some(v)));
        let mut parents = Vec::new();
        for p in chain.iter().skip(1) {
            let f = file(p);
            parents.push(tilde(&f, &self.home));
            if f.contains(".app/") || parents.len() >= 8 {
                break;
            }
        }
        json!({ "program": tilde(&file(&chain[0]), &self.home), "parents": parents, "folder": if truthy(Some(cwd)) { Value::String(tilde(&template(Some(cwd)), &self.home)) } else { Value::Null } })
    }

    /// Who asked, from what the process check saw (`checked`, its `chain`
    /// of processes and the `cwd`), or null when it saw nothing.
    fn asker(&self, checked: bool, chain: &[Value], cwd: &Value) -> Value {
        if checked && !chain.is_empty() {
            self.asker_of(chain, cwd)
        } else {
            Value::Null
        }
    }

    // ---------- tokens ----------

    fn revoke(&self, hash: &Value, id: &str, reason: &str, t: f64) -> Result<()> {
        let row = json!({ "token_hash": hash, "actor": id, "reason": reason, "at": num(t) });
        Ok(self.store.revoke_actor_token(&row)?)
    }

    // A registration that waited too long: its token is revoked.
    fn lapse(&self, r: &Value, t: f64) -> Result<()> {
        if self.store.decide_actor_request(&col(r, "id"), "expired", t)? {
            self.revoke(&col(r, "token_hash"), &id_of(text(r, "slug").unwrap_or("")), "expired", t)?;
        }
        Ok(())
    }

    fn expire_lapsed(&mut self, t: f64) -> Result<()> {
        let before = self.gateway_asks.len();
        self.gateway_asks.retain(|g| g.expires_at > t);
        let gone = before - self.gateway_asks.len();
        let lapsed = self.store.lapsed_actor_requests(t)?;
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
        let waiting = self.store.waiting_actor_requests(t)?.len() + self.gateway_asks.len();
        self.emits.push(("requests", json!({ "at": num(t), "waiting": waiting })));
        Ok(())
    }

    // A registration, waiting from now: its credentials, and the request's
    // id.
    fn open(&mut self, c: &mut dyn Host, ask: &Ask, asker: &Value, t: f64) -> Result<(String, Value)> {
        self.transaction(|me| {
            let waiting = me.store.waiting_actor_requests(t)?;
            // The same program asking again with the same name takes its old
            // request's place, and the old token stops working.
            let program_of = |who: &Value| if truthy(Some(who)) { Some(xenonet_text::json::stringify(&json!([who.get("program"), who.get("parents")]), 0)) } else { None };
            let mine = program_of(asker);
            let same = waiting.iter().position(|r| {
                text(r, "slug") == Some(ask.slug.as_str())
                    && parse(text(r, "ask")).get("name").and_then(Value::as_str) == Some(ask.name.as_str())
                    && program_of(&parse(text(r, "asker"))) == mine
            });
            let product = ask.product_id();
            if me.taken_by(c, &ask.slug, product.as_deref(), same.and_then(|i| text(&waiting[i], "id")), Some(t))? {
                return me.taken(c, &ask.slug, product.as_deref());
            }
            if Self::full(c, &id_of(&ask.slug)) {
                return Self::full_error();
            }
            if waiting.len() - usize::from(same.is_some()) >= WAITING_MAX {
                return fail(409.0, "full", format!("{WAITING_MAX} registrations are waiting for an answer already; try again once they're answered or have lapsed"), json!({}));
            }
            // The hour before now, so a retry after Retry-After is taken.
            let hour = me.store.actor_request_times(t - HOUR_MS + 1.0)?;
            if hour.len() >= REQUESTS_PER_HOUR {
                let first = hour[0].as_f64().unwrap_or(f64::NAN);
                return fail(429.0, "too-many", format!("XenoNet takes at most {REQUESTS_PER_HOUR} registrations an hour"), json!({ "retryAfterMs": num(first + HOUR_MS - t) }));
            }
            if let Some(i) = same {
                let r = &waiting[i];
                me.store.decide_actor_request(&col(r, "id"), "replaced", t)?;
                me.revoke(&col(r, "token_hash"), &id_of(text(r, "slug").unwrap_or("")), "rotated", t)?;
            }
            let (token, hash) = token()?;
            let id = random_id(9)?;
            let row = json!({
                "id": id,
                "slug": ask.slug,
                "ask": xenonet_text::json::stringify(&ask.json(), 0),
                "token_hash": hash,
                "asker": if asker.is_null() { Value::Null } else { Value::String(xenonet_text::json::stringify(asker, 0)) },
                "created_at": num(t),
                "expires_at": num(t + REQUEST_MS),
            });
            me.store.add_actor_request(&row)?;
            Ok((
                id,
                json!({ "schema": "actor/credentials/v1", "id": id_of(&ask.slug), "token": token, "state": "waiting", "expiresAt": num(t + REQUEST_MS), "ignored": ask.ignored }),
            ))
        })
    }

    /// How many wait for you now, registrations and gateway token asks.
    pub fn waiting(&mut self, t: f64) -> Result<usize> {
        self.expire_lapsed(t)?;
        Ok(self.store.waiting_actor_requests(t)?.len() + self.gateway_asks.len())
    }

    /// A program asks to be an actor: the credentials, waiting. `checked`,
    /// `chain` and `cwd`: what the process check saw.
    pub fn register(&mut self, c: &mut dyn Host, body: &Value, checked: bool, chain: &[Value], cwd: &Value) -> Result<Value> {
        let t = c.now();
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
    pub fn requests(&mut self, c: &mut dyn Host) -> Result<Value> {
        let t = c.now();
        self.expire_lapsed(t)?;
        let mut requests = Vec::new();
        for r in self.store.waiting_actor_requests(t)? {
            let ask = parse(text(&r, "ask"));
            let ask = if ask.is_object() { ask } else { json!({}) };
            let slug = text(&r, "slug").unwrap_or("").to_string();
            let there = c.info(&id_of(&slug));
            let pid = ask.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| template(Some(v)));
            let mut o = Map::new();
            o.insert("id".into(), col(&r, "id"));
            o.insert("actor".into(), json!(id_of(&slug)));
            if let Some(n) = ask.get("name") {
                o.insert("name".into(), n.clone());
            }
            o.insert("product".into(), ask.get("product").cloned().unwrap_or(Value::Null));
            o.insert("capabilities".into(), ask.get("capabilities").filter(|v| !v.is_null()).cloned().unwrap_or(json!([])));
            o.insert("heartbeatMs".into(), ask.get("heartbeatMs").cloned().unwrap_or(Value::Null));
            o.insert("pid".into(), ask.get("pid").cloned().unwrap_or(Value::Null));
            o.insert("asker".into(), parse(text(&r, "asker")));
            o.insert("joins".into(), json!(Self::joins(&there, pid.as_deref())));
            o.insert("at".into(), col(&r, "created_at"));
            o.insert("expiresAt".into(), col(&r, "expires_at"));
            requests.push(Value::Object(o));
        }
        let mut asks = Vec::new();
        for g in self.gateway_asks.clone() {
            let there = c.info(&id_of(&g.slug));
            let name = there.get("name").filter(|n| !n.is_null()).cloned().unwrap_or(json!(g.slug));
            let replaces = self.store.gateway_token(&g.slug)?.is_some();
            asks.push(json!({ "id": g.id, "actor": id_of(&g.slug), "name": name, "replaces": replaces, "asker": g.asker, "at": num(g.at), "expiresAt": num(g.expires_at) }));
        }
        Ok(json!({ "schema": "actor/requests/v1", "at": num(t), "requests": requests, "gatewayAsks": asks }))
    }

    /// Your answer to a registration, from the window.
    pub fn decide(&mut self, c: &mut dyn Host, request_id: &str, decision: &Value) -> Result<Value> {
        let decision = match decision.as_str() {
            Some(d @ ("allow" | "deny")) => d.to_string(),
            _ => return bad("the decision is \"allow\" or \"deny\""),
        };
        let t = c.now();
        self.expire_lapsed(t)?;
        let rid = request_id.to_string();
        // A session's ask for a gateway token: Allow makes it now, and the
        // answer, for the window alone, holds it once. Deny drops the ask.
        if let Some(gate) = self.gateway_asks.iter().find(|g| g.id == rid).cloned() {
            if decision == "deny" {
                self.gateway_asks.retain(|g| g.slug != gate.slug);
                self.changed(t)?;
                return Ok(json!({ "request": gate.id, "decision": decision, "actor": id_of(&gate.slug) }));
            }
            let credentials = self.gateway(c, &gate.slug, &json!({}))?;
            return Ok(json!({ "request": gate.id, "decision": decision, "actor": id_of(&gate.slug), "credentials": credentials }));
        }
        let out = self.transaction(|me| {
            let Some(r) = me.store.actor_request(&rid)?.filter(|r| col(r, "decision").is_null()) else {
                return fail(404.0, "not-found", "no registration with that id is waiting; it may have been answered or have lapsed", json!({}));
            };
            let slug = text(&r, "slug").unwrap_or("").to_string();
            let id = id_of(&slug);
            if decision == "deny" {
                me.store.decide_actor_request(&col(&r, "id"), "deny", t)?;
                me.revoke(&col(&r, "token_hash"), &id, "denied", t)?;
                return Ok(json!({ "request": col(&r, "id"), "decision": decision, "actor": id }));
            }
            let ask = parse(text(&r, "ask"));
            let ask = if ask.is_object() { ask } else { json!({}) };
            let product = ask.get("product").and_then(|p| p.get("id")).filter(|v| !v.is_null()).map(|v| template(Some(v)));
            let there = c.info(&id);
            if truthy(Some(&there)) && !Self::joins(&there, product.as_deref()) {
                return me.taken(c, &slug, product.as_deref());
            }
            if Self::full(c, &id) {
                return Self::full_error();
            }
            if let Some(old) = me.store.actor_token(&slug)? {
                me.revoke(&col(&old, "token_hash"), &id, "rotated", t)?;
            }
            let there_product = there.get("product").filter(|v| !v.is_null()).cloned();
            let mut def = Map::new();
            def.insert("id".into(), json!(id));
            if let Some(name) = ask.get("name") {
                def.insert("name".into(), name.clone());
            }
            def.insert("product".into(), there_product.or_else(|| ask.get("product").filter(|v| !v.is_null()).cloned()).unwrap_or(Value::Null));
            def.insert("via".into(), json!(with_registered(&there)));
            if let Some(h) = ask.get("heartbeatMs") {
                def.insert("heartbeatMs".into(), h.clone());
            }
            def.insert("capabilities".into(), ask.get("capabilities").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!([])));
            def.insert("identity".into(), col(&there, "identity"));
            def.insert("allowedAt".into(), num(t));
            def.insert("at".into(), num(t));
            c.upsert(&Value::Object(def))?;
            me.store.set_actor_token(&slug, &col(&r, "token_hash"), t)?;
            me.store.decide_actor_request(&col(&r, "id"), "allow", t)?;
            if let Some(pid) = ask.get("pid").filter(|p| truthy(Some(p))) {
                c.watch_pid(&id, pid)?;
            }
            let snapshot = c.snapshot(&id)?;
            Ok(json!({ "request": col(&r, "id"), "decision": decision, "actor": snapshot }))
        })?;
        self.changed(t)?;
        Ok(out)
    }

    /// An actor added from the window: allowed at once, with its token,
    /// once.
    pub fn add(&mut self, c: &mut dyn Host, body: &Value) -> Result<Value> {
        let t = c.now();
        let ask = Self::read_ask(body, true)?;
        self.expire_lapsed(t)?;
        self.transaction(|me| {
            let id = id_of(&ask.slug);
            let product = ask.product_id();
            if me.taken_by(c, &ask.slug, product.as_deref(), None, Some(t))? {
                return me.taken(c, &ask.slug, product.as_deref());
            }
            if Self::full(c, &id) {
                return Self::full_error();
            }
            let there = c.info(&id);
            if let Some(old) = me.store.actor_token(&ask.slug)? {
                me.revoke(&col(&old, "token_hash"), &id, "rotated", t)?;
            }
            let product = there.get("product").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| ask.product.clone());
            let def = json!({
                "id": id,
                "name": ask.name,
                "product": product,
                "via": with_registered(&there),
                "heartbeatMs": ask.heartbeat_ms,
                "capabilities": ask.capabilities,
                "identity": col(&there, "identity"),
                "allowedAt": num(t),
                "at": num(t),
            });
            c.upsert(&def)?;
            let (token, hash) = token()?;
            me.store.set_actor_token(&ask.slug, &Value::String(hash), t)?;
            Ok(json!({ "schema": "actor/credentials/v1", "id": id, "token": token, "state": "allowed", "ignored": ask.ignored }))
        })
    }

    /// A new token for a registered actor, from the window: the old one
    /// stops working at once.
    pub fn rotate(&mut self, c: &mut dyn Host, slug: &str) -> Result<Value> {
        let slug = slug.to_string();
        let id = id_of(&slug);
        let there = c.info(&id);
        if id == YOU || !via_has(&there, "registered") {
            return fail(404.0, "not-found", "XenoNet has no registered actor by that slug", json!({}));
        }
        let t = c.now();
        self.transaction(|me| {
            if let Some(old) = me.store.actor_token(&slug)? {
                me.revoke(&col(&old, "token_hash"), &id, "rotated", t)?;
            }
            let (token, hash) = token()?;
            me.store.set_actor_token(&slug, &Value::String(hash), t)?;
            c.allow(&id, t)?;
            Ok(json!({ "schema": "actor/credentials/v1", "id": id, "token": token, "state": "allowed", "ignored": [] }))
        })
    }

    /// An actor removed (yours) or gone of its own accord (`reason` left).
    pub fn remove(&mut self, c: &mut dyn Host, slug: &str, reason: &str) -> Result<Value> {
        let slug = slug.to_string();
        let id = id_of(&slug);
        if id == YOU {
            return bad("you can't be removed; turn off Show me on the map in the Actors panel instead");
        }
        if !c.has(&id) {
            return fail(404.0, "not-found", "XenoNet has no such actor", json!({}));
        }
        if c.only_observed(&id) {
            let a = c.info(&id);
            let product = product_id_of(&a).and_then(|p| product_of(&p));
            let how = match product {
                Some(p) => format!("the {} switch under observers in XenoNet's Actors panel, or xenonet actor observe {} off", p.name, p.id),
                None => "its switch under observers in XenoNet's Actors panel".into(),
            };
            let name = template(a.get("name"));
            return fail(
                409.0,
                "observed",
                format!("XenoNet only observes {name}, so it can't be removed; turn off its observer instead, with {how}"),
                product.map_or(json!({}), |p| json!({ "product": p.id })),
            );
        }
        let t = c.now();
        let registered = c.is_registered(&id);
        let before = self.gateway_asks.len();
        self.gateway_asks.retain(|g| g.slug != slug);
        if before != self.gateway_asks.len() {
            self.changed(t)?;
        }
        self.transaction(|me| {
            if let Some(old) = me.store.actor_token(&slug)? {
                me.revoke(&col(&old, "token_hash"), &id, reason, t)?;
            }
            me.store.drop_actor_token(&slug)?;
            if let Some(gate) = me.store.gateway_token(&slug)? {
                me.revoke(&col(&gate, "gateway_hash"), &id, reason, t)?;
            }
            me.store.drop_gateway_token(&slug)?;
            me.store.release_claims(&id)?;
            me.store.clear_focus(&id, t)?;
            // Removed by you, its remote connections end with it; leaving on
            // its own, it keeps them.
            if registered {
                c.unregister(&id, reason != "removed")?;
            } else {
                c.remove(&id)?;
            }
            Ok(())
        })?;
        let observed = c.has(&id);
        Ok(if observed { json!({ "removed": id, "observed": true }) } else { json!({ "removed": id }) })
    }

    /// The actor a token belongs to, or the 401 it gets.
    pub fn authenticate(&mut self, c: &mut dyn Host, token: &str) -> Result<Value> {
        let unknown = json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that token" });
        if !is_token(token) {
            return Ok(unknown);
        }
        let hash = hash_token(token);
        let t = c.now();
        if let Some(held) = self.store.actor_by_token(&hash)? {
            let slug = text(&held, "slug").unwrap_or("").to_string();
            if c.has(&id_of(&slug)) {
                let at = num_of(&held, "token_at");
                if token_expired(at, t) {
                    self.transaction(|me| {
                        me.revoke(&Value::String(hash.clone()), &id_of(&slug), "expired", t)?;
                        Ok(me.store.drop_actor_token(&slug)?)
                    })?;
                } else {
                    // Its first use is written exactly; after that, at most
                    // once an hour.
                    let made = c.info(&id_of(&slug)).get("allowedAt").and_then(Value::as_f64);
                    let at = at.unwrap_or(f64::NAN);
                    if made.is_some_and(|m| at <= m) || t - at >= TOUCH_MS {
                        self.store.touch_actor_token(&slug, t)?;
                    }
                    return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "allowed" }));
                }
            }
        }
        if let Some(revoked) = self.store.revoked_actor_token(&hash)? {
            let reason = text(&revoked, "reason").unwrap_or("").to_string();
            return Ok(
                json!({ "ok": false, "status": 401, "code": "revoked", "reason": col(&revoked, "reason"), "error": revoked_words(&reason).unwrap_or("that token was revoked") }),
            );
        }
        if let Some(r) = self.store.actor_request_by_token(&hash)?.filter(|r| !truthy(r.get("decision"))) {
            let slug = text(&r, "slug").unwrap_or("").to_string();
            if num_of(&r, "expires_at").is_some_and(|e| e > t) {
                return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "waiting", "request": r }));
            }
            self.lapse(&r, t)?;
            self.changed(t)?;
            return Ok(json!({ "ok": false, "status": 401, "code": "revoked", "reason": "expired", "error": revoked_words("expired") }));
        }
        Ok(unknown)
    }

    // ---------- the actor gateway ----------

    /// A gateway token for an actor, from XenoNet's window only, shown once.
    pub fn gateway(&mut self, c: &mut dyn Host, slug: &str, body: &Value) -> Result<Value> {
        let slug = slug.to_string();
        let id = id_of(&slug);
        if id == YOU {
            return bad("you reach XenoNet through its window, not the gateway");
        }
        let t = c.now();
        self.expire_lapsed(t)?;
        let out = self.transaction(|me| {
            let there = c.info(&id);
            if !via_has(&there, "registered") {
                let product = product_id_of(&there).and_then(|p| product_of(&p));
                let Some(product) = product.filter(|p| truthy(Some(&there)) && p.registers) else {
                    return fail(404.0, "not-found", "XenoNet has no registered actor by that slug; add or allow it first", json!({}));
                };
                if Self::full(c, &id) {
                    return Self::full_error();
                }
                let caps = body.get("capabilities").filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!(["message-agents"]));
                let hb = body.get("heartbeatMs").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| num(HEARTBEAT_MAX));
                let mut ask_body = Map::new();
                if let Some(name) = there.get("name") {
                    ask_body.insert("name".into(), name.clone());
                }
                ask_body.insert("product".into(), json!({ "id": product.id }));
                ask_body.insert("capabilities".into(), caps);
                ask_body.insert("heartbeatMs".into(), hb);
                let ask = Self::read_ask(&Value::Object(ask_body), true)?;
                let def = json!({
                    "id": id,
                    "name": ask.name,
                    "product": col(&there, "product"),
                    "via": with_registered(&there),
                    "heartbeatMs": ask.heartbeat_ms,
                    "capabilities": ask.capabilities,
                    "identity": col(&there, "identity"),
                    "allowedAt": num(t),
                    "at": num(t),
                });
                c.upsert(&def)?;
            }
            if let Some(old) = me.store.gateway_token(&slug)? {
                me.revoke(&col(&old, "gateway_hash"), &id, "rotated", t)?;
            }
            let (token, hash) = new_gateway_token().map_err(|e| Fail::Error(store::Error::new(format!("no random bytes: {e}"))))?;
            me.store.set_gateway_token(&slug, &hash, t)?;
            // Any session's ask for this token is answered by it.
            me.gateway_asks.retain(|g| g.slug != slug);
            me.changed(t)?;
            Ok(json!({ "schema": "actor/gateway-credentials/v1", "id": id, "token": token }))
        })?;
        Ok(out)
    }

    /// A session's ask for an actor's gateway token: it makes no token and
    /// answers with none; it only leaves a card in XenoNet's window.
    pub fn ask_gateway(&mut self, c: &mut dyn Host, slug: &str, checked: bool, chain: &[Value], cwd: &Value) -> Result<Value> {
        let slug = slug.to_string();
        let id = id_of(&slug);
        if id == YOU {
            return bad("you reach XenoNet through its window, not the gateway");
        }
        let t = c.now();
        self.expire_lapsed(t)?;
        let there = c.info(&id);
        if !via_has(&there, "registered") {
            let product = product_id_of(&there).and_then(|p| product_of(&p));
            if !truthy(Some(&there)) || !product.is_some_and(|p| p.registers) {
                return fail(404.0, "not-found", "XenoNet has no registered actor by that slug; add or allow it first", json!({}));
            }
            if Self::full(c, &id) {
                return Self::full_error();
            }
        }
        if self.gateway_asks.iter().filter(|g| g.slug != slug).count() >= WAITING_MAX {
            return fail(409.0, "full", format!("{WAITING_MAX} gateway tokens are asked for already; try again once they're answered or have lapsed"), json!({}));
        }
        self.ask_times.retain(|at| *at > t - HOUR_MS);
        if self.ask_times.len() >= REQUESTS_PER_HOUR {
            let first = self.ask_times[0];
            return fail(
                429.0,
                "too-many",
                format!("XenoNet takes at most {REQUESTS_PER_HOUR} asks for a gateway token an hour"),
                json!({ "retryAfterMs": num(first + HOUR_MS - t) }),
            );
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
        let replaces = self.store.gateway_token(&slug)?.is_some();
        Ok(json!({ "schema": "actor/gateway-ask/v1", "id": id, "state": "waiting", "request": ask_id, "expiresAt": num(expires_at), "replaces": replaces }))
    }

    /// An actor's gateway token taken away: its own token, if any, stands.
    pub fn drop_gateway(&mut self, c: &mut dyn Host, slug: &str) -> Result<Value> {
        let slug = slug.to_string();
        let id = id_of(&slug);
        let t = c.now();
        self.transaction(|me| {
            let Some(old) = me.store.gateway_token(&slug)? else {
                return fail(404.0, "not-found", "that actor has no gateway token", json!({}));
            };
            me.revoke(&col(&old, "gateway_hash"), &id, "removed", t)?;
            me.store.drop_gateway_token(&slug)?;
            Ok(json!({ "actor": id, "gateway": false }))
        })
    }

    /// The actor a gateway token belongs to, or a 401. An actor's own
    /// token is refused here by name.
    pub fn authenticate_gateway(&mut self, c: &mut dyn Host, token: &str) -> Result<Value> {
        let unknown = json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that gateway token" });
        if is_token(token) {
            return Ok(
                json!({ "ok": false, "status": 401, "code": "wrong-token", "error": "that's an actor's own token, which works only on this machine; the gateway takes a gateway token, made in XenoNet's window" }),
            );
        }
        if !is_gateway_token(token) {
            return Ok(unknown);
        }
        let hash = hash_token(token);
        let t = c.now();
        if let Some(held) = self.store.actor_by_gateway_token(&hash)? {
            let slug = text(&held, "slug").unwrap_or("").to_string();
            if c.is_registered(&id_of(&slug)) {
                let at = num_of(&held, "gateway_at");
                if token_expired(at, t) {
                    self.transaction(|me| {
                        me.revoke(&Value::String(hash.clone()), &id_of(&slug), "expired", t)?;
                        Ok(me.store.drop_gateway_token(&slug)?)
                    })?;
                } else {
                    if t - at.unwrap_or(f64::NAN) >= TOUCH_MS {
                        self.store.touch_gateway_token(&slug, t)?;
                    }
                    return Ok(json!({ "ok": true, "id": id_of(&slug), "slug": slug, "state": "allowed" }));
                }
            }
        }
        if let Some(revoked) = self.store.revoked_actor_token(&hash)? {
            let reason = text(&revoked, "reason").unwrap_or("").to_string();
            return Ok(
                json!({ "ok": false, "status": 401, "code": "revoked", "reason": col(&revoked, "reason"), "actor": col(&revoked, "actor"), "error": revoked_words(&reason).unwrap_or("that token was revoked") }),
            );
        }
        Ok(unknown)
    }

    /// The product an authenticated actor is (its id), for the process
    /// check.
    pub fn product_of(c: &mut dyn Host, auth: &Value) -> Result<Value> {
        if auth.get("state").and_then(Value::as_str) == Some("waiting") {
            let ask = parse(auth.get("request").and_then(|r| r.get("ask")).and_then(Value::as_str));
            return Ok(ask.get("product").and_then(|p| p.get("id")).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null));
        }
        let info = c.info(&template(auth.get("id")));
        Ok(info.get("product").and_then(|p| p.get("id")).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null))
    }

    /// An actor as it sees itself (actor/me/v1), with `extra`'s fields.
    pub fn me(&self, c: &mut dyn Host, auth: &Value, extra: &Value) -> Result<Value> {
        let id = col(auth, "id");
        let mut o = if auth.get("state").and_then(Value::as_str) == Some("waiting") {
            let ask = parse(auth.get("request").and_then(|r| r.get("ask")).and_then(Value::as_str));
            json!({ "schema": "actor/me/v1", "id": id, "state": "waiting", "presence": "unknown", "heartbeatMs": ask.get("heartbeatMs").cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null), "unread": 0 })
        } else {
            let a = c.info(&template(auth.get("id")));
            let unread = self.store.unseen_count(&template(auth.get("id")))?;
            json!({ "schema": "actor/me/v1", "id": id, "state": "allowed", "presence": a.get("presence").filter(|v| !v.is_null()).cloned().unwrap_or(json!("unknown")), "heartbeatMs": a.get("heartbeatMs").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null), "unread": unread })
        };
        if let (Value::Object(m), Value::Object(e)) = (&mut o, extra) {
            for (k, v) in e {
                m.insert(k.clone(), v.clone());
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
        let stale = self.store.stale_actor_tokens(t - TOKEN_UNUSED_MS + 1.0)?;
        if !stale.is_empty() {
            self.transaction(|me| {
                for row in &stale {
                    let slug = text(row, "slug").unwrap_or("");
                    me.revoke(&col(row, "token_hash"), &id_of(slug), "expired", t)?;
                    me.store.drop_actor_token(slug)?;
                }
                Ok(())
            })?;
        }
        let gates = self.store.stale_gateway_tokens(t - TOKEN_UNUSED_MS + 1.0)?;
        if !gates.is_empty() {
            self.transaction(|me| {
                for row in &gates {
                    let slug = text(row, "slug").unwrap_or("");
                    me.revoke(&col(row, "gateway_hash"), &id_of(slug), "expired", t)?;
                    me.store.drop_gateway_token(slug)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    // ---------- observers ----------

    /// Whether a product's observer is on: unless you turned it off.
    pub fn observing(c: &mut dyn Host, product: &str) -> Result<bool> {
        for id in c.of_product(product) {
            if !c.enabled(&id) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The products XenoNet observes, each with whether its observer is on
    /// and the actors it knows of it.
    pub fn observers(c: &mut dyn Host) -> Result<Value> {
        let at = c.now();
        let mut out = Vec::new();
        for p in PRODUCTS.iter().filter(|p| p.observed) {
            let on = c.observing(p.id)?;
            let actors = c.of_product(p.id);
            out.push(json!({ "product": p.id, "name": p.name, "on": on, "actors": actors }));
        }
        Ok(json!({ "at": num(at), "observers": out }))
    }

    /// Turns a product's observer on or off (yours).
    pub fn observe(c: &mut dyn Host, product: &Value, on: &Value) -> Result<Value> {
        let Some(p) = product.as_str().and_then(product_of).filter(|p| p.observed) else {
            return fail(404.0, "not-found", "XenoNet has no observer for that product", json!({}));
        };
        let Value::Bool(on) = on else { return bad("on is true or false") };
        let mut ids = c.of_product(p.id);
        if ids.is_empty() {
            let def = json!({ "id": id_of(p.slug), "name": p.short, "product": { "id": p.id, "vendor": p.vendor, "version": null }, "via": ["observed"], "enabled": on });
            c.upsert(&def)?;
            ids = vec![id_of(p.slug)];
        } else {
            for id in &ids {
                c.set_enabled(id, *on)?;
            }
        }
        Ok(json!({ "product": product, "on": on, "actors": ids }))
    }

    // ---------- XenoNet Dev ----------

    /// actor-request, in the demo and test instances only: a made-up
    /// program asks to register, so the request card can be built against
    /// it.
    pub fn simulate(&mut self, c: &mut dyn Host, kind: &str, a: Option<&str>) -> Result<Value> {
        if kind != "actor-request" || !c.demo() {
            return Ok(Value::Null);
        }
        let t = c.now();
        let made = (|| -> Result<Value> {
            let raw = a.filter(|s| !xenonet_text::text::trim(s).is_empty()).unwrap_or("Heron");
            let name = check_name(Some(raw), None).or_else(bad)?;
            let base = slug_from(&name);
            let slug = if slug_refusal(Some(&base), None).is_none() && !self.taken_by(c, &base, None, None, Some(t))? {
                Some(base)
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
            let slug_text = slug.unwrap_or_else(|| "null".into());
            let ask = Ask {
                name,
                slug: slug_text.clone(),
                product: Value::Null,
                capabilities: vec!["run-commands".into(), "edit-repos".into(), "message-agents".into()],
                ignored: vec![],
                heartbeat_ms: num(60e3),
                pid: Value::Null,
            };
            let asker = json!({ "program": format!("~/bin/{slug_text}"), "parents": ["/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], "folder": format!("~/Projects/{slug_text}") });
            self.expire_lapsed(t)?;
            let (id, credentials) = self.open(c, &ask, &asker, t)?;
            self.changed(t)?;
            Ok(json!({ "ok": true, "kind": kind, "a": credentials["id"], "request": id, "token": credentials["token"], "expiresAt": credentials["expiresAt"] }))
        })();
        match made {
            Err(Fail::Actor(e)) => Ok(json!({ "ok": false, "kind": kind, "code": e.code, "error": e.message })),
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

fn num(n: f64) -> Value {
    xenonet_text::json::number(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_read_as_the_table_says() {
        let body = json!({ "name": "Wren", "capabilities": ["browse", "fly\u{1F600}"] });
        let ask = Registration::read_ask(&body, false).unwrap();
        assert_eq!((ask.slug.as_str(), ask.capabilities.clone(), ask.ignored.clone()), ("wren", vec!["browse".to_string()], vec!["fly??".to_string()]));
        let slow = json!({ "name": "Wren", "heartbeatMs": 10 });
        assert!(matches!(Registration::read_ask(&slow, false), Err(Fail::Actor(ActorError { status, .. })) if status == 400.0));
        let dots = json!({ "name": "Dot", "product": { "id": "dots" } });
        assert!(matches!(Registration::read_ask(&dots, false), Err(Fail::Actor(ActorError { code: "not-allowed", .. }))));
        assert_eq!(base64url(&[0, 0, 0, 0, 0, 0, 0, 0, 0]).len(), 12);
    }
}
