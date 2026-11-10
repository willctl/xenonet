//! The delivery router: for each message and each session it's for, the
//! fastest way in that's allowed, then whether it arrived:
//! core/delivery/router.js.
//!
//!   live       into the session now: its Stop hook listening after a turn,
//!              a Claude Code channel, an OpenCode server's prompt_async,
//!              typed into its tmux pane, or a headless resume of a session
//!              whose process has gone
//!   next turn  waits for the session's hooks
//!   notified   nothing picked it up within ackMs, so XenoNet told you
//!
//! Waking a session spends its plan, so every wake is checked first:
//! against the policy (pause switch, wake budget), and with `reach`, which
//! decides what this machine lets the sender do (your consent); a message
//! that may not wake anything still waits for the next turn.
//!
//! The records are the router's; what it asks of the machine and the other
//! cores (the clock, the engine's views and store, the policy, the
//! channels, the ways in, the ack timers and the events) is `Host`'s.

use super::policy::{framed, hash_text, is_you, WakeAllowed};
use super::reach::{self, Action, Consent, Decision, Kind, Settings, Wake};
use super::{template, text_of, truthy, Delivered, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use xenonet_os::sync::Locked;

pub const ACK_MS: f64 = 60e3;
const KEEP: usize = 500;

/// The router's collaborators.
pub trait Host {
    /// The time, in epoch ms.
    fn now(&mut self) -> f64;
    /// How much of its plan an app's provider has used, 0 to 1 (`usage`);
    /// None without a fresh reading.
    fn usage(&mut self, app: &str) -> Option<f64>;
    /// This machine's wake setting (Settings → Network).
    fn wake_setting(&mut self) -> Wake;
    /// You're told a push nothing acknowledged: { message, agent, title,
    /// from }.
    fn notify(&mut self, notice: Value);
    /// A record changed: the delivery event, with the record.
    fn emit(&mut self, record: Value);
    /// The record's ack timer, set for `ms` from now: `expire` is called
    /// for the key when it fires.
    fn arm(&mut self, key: &str, ms: f64);
    fn clear(&mut self, key: &str);

    // The engine.

    /// A session's or actor's view, None when there's none.
    fn view(&mut self, id: &str) -> Option<Value>;
    /// Whether the session's Stop hook is listening for messages now.
    fn listening(&mut self, id: &str) -> bool;
    /// The process the session last made contact from, when known.
    fn contact_pid(&mut self, agent: &str) -> Option<f64>;
    fn mark_seen(&mut self, agent: &str, ids: &[Value], now: f64) -> Result<()>;
    fn mark_unseen(&mut self, agent: &str, ids: &[Value]) -> Result<()>;

    // The policy.

    /// Whether you allowed the session to be woken, from here or (`remote`)
    /// by actors too.
    fn consented(&mut self, session: &str, remote: bool) -> bool;
    /// The sessions with an allowance, live or not.
    fn consent_sessions(&mut self) -> Vec<String>;
    fn wake_allowed(&mut self, sender: &str, target: &str, usage: Option<f64>) -> WakeAllowed;
    fn spend_wake(&mut self, sender: &str, target: &str);
    fn audit(&mut self, entry: Value) -> Result<()>;

    // The ways in.

    /// Whether a Claude Code channel is live for the session.
    fn channel_live(&mut self, agent: &str) -> bool;
    /// An item handed to the session's channel; false when none is live.
    fn channel_push(&mut self, agent: &str, item: Value) -> bool;
    /// What the session's own OpenCode says of it, None when none knows it.
    fn opencode_info(&mut self, id: &str) -> Option<Value>;
    fn opencode_deliver(&mut self, id: &str, text: &str) -> Delivered;
    fn tmux_deliver(&mut self, pid: f64, text: &str) -> Delivered;
    /// Whether a headless resume is set up here (never in the demo).
    fn resumes(&mut self) -> bool;
    /// A turn resumed for the session (or, `fork`, a copy of it) with the
    /// message as its prompt.
    fn resume_deliver(&mut self, agent: &str, cwd: Option<&str>, text: &str, fork: bool) -> Delivered;
}

/// What the engine asks of live delivery.
pub trait Delivery {
    /// The OpenCode sessions you allowed XenoNet to wake.
    fn known(&self) -> Vec<String>;
    /// Whether a session can be reached without hooks.
    fn reaches(&self, id: &str) -> bool;
    /// Whether a session that ended can still be messaged.
    fn resumable(&self, view: &Value) -> bool;
    /// A wake for a session's listening Stop hook, spent when allowed.
    fn wake(&self, from: &str, agent: &str) -> WakeAllowed;
    /// How a message from `me` would reach `recipient` if sent now: None
    /// when it waits for the hooks.
    fn way(&self, me: &str, recipient: &Value) -> Option<&'static str>;
    /// What its own OpenCode says of an OpenCode session.
    fn describe(&self, id: &str) -> Option<Value>;
}

/// A delivery: one message to one session, and how it went.
#[derive(Clone, Debug)]
struct Rec {
    /// The message's id.
    message: Value,
    agent: String,
    from: String,
    /// queued, live, next-turn or notified.
    state: &'static str,
    /// listen, channel, opencode, tmux, resume, fork or hooks.
    how: Option<&'static str>,
    at: f64,
    acked_at: Option<f64>,
    why: Option<String>,
    from_name: Option<String>,
    /// Where a live delivery went: a pane, a server, a pid.
    location: Option<String>,
    /// An attempt under way, and a timer set.
    attempting: bool,
    timer: bool,
}

impl Rec {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("message".into(), self.message.clone());
        m.insert("agent".into(), Value::String(self.agent.clone()));
        m.insert("from".into(), Value::String(self.from.clone()));
        m.insert("state".into(), Value::String(self.state.into()));
        m.insert("how".into(), self.how.map_or(Value::Null, |h| Value::String(h.into())));
        m.insert("at".into(), xenonet_text::json::number(self.at));
        m.insert("ackedAt".into(), self.acked_at.map_or(Value::Null, xenonet_text::json::number));
        m.insert("why".into(), self.why.clone().map_or(Value::Null, Value::String));
        if let Some(n) = &self.from_name {
            m.insert("fromName".into(), Value::String(n.clone()));
        }
        if let Some(w) = &self.location {
            m.insert("where".into(), Value::String(w.clone()));
        }
        Value::Object(m)
    }
}

#[derive(Default)]
struct Inner {
    /// The records listed, oldest first, by `${message}|${agent}`.
    order: Vec<String>,
    recs: HashMap<String, Rec>,
}

#[derive(Default)]
pub struct Router {
    inner: Mutex<Inner>,
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

impl Router {
    fn with<T>(&self, key: &str, f: impl FnOnce(&mut Rec) -> T) -> Option<T> {
        self.inner.locked().recs.get_mut(key).map(f)
    }

    fn acked(&self, key: &str) -> bool {
        self.with(key, |r| r.acked_at.is_some()).unwrap_or(false)
    }

    /// The record as it is, for the delivery event.
    fn emit(&self, h: &mut dyn Host, key: &str) {
        if let Some(rec) = self.with(key, |r| r.to_json()) {
            h.emit(rec);
        }
    }

    /// A record let go once it's no longer listed and nothing waits on it.
    fn settle(&self, key: &str) {
        let mut i = self.inner.locked();
        let listed = i.order.iter().any(|k| k == key);
        if i.recs.get(key).is_some_and(|r| !r.attempting && !r.timer && !listed) {
            i.recs.remove(key);
        }
    }

    /// The latest `n` deliveries, oldest first.
    pub fn list(&self, n: usize) -> Vec<Value> {
        let i = self.inner.locked();
        let from = i.order.len().saturating_sub(n);
        i.order[from..].iter().filter_map(|k| i.recs.get(k).map(Rec::to_json)).collect()
    }

    /// Whether this machine lets `from` wake `agent` (`reach`): its wake
    /// setting, else your allowance for the session, here and for actors.
    /// `granted`: the message is a start's end, told to the session that
    /// asked for the start, whose ask is its consent to this wake.
    fn may_wake_for(h: &mut dyn Host, from: &str, agent: &str, granted: bool) -> Decision {
        if granted {
            return Decision::Allow;
        }
        Self::may_wake(h, from, agent)
    }

    fn may_wake(h: &mut dyn Host, from: &str, agent: &str) -> Decision {
        let consent = Consent { here: h.consented(agent, false), remote: h.consented(agent, true) };
        let wake = h.wake_setting();
        reach::decide(Action::Wake, &reach::Sender::from_id(from), agent, &Settings { consent, wake, ..Settings::default() })
    }

    fn wake_network(h: &mut dyn Host) -> bool {
        h.wake_setting() == Wake::Network
    }

    /// Whether a message to a Claude Code session that a process holds (its
    /// app, its terminal, or a run XenoNet resumed) and that isn't at work
    /// goes to a copy of it, forked with its whole context: under wake
    /// network only, since the session itself can't take it now.
    pub fn forkable(&self, h: &mut dyn Host, view: Option<&Value>) -> bool {
        Self::forkable_for(h, view, false)
    }

    /// `forkable`, and also without wake network when `granted` (a start's
    /// end for the session that asked for the start).
    fn forkable_for(h: &mut dyn Host, view: Option<&Value>, granted: bool) -> bool {
        let Some(view) = view.filter(|v| truthy(Some(v))) else { return false };
        if truthy(view.get("ended")) || view.get("status").and_then(Value::as_str) == Some("working") {
            return false;
        }
        let id = text_of(view.get("id"));
        if !id.starts_with("claude:") || !h.resumes() || super::resume::resume_command(&id, None).is_none() {
            return false;
        }
        granted || Self::wake_network(h)
    }

    /// Whether a session can be reached without hooks (an OpenCode run
    /// XenoNet has a way into).
    pub fn reaches(&self, h: &mut dyn Host, id: &str) -> bool {
        super::apps::served(id) && h.consented(id, false)
    }

    /// The OpenCode sessions you allowed XenoNet to wake.
    pub fn known(&self, h: &mut dyn Host) -> Vec<String> {
        h.consent_sessions().into_iter().filter(|id| super::apps::served(id) && h.consented(id, false)).collect()
    }

    /// Whether a session that ended can still be messaged: you allowed it
    /// (or wake is network), and a headless resume would start it for the
    /// message.
    pub fn resumable(&self, h: &mut dyn Host, view: &Value) -> bool {
        let id = text_of(view.get("id"));
        if !h.resumes() || !truthy(view.get("ended")) || super::resume::resume_command(&id, None).is_none() {
            return false;
        }
        Self::wake_network(h) || h.consented(&id, false)
    }

    /// A wake for a session's listening Stop hook to hand a message over:
    /// the same budget as any other wake, spent when allowed.
    pub fn wake(&self, h: &mut dyn Host, from: &str, agent: &str) -> WakeAllowed {
        let app = agent.split(':').next().unwrap_or("").to_string();
        let usage = h.usage(&app);
        let w = h.wake_allowed(from, agent, usage);
        if w.ok() {
            h.spend_wake(from, agent);
        }
        w
    }

    /// How a message from `from` would reach `a` if sent now: None when it
    /// waits for the hooks.
    pub fn way(&self, h: &mut dyn Host, from: &str, a: &Value) -> Option<&'static str> {
        let status = a.get("status").and_then(Value::as_str);
        let idle = matches!(status, Some("idle" | "ended"));
        let id = text_of(a.get("id"));
        if idle {
            let usage = h.usage(&text_of(a.get("app")));
            if !h.wake_allowed(from, &id, usage).ok() {
                return None;
            }
        }
        if h.listening(&id) {
            return Some("right away: it's listening for messages");
        }
        let ways = a.get("app").and_then(Value::as_str).and_then(super::apps::of);
        if ways.is_some_and(|w| w.channel) && status == Some("idle") && h.channel_live(&id) {
            return Some("right away, through its channel");
        }
        if status == Some("ended") && self.resumable(h, a) && Self::may_wake(h, from, &id).allowed() {
            return Some("right away, in a turn XenoNet resumes it for");
        }
        if self.forkable(h, Some(a)) && Self::may_wake(h, from, &id).allowed() {
            return Some("right away, in a copy XenoNet forks from it, since it's open elsewhere");
        }
        if ways.is_some_and(|w| w.server) && Self::may_wake(h, from, &id).allowed() {
            return Some("right away, through its OpenCode server");
        }
        None
    }

    /// What its own OpenCode says of an OpenCode session.
    pub fn describe(&self, h: &mut dyn Host, id: &str) -> Option<Value> {
        if !super::apps::served(id) {
            return None;
        }
        h.opencode_info(id)
    }

    /// A message routed: a record for each session it's for, its way in
    /// tried, and its ack time set. The records' keys. `m` is the message
    /// as the engine tells it: { id, from, to[], text, verified?, quiet?,
    /// startEnd?, fromName? }.
    pub fn route(&self, h: &mut dyn Host, m: &Value) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let to: Vec<String> = m.get("to").and_then(Value::as_array).map(|l| l.iter().map(template).collect()).unwrap_or_default();
        for agent in to {
            if agent.starts_with("actor:") || agent == "you" {
                continue;
            }
            let at = h.now();
            let message = m.get("id").cloned().unwrap_or(Value::Null);
            let key = format!("{}|{agent}", text_of(m.get("id")));
            let rec = Rec {
                message,
                agent: agent.clone(),
                from: text_of(m.get("from")),
                state: "queued",
                how: None,
                at,
                acked_at: None,
                why: None,
                from_name: None,
                location: None,
                attempting: true,
                timer: false,
            };
            let trimmed = {
                let mut i = self.inner.locked();
                if !i.order.contains(&key) {
                    i.order.push(key.clone());
                }
                i.recs.insert(key.clone(), rec);
                let mut gone = Vec::new();
                while i.order.len() > KEEP {
                    gone.push(i.order.remove(0));
                }
                gone
            };
            for k in trimmed {
                self.settle(&k);
            }
            let tried = self.attempt(h, &key, m);
            // A quiet note isn't one you're told about if no turn takes it.
            if tried.is_ok() && !truthy(m.get("quiet")) {
                self.arm(h, &key);
            }
            self.with(&key, |r| r.attempting = false);
            self.settle(&key);
            tried?;
            out.push(key);
        }
        Ok(out)
    }

    fn set_state(&self, h: &mut dyn Host, key: &str, state: &'static str, how: Option<&'static str>, why: Option<String>) {
        self.with(key, |r| {
            r.state = state;
            r.how = how;
            r.why = why;
        });
        self.emit(h, key);
    }

    fn attempt(&self, h: &mut dyn Host, key: &str, m: &Value) -> Result<bool> {
        // XenoNet's own word to whoever asked it something (a start's
        // answer): it waits for the next turn, and never wakes a session.
        if truthy(m.get("quiet")) {
            self.set_state(h, key, "next-turn", Some("hooks"), Some("a note from XenoNet: it waits for the next turn".into()));
            return Ok(false);
        }
        let agent = self.with(key, |r| r.agent.clone()).unwrap_or_default();
        let view = h.view(&agent).filter(|v| truthy(Some(v)));
        let app = agent.split(':').next().unwrap_or("").to_string();
        let status = view.as_ref().and_then(|v| v.get("status")).and_then(Value::as_str);
        let idle = view.is_none() || matches!(status, Some("idle" | "ended"));
        let from = text_of(m.get("from"));
        // A start's end, told to the session that asked for the start.
        let granted = truthy(m.get("startEnd"));
        let from_name = match m.get("fromName").filter(|n| !n.is_null()) {
            Some(n) => template(n),
            None => {
                let title = if is_you(&from) { Some("your user".to_string()) } else { h.view(&from).and_then(|v| v.get("title").filter(|t| !t.is_null()).map(template)) };
                title.unwrap_or_else(|| from.clone())
            }
        };
        let verified = m.get("verified").filter(|v| truthy(Some(v))).map(template);
        let text = framed(&text_of(m.get("id")), &from_name, verified.as_deref(), &text_of(m.get("text")), is_you(&from));
        self.with(key, |r| r.from_name = Some(from_name.clone()));
        let mut tried: Vec<String> = Vec::new();
        let wake = if idle {
            let usage = h.usage(&app);
            h.wake_allowed(&from, &agent, usage)
        } else {
            WakeAllowed::Yes
        };
        let hash = hash_text(&text_of(m.get("text")));
        let message = m.get("id").cloned().unwrap_or(Value::Null);
        let success = |me: &Self, h: &mut dyn Host, how: &'static str, acked_at: Option<f64>, location: Option<String>| -> Result<bool> {
            me.with(key, |r| {
                r.state = "live";
                r.how = Some(how);
                if acked_at.is_some() {
                    r.acked_at = acked_at;
                }
                if location.is_some() {
                    r.location = location;
                }
            });
            if idle {
                h.spend_wake(&from, &agent);
            }
            let t = h.now();
            h.mark_seen(&agent, std::slice::from_ref(&message), t)?;
            h.audit(json!({ "kind": "delivered", "message": message, "from": from, "to": agent, "how": how, "text": hash }))?;
            me.emit(h, key);
            Ok(true)
        };
        // 0. Its Stop hook, listening after a turn: the hook hands the
        // message over itself, and checks the wake then.
        if h.listening(&agent) {
            self.set_state(h, key, "live", Some("listen"), None);
            return Ok(true);
        }
        match wake {
            WakeAllowed::No(why) => tried.push(format!("no wake: {why}")),
            WakeAllowed::Yes => {
                let ways = super::apps::of(&app);
                let ended = view.as_ref().is_some_and(|v| truthy(v.get("ended")));
                let cwd = view.as_ref().and_then(|v| s(v.get("cwd")));
                // 1. A Claude Code channel, only to wake an idle session.
                if ways.is_some_and(|w| w.channel) && idle && h.channel_live(&agent) {
                    let item = json!({ "id": message, "from": from_name, "text": text });
                    if h.channel_push(&agent, item) {
                        return success(self, h, "channel", None, None);
                    }
                    tried.push("channel closed".into());
                }
                // 2. OpenCode's own server, with your consent for that session.
                if ways.is_some_and(|w| w.server) {
                    if let Decision::Refuse(why) = Self::may_wake_for(h, &from, &agent, granted) {
                        tried.push(format!("opencode: {why}"));
                    } else {
                        match h.opencode_deliver(&agent, &text) {
                            Ok(location) => {
                                let t = h.now();
                                return success(self, h, "opencode", Some(t), Some(location));
                            }
                            Err(why) => tried.push(format!("opencode: {why}")),
                        }
                    }
                }
                // 3. Typing into its tmux pane: consent, a sender XenoNet
                // verified with a token or you, and an idle session that isn't
                // waiting on its user.
                if idle && !ended && !view.as_ref().is_some_and(|v| truthy(v.get("needsYou"))) {
                    // Only you, or an actor this machine checked by its token.
                    let who = reach::Sender::from_id(&from);
                    let sender = who.kind == Kind::You || (who.kind == Kind::Actor && who.host.is_none());
                    let pid = h.contact_pid(&agent).filter(|p| *p != 0.0 && !p.is_nan());
                    if let Decision::Refuse(why) = Self::may_wake_for(h, &from, &agent, granted) {
                        tried.push(format!("tmux: {why}"));
                    } else if !sender {
                        tried.push("tmux: only you or an actor with a token may type".into());
                    } else if let Some(pid) = pid {
                        let r = h.tmux_deliver(pid, &text);
                        h.audit(json!({
                            "kind": "typed",
                            "message": message,
                            "from": from,
                            "to": agent,
                            "pane": r.as_ref().ok(),
                            "ok": r.is_ok(),
                            "why": r.as_ref().err(),
                            "text": hash,
                        }))?;
                        match r {
                            Ok(location) => return success(self, h, "tmux", None, Some(location)),
                            Err(why) => tried.push(format!("tmux: {why}")),
                        }
                    } else {
                        tried.push("tmux: no process known".into());
                    }
                }
                // 4. A headless resume, for a Claude Code or Codex session whose
                // process has gone, with your consent: never while it runs.
                if h.resumes() && ended && super::resume::resume_command(&agent, None).is_some() {
                    if let Decision::Refuse(why) = Self::may_wake_for(h, &from, &agent, granted) {
                        tried.push(format!("resume: {why}"));
                    } else {
                        let r = h.resume_deliver(&agent, cwd.as_deref(), &text, false);
                        h.audit(json!({ "kind": "resumed", "message": message, "from": from, "to": agent, "ok": r.is_ok(), "why": r.as_ref().err(), "text": hash }))?;
                        match r {
                            Ok(location) => return success(self, h, "resume", None, Some(location)),
                            Err(why) => tried.push(format!("resume: {why}")),
                        }
                    }
                } else if Self::forkable_for(h, view.as_ref(), granted) {
                    // 5. A copy of a Claude Code session a process holds, under
                    // wake network: it answers with the whole context, now.
                    if let Decision::Refuse(why) = Self::may_wake_for(h, &from, &agent, granted) {
                        tried.push(format!("fork: {why}"));
                    } else {
                        let r = h.resume_deliver(&agent, cwd.as_deref(), &text, true);
                        h.audit(json!({ "kind": "forked", "message": message, "from": from, "to": agent, "ok": r.is_ok(), "why": r.as_ref().err(), "text": hash }))?;
                        match r {
                            Ok(location) => return success(self, h, "fork", None, Some(location)),
                            Err(why) => tried.push(format!("fork: {why}")),
                        }
                    }
                }
            }
        }
        let why = if tried.is_empty() { None } else { Some(tried.join("; ")) };
        self.set_state(h, key, "next-turn", Some("hooks"), why);
        Ok(false)
    }

    /// The ack time set, unless it's acked already.
    fn arm(&self, h: &mut dyn Host, key: &str) {
        if self.acked(key) {
            return;
        }
        self.with(key, |r| r.timer = true);
        h.arm(key, ACK_MS);
    }

    /// Its ack time came: a push nothing acked goes back to the hooks, and
    /// you're told.
    pub fn expire(&self, h: &mut dyn Host, key: &str) -> Result<()> {
        self.with(key, |r| r.timer = false);
        let r = self.expire_inner(h, key);
        self.settle(key);
        r
    }

    fn expire_inner(&self, h: &mut dyn Host, key: &str) -> Result<()> {
        let Some((acked, how, agent, message, from, from_name)) =
            self.with(key, |r| (r.acked_at.is_some(), r.how, r.agent.clone(), r.message.clone(), r.from.clone(), r.from_name.clone()))
        else {
            return Ok(());
        };
        if acked {
            return Ok(());
        }
        // Claude Code drops a push on a channel it didn't register without a
        // word, a typed line can be lost, and a resumed turn can fail to
        // start: with no hook since, the message waits for the hooks again.
        if matches!(how, Some("channel" | "tmux" | "resume")) {
            h.mark_unseen(&agent, std::slice::from_ref(&message))?;
        }
        self.with(key, |r| r.state = "notified");
        h.audit(json!({ "kind": "notified", "message": message, "to": agent }))?;
        self.emit(h, key);
        let title = h.view(&agent).and_then(|v| v.get("title").filter(|t| !t.is_null()).map(template)).unwrap_or_else(|| agent.clone());
        h.notify(json!({ "message": message, "agent": agent, "title": title, "from": from_name.unwrap_or(from) }));
        Ok(())
    }

    fn ack(&self, h: &mut dyn Host, key: &str) {
        if self.acked(key) {
            return;
        }
        let t = h.now();
        self.with(key, |r| r.acked_at = Some(t));
        h.clear(key);
        self.with(key, |r| r.timer = false);
        self.emit(h, key);
    }

    /// Messages the engine says a session has seen: their records acked.
    pub fn seen(&self, h: &mut dyn Host, agent: &str, ids: &[Value]) {
        for id in ids {
            let key = format!("{}|{agent}", template(id));
            let listed = self.inner.locked().order.contains(&key);
            if listed {
                self.ack(h, &key);
            }
        }
    }

    /// A hook from the session after a push acks it.
    pub fn contact(&self, h: &mut dyn Host, agent: &str) {
        let keys: Vec<String> = self.inner.locked().order.clone();
        for key in keys {
            let due = self.with(&key, |r| r.agent == agent && r.state == "live" && r.how != Some("listen") && r.acked_at.is_none()).unwrap_or(false);
            if due {
                self.ack(h, &key);
            }
        }
    }
}
