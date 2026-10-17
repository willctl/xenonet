//! core/assistant/client.js: the adapter's way to the service. `xenonet
//! mcp assistant --grant <grant>` finds the private endpoint by the
//! XenoNet it's for, reads its grant's credential from that XenoNet's
//! state folder, and says hello with it, over 127.0.0.1 only. When the
//! connection fails, the next try waits two seconds, then four, and so on
//! up to a minute, with some jitter.
//!
//! The connection and its requests are the twin's (components/rpc.js);
//! when to try, the endpoint file's checks and the grant's standing are
//! here. A credential never leaves this process except in hello.

use crate::credentials::{is_disabled, is_grant, is_marked_revoked, read_credential, revision_of};
use serde_json::{json, Value};

pub const CALL_MS: f64 = 12e3;
const FIRST: f64 = 2000.0;
const MAX: f64 = 60e3;

pub const ERROR_CODES: [&str; 13] = ["NOT_AUTHORIZED", "NOT_FOUND", "GRANT_EXPIRED", "GRANT_REVOKED", "DISABLED", "INVALID_ARGUMENT", "CURSOR_EXPIRED", "SOURCE_UNAVAILABLE", "SERVICE_UNAVAILABLE", "RATE_LIMITED", "INCOMPATIBLE_VERSION", "TIMEOUT", "INTERNAL_ERROR"];

/// A refusal: its code and what it says (AssistantRefused).
#[derive(Debug, PartialEq)]
pub struct Refused {
    pub code: String,
    pub detail: String,
}

fn refused(code: &str, detail: &str) -> Refused {
    Refused { code: code.into(), detail: detail.into() }
}

/// The endpoint the file names: its port and pid.
pub struct Endpoint {
    pub port: f64,
    pub pid: f64,
}

/// One grant's adapter.
pub struct Adapter {
    pub grant: Value,
    failures: u32,
    pub retry_at: f64,
    /// The state folder the endpoint named last, for standing().
    pub state_dir: Option<String>,
}

impl Adapter {
    pub fn new(grant: &Value) -> Result<Self, String> {
        if !is_grant(&xenonet_base::text::js_string(grant)) {
            return Err("--grant takes a grant's id (gr_…), as XenoNet's window shows it".into());
        }
        Ok(Adapter { grant: grant.clone(), failures: 0, retry_at: 0.0, state_dir: None })
    }

    /// Whether it may try to connect now.
    pub fn may_try(&self, now: f64) -> bool {
        now >= self.retry_at
    }

    /// The endpoint file's text (None when it couldn't be read): the
    /// endpoint, or why not. `alive(pid)` says whether its writer still
    /// runs; a file a crashed service left names a port anyone could
    /// listen on since, and a credential never goes there.
    pub fn endpoint(&mut self, text: Option<&str>, alive: &mut dyn FnMut(f64) -> bool) -> Result<Endpoint, Refused> {
        let d = text.and_then(|t| xenonet_base::json::parse(t).ok()).ok_or_else(|| refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint for remote actors isn't listening: XenoNet isn't running, or no grant is active"))?;
        let int = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|n| n.fract() == 0.0 && n.is_finite());
        let (Some(port), Some(state)) = (int(d.get("port")), d.get("state").and_then(Value::as_str)) else { return Err(refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is unreadable")) };
        self.state_dir = Some(state.to_string());
        let pid = int(d.get("pid")).filter(|p| alive(*p)).ok_or_else(|| refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is from a service that isn't running"))?;
        Ok(Endpoint { port, pid })
    }

    /// The grant's credential's token in the state folder the endpoint
    /// named, for hello.
    pub fn token(&self) -> Result<String, Refused> {
        let dir = self.state_dir.as_deref().unwrap_or("");
        read_credential(dir, &self.grant).and_then(|c| c["token"].as_str().map(String::from)).ok_or_else(|| refused("NOT_AUTHORIZED", "this computer has no credential for that grant: it was revoked, or made by another XenoNet"))
    }

    pub fn connected(&mut self) {
        self.failures = 0;
        self.retry_at = 0.0;
    }

    /// A try failed: the next waits longer.
    pub fn failed(&mut self, now: f64, random: f64) {
        self.failures += 1;
        let base = MAX.min(FIRST * 2f64.powi(self.failures as i32 - 1));
        self.retry_at = now + xenonet_base::units::js_round(base * (0.8 + 0.4 * random));
    }

    /// Whether the grant still stands as far as this computer's files say:
    /// { ok, revision } or { ok: false, code }.
    pub fn standing(&self, now: f64) -> Value {
        let Some(dir) = self.state_dir.as_deref() else { return json!({ "ok": false, "code": "SERVICE_UNAVAILABLE" }) };
        if is_disabled(dir) {
            return json!({ "ok": false, "code": "DISABLED" });
        }
        if is_marked_revoked(dir, &self.grant) {
            return json!({ "ok": false, "code": "GRANT_REVOKED" });
        }
        let Some(c) = read_credential(dir, &self.grant) else { return json!({ "ok": false, "code": "GRANT_REVOKED" }) };
        if c["expiresAt"].as_f64().is_some_and(|e| e <= now) {
            return json!({ "ok": false, "code": "GRANT_EXPIRED" });
        }
        json!({ "ok": true, "revision": revision_of(dir, &self.grant).map_or(Value::Null, xenonet_base::json::number) })
    }
}

/// The code a failed hello is passed on with.
pub fn hello_code(message: &str) -> &str {
    ERROR_CODES.iter().find(|c| **c == message).copied().unwrap_or("SERVICE_UNAVAILABLE")
}

/// The code a read that threw is answered with.
pub fn read_code(message: &str) -> &'static str {
    if message.contains("did not answer") {
        "TIMEOUT"
    } else {
        "SERVICE_UNAVAILABLE"
    }
}

/// Whether the service's answer is an envelope the adapter passes on.
pub fn is_answer(r: &Value) -> bool {
    r.is_object() && matches!(r.get("ok"), Some(Value::Bool(_)))
}
