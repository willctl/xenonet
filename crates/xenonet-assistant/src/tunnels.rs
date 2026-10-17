//! core/assistant/tunnels.js: tunnels XenoNet sets up and runs for remote
//! actors' connections through OpenAI's Secure MCP Tunnel. Which tunnel a
//! connection uses, where its key is kept, each run's state and its
//! backoff after an exit, its log as XenoNet shows it, and the client's
//! own environment are here; the processes, the timers, the credential
//! store and the health probes are the twin's.
//!
//! The key goes only into tunnel-client's own environment
//! (CONTROL_PLANE_API_KEY), never into a file, a command line or a log;
//! its log lines are read as JSON and anything that looks like a key is
//! taken out before XenoNet keeps or shows one.

use crate::tunnel_client::{is_runtime_key, is_tunnel_id, log_line, mask_keys};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use xenonet_app::jsv::truthy;
use xenonet_base::json::number as num;
use xenonet_base::text::js_string;

pub const TUNNEL_TRANSPORT: &str = "openai-secure-mcp-tunnel";
pub const GUIDE: &str = "https://developers.openai.com/api/docs/guides/secure-mcp-tunnels";
pub const POLL_MS: f64 = 5000.0;
pub const FIRST_POLL_MS: f64 = 1500.0;
// After it exits by itself: wait, then try again, longer each time; after
// as many exits in ten minutes as there are waits, it stays failed.
pub const BACKOFF_MS: [f64; 3] = [5e3, 20e3, 60e3];
pub const WINDOW_MS: f64 = 10.0 * 60e3;
pub const LINES: usize = 30;
pub const HAS_MS: f64 = 60e3;

/// The store's item for a tunnel's key, unless you name one you keep.
pub fn key_target_for(tunnel_id: &str) -> String {
    format!("XenoNet/OpenAI tunnel/{tunnel_id}")
}

/// An error with its HTTP status and code (TunnelError).
#[derive(Debug, PartialEq)]
pub struct Refused {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

fn fail<T>(status: u16, code: &'static str, message: impl Into<String>) -> Result<T, Refused> {
    Err(Refused { status, code, message: message.into() })
}

/// Which tunnel a connection uses, where its key is kept, and whether it
/// starts with XenoNet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub tunnel_id: Option<String>,
    pub key_target: Option<String>,
    pub autostart: bool,
}

/// A connection's transport_binding, as kept (JSON text, or none);
/// `target_ok` is the credential store's check of an item's name.
pub fn binding_of(text: Option<&str>, target_ok: &dyn Fn(&str) -> bool) -> Binding {
    let b = text.filter(|t| !t.is_empty()).and_then(|t| xenonet_base::json::parse(t).ok()).unwrap_or_else(|| json!({}));
    let s = |k: &str| b.get(k).filter(|v| !v.is_null()).map_or_else(String::new, js_string);
    let t = s("tunnel_id");
    let k = s("key_target");
    Binding { tunnel_id: is_tunnel_id(&t).then_some(t), key_target: target_ok(&k).then_some(k), autostart: b.get("autostart") == Some(&json!(true)) }
}

pub fn key_target_of(b: &Binding) -> Option<String> {
    b.key_target.clone().or_else(|| b.tunnel_id.as_deref().map(key_target_for))
}

impl Binding {
    /// As transport_binding keeps it.
    pub fn text(&self) -> String {
        xenonet_base::json::stringify(&json!({ "tunnel_id": self.tunnel_id, "key_target": self.key_target, "autostart": self.autostart }), 0)
    }
}

/// configure(): the binding with what was given (undefined as None)
/// changed, or why not.
pub fn configure(b: &Binding, tunnel_id: Option<&Value>, key_target: Option<&Value>, autostart: Option<&Value>, target_ok: &dyn Fn(&str) -> bool) -> Result<Binding, Refused> {
    let mut next = b.clone();
    if let Some(t) = tunnel_id {
        let s = if t.is_null() { String::new() } else { js_string(t) };
        if !is_tunnel_id(&s) {
            return fail(400, "bad-request", "a tunnel's id starts tunnel_, as OpenAI's Tunnels page shows it");
        }
        next.tunnel_id = Some(s);
    }
    if let Some(k) = key_target {
        if !k.is_null() && k != &json!("") && !target_ok(&js_string(k)) {
            return fail(400, "bad-request", "a credential's name is printable text, without quotes or backslashes");
        }
        next.key_target = if truthy(Some(k)) { Some(js_string(k)) } else { None };
    }
    if let Some(a) = autostart {
        next.autostart = a == &json!(true);
    }
    Ok(next)
}

/// The runtime key as storeKey takes it, trimmed, or why not.
pub fn runtime_key(b: &Binding, key: Option<&Value>) -> Result<String, Refused> {
    if b.tunnel_id.is_none() {
        return fail(409, "no-tunnel", "say which tunnel it is first");
    }
    let k = key.filter(|v| !v.is_null()).map_or_else(String::new, js_string);
    let k = xenonet_base::text::js_trim(&k).to_string();
    if !is_runtime_key(&k) {
        return fail(400, "bad-request", "that isn't an OpenAI API key (sk-…)");
    }
    Ok(k)
}

/// tunnel-client's own environment: XenoNet's, with the key, the port and
/// how to run the adapter; a run's never has OPENAI_API_KEY (`run`), and
/// its doctor's keeps what XenoNet's had, as the JS did.
pub fn child_env(env: &Map<String, Value>, key: &str, port: &Value, run_as_node: bool, run: bool) -> Map<String, Value> {
    let mut e = env.clone();
    e.insert("CONTROL_PLANE_API_KEY".into(), json!(key));
    e.insert("XENONET_PORT".into(), json!(js_string(port)));
    if run_as_node {
        e.insert("ELECTRON_RUN_AS_NODE".into(), json!("1"));
    }
    if run {
        e.shift_remove("OPENAI_API_KEY");
    }
    e
}

/// The address a profile of your own answers its readiness on, for one
/// with a fixed health address on this machine.
pub fn readyz_of(health: Option<&str>) -> Option<String> {
    let h = health?;
    let (host, port) = h.rsplit_once(':')?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") || !(2..=5).contains(&port.len()) || !port.bytes().all(|c| c.is_ascii_digit()) || port == "0" {
        return None;
    }
    Some(format!("http://{host}:{port}/readyz"))
}

/// The health address a run wrote, as the URL to poll, or None for one
/// that isn't on this machine.
pub fn health_url(written: &str) -> Option<String> {
    let u = xenonet_base::text::js_trim(written);
    let rest = u.strip_prefix("http://")?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (host, port) = rest.rsplit_once(':')?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") || port.is_empty() || !port.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}/health?details=true", u.strip_suffix('/').unwrap_or(u)))
}

/// tunnel-client doctor's output as XenoNet shows it: anything like a key
/// taken out, the home folder as ~, at most 80 lines.
pub fn check_lines(text: &str, home: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .map(|l| {
            let l: String = mask_keys(l).chars().filter(|c| !matches!(*c as u32, 0..=8 | 0x0b..=0x1f)).collect();
            xenonet_base::paths::tilde(&l, home).trim_end_matches(xenonet_base::text::is_js_space).to_string()
        })
        .filter(|l| !xenonet_base::text::js_trim(l).is_empty())
        .take(80)
        .collect()
}

struct Run {
    state: String,
    why: Option<String>,
    since: f64,
    version: Value,
    polled_at: Value,
    exits: Vec<f64>,
    lines: Vec<Value>,
    stopping: bool,
}

/// What became of a run after something happened to it.
#[derive(Debug, PartialEq)]
pub struct After {
    /// Whether its state or why changed (the twin says "changed").
    pub changed: bool,
    /// Try again after this long, for an exit with tries left.
    pub retry_ms: Option<f64>,
    /// What to log, if anything.
    pub log: Option<String>,
}

/// The runs, by grant, and what the credential store said lately.
#[derive(Default)]
pub struct Tunnels {
    runs: HashMap<String, Run>,
    order: Vec<String>,
    has: HashMap<String, (f64, bool)>,
}

impl Tunnels {
    /// A run's view: off when there's none. `pid` is its program's.
    pub fn view(&self, grant: &str, pid: Option<f64>) -> Value {
        match self.runs.get(grant) {
            None => json!({ "state": "off", "why": null, "since": null, "version": null, "polled_at": null, "pid": null, "restarts": 0, "log": [] }),
            Some(r) => {
                let log: Vec<Value> = r.lines.iter().skip(r.lines.len().saturating_sub(8)).cloned().collect();
                json!({ "state": r.state, "why": r.why, "since": num(r.since), "version": r.version, "polled_at": r.polled_at, "pid": pid.map_or(Value::Null, num), "restarts": r.exits.len(), "log": log })
            }
        }
    }

    pub fn has_run(&self, grant: &str) -> bool {
        self.runs.contains_key(grant)
    }

    pub fn stopping(&self, grant: &str) -> bool {
        self.runs.get(grant).is_some_and(|r| r.stopping)
    }

    pub fn grants(&self) -> Vec<String> {
        self.order.clone()
    }

    fn set(&mut self, grant: &str, state: &str, why: Option<String>, now: f64) -> bool {
        let Some(r) = self.runs.get_mut(grant) else { return false };
        if r.state == state && r.why == why {
            return false;
        }
        r.state = state.into();
        r.why = why;
        r.since = now;
        true
    }

    /// A run about to start: the one there was, or a new one, off.
    pub fn launching(&mut self, grant: &str, now: f64) -> bool {
        if !self.runs.contains_key(grant) {
            self.order.push(grant.into());
            self.runs.insert(grant.into(), Run { state: "off".into(), why: None, since: now, version: Value::Null, polled_at: Value::Null, exits: vec![], lines: vec![], stopping: false });
        }
        let r = self.runs.get_mut(grant).unwrap();
        r.stopping = false;
        self.set(grant, "starting", Some("tunnel-client is starting".into()), now)
    }

    /// A line of the run's log, kept as XenoNet shows it; what to log of
    /// it (its errors and warnings).
    pub fn line(&mut self, grant: &str, raw: &str, now: f64, home: &str) -> Option<String> {
        let line = log_line(xenonet_base::text::js_trim(raw))?;
        let r = self.runs.get_mut(grant)?;
        let level = js_string(&line["level"]);
        let msg = js_string(&line["msg"]);
        r.lines.push(json!({ "at": num(now), "level": level, "msg": xenonet_base::paths::tilde(&msg, home) }));
        if r.lines.len() > LINES {
            let extra = r.lines.len() - LINES;
            r.lines.drain(..extra);
        }
        (level == "error" || level == "warn").then(|| format!("tunnel {grant}: {level}: {msg}"))
    }

    /// What its health check said (None when it didn't answer).
    pub fn health(&mut self, grant: &str, health: Option<&Value>, now: f64) -> bool {
        let s = crate::tunnel_client::health_of(health);
        if let Some(r) = self.runs.get_mut(grant) {
            if let Some(v) = s.get("version").filter(|v| !v.is_null()) {
                r.version = v.clone();
            }
            if let Some(p) = s.get("polledAt").filter(|v| !v.is_null()) {
                r.polled_at = p.clone();
            }
        }
        let why = s.get("why").filter(|v| !v.is_null()).map(js_string);
        self.set(grant, &js_string(&s["state"]), why, now)
    }

    /// Its program ended (or couldn't start): off when it was stopped,
    /// else failed, tried again after a wait while there are tries left.
    pub fn exited(&mut self, grant: &str, why: &str, now: f64) -> After {
        let Some(r) = self.runs.get_mut(grant) else { return After { changed: false, retry_ms: None, log: None } };
        if r.stopping {
            return After { changed: self.set(grant, "off", None, now), retry_ms: None, log: None };
        }
        r.exits.retain(|at| now - at < WINDOW_MS);
        r.exits.push(now);
        let last = r.lines.iter().rev().find(|l| l["level"] == json!("error")).map(|l| js_string(&l["msg"]));
        let said = match last {
            Some(m) if !m.is_empty() => format!("{why}: {m}"),
            _ => why.to_string(),
        };
        let n = r.exits.len();
        let log = Some(format!("tunnel {grant}: {said}"));
        if n > BACKOFF_MS.len() {
            return After { changed: self.set(grant, "failed", Some(said), now), retry_ms: None, log };
        }
        let wait = BACKOFF_MS[n - 1];
        let changed = self.set(grant, "failed", Some(format!("{said}; trying again in {} s", xenonet_base::units::js_round(wait / 1000.0))), now);
        After { changed, retry_ms: Some(wait), log }
    }

    /// A start that failed after an exit's wait: failed, saying why.
    pub fn failed(&mut self, grant: &str, message: &str, now: f64) -> bool {
        self.set(grant, "failed", Some(message.into()), now)
    }

    /// Stopped by you, or because its grant can't be used now: stopping
    /// while its program is there, off once it isn't. None for no run.
    pub fn stop(&mut self, grant: &str, reason: Option<&str>, has_child: bool, now: f64) -> Option<bool> {
        let r = self.runs.get_mut(grant)?;
        r.stopping = true;
        r.exits.clear();
        Some(self.set(grant, if has_child { "stopping" } else { "off" }, reason.map(String::from), now))
    }

    /// What the store said of a key lately, or None to ask it again.
    pub fn has_key(&self, target: &str, now: f64) -> Option<bool> {
        self.has.get(target).filter(|(at, _)| now - at < HAS_MS).map(|(_, h)| *h)
    }

    pub fn set_has(&mut self, target: &str, has: bool, now: f64) {
        self.has.insert(target.into(), (now, has));
    }
}

/// A tunnel as status() lists it: the connection `x`, its binding, whether
/// its key is stored, a profile of your own for its grant, and its run.
pub fn status_entry(x: &Value, b: &Binding, key_stored: bool, yours: Option<&Value>, run: &Value) -> Value {
    let target = key_target_of(b);
    let mut m = Map::new();
    m.insert("grant".into(), x.get("grant_id").cloned().unwrap_or(Value::Null));
    m.insert("connection".into(), x.get("connection_id").cloned().unwrap_or(Value::Null));
    m.insert("actor".into(), x.get("actor_id").cloned().unwrap_or(Value::Null));
    m.insert("label".into(), x.get("label").cloned().unwrap_or(Value::Null));
    m.insert("tunnel_id".into(), json!(b.tunnel_id));
    m.insert("key_target".into(), json!(target));
    m.insert("key_named".into(), json!(b.key_target.is_some()));
    m.insert("key".into(), json!(if target.is_some() && key_stored { "stored" } else { "missing" }));
    m.insert("autostart".into(), json!(b.autostart));
    m.insert("profile".into(), yours.map_or(Value::Null, |p| json!({ "name": p.get("name"), "tunnel_id": p.get("tunnelId") })));
    for (k, v) in run.as_object().into_iter().flatten() {
        m.insert(k.clone(), v.clone());
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exits_back_off_then_stay_failed() {
        let mut t = Tunnels::default();
        t.launching("gr_x", 0.0);
        let a = t.exited("gr_x", "it exited with code 1", 1.0);
        assert_eq!(a.retry_ms, Some(5e3));
        assert_eq!(t.view("gr_x", None)["why"], json!("it exited with code 1; trying again in 5 s"));
        t.exited("gr_x", "it exited", 2.0);
        t.exited("gr_x", "it exited", 3.0);
        let last = t.exited("gr_x", "it exited", 4.0);
        assert_eq!(last.retry_ms, None);
        assert_eq!(t.view("gr_x", None)["state"], json!("failed"));
        assert_eq!(t.view("gr_x", None)["restarts"], json!(4));
    }

    #[test]
    fn the_key_stays_in_the_clients_environment() {
        let mut env = Map::new();
        env.insert("OPENAI_API_KEY".into(), json!("sk-mine"));
        env.insert("PATH".into(), json!("/bin"));
        let e = child_env(&env, "sk-runtime", &json!(47841), false, true);
        assert_eq!(e.get("CONTROL_PLANE_API_KEY"), Some(&json!("sk-runtime")));
        assert!(!e.contains_key("OPENAI_API_KEY"));
        assert_eq!(check_lines("ok sk-abcdefghijk\n\n  \nbye\r\n", ""), vec!["ok sk-…", "bye"]);
        assert_eq!(readyz_of(Some("127.0.0.1:8080")).as_deref(), Some("http://127.0.0.1:8080/readyz"));
        assert_eq!(readyz_of(Some("127.0.0.1:0")), None);
        assert_eq!(health_url("http://127.0.0.1:4242/\n").as_deref(), Some("http://127.0.0.1:4242/health?details=true"));
        assert_eq!(health_url("http://evil.example:80"), None);
    }
}
