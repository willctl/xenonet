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
//! The records are Rust's; the twin keeps a JS object for each, which it's
//! told of ("listed", "unlisted", "sync", "release"), and the events and
//! timers ("emit", "arm", "clear"). "router" is the twin itself, whose
//! engine, policy, channels, opencode, tmux, resume, notify, usage and now
//! are called as the JS called them.

use super::policy::{framed, hash_text, is_you};
use super::reach::{self, Action, Consent, Decision, Kind, Settings, Wake};
use crate::peer::{list, opt_call, Calls, Result};
use crate::sql::Arg;
use crate::store::js_text;
use std::collections::HashMap;
use std::sync::Mutex;
use xenonet_base::sync::Locked;

pub const ACK_MS: f64 = 60e3;
const KEEP: usize = 500;

struct Rec {
    /// The record's fields, as the JS object has them.
    fields: Vec<(String, Arg)>,
    /// The message's id and sender, for its timer.
    message: Arg,
    from: Arg,
    /// An attempt under way, and a timer set.
    attempting: bool,
    timer: bool,
}

impl Rec {
    fn get(&self, k: &str) -> Arg {
        self.fields.iter().find(|(n, _)| n == k).map_or(Arg::Undef, |(_, v)| v.clone())
    }
    fn set(&mut self, k: &str, v: Arg) {
        match self.fields.iter_mut().find(|(n, _)| n == k) {
            Some(e) => e.1 = v,
            None => self.fields.push((k.to_string(), v)),
        }
    }
    fn arg(&self) -> Arg {
        Arg::Obj(self.fields.clone())
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

fn starts(a: &Arg, prefix: &str) -> bool {
    js_text(a).starts_with(prefix)
}

fn obj(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// `x.catch((err) => ({ ok: false, why: err.message }))`.
fn caught(r: Result<Arg>) -> Arg {
    r.unwrap_or_else(|t| obj(vec![("ok", Arg::Bool(false)), ("why", Arg::text(t.message))]))
}

fn truthy_at(c: &mut dyn Calls, path: &str) -> Result<bool> {
    Ok(match c.kind(path)?.as_str() {
        "undefined" | "null" => false,
        "function" | "object" | "symbol" => true,
        _ => c.get(path)?.truthy(),
    })
}

impl Router {
    fn now(c: &mut dyn Calls) -> Result<f64> {
        let t = c.call("router", "now", vec![])?;
        Ok(match t {
            Arg::Num(n) => n,
            other => xenonet_base::text::js_number(Some(&other.to_json())),
        })
    }

    fn with<T>(&self, key: &str, f: impl FnOnce(&mut Rec) -> T) -> Option<T> {
        self.inner.locked().recs.get_mut(key).map(f)
    }

    fn field(&self, key: &str, k: &str) -> Arg {
        self.with(key, |r| r.get(k)).unwrap_or(Arg::Undef)
    }

    /// Fields set on a record, and the twin's object told.
    fn assign(&self, c: &mut dyn Calls, key: &str, fields: Vec<(&str, Arg)>) -> Result<()> {
        let rec = self.with(key, |r| {
            for (k, v) in fields {
                r.set(k, v);
            }
            r.arg()
        });
        if let Some(rec) = rec {
            c.call("", "sync", vec![Arg::text(key), rec])?;
        }
        Ok(())
    }

    fn emit(c: &mut dyn Calls, key: &str) -> Result<()> {
        c.call("", "emit", vec![Arg::text(key)])?;
        Ok(())
    }

    /// A record let go once it's no longer listed and nothing waits on it.
    fn settle(&self, c: &mut dyn Calls, key: &str) -> Result<()> {
        let gone = {
            let mut i = self.inner.locked();
            let listed = i.order.iter().any(|k| k == key);
            let free = i.recs.get(key).is_some_and(|r| !r.attempting && !r.timer && !listed);
            if free {
                i.recs.remove(key);
            }
            free
        };
        if gone {
            c.call("", "release", vec![Arg::text(key)])?;
        }
        Ok(())
    }

    fn usage(c: &mut dyn Calls, app: &str) -> Result<Arg> {
        c.call("router", "usage", vec![Arg::text(app)])
    }

    fn consented(c: &mut dyn Calls, id: &Arg, remote: bool) -> Result<bool> {
        Ok(c.call("router.policy", "consented", vec![id.clone(), obj(vec![("remote", Arg::Bool(remote))])])?.truthy())
    }

    /// This machine's wake setting (`router.reach()`'s wake): network lets
    /// any sender wake a session within the budgets.
    fn wake_network(c: &mut dyn Calls) -> Result<bool> {
        Ok(opt_call(c, "router", "reach", vec![])?.field("wake").as_str() == Some("network"))
    }

    /// Whether this machine lets `from` wake `agent` (`reach`): its wake
    /// setting, else your allowance for the session, here and for actors.
    fn may_wake(c: &mut dyn Calls, from: &Arg, agent: &Arg) -> Result<Decision> {
        let consent = Consent { here: Self::consented(c, agent, false)?, remote: Self::consented(c, agent, true)? };
        let wake = if Self::wake_network(c)? { Wake::Network } else { Wake::Consent };
        Ok(reach::decide(Action::Wake, &reach::sender_of(from), &js_text(agent), &Settings { consent, wake, ..Settings::default() }))
    }

    /// Whether a message to a Claude Code session that a process holds (its
    /// app, its terminal, or a run XenoNet resumed) and that isn't at work
    /// goes to a copy of it, forked with its whole context: under wake
    /// network only, since the session itself can't take it now.
    pub fn forkable(&self, c: &mut dyn Calls, view: &Arg) -> Result<bool> {
        if !view.truthy() || view.field("ended").truthy() || view.field("status").as_str() == Some("working") {
            return Ok(false);
        }
        let id = view.field("id");
        if !js_text(&id).starts_with("claude:") || !truthy_at(c, "router.resume.fork")? || !c.call("router.resume", "can", vec![id])?.truthy() {
            return Ok(false);
        }
        Self::wake_network(c)
    }

    /// Whether a session can be reached without hooks (an OpenCode run
    /// XenoNet has a way into).
    pub fn reaches(&self, c: &mut dyn Calls, id: &Arg) -> Result<Arg> {
        if !super::apps::served(&js_text(id)) || !truthy_at(c, "router.opencode")? {
            return Ok(Arg::Bool(false));
        }
        c.call("router.policy", "consented", vec![id.clone()])
    }

    /// The OpenCode sessions you allowed XenoNet to wake.
    pub fn known(&self, c: &mut dyn Calls) -> Result<Vec<Arg>> {
        if !truthy_at(c, "router.opencode")? {
            return Ok(Vec::new());
        }
        let consent = match c.kind("router.policy.data")?.as_str() {
            "undefined" | "null" => Arg::Undef,
            _ => c.get("router.policy.data.consent")?,
        };
        let ids: Vec<String> = match consent {
            Arg::Obj(f) => f.into_iter().map(|(k, _)| k).collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for id in ids {
            if super::apps::served(&id) && c.call("router.policy", "consented", vec![Arg::text(id.clone())])?.truthy() {
                out.push(Arg::text(id));
            }
        }
        Ok(out)
    }

    /// Whether a session that ended can still be messaged: you allowed it
    /// (or wake is network), and a headless resume would start it for the
    /// message.
    pub fn resumable(&self, c: &mut dyn Calls, view: &Arg) -> Result<Arg> {
        if !truthy_at(c, "router.resume")? || !view.field("ended").truthy() || !c.call("router.resume", "can", vec![view.field("id")])?.truthy() {
            return Ok(Arg::Bool(false));
        }
        if Self::wake_network(c)? {
            return Ok(Arg::Bool(true));
        }
        c.call("router.policy", "consented", vec![view.field("id")])
    }

    /// A wake for a session's listening Stop hook to hand a message over:
    /// the same budget as any other wake, spent when allowed.
    pub fn wake(&self, c: &mut dyn Calls, from: &Arg, agent: &Arg) -> Result<Arg> {
        let app = js_text(agent).split(':').next().unwrap_or("").to_string();
        let usage = Self::usage(c, &app)?;
        let w = c.call("router.policy", "wakeAllowed", vec![from.clone(), agent.clone(), obj(vec![("usage", usage)])])?;
        if w.field("ok").truthy() {
            c.call("router.policy", "spendWake", vec![from.clone(), agent.clone()])?;
        }
        Ok(w)
    }

    /// How a message from `from` would reach `a` if sent now: None when it
    /// waits for the hooks.
    pub fn way(&self, c: &mut dyn Calls, from: &Arg, a: &Arg) -> Result<Option<&'static str>> {
        let status = a.field("status");
        let idle = matches!(status.as_str(), Some("idle" | "ended"));
        let id = a.field("id");
        if idle {
            let usage = c.call("router", "usage", vec![a.field("app")])?;
            if !c.call("router.policy", "wakeAllowed", vec![from.clone(), id.clone(), obj(vec![("usage", usage)])])?.field("ok").truthy() {
                return Ok(None);
            }
        }
        if opt_call(c, "router.engine.listening", "has", vec![id.clone()])?.truthy() {
            return Ok(Some("right away: it's listening for messages"));
        }
        let app = a.field("app");
        let ways = app.as_str().and_then(super::apps::of);
        if ways.is_some_and(|w| w.channel) && status.as_str() == Some("idle") && opt_call(c, "router.channels", "live", vec![id.clone()])?.truthy() {
            return Ok(Some("right away, through its channel"));
        }
        if status.as_str() == Some("ended") && self.resumable(c, a)?.truthy() && Self::may_wake(c, from, &id)?.allowed() {
            return Ok(Some("right away, in a turn XenoNet resumes it for"));
        }
        if self.forkable(c, a)? && Self::may_wake(c, from, &id)?.allowed() {
            return Ok(Some("right away, in a copy XenoNet forks from it, since it's open elsewhere"));
        }
        if ways.is_some_and(|w| w.server) && truthy_at(c, "router.opencode")? && Self::may_wake(c, from, &id)?.allowed() {
            return Ok(Some("right away, through its OpenCode server"));
        }
        Ok(None)
    }

    /// What its own OpenCode says of an OpenCode session.
    pub fn describe(&self, c: &mut dyn Calls, id: &Arg) -> Result<Arg> {
        if !super::apps::served(&js_text(id)) || !truthy_at(c, "router.opencode.info")? {
            return Ok(Arg::Null);
        }
        Ok(c.call("router.opencode", "info", vec![id.clone()]).unwrap_or(Arg::Null))
    }

    /// A message routed: a record for each session it's for, its way in
    /// tried, and its ack time set. The records' keys.
    pub fn route(&self, c: &mut dyn Calls, m: &Arg) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for agent in list(m.field("to")) {
            if starts(&agent, "actor:") || agent.as_str() == Some("you") {
                continue;
            }
            let at = Self::now(c)?;
            let key = format!("{}|{}", js_text(&m.field("id")), js_text(&agent));
            let rec = Rec {
                fields: vec![
                    ("message".into(), m.field("id")),
                    ("agent".into(), agent.clone()),
                    ("from".into(), m.field("from")),
                    ("state".into(), Arg::text("queued")),
                    ("how".into(), Arg::Null),
                    ("at".into(), Arg::num(at)),
                    ("ackedAt".into(), Arg::Null),
                    ("why".into(), Arg::Null),
                ],
                message: m.field("id"),
                from: m.field("from"),
                attempting: true,
                timer: false,
            };
            let first = rec.arg();
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
            c.call("", "listed", vec![Arg::text(key.clone()), first])?;
            for k in trimmed {
                c.call("", "unlisted", vec![Arg::text(k.clone())])?;
                self.settle(c, &k)?;
            }
            let tried = self.attempt(c, &key, m);
            // A quiet note isn't one you're told about if no turn takes it.
            if tried.is_ok() && !m.field("quiet").truthy() {
                self.arm(c, &key)?;
            }
            self.with(&key, |r| r.attempting = false);
            self.settle(c, &key)?;
            tried?;
            out.push(key);
        }
        Ok(out)
    }

    fn attempt(&self, c: &mut dyn Calls, key: &str, m: &Arg) -> Result<bool> {
        // XenoNet's own word to whoever asked it something (a start's
        // answer): it waits for the next turn, and never wakes a session.
        if m.field("quiet").truthy() {
            self.assign(c, key, vec![("state", Arg::text("next-turn")), ("how", Arg::text("hooks")), ("why", Arg::text("a note from XenoNet: it waits for the next turn"))])?;
            Self::emit(c, key)?;
            return Ok(false);
        }
        let agent = self.field(key, "agent");
        let view = c.call("router.engine", "view", vec![agent.clone()]).unwrap_or(Arg::Null);
        let app = js_text(&agent).split(':').next().unwrap_or("").to_string();
        let status = view.field("status");
        let idle = !view.truthy() || matches!(status.as_str(), Some("idle" | "ended"));
        let from = m.field("from");
        let from_name = match m.field("fromName") {
            n if !n.is_nullish() => n,
            _ => {
                let title = if is_you(&from) { Arg::text("your user") } else { c.call("router.engine", "view", vec![from.clone()]).unwrap_or(Arg::Null).field("title") };
                title.or(from.clone())
            }
        };
        let text = Arg::text(framed(&m.field("id"), &from_name, &m.field("verified"), &m.field("text"), is_you(&from)));
        self.assign(c, key, vec![("fromName", from_name.clone())])?;
        let mut tried: Vec<String> = Vec::new();
        let wake = if idle {
            let usage = Self::usage(c, &app)?;
            c.call("router.policy", "wakeAllowed", vec![from.clone(), agent.clone(), obj(vec![("usage", usage)])])?
        } else {
            obj(vec![("ok", Arg::Bool(true))])
        };
        let hash = Arg::text(hash_text(&js_text(&m.field("text"))));
        let success = |me: &Self, c: &mut dyn Calls, how: &str, extra: Vec<(&str, Arg)>| -> Result<bool> {
            let mut fields = vec![("state", Arg::text("live")), ("how", Arg::text(how))];
            fields.extend(extra);
            me.assign(c, key, fields)?;
            if idle {
                c.call("router.policy", "spendWake", vec![from.clone(), agent.clone()])?;
            }
            let t = Self::now(c)?;
            c.call("router.engine.store", "markSeen", vec![agent.clone(), Arg::List(vec![m.field("id")]), Arg::num(t)])?;
            c.call(
                "router.policy",
                "audit",
                vec![obj(vec![
                    ("kind", Arg::text("delivered")),
                    ("message", m.field("id")),
                    ("from", from.clone()),
                    ("to", agent.clone()),
                    ("how", Arg::text(how)),
                    ("text", hash.clone()),
                ])],
            )?;
            Self::emit(c, key)?;
            Ok(true)
        };
        // 0. Its Stop hook, listening after a turn: the hook hands the
        // message over itself, and checks the wake then.
        if opt_call(c, "router.engine.listening", "has", vec![agent.clone()])?.truthy() {
            self.assign(c, key, vec![("state", Arg::text("live")), ("how", Arg::text("listen"))])?;
            Self::emit(c, key)?;
            return Ok(true);
        }
        if !wake.field("ok").truthy() {
            tried.push(format!("no wake: {}", js_text(&wake.field("why"))));
        } else {
            // 1. A Claude Code channel, only to wake an idle session.
            let ways = super::apps::of(&app);
            if ways.is_some_and(|w| w.channel) && idle && opt_call(c, "router.channels", "live", vec![agent.clone()])?.truthy() {
                let item = obj(vec![("id", m.field("id")), ("from", from_name.clone()), ("text", text.clone())]);
                if c.call("router.channels", "push", vec![agent.clone(), item])?.truthy() {
                    return success(self, c, "channel", vec![]);
                }
                tried.push("channel closed".into());
            }
            // 2. OpenCode's own server, with your consent for that session.
            if ways.is_some_and(|w| w.server) && truthy_at(c, "router.opencode")? {
                if let Decision::Refuse(why) = Self::may_wake(c, &from, &agent)? {
                    tried.push(format!("opencode: {why}"));
                } else {
                    let r = caught(c.call("router.opencode", "deliver", vec![agent.clone(), text.clone()]));
                    if r.field("ok").truthy() {
                        let t = Self::now(c)?;
                        return success(self, c, "opencode", vec![("ackedAt", Arg::num(t)), ("where", r.field("where"))]);
                    }
                    tried.push(format!("opencode: {}", js_text(&r.field("why"))));
                }
            }
            // 3. Typing into its tmux pane: consent, a sender XenoNet
            // verified with a token or you, and an idle session that isn't
            // waiting on its user.
            if truthy_at(c, "router.tmux")? && idle && !view.field("ended").truthy() && !view.field("needsYou").truthy() {
                // Only you, or an actor this machine checked by its token.
                let who = reach::sender_of(&from);
                let sender = who.kind == Kind::You || (who.kind == Kind::Actor && who.host.is_none());
                let pid = opt_call(c, "router.engine.store", "contact", vec![agent.clone()])?.field("pid").or(Arg::Null);
                if let Decision::Refuse(why) = Self::may_wake(c, &from, &agent)? {
                    tried.push(format!("tmux: {why}"));
                } else if !sender {
                    tried.push("tmux: only you or an actor with a token may type".into());
                } else if !pid.truthy() {
                    tried.push("tmux: no process known".into());
                } else {
                    let r = caught(c.call("router.tmux", "deliver", vec![obj(vec![("pid", pid), ("text", text.clone())])]));
                    c.call(
                        "router.policy",
                        "audit",
                        vec![obj(vec![
                            ("kind", Arg::text("typed")),
                            ("message", m.field("id")),
                            ("from", from.clone()),
                            ("to", agent.clone()),
                            ("pane", r.field("where").or(Arg::Null)),
                            ("ok", r.field("ok")),
                            ("why", r.field("why").or(Arg::Null)),
                            ("text", hash.clone()),
                        ])],
                    )?;
                    if r.field("ok").truthy() {
                        return success(self, c, "tmux", vec![("where", r.field("where"))]);
                    }
                    tried.push(format!("tmux: {}", js_text(&r.field("why"))));
                }
            }
            // 4. A headless resume, for a Claude Code or Codex session whose
            // process has gone, with your consent: never while it runs.
            if truthy_at(c, "router.resume")? && view.field("ended").truthy() && c.call("router.resume", "can", vec![agent.clone()])?.truthy() {
                if let Decision::Refuse(why) = Self::may_wake(c, &from, &agent)? {
                    tried.push(format!("resume: {why}"));
                } else {
                    let r = caught(c.call("router.resume", "deliver", vec![obj(vec![("agent", agent.clone()), ("cwd", view.field("cwd")), ("text", text.clone())])]));
                    c.call(
                        "router.policy",
                        "audit",
                        vec![obj(vec![
                            ("kind", Arg::text("resumed")),
                            ("message", m.field("id")),
                            ("from", from.clone()),
                            ("to", agent.clone()),
                            ("ok", r.field("ok")),
                            ("why", r.field("why").or(Arg::Null)),
                            ("text", hash.clone()),
                        ])],
                    )?;
                    if r.field("ok").truthy() {
                        return success(self, c, "resume", vec![("where", r.field("where"))]);
                    }
                    tried.push(format!("resume: {}", js_text(&r.field("why"))));
                }
            } else if self.forkable(c, &view)? {
                // 5. A copy of a Claude Code session a process holds, under
                // wake network: it answers with the whole context, now.
                if let Decision::Refuse(why) = Self::may_wake(c, &from, &agent)? {
                    tried.push(format!("fork: {why}"));
                } else {
                    let r = caught(c.call(
                        "router.resume",
                        "deliver",
                        vec![obj(vec![("agent", agent.clone()), ("cwd", view.field("cwd")), ("text", text.clone()), ("fork", Arg::Bool(true))])],
                    ));
                    c.call(
                        "router.policy",
                        "audit",
                        vec![obj(vec![
                            ("kind", Arg::text("forked")),
                            ("message", m.field("id")),
                            ("from", from.clone()),
                            ("to", agent.clone()),
                            ("ok", r.field("ok")),
                            ("why", r.field("why").or(Arg::Null)),
                            ("text", hash.clone()),
                        ])],
                    )?;
                    if r.field("ok").truthy() {
                        return success(self, c, "fork", vec![("where", r.field("where"))]);
                    }
                    tried.push(format!("fork: {}", js_text(&r.field("why"))));
                }
            }
        }
        let why = if tried.is_empty() { Arg::Null } else { Arg::text(tried.join("; ")) };
        self.assign(c, key, vec![("state", Arg::text("next-turn")), ("how", Arg::text("hooks")), ("why", why)])?;
        Self::emit(c, key)?;
        Ok(false)
    }

    /// The ack time set, unless it's acked already.
    fn arm(&self, c: &mut dyn Calls, key: &str) -> Result<()> {
        if self.field(key, "ackedAt").truthy() {
            return Ok(());
        }
        let ms = c.get("router.ackMs")?;
        self.with(key, |r| r.timer = true);
        c.call("", "arm", vec![Arg::text(key), ms])?;
        Ok(())
    }

    /// Its ack time came: a push nothing acked goes back to the hooks, and
    /// you're told.
    pub fn expire(&self, c: &mut dyn Calls, key: &str) -> Result<()> {
        self.with(key, |r| r.timer = false);
        let r = self.expire_inner(c, key);
        self.settle(c, key)?;
        r
    }

    fn expire_inner(&self, c: &mut dyn Calls, key: &str) -> Result<()> {
        let Some((acked, how, agent, message, from, from_name)) =
            self.with(key, |r| (r.get("ackedAt").truthy(), r.get("how"), r.get("agent"), r.message.clone(), r.from.clone(), r.get("fromName")))
        else {
            return Ok(());
        };
        if acked {
            return Ok(());
        }
        // Claude Code drops a push on a channel it didn't register without a
        // word, a typed line can be lost, and a resumed turn can fail to
        // start: with no hook since, the message waits for the hooks again.
        if matches!(how.as_str(), Some("channel" | "tmux" | "resume")) {
            c.call("router.engine.store", "markUnseen", vec![agent.clone(), Arg::List(vec![message.clone()])])?;
        }
        self.assign(c, key, vec![("state", Arg::text("notified"))])?;
        c.call("router.policy", "audit", vec![obj(vec![("kind", Arg::text("notified")), ("message", message.clone()), ("to", agent.clone())])])?;
        Self::emit(c, key)?;
        let view = c.call("router.engine", "view", vec![agent.clone()]).unwrap_or(Arg::Null);
        let title = view.field("title").or(agent.clone());
        c.call("router", "notify", vec![obj(vec![("message", message), ("agent", agent), ("title", title), ("from", from_name.or(from))])])?;
        Ok(())
    }

    fn ack(&self, c: &mut dyn Calls, key: &str) -> Result<()> {
        if self.field(key, "ackedAt").truthy() {
            return Ok(());
        }
        let t = Self::now(c)?;
        self.assign(c, key, vec![("ackedAt", Arg::num(t))])?;
        c.call("", "clear", vec![Arg::text(key)])?;
        self.with(key, |r| r.timer = false);
        Self::emit(c, key)
    }

    /// Messages the engine says a session has seen: their records acked.
    pub fn seen(&self, c: &mut dyn Calls, agent: &Arg, ids: &[Arg]) -> Result<()> {
        for id in ids {
            let key = format!("{}|{}", js_text(id), js_text(agent));
            let listed = self.inner.locked().order.contains(&key);
            if listed {
                self.ack(c, &key)?;
            }
        }
        Ok(())
    }

    /// A hook from the session after a push acks it.
    pub fn contact(&self, c: &mut dyn Calls, agent: &Arg) -> Result<()> {
        let keys: Vec<String> = self.inner.locked().order.clone();
        for key in keys {
            let due = self
                .with(&key, |r| r.get("agent") == *agent && r.get("state").as_str() == Some("live") && r.get("how").as_str() != Some("listen") && !r.get("ackedAt").truthy())
                .unwrap_or(false);
            if due {
                self.ack(c, &key)?;
            }
        }
        Ok(())
    }
}
