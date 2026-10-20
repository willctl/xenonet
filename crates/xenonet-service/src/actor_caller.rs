//! Who's calling with an actor's token (server/routes/actors.js's
//! createActorCaller), shared by the actors' routes and the engine's: the
//! token's actor, refused while it waits for you (unless `waiting`), and
//! held to its processes on macOS. A process that sends 20 tokens XenoNet
//! doesn't know in a minute is answered 429 for a minute after (good tokens
//! still work). The actors' rates and the bad tokens' are kept apart, so
//! processes sending bad tokens can't crowd out an actor's buckets.

use crate::buckets::{now_ms, Buckets, Rate};
use crate::cores::{Registration, Verify};
use crate::http::Req;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_actors::tokens::bearer_of;
use xenonet_base::sync::Locked;

/// Rates, per the interface's table of limits (server/routes/actors.js).
pub mod rates {
    use crate::buckets::Rate;
    pub const HEARTBEAT: Rate = Rate { rate: 1.0, burst: 2.0 };
    pub const HEARTBEATS: Rate = Rate { rate: 8.0, burst: 16.0 };
    pub const EVENTS: Rate = Rate { rate: 1.0, burst: 120.0 };
    pub const TARGETS: Rate = Rate { rate: 0.5, burst: 10.0 };
    pub const BAD_TOKENS: Rate = Rate { rate: 20.0 / 60.0, burst: 20.0 };
    pub const MESSAGES: Rate = Rate { rate: 60.0 / 3600.0, burst: 60.0 };
    pub const WIDE: Rate = Rate { rate: 6.0 / 3600.0, burst: 6.0 };
}

pub const BLOCK_MS: f64 = 60e3;

/// A refusal: `{ ok: false, status, code, error, extra, headers }`.
#[derive(Clone, Debug)]
pub struct Refusal {
    pub status: u16,
    pub code: String,
    pub error: String,
    pub extra: Value,
    pub headers: Vec<(String, String)>,
}

fn no(status: u16, code: &str, error: impl Into<String>) -> Refusal {
    Refusal { status, code: code.into(), error: error.into(), extra: json!({}), headers: Vec::new() }
}

fn unauthorized(code: &str, error: String, extra: Value) -> Refusal {
    let realm = if code == "no-token" { "Bearer realm=\"xenonet\"".to_string() } else { "Bearer realm=\"xenonet\", error=\"invalid_token\"".to_string() };
    Refusal { status: 401, code: code.into(), error, extra, headers: vec![("www-authenticate".into(), realm)] }
}

/// `{ "retry-after": seconds }`, at least 1, a minute for a wait that never ends.
pub fn retry_after(ms: f64) -> (String, String) {
    let ms = if ms.is_finite() { ms } else { BLOCK_MS };
    ("retry-after".into(), (ms / 1000.0).ceil().max(1.0).to_string())
}

pub struct ActorCaller {
    registration: Arc<dyn Registration>,
    verify: Arc<dyn Verify>,
    buckets: Mutex<Buckets>,
    bad: Mutex<Buckets>,
    blocked: Mutex<HashMap<String, f64>>,
    now: Box<dyn Fn() -> f64 + Send + Sync>,
}

impl ActorCaller {
    pub fn new(registration: Arc<dyn Registration>, verify: Arc<dyn Verify>) -> ActorCaller {
        ActorCaller::with_clock(registration, verify, Box::new(now_ms))
    }

    pub fn with_clock(registration: Arc<dyn Registration>, verify: Arc<dyn Verify>, now: Box<dyn Fn() -> f64 + Send + Sync>) -> ActorCaller {
        ActorCaller { registration, verify, buckets: Mutex::new(Buckets::default()), bad: Mutex::new(Buckets::default()), blocked: Mutex::new(HashMap::new()), now }
    }

    async fn bad_token(&self, req: &Req) -> Option<Refusal> {
        let peer = self.verify.peer(req.peer).await;
        let t = (self.now)();
        let until = self.blocked.locked().get(&peer).copied().unwrap_or(0.0);
        let words = "too many tokens XenoNet doesn't know came from this process; wait a minute";
        if until > t {
            let mut r = no(429, "too-many", words);
            r.headers.push(retry_after(until - t));
            return Some(r);
        }
        if self.bad.locked().take(&format!("bad|{peer}"), rates::BAD_TOKENS, 1.0, t) != 0.0 {
            let mut blocked = self.blocked.locked();
            if blocked.len() > 256 {
                blocked.clear();
            }
            blocked.insert(peer, t + BLOCK_MS);
            let mut r = no(429, "too-many", words);
            r.headers.push(retry_after(BLOCK_MS));
            return Some(r);
        }
        None
    }

    /// The actor a request's token belongs to: its auth, or a refusal.
    pub async fn authenticate(&self, req: &Req, waiting: bool) -> Result<Value, Refusal> {
        let token = req.header("authorization").map(|raw| bearer_of(raw).map(str::to_string));
        // bearerOf: undefined without a header, null for one that isn't a Bearer.
        let Some(Some(token)) = token else {
            return Err(unauthorized("no-token", "this route takes an actor's token, as Authorization: Bearer <token>".into(), json!({})));
        };
        if req.header("x-xenonet-agent").is_some() {
            return Err(no(400, "bad-request", "a caller is a session or an actor, never both, so send x-xenonet-agent or a token, not both"));
        }
        let auth = self.registration.authenticate(&token);
        if auth["ok"] != json!(true) {
            let code = auth["code"].as_str().unwrap_or("bad-token").to_string();
            if code == "bad-token" {
                if let Some(r) = self.bad_token(req).await {
                    return Err(r);
                }
            }
            let extra = match auth.get("reason") {
                Some(r) if !r.is_null() && r != &json!("") && r != &json!(false) => json!({ "reason": r }),
                _ => json!({}),
            };
            return Err(unauthorized(&code, xenonet_base::text::js_string(&auth["error"]), extra));
        }
        if auth["state"] == "waiting" && !waiting {
            return Err(no(403, "waiting", "this actor is waiting for you to allow it in XenoNet's window; try again later"));
        }
        Ok(auth)
    }

    /// Whether the request comes from where its token may be used, naming
    /// `pid` or not: the verdict, or a refusal.
    pub async fn check(&self, req: &Req, auth: &Value, pid: Value) -> Result<Value, Refusal> {
        let slug = auth["slug"].as_str().unwrap_or("");
        let verdict = self.verify.actor(req.peer, slug, self.registration.product_of(auth), pid).await;
        if verdict["ok"] == json!(true) {
            Ok(verdict)
        } else {
            Err(no(403, "wrong-process", crate::cores::reason(&verdict)))
        }
    }

    /// A rate for `key`: 0, or the milliseconds until it allows n.
    pub fn take(&self, key: &str, rate: Rate, n: f64) -> f64 {
        self.buckets.locked().take(key, rate, n, (self.now)())
    }

    pub fn forget(&self, slug: &str) {
        self.buckets.locked().forget(&format!("a:{slug}|"));
    }
}

/// `bearerOf(req.headers) !== undefined`: the request carries an
/// Authorization header at all.
pub fn has_token(req: &Req) -> bool {
    req.header("authorization").is_some()
}
