//! core/native/opencode-events.js: what an OpenCode session's XenoNet plugin
//! tells XenoNet as it happens, kept in memory per session. Whether a turn
//! is under way, a permission it waits on, the files it edited, and what its
//! finished assistant messages used. Metadata only: the plugin sends no
//! prompt, reply, tool output or file content, and this keeps none.
//!
//! The map prefers it to OpenCode's database for as long as it's fresh and
//! falls back to the database when the plugin is gone: a working mark lapses
//! `WORKING_MS` after the session's last event, a wait after `WAIT_MS`.
//!
//! An event is `{ type, … }`: `status { state: busy | idle }`,
//! `permission { state: asked | replied, id, tool? }`, `edit { path }`, or
//! `message { id, cost?, tokens?, provider?, model? }`. Anything else, or a
//! field of the wrong kind, is ignored, so a plugin newer than this XenoNet
//! does no harm.

use serde_json::{json, Value};
use std::collections::HashMap;
use xenonet_adapter::reader::{PluginEvents, PluginState};
use xenonet_sources::native::{edit_file, older, push_edit, Edits, EDIT_KEEP_MS};

pub const WORKING_MS: f64 = 5.0 * 60e3;
pub const WAIT_MS: f64 = 30.0 * 60e3;
pub const SESSIONS_MAX: usize = 200;
const WAITS_MAX: usize = 16;
const MESSAGES_MAX: usize = 64;
const EVENTS_MAX: usize = 20;
const PATH_MAX: usize = 4096;
const TOKENS: [&str; 5] = ["input", "output", "reasoning", "cacheRead", "cacheWrite"];

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `^[\w.:\/-]{1,80}$`
fn name_ok(s: &str) -> bool {
    (1..=80).contains(&s.chars().count()) && s.chars().all(|c| word(c) || matches!(c, '.' | ':' | '/' | '-'))
}

/// `^[\w.:-]{1,128}$`
fn id_ok(s: &str) -> bool {
    (1..=128).contains(&s.chars().count()) && s.chars().all(|c| word(c) || matches!(c, '.' | ':' | '-'))
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0)
}

struct Wait {
    id: String,
    tool: Option<String>,
    since: f64,
}

struct Session {
    at: f64,
    working: bool,
    working_at: f64,
    waits: Vec<Wait>,
    edits: Edits,
    messages: Vec<String>,
    cost: Option<f64>,
    tokens: Option<[Option<f64>; 5]>,
    model: Option<String>,
    provider: Option<String>,
    cwd: Option<String>,
}

#[derive(Default)]
pub struct OpenCodeEvents {
    sessions: HashMap<String, Session>,
}

impl OpenCodeEvents {
    pub fn new() -> Self {
        Self::default()
    }

    fn of(&mut self, id: &str, now: f64) -> &mut Session {
        if !self.sessions.contains_key(id) && self.sessions.len() >= SESSIONS_MAX {
            if let Some(oldest) = self.sessions.iter().min_by(|a, b| a.1.at.total_cmp(&b.1.at)).map(|(k, _)| k.clone()) {
                self.sessions.remove(&oldest);
            }
        }
        self.sessions.entry(id.to_string()).or_insert_with(|| Session {
            at: now,
            working: false,
            working_at: 0.0,
            waits: Vec::new(),
            edits: Edits::new(),
            messages: Vec::new(),
            cost: None,
            tokens: None,
            model: None,
            provider: None,
            cwd: None,
        })
    }
}

impl PluginEvents for OpenCodeEvents {
    /// Takes a request's events for session `id` (opencode:ses-…), run in
    /// `cwd`. Returns how many it used.
    fn ingest(&mut self, id: &str, events: &Value, cwd: Option<&str>, now: f64) -> usize {
        let Some(list) = events.as_array() else { return 0 };
        let mut used = 0;
        for e in list.iter().take(EVENTS_MAX) {
            if !e.is_object() {
                continue;
            }
            let s = self.of(id, now);
            if let Some(c) = cwd.filter(|c| !c.is_empty()) {
                s.cwd = Some(c.to_string());
            }
            if take(s, e, now) {
                s.at = now;
                used += 1;
            }
        }
        used
    }

    /// The session as the plugin last said it, or None when it hasn't been
    /// heard from lately enough to go by (the database decides then).
    fn state(&mut self, id: &str, now: f64) -> Option<PluginState> {
        let s = self.sessions.get_mut(id)?;
        s.edits.retain(|_, list| !list.last().is_some_and(|e| older(now, e.ts, EDIT_KEEP_MS)));
        let heard = now - s.at < WORKING_MS;
        let wait = s.waits.iter().filter(|w| now - w.since < WAIT_MS).min_by(|a, b| a.since.total_cmp(&b.since));
        if !heard && wait.is_none() && s.edits.is_empty() {
            return None;
        }
        let usage = (s.cost.is_some() || s.tokens.is_some() || s.model.is_some() || s.provider.is_some()).then(|| {
            let t = s.tokens.unwrap_or([None; 5]);
            let tokens: serde_json::Map<String, Value> = TOKENS.iter().zip(t).map(|(k, v)| ((*k).to_string(), v.map_or(Value::Null, |n| json!(n)))).collect();
            json!({ "costUsd": s.cost, "tokens": tokens, "provider": s.provider, "model": s.model })
        });
        Some(PluginState {
            last_event_at: s.at,
            working: s.working && now - s.working_at < WORKING_MS,
            needs: wait.map(|w| (w.tool.as_ref().map_or_else(|| "Permission needed".to_string(), |t| format!("Permission to use {t}")), w.since)),
            edits: s.edits.iter().filter_map(|(_, list)| list.last().map(|e| (e.path.clone(), e.ts))).collect(),
            model: s.model.clone(),
            usage,
        })
    }

    /// The session as the engine reads a live one: `{ observed, lastEventAt,
    /// status, needsYou (its kind), edits: [{ path, ts }] }`.
    fn live(&mut self, id: &str, now: f64) -> Option<Value> {
        let st = self.state(id, now)?;
        Some(json!({
            "observed": false,
            "lastEventAt": st.last_event_at,
            "status": if st.working { "working" } else { "idle" },
            "needsYou": if st.needs.is_some() { json!("permission") } else { Value::Null },
            "edits": st.edits.iter().map(|(p, t)| json!({ "path": p, "ts": t })).collect::<Vec<_>>(),
        }))
    }

    /// The live view's session snapshots, in the shape the tailer's have, for
    /// the sessions the plugin has spoken for and `has` says are on the map.
    fn snapshot(&mut self, now: f64, has: &dyn Fn(&str) -> bool) -> Vec<Value> {
        let mut ids: Vec<String> = self.sessions.keys().cloned().collect();
        ids.sort();
        let mut out = Vec::new();
        for id in ids {
            if !has(&id) {
                continue;
            }
            let Some(st) = self.state(&id, now) else { continue };
            let mut files = st.edits.clone();
            files.sort_by(|a, b| b.1.total_cmp(&a.1));
            out.push(json!({
                "id": id,
                "app": "opencode",
                "status": if st.working { "thinking" } else { "idle" },
                "tokens": { "processed": 0, "output": 0 },
                "tokensToday": 0,
                "tpm": 0,
                "context": { "used": 0, "window": 200_000 },
                "forecast": null,
                "model": st.model,
                "turnStartedAt": null,
                "lastTool": null,
                "toolCounts": {},
                "subagents": [],
                "files": files.iter().take(10).map(|(p, t)| json!({ "path": p, "ts": t })).collect::<Vec<_>>(),
                "mcp": [],
                "needsYou": st.needs.as_ref().map_or(Value::Null, |(text, since)| json!({ "kind": "permission", "text": text, "since": since })),
                "stuck": null,
                "lastMessage": null,
                "lastTurnEndAt": null,
                "interruptedAt": null,
                "lastEventAt": st.last_event_at,
            }));
        }
        out
    }
}

fn text<'a>(e: &'a Value, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str)
}

fn take(s: &mut Session, e: &Value, now: f64) -> bool {
    match text(e, "type") {
        Some("status") => {
            let busy = match text(e, "state") {
                Some("busy") => true,
                Some("idle") => false,
                _ => return false,
            };
            s.working = busy;
            s.working_at = now;
            // A turn that ended asks for nothing more.
            if !busy {
                s.waits.clear();
            }
            true
        }
        Some("permission") => {
            let Some(id) = text(e, "id").filter(|i| id_ok(i)) else { return false };
            match text(e, "state") {
                Some("asked") => {
                    s.waits.retain(|w| w.id != id);
                    if s.waits.len() >= WAITS_MAX {
                        s.waits.remove(0);
                    }
                    let tool = text(e, "tool").filter(|t| name_ok(t)).map(str::to_string);
                    s.waits.push(Wait { id: id.to_string(), tool, since: now });
                    true
                }
                Some("replied") => {
                    s.waits.retain(|w| w.id != id);
                    true
                }
                _ => false,
            }
        }
        Some("edit") => {
            let Some(path) = text(e, "path").filter(|p| !p.is_empty() && p.chars().count() <= PATH_MAX) else { return false };
            let Some(file) = edit_file(s.cwd.as_deref(), path) else { return false };
            push_edit(&mut s.edits, &file, now);
            true
        }
        Some("message") => {
            let Some(id) = text(e, "id").filter(|i| id_ok(i)) else { return false };
            if s.messages.iter().any(|m| m == id) {
                return false;
            }
            s.messages.push(id.to_string());
            if s.messages.len() > MESSAGES_MAX {
                s.messages.remove(0);
            }
            if let Some(c) = num(e.get("cost")) {
                s.cost = Some(s.cost.unwrap_or(0.0) + c);
            }
            if let Some(t) = e.get("tokens").filter(|t| t.is_object()) {
                for (i, k) in TOKENS.iter().enumerate() {
                    if let Some(n) = num(t.get(*k)) {
                        let slot = &mut s.tokens.get_or_insert([None; 5])[i];
                        *slot = Some(slot.unwrap_or(0.0) + n);
                    }
                }
            }
            if let Some(m) = text(e, "model").filter(|m| name_ok(m)) {
                s.model = Some(m.to_string());
            }
            if let Some(p) = text(e, "provider").filter(|p| name_ok(p)) {
                s.provider = Some(p.to_string());
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "opencode:ses-abc123";

    #[test]
    fn working_idle_and_a_permission_wait_lapse_when_the_plugin_goes_quiet() {
        let mut ev = OpenCodeEvents::new();
        let t0 = 1_000_000.0;
        assert!(ev.state(ID, t0).is_none(), "nothing heard: the database decides");
        ev.ingest(ID, &json!([{ "type": "status", "state": "busy" }]), None, t0);
        assert!(ev.state(ID, t0 + 1000.0).unwrap().working);
        ev.ingest(ID, &json!([{ "type": "permission", "state": "asked", "id": "per_1", "tool": "edit" }]), None, t0 + 2000.0);
        assert_eq!(ev.state(ID, t0 + 3000.0).unwrap().needs, Some(("Permission to use edit".into(), t0 + 2000.0)));
        ev.ingest(ID, &json!([{ "type": "permission", "state": "replied", "id": "per_1" }]), None, t0 + 4000.0);
        assert!(ev.state(ID, t0 + 5000.0).unwrap().needs.is_none());
        ev.ingest(ID, &json!([{ "type": "permission", "state": "asked", "id": "per_2" }]), None, t0 + 6000.0);
        assert_eq!(ev.state(ID, t0 + 7000.0).unwrap().needs.unwrap().0, "Permission needed");
        ev.ingest(ID, &json!([{ "type": "status", "state": "idle" }]), None, t0 + 8000.0);
        let idle = ev.state(ID, t0 + 9000.0).unwrap();
        assert!(!idle.working && idle.needs.is_none(), "a turn that ended asks nothing");
        ev.ingest(ID, &json!([{ "type": "status", "state": "busy" }]), None, t0 + 10_000.0);
        assert!(ev.state(ID, t0 + 10_000.0 + WORKING_MS - 1.0).unwrap().working);
        assert!(ev.state(ID, t0 + 10_000.0 + WORKING_MS + 1.0).is_none(), "gone quiet: back to the database");
        ev.ingest(ID, &json!([{ "type": "permission", "state": "asked", "id": "per_3" }]), None, t0 + 20_000.0);
        assert!(ev.state(ID, t0 + 20_000.0 + WORKING_MS + 1.0).unwrap().needs.is_some(), "a wait outlives a quiet plugin");
        assert!(ev.state(ID, t0 + 20_000.0 + WAIT_MS + 1.0).is_none());
    }

    #[test]
    fn edits_resolve_against_the_folder_and_messages_sum_once_each() {
        let mut ev = OpenCodeEvents::new();
        let now = 5e12;
        ev.ingest(ID, &json!([{ "type": "edit", "path": "src/a.js" }, { "type": "edit", "path": "/w/other/b.js" }, { "type": "edit", "path": "" }]), Some("/w/atlas"), now);
        let mut files: Vec<String> = ev.state(ID, now).unwrap().edits.into_iter().map(|e| e.0).collect();
        files.sort();
        assert_eq!(files, ["/w/atlas/src/a.js", "/w/other/b.js"]);
        let m = json!({ "type": "message", "id": "msg_1", "cost": 0.5, "tokens": { "input": 10, "output": 2 }, "provider": "opencode", "model": "big-pickle" });
        let m2 = json!({ "type": "message", "id": "msg_2", "cost": 0.25, "tokens": { "input": 10, "output": 2 } });
        ev.ingest(ID, &json!([m, m, m2]), None, now);
        let u = ev.state(ID, now).unwrap().usage.unwrap();
        assert_eq!(u["costUsd"], 0.75);
        assert_eq!(u["tokens"], json!({ "input": 20.0, "output": 4.0, "reasoning": null, "cacheRead": null, "cacheWrite": null }));
        assert_eq!((u["provider"].as_str(), u["model"].as_str()), (Some("opencode"), Some("big-pickle")));
    }

    #[test]
    fn what_it_does_not_know_or_cannot_use_is_ignored() {
        let mut ev = OpenCodeEvents::new();
        let bad = json!([null, 7, "x", {}, { "type": "nope" }, { "type": "status", "state": "sideways" }, { "type": "permission", "state": "asked" },
            { "type": "permission", "state": "asked", "id": "has space" }, { "type": "edit", "path": 5 },
            { "type": "message", "id": "m", "cost": -1, "tokens": { "input": "x" }, "model": "bad model!" }]);
        assert_eq!(ev.ingest(ID, &bad, None, 1.0), 1, "only the message with an id counts, with nothing in it to keep");
        assert_eq!(ev.ingest(ID, &json!("not a list"), None, 1.0), 0);
        assert!(ev.state(ID, 1.0).unwrap().usage.is_none());
        let fifty: Vec<Value> = (0..50).map(|_| json!({ "type": "status", "state": "busy" })).collect();
        assert_eq!(ev.ingest(ID, &Value::Array(fifty), None, 1.0), 20, "a request is read to its first 20 events");
    }

    #[test]
    fn the_snapshot_has_a_waiting_session_once_it_is_on_the_map() {
        let mut ev = OpenCodeEvents::new();
        let now = 5e12;
        ev.ingest(ID, &json!([{ "type": "status", "state": "busy" }, { "type": "permission", "state": "asked", "id": "per_1", "tool": "edit" }, { "type": "message", "id": "m1", "model": "big-pickle" }]), None, now);
        assert!(ev.snapshot(now, &|_| false).is_empty(), "a session the map doesn't have isn't shown");
        let s = &ev.snapshot(now, &|id| id == ID)[0];
        assert_eq!((s["id"].as_str(), s["app"].as_str(), s["status"].as_str(), s["model"].as_str()), (Some(ID), Some("opencode"), Some("thinking"), Some("big-pickle")));
        assert_eq!(s["needsYou"], json!({ "kind": "permission", "text": "Permission to use edit", "since": now }));
        let live = ev.live(ID, now).unwrap();
        assert_eq!((live["status"].as_str(), live["needsYou"].as_str()), (Some("working"), Some("permission")));
    }
}
