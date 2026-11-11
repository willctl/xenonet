//! Permission prompts from the runs XenoNet starts (docs/guide/network.md,
//! "Permission prompts"): Claude Code asks the xenonet MCP server's
//! `permission` tool whenever its settings don't settle a tool call, and
//! XenoNet asks you.
//!
//! - Only a run XenoNet started or resumed asks (the runs table names the
//!   caller); any other caller is refused.
//! - The ask shows in Needs you on this machine and on every window host:
//!   it crosses as a message frame to `xenonet:ask`, one to each window
//!   host, never in presence. Done, it's taken back the same way.
//! - An answer is yours: your device's signature over it, and a presence
//!   block for it (Touch ID, or polkit or a passphrase where that's what
//!   the device has), here and from another machine, which sends it as
//!   `network/operator/v2`. This machine decides it (xenonet-authz) on its
//!   own copy of your identity, and takes each presence block once. The
//!   one exception is a run started under your standing grant (start:
//!   delegate): what the grant's limits cover (the delegation judge), the
//!   machine answers itself.
//! - The call is held until you answer, the hold at most (ten minutes
//!   unless XENONET_PERMISSION_HOLD_MS says otherwise); no answer is a
//!   deny that says nobody answered.
//!
//! The answer is what Claude Code takes from a permission prompt tool:
//! `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`.

use crate::network::{clip, ms, net, send};
use crate::runtime::Runtime;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use xenonet_api::From;
use xenonet_model::ops::{Kind as FrameKind, To};
use xenonet_os::sync::Locked;
use xenonet_text::time::now_ms;

/// The reserved target an ask crosses to window hosts as.
pub const TARGET: &str = "xenonet:ask";
/// How long an ask is held, unless XENONET_PERMISSION_HOLD_MS says.
pub const HOLD_MS: f64 = 10.0 * 60e3;
/// Asks from other machines kept at most.
const THERE_MAX: usize = 50;
use xenonet_engine::delivery::delegation::{self, Verdict};
pub use xenonet_engine::delivery::starts::ENV_WARNING;

/// One ask, as Needs you shows it.
#[derive(Clone, Debug)]
pub struct Ask {
    pub id: String,
    /// The machine it's on: None for this one.
    pub host: Option<String>,
    pub session: String,
    pub run: String,
    pub title: String,
    pub tool: String,
    pub input: String,
    /// Whether it may read the run's environment (`reads_env`).
    pub warn: bool,
    pub at: f64,
    pub until: f64,
}

impl Ask {
    fn to_json(&self, here: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "host": self.host.as_deref().or(here),
            "here": self.host.is_none(),
            "session": self.session,
            "run": self.run,
            "title": self.title,
            "tool": self.tool,
            "input": self.input,
            "warning": if self.warn { json!(ENV_WARNING) } else { Value::Null },
            "at": ms(self.at),
            "until": ms(self.until),
        })
    }

    fn of(v: &Value, host: &str) -> Option<Ask> {
        let s = |k: &str, max: usize| v.get(k).and_then(Value::as_str).map(|t| clip(&xenonet_engine::delivery::policy::sanitize(t, true, max as f64), max));
        let input = s("input", 300).unwrap_or_default();
        // The other machine's word that it may read the environment, or
        // this one's from the call as it shows: the card's words are ours.
        let warn = v.get("warning").is_some_and(|w| w.as_str().is_some_and(|t| !t.is_empty())) || reads_env(&json!({ "command": input }));
        Some(Ask {
            id: s("id", 64).filter(|i| i.starts_with("ask_"))?,
            host: Some(host.to_string()),
            session: s("session", 200).unwrap_or_default(),
            run: s("run", 64).unwrap_or_default(),
            title: s("title", 200).unwrap_or_default(),
            tool: s("tool", 80).unwrap_or_default(),
            input,
            warn,
            at: v.get("at").and_then(Value::as_f64).unwrap_or_else(now_ms),
            until: v.get("until").and_then(Value::as_f64).unwrap_or_else(|| now_ms() + HOLD_MS),
        })
    }
}

/// An ask held here, and its answer once there is one: allow or deny, and
/// who gave it.
struct Held {
    ask: Ask,
    answer: Option<(String, String)>,
}

#[derive(Default)]
pub struct AskSide {
    here: Mutex<HashMap<String, Held>>,
    answered: Condvar,
    /// Asks from other machines, for this console.
    there: Mutex<Vec<Ask>>,
}

/// What a tool call asks to do, in a line: a command's text, a file's path,
/// else its input as JSON; made safe to show, the home folder as ~.
pub fn input_words(input: &Value, home: &str) -> String {
    let pick = ["command", "file_path", "path", "url", "pattern", "query"].iter().find_map(|k| input.get(*k).and_then(Value::as_str).map(str::to_string));
    let text = pick.unwrap_or_else(|| input.to_string());
    let text = xenonet_os::paths::tilde(&text, home);
    clip(&xenonet_engine::delivery::policy::sanitize(&text, true, 300.0), 300)
}

/// Whether a tool call may read the run's environment, or names the
/// token: a command that prints it (env, printenv or set on their own),
/// PowerShell's `env:` drive, `/proc/<pid>/environ`, an environment API, or
/// OAUTH_TOKEN anywhere. Only what runs or is read counts (a command, a
/// path, a pattern, a query, a URL), not a description or a file's new
/// text. It warns; it doesn't decide.
pub fn reads_env(input: &Value) -> bool {
    const FIELDS: [&str; 7] = ["command", "file_path", "path", "pattern", "query", "url", "script"];
    let text: Vec<String> = FIELDS.iter().filter_map(|k| input.get(*k).and_then(Value::as_str)).map(str::to_lowercase).collect();
    if text.iter().any(|t| ["oauth_token", "environ", "env:", "process.env", "getenv"].iter().any(|n| t.contains(n))) {
        return true;
    }
    let Some(cmd) = input.get("command").and_then(Value::as_str) else { return false };
    let cmd = cmd.to_lowercase();
    // Each simple command: its first word that isn't an assignment, by its
    // name without a path; env with no command after it, set alone.
    cmd.split(['|', ';', '&', '(', ')', '`', '\n']).any(|seg| {
        let mut words = seg.split_whitespace().skip_while(|w| w.contains('='));
        let Some(first) = words.next() else { return false };
        let rest: Vec<&str> = words.collect();
        match first.rsplit(['/', '\\']).next().unwrap_or(first).trim_end_matches(".exe") {
            "printenv" => true,
            "env" => rest.iter().all(|w| w.starts_with('-')),
            "set" => rest.is_empty(),
            _ => false,
        }
    })
}

/// The answer Claude Code takes.
fn answer_json(decision: &str, input: &Value, why: &str) -> Value {
    if decision == "allow" {
        json!({ "behavior": "allow", "updatedInput": input })
    } else {
        json!({ "behavior": "deny", "message": why })
    }
}

fn hold_ms(w: &Runtime) -> f64 {
    w.setup.permission_hold_ms.unwrap_or(HOLD_MS)
}

fn minutes(ms_: f64) -> String {
    let m = (ms_ / 60e3).round();
    if m <= 1.0 {
        "a minute".into()
    } else {
        format!("{m} minutes")
    }
}

/// The run XenoNet started or resumed that `me` is: its record's id, or
/// None for a session that isn't one (a fork's copy counts by the session
/// it came from).
fn run_of(w: &Runtime, me: &str) -> Option<String> {
    let runner = &w.router.resume.as_ref()?.runner;
    let parent = crate::starts::forks(w).into_iter().find(|(child, _)| child == me).map(|(_, p)| format!("fork:{p}"));
    runner.list().into_iter().rev().find(|r| r.end.is_none() && (r.key == me || parent.as_deref() == Some(r.key.as_str()))).map(|r| r.id)
}

/// `run_of`, but a fork's copy is known only once its stream names it,
/// which its watcher reads every 400 ms: while a fork runs, a caller that
/// isn't known yet is looked for again, for three seconds at most, before
/// it's refused.
fn run_of_soon(w: &Runtime, me: &str) -> Option<String> {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(run) = run_of(w, me) {
            return Some(run);
        }
        let forking = w.router.resume.as_ref().is_some_and(|r| r.runner.list().iter().any(|r| r.end.is_none() && r.key.starts_with("fork:")));
        if !forking || Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The other machines that are window hosts: where an ask shows.
fn window_hosts(w: &Runtime) -> Vec<String> {
    net(w).hosts().into_iter().filter(|h| h.window).map(|h| h.id).collect()
}

fn to_windows(w: &Runtime, me: &str, body: &Value) {
    if net(w).me().is_none() {
        return;
    }
    let text = body.to_string();
    for host in window_hosts(w) {
        let Ok(mid) = xenonet_engine::network::new_id("msg") else { continue };
        let frame = json!({
            "schema": "network/message/v2",
            "mid": mid,
            "sender": me,
            "senderKind": "agent",
            "authority": {},
            "target": TARGET,
            "body": text,
            "replyTo": null,
            "scope": "direct",
            "kind": "message",
            "hops": 0,
            "at": ms(now_ms()),
        });
        send(w, To::Host(host), FrameKind::Message, frame);
    }
}

/// The permission tool's call from `me`: `tool_name`, `input` and
/// `tool_use_id`, as Claude Code passes them. Held until you answer, or the
/// hold runs out; then the answer Claude Code takes. Err(403) for a caller
/// that isn't a run XenoNet started.
pub fn ask(w: &Runtime, me: &str, title: Option<&str>, body: &Value) -> Result<Value, (u16, String)> {
    let Some(run) = run_of_soon(w, me) else {
        return Err((403, "only the sessions XenoNet runs ask through this tool; anyone else asks in its own app".into()));
    };
    let input = body.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("a tool").to_string();
    // A session started under this machine's standing delegation: what the
    // delegation covers is allowed here and now; the rest comes to you,
    // with why it wasn't covered.
    let mut outside = None;
    let id = xenonet_engine::network::new_id("ask").map_err(|t| (500, t.message))?;
    if let Some(folder) = crate::starts::delegated_folder(w, me) {
        match delegation::judge(&tool, &input, &folder, &w.setup.home, cfg!(windows)) {
            Verdict::Allow if !reads_env(&input) => match granted(w, me, &id) {
                Ok(()) => {
                    w.log("info", "ask", &format!("{me}'s {tool} ({}) is allowed by your standing grant for this machine", input_words(&input, &w.setup.home)));
                    return Ok(answer_json("allow", &input, ""));
                }
                Err(why) => outside = Some(why),
            },
            Verdict::Allow => outside = Some("it may read the run's environment".to_string()),
            Verdict::Ask(why) => outside = Some(why),
        }
    }
    let here = net(w).me().map(|m| m.host);
    let label = here.clone().unwrap_or_else(|| "This machine".into());
    let session_title = title.map_or_else(|| me.to_string(), |t| xenonet_engine::delivery::policy::sanitize(t, true, 120.0));
    let now = now_ms();
    let hold = hold_ms(w);
    let a = Ask {
        id,
        host: None,
        session: me.to_string(),
        run,
        title: clip(&format!("{label} · {session_title}"), 200),
        tool: clip(&xenonet_engine::delivery::policy::sanitize(&tool, true, 80.0), 80),
        input: match &outside {
            Some(why) => clip(&format!("[outside what you delegated: {why}] {}", input_words(&input, &w.setup.home)), 300),
            None => input_words(&input, &w.setup.home),
        },
        warn: reads_env(&input),
        at: now,
        until: now + hold,
    };
    let id = a.id.clone();
    let shown = a.to_json(here.as_deref());
    w.asks.here.locked().insert(id.clone(), Held { ask: a, answer: None });
    w.log("info", "ask", &format!("{me} asks to run {tool}: waiting for you"));
    w.emit(From::Engine, "ask", shown.clone());
    let mut out = shown;
    out["op"] = json!("ask");
    to_windows(w, me, &out);
    // Held until you answer, or the hold runs out.
    let side = &w.asks;
    let deadline = Instant::now() + Duration::from_secs_f64(hold / 1000.0);
    let mut held = side.here.locked();
    let answer = loop {
        if let Some(a) = held.get(&id).and_then(|h| h.answer.clone()) {
            break Some(a);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break None;
        }
        held = side.answered.wait_timeout(held, left).map_or_else(|e| e.into_inner().0, |(g, _)| g);
    };
    held.remove(&id);
    drop(held);
    let (decision, why) = match &answer {
        Some((d, _)) if d == "allow" => ("allow".to_string(), String::new()),
        Some((_, by)) => ("deny".to_string(), format!("your user denied it, from {by}")),
        None => ("deny".to_string(), format!("nobody answered in {}", minutes(hold))),
    };
    let done = json!({ "op": "done", "id": id, "decision": decision });
    w.emit(From::Engine, "ask-done", done.clone());
    to_windows(w, me, &done);
    w.log("info", "ask", &format!("{me}'s ask to run {tool}: {decision}{}", if why.is_empty() { String::new() } else { format!(" ({why})") }));
    Ok(answer_json(&decision, &input, &why))
}

/// Your Allow or Deny for an ask here (`by` says from where: this
/// machine's console, or another window host's). False for one that isn't
/// waiting.
pub fn decide(w: &Runtime, id: &str, decision: &str, by: &str) -> bool {
    let side = &w.asks;
    let mut held = side.here.locked();
    let Some(h) = held.get_mut(id).filter(|h| h.answer.is_none()) else { return false };
    h.answer = Some((if decision == "allow" { "allow" } else { "deny" }.into(), by.to_string()));
    side.answered.notify_all();
    true
}

/// Whether this machine answers `me`'s ask `id` itself, under your
/// standing grant for it: decided as the machine's own act.
fn granted(w: &Runtime, me: &str, id: &str) -> Result<(), String> {
    let g = crate::ground::ground(w, Some(id)).ok_or("this machine's governance isn't plugged in")?;
    let (resource, act) = answer_act(me, id, "allow")?;
    let request = crate::ground::local(&g, xenonet_authz::Principal::Machine { id: g.facts.machine.clone() }, resource, act);
    crate::ground::judge(&g, &request, None)
}

/// An answer's resource and act: the session whose run asked, the ask and
/// the decision, which your device signs and a presence block names.
fn answer_act(session: &str, ask: &str, decision: &str) -> Result<(xenonet_authz::Resource, xenonet_authz::Act), String> {
    let session = xenonet_authz::SessionId::new(session).map_err(|e| e.message)?;
    let ask = xenonet_authz::AskId::new(ask).map_err(|e| e.message)?;
    let decision = if decision == "allow" { xenonet_authz::AnswerDecision::Allow } else { xenonet_authz::AnswerDecision::Deny };
    Ok((xenonet_authz::Resource::Session(session), xenonet_authz::Act::Answer { ask, decision }))
}

/// The ids of the asks waiting here, for the ground's facts.
pub fn waiting_ids(w: &Runtime) -> Vec<String> {
    w.asks.here.locked().iter().filter(|(_, h)| h.answer.is_none()).map(|(id, _)| id.clone()).collect()
}

/// Your answer from this console, which only the window gives: signed by
/// your device here, with your presence for it (it asks you: Touch ID on a
/// Mac, twice, once for the signature and once for the presence). An ask
/// here is decided and answered at once; for one from another machine, the
/// answer goes there as `network/operator/v2`, for it to decide, and its
/// host comes back (Some). That ask stays until the frame is on its way, so
/// a send that fails leaves it to answer again. Err for one that isn't
/// waiting, or an answer that wasn't allowed.
pub fn answer(w: &Runtime, id: &str, decision: &str) -> Result<Option<String>, String> {
    if !matches!(decision, "allow" | "deny") {
        return Err("the answer is allow or deny".into());
    }
    let label = net(w).me().map_or_else(|| "this machine's console".to_string(), |m| format!("{}'s console", m.host));
    let g = crate::ground::ground(w, None).ok_or("this machine's governance isn't plugged in, so it answers nothing")?;
    let reason = |tool: &str| format!("{} {tool}", if decision == "allow" { "allow" } else { "deny" });
    let here = w.asks.here.locked().get(id).filter(|h| h.answer.is_none()).map(|h| (h.ask.session.clone(), h.ask.tool.clone()));
    if let Some((session, tool)) = here {
        let (resource, act) = answer_act(&session, id, decision)?;
        let auth = crate::ground::authority(&g.facts.identity, true, &resource, &act, &reason(&tool), g.facts.now)?;
        let carried = crate::ground::Carried::parse(&auth);
        let principal = crate::ground::person(w, &g, None, &carried, &resource, &act)?;
        crate::ground::judge(&g, &crate::ground::local(&g, principal, resource, act), carried.presence.as_ref())?;
        if decide(w, id, decision, &label) {
            return Ok(None);
        }
    }
    let now = now_ms();
    let (host, session, tool) = {
        let there = w.asks.there.locked();
        let Some(a) = there.iter().find(|a| a.id == id && a.until > now) else { return Err("that ask isn't waiting: it was answered, or its time ran out".into()) };
        (a.host.clone().unwrap_or_default(), a.session.clone(), a.tool.clone())
    };
    let (resource, act) = answer_act(&session, id, decision)?;
    let authority = crate::ground::authority(&g.facts.identity, true, &resource, &act, &reason(&tool), g.facts.now)?;
    let body = json!({
        "schema": "network/operator/v2",
        "answer": { "ask": id, "session": session, "decision": decision },
        "authority": authority,
        "at": ms(g.facts.now as f64),
    });
    net(w).send(xenonet_model::ops::Outgoing { to: To::Host(host.clone()), kind: FrameKind::Operator, body })?;
    answer_sent(w, id, decision);
    Ok(Some(host))
}

/// Another machine's ask, once your answer's frame is on its way there:
/// taken off this console.
pub fn answer_sent(w: &Runtime, id: &str, decision: &str) {
    let had = {
        let mut there = w.asks.there.locked();
        let n = there.len();
        there.retain(|a| a.id != id);
        n != there.len()
    };
    if had {
        w.emit(From::Engine, "ask-done", json!({ "op": "done", "id": id, "decision": decision }));
    }
}

/// An ask from another machine (`message_in` routes the target here), or
/// its end.
pub fn ask_in(w: &Runtime, from: &str, body: &str) {
    let Ok(v) = serde_json::from_str::<Value>(body) else { return };
    let side = &w.asks;
    match v.get("op").and_then(Value::as_str) {
        Some("ask") => {
            let Some(a) = Ask::of(&v, from) else { return };
            let shown = a.to_json(None);
            {
                let mut there = side.there.locked();
                let now = now_ms();
                there.retain(|x| x.until > now && x.id != a.id);
                there.push(a);
                let from = there.len().saturating_sub(THERE_MAX);
                there.drain(..from);
            }
            w.emit(From::Engine, "ask", shown);
        }
        Some("done") => {
            let Some(id) = v.get("id").and_then(Value::as_str) else { return };
            let had = {
                let mut there = side.there.locked();
                let n = there.len();
                there.retain(|a| !(a.id == id && a.host.as_deref() == Some(from)));
                n != there.len()
            };
            if had {
                w.emit(From::Engine, "ask-done", json!({ "op": "done", "id": id, "decision": v.get("decision").cloned().unwrap_or(Value::Null) }));
            }
        }
        _ => {}
    }
}

/// Your Allow or Deny from another machine's console
/// (`network/operator/v2`): decided here, on this machine's own copy of
/// your identity, with the presence block it came with taken once.
pub fn answer_in(w: &Runtime, from: &str, body: &Value) {
    let a = body.get("answer").cloned().unwrap_or(Value::Null);
    let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (id, session, decision) = (text("ask"), text("session"), text("decision"));
    if !matches!(decision.as_str(), "allow" | "deny") {
        return;
    }
    let refused = |why: String| w.log("info", "ask", &format!("an answer from {from} was refused: {why}"));
    // The ask is this machine's own, waiting, for that session.
    let waits = w.asks.here.locked().get(&id).is_some_and(|h| h.answer.is_none() && h.ask.session == session);
    if !waits {
        return w.log("info", "ask", &format!("{from} answered an ask that isn't waiting here"));
    }
    let Some(g) = crate::ground::ground(w, None) else { return refused("this machine's governance isn't plugged in".into()) };
    let (resource, act) = match answer_act(&session, &id, &decision) {
        Ok(x) => x,
        Err(why) => return refused(why),
    };
    let carried = crate::ground::Carried::parse(body.get("authority").unwrap_or(&Value::Null));
    let decided = crate::ground::person(w, &g, Some(from), &carried, &resource, &act)
        .and_then(|p| crate::ground::judge(&g, &crate::ground::remote(&g, p, resource, act), carried.presence.as_ref()));
    if let Err(why) = decided {
        return refused(why);
    }
    if !decide(w, &id, &decision, &format!("{from}'s console")) {
        w.log("info", "ask", &format!("{from} answered an ask that isn't waiting here"));
    }
}

/// Every ask waiting: this machine's and the others', for the console.
pub fn list(w: &Runtime) -> Value {
    let here = net(w).me().map(|m| m.host);
    let side = &w.asks;
    let now = now_ms();
    let mut out: Vec<Value> = side.here.locked().values().filter(|h| h.answer.is_none()).map(|h| h.ask.to_json(here.as_deref())).collect();
    out.extend(side.there.locked().iter().filter(|a| a.until > now).map(|a| a.to_json(None)));
    json!({ "asks": out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_call_asks_in_a_line_and_the_answers_claude_code_takes() {
        assert_eq!(input_words(&json!({ "command": "npm test\n&& rm -rf /home/jo/x", "description": "d" }), "/home/jo"), "npm test && rm -rf ~/x");
        assert_eq!(input_words(&json!({ "file_path": "/home/jo/src/a.rs" }), "/home/jo"), "~/src/a.rs");
        assert_eq!(input_words(&json!({ "a": 1 }), "/home/jo"), "{\"a\":1}");
        assert_eq!(input_words(&json!({ "command": "x".repeat(400) }), "/h").chars().count(), 300);
        assert_eq!(answer_json("allow", &json!({ "command": "ls" }), ""), json!({ "behavior": "allow", "updatedInput": { "command": "ls" } }));
        assert_eq!(answer_json("deny", &json!({}), "nobody answered in 10 minutes"), json!({ "behavior": "deny", "message": "nobody answered in 10 minutes" }));
        assert_eq!(minutes(HOLD_MS), "10 minutes");
        assert_eq!(minutes(30e3), "a minute");
        let a = Ask::of(&json!({ "id": "ask_1", "title": "desk · Notes\u{1b}[31m", "tool": "Bash", "input": "ls", "at": 1.0, "until": 2.0 }), "desk").unwrap();
        assert_eq!((a.host.as_deref(), a.title.as_str()), (Some("desk"), "desk · Notes"));
        assert!(Ask::of(&json!({ "id": "x" }), "desk").is_none(), "only an ask's id");
    }

    #[test]
    fn a_call_that_may_read_the_runs_environment_is_flagged() {
        let cmd = |c: &str| reads_env(&json!({ "command": c, "description": "print the environment" }));
        for c in [
            "env",
            "env | grep -i token",
            "/usr/bin/env -0",
            "printenv CLAUDE_CODE_OAUTH_TOKEN",
            "echo $CLAUDE_CODE_OAUTH_TOKEN",
            "set",
            "cd /tmp && set | sort",
            "cat /proc/self/environ",
            "tr '\\0' '\\n' < /proc/$$/environ",
            "Get-ChildItem Env:",
            "echo $env:CLAUDE_CODE_OAUTH_TOKEN",
            "python3 -c 'import os; print(os.environ)'",
            "node -e 'console.log(process.env)'",
            "X=1 printenv",
        ] {
            assert!(cmd(c), "{c}");
        }
        for c in ["npm test", "set -euo pipefail; make", "env FOO=1 node build.js", "npm config set registry https://r", "git log --oneline", "cat .env.example"] {
            assert!(!cmd(c), "{c}");
        }
        assert!(reads_env(&json!({ "file_path": "/proc/self/environ" })));
        assert!(reads_env(&json!({ "pattern": "OAUTH_TOKEN", "path": "." })));
        assert!(!reads_env(&json!({ "file_path": "/w/notes.md", "content": "os.environ and printenv" })), "a file's new text runs nothing");
        // Its card says so, here and on a window host, in this machine's
        // words whatever the other machine sends.
        let a = Ask::of(&json!({ "id": "ask_1", "input": "printenv", "warning": "anything" }), "desk").unwrap();
        assert_eq!(a.to_json(None)["warning"], json!(ENV_WARNING));
        assert_eq!(Ask::of(&json!({ "id": "ask_2", "input": "npm test" }), "desk").unwrap().to_json(None)["warning"], Value::Null);
        assert!(Ask::of(&json!({ "id": "ask_3", "input": "env | sort", "warning": null }), "desk").unwrap().warn, "worked out here too");
    }
}
