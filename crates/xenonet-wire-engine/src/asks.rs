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
//! - Allow or Deny from another machine comes back as an `operator` frame
//!   with the action `answer`: yours, from a window host, checked strongly.
//! - The call is held until you answer, the hold at most (ten minutes
//!   unless XENONET_PERMISSION_HOLD_MS says otherwise); no answer is a
//!   deny that says nobody answered.
//!
//! The answer is what Claude Code takes from a permission prompt tool:
//! `{"behavior":"allow","updatedInput":…}` or `{"behavior":"deny","message":…}`.

use crate::js::{now_ms, Locked};
use crate::network::{clip, ms, net, send};
use crate::world::World;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use xenonet_base::network::{Kind as FrameKind, To};
use xenonet_service::glue::From;

/// The reserved target an ask crosses to window hosts as.
pub const TARGET: &str = "xenonet:ask";
/// How long an ask is held, unless XENONET_PERMISSION_HOLD_MS says.
pub const HOLD_MS: f64 = 10.0 * 60e3;
/// Asks from other machines kept at most.
const THERE_MAX: usize = 50;
/// What the Allow card says of a call that may read the run's environment:
/// Claude Code passes its own environment to its tools, so a token given
/// to XenoNet's runs (`xenonet runs sign-in`) is readable there.
pub const ENV_WARNING: &str =
    "This may read the run's environment, which holds the Claude Code sign-in token you gave XenoNet's runs, if you gave one. Deny it unless the session may see that token.";

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
    let text = xenonet_base::paths::tilde(&text, home);
    clip(&xenonet_engine::delivery::policy::sanitize(&text, true, 300.0), 300)
}

/// Whether a tool call may read the run's environment, or names the
/// token: a command that prints it (env, printenv or set on their own),
/// PowerShell's env: drive, /proc/<pid>/environ, an environment API, or
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

fn hold_ms(w: &World) -> f64 {
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
fn run_of(w: &World, me: &str) -> Option<String> {
    let runner = &w.router.resume.as_ref()?.runner;
    let parent = crate::starts::forks(w).into_iter().find(|(child, _)| child == me).map(|(_, p)| format!("fork:{p}"));
    runner.list().into_iter().rev().find(|r| r.end.is_none() && (r.key == me || parent.as_deref() == Some(r.key.as_str()))).map(|r| r.id)
}

/// The other machines that are window hosts: where an ask shows.
fn window_hosts(w: &World) -> Vec<String> {
    net(w).hosts().into_iter().filter(|h| h.window).map(|h| h.id).collect()
}

fn to_windows(w: &World, me: &str, body: &Value) {
    if net(w).me().is_none() {
        return;
    }
    let text = body.to_string();
    for host in window_hosts(w) {
        let Ok(mid) = xenonet_engine::network::new_id("msg") else { continue };
        let frame = json!({
            "schema": "network/message/v1",
            "mid": mid,
            "sender": me,
            "senderKind": "agent",
            "auth": { "method": "session", "strength": "weak" },
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
pub fn ask(w: &World, me: &str, title: Option<&str>, body: &Value) -> Result<Value, (u16, String)> {
    let Some(run) = run_of(w, me) else {
        return Err((403, "only the sessions XenoNet runs ask through this tool; anyone else asks in its own app".into()));
    };
    let input = body.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("a tool").to_string();
    let here = net(w).me().map(|m| m.host);
    let label = here.clone().unwrap_or_else(|| "This machine".into());
    let session_title = title.map_or_else(|| me.to_string(), |t| xenonet_engine::delivery::policy::sanitize(t, true, 120.0));
    let now = now_ms();
    let hold = hold_ms(w);
    let a = Ask {
        id: xenonet_engine::network::new_id("ask").map_err(|t| (500, t.message))?,
        host: None,
        session: me.to_string(),
        run,
        title: clip(&format!("{label} · {session_title}"), 200),
        tool: clip(&xenonet_engine::delivery::policy::sanitize(&tool, true, 80.0), 80),
        input: input_words(&input, &w.setup.home),
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
pub fn decide(w: &World, id: &str, decision: &str, by: &str) -> bool {
    let side = &w.asks;
    let mut held = side.here.locked();
    let Some(h) = held.get_mut(id).filter(|h| h.answer.is_none()) else { return false };
    h.answer = Some((if decision == "allow" { "allow" } else { "deny" }.into(), by.to_string()));
    side.answered.notify_all();
    true
}

/// Your answer from this console: an ask here is answered at once; for one
/// from another machine, its host (Some), where the service sends the
/// `operator` frame that answers it. That ask stays until the frame is on
/// its way (`answer_sent`), so a send that fails leaves it to answer again.
/// Err for one that isn't waiting.
pub fn answer(w: &World, id: &str, decision: &str) -> Result<Option<String>, String> {
    if !matches!(decision, "allow" | "deny") {
        return Err("the answer is allow or deny".into());
    }
    let label = net(w).me().map_or_else(|| "this machine's console".to_string(), |m| format!("{}'s console", m.host));
    if decide(w, id, decision, &label) {
        return Ok(None);
    }
    let now = now_ms();
    let there = w.asks.there.locked();
    let Some(a) = there.iter().find(|a| a.id == id && a.until > now) else { return Err("that ask isn't waiting: it was answered, or its time ran out".into()) };
    Ok(a.host.clone())
}

/// Another machine's ask, once your answer's frame is on its way there:
/// taken off this console.
pub fn answer_sent(w: &World, id: &str, decision: &str) {
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
pub fn ask_in(w: &World, from: &str, body: &str) {
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

/// Your Allow or Deny from another window host's console: an `operator`
/// frame's `answer` (`{ ask, decision }`), which the network checked came
/// from a window host, strongly, fresh.
pub fn answer_in(w: &World, from: &str, args: &Value) {
    let (Some(id), Some(decision)) = (args.get("ask").and_then(Value::as_str), args.get("decision").and_then(Value::as_str)) else { return };
    if !matches!(decision, "allow" | "deny") {
        return;
    }
    if !decide(w, id, decision, &format!("{from}'s console")) {
        w.log("info", "ask", &format!("{from} answered an ask that isn't waiting here"));
    }
}

/// Every ask waiting: this machine's and the others', for the console.
pub fn list(w: &World) -> Value {
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
