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
//! - An Allow widens what a run may do, so it's yours: a presence block for
//!   it from your device (Touch ID, or polkit, a passphrase or a PIN where
//!   that's what the device has), which stands for your device's signature
//!   too, so you're asked once. Here and from another machine, which sends
//!   it as `network/operator/v2`. This machine decides it (xenonet-authz)
//!   on its own copy of your identity, and takes each presence block once.
//! - A Deny can only narrow, so it asks nothing: from this machine's window
//!   it's the machine's own answer; from another machine's, that window
//!   host relays it in its signed frame.
//! - Two others answer. An actor whose grant has the right to answer
//!   denies; it allows only inside the allow scope its grant's row holds
//!   here (sessions, kinds of prompt, until). And a run started under your
//!   delegation (start: delegate, and a delegation in your chain naming
//!   this machine's device and the one it was started from): what the
//!   delegation answers and its limits cover (the delegation judge), the
//!   machine allows itself.
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

/// An ask held here, its kind of prompt, how its caller was checked when
/// it asked, and its answer once there is one: allow or deny, and who gave
/// it.
struct Held {
    ask: Ask,
    kind: xenonet_authz::PromptKind,
    by: xenonet_authz::Attestation,
    answer: Option<(String, String)>,
    /// What decide allowed for the answer, which an ACP session's
    /// allow-once is picked on.
    allowed: Option<Box<xenonet_authz::Authorized>>,
}

/// An ask waiting here, as the ground reads it: its session, its kind, how
/// the caller was checked when it asked, and where its run was started
/// from under your delegation (None: it wasn't; Some(None): from this
/// machine).
pub struct Waiting {
    pub session: String,
    pub kind: xenonet_authz::PromptKind,
    pub by: xenonet_authz::Attestation,
    pub delegated_from: Option<Option<String>>,
    /// For a run started for an intent: the delegation that intent names,
    /// which alone answers its prompts.
    pub delegation: Option<String>,
}

/// A tool call's kind of prompt.
pub fn prompt_kind(tool: &str) -> xenonet_authz::PromptKind {
    xenonet_authz::PromptKind::parse(delegation::prompt_kind(tool)).unwrap_or(xenonet_authz::PromptKind::Tool)
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
            "schema": "network/send/v1",
            "mid": mid,
            "sender": me,
            "senderKind": "agent",
            "agent": crate::network::wire_id(w, me),
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
/// `tool_use_id`, as Claude Code passes them, with what the process check
/// found of the caller (`by`). Held until you answer, or the hold runs
/// out; then the answer Claude Code takes. Err(403) for a caller that
/// isn't a run XenoNet started.
///
/// Only a run XenoNet started asks, but the caller is that run only as far
/// as the process check could see: where it saw nothing, the ask is
/// recorded as unchecked, and nobody but the person may Allow it.
pub fn ask(w: &Runtime, me: &str, title: Option<&str>, body: &Value, by: xenonet_authz::Attestation) -> Result<Value, (u16, String)> {
    let Some(run) = run_of_soon(w, me) else {
        return Err((403, "only the sessions XenoNet runs ask through this tool; anyone else asks in its own app".into()));
    };
    let input = body.get("input").cloned().unwrap_or_else(|| json!({}));
    let tool = body.get("tool_name").and_then(Value::as_str).unwrap_or("a tool").to_string();
    // A session started under your delegation: what the delegation covers
    // is allowed here and now; the rest comes to you, with why it wasn't
    // covered.
    let mut outside = None;
    let id = xenonet_engine::network::new_id("ask").map_err(|t| (500, t.message))?;
    let kind = prompt_kind(&tool);
    if let Some((folder, from)) = crate::starts::delegated(w, me) {
        match delegation::judge(&tool, &input, &folder, &w.setup.home, cfg!(windows)) {
            Verdict::Allow if !reads_env(&input) => {
                match granted(w, &id, Waiting { session: me.to_string(), kind, by, delegated_from: Some(from), delegation: crate::intents::of_session(w, me).map(|(_, d)| d) }) {
                    Ok(()) => {
                        w.log("info", "ask", &format!("{me}'s {tool} ({}) is allowed by your delegation", input_words(&input, &w.setup.home)));
                        return Ok(answer_json("allow", &input, ""));
                    }
                    Err(why) => outside = Some(why),
                }
            }
            Verdict::Allow => outside = Some("it may read the run's environment".to_string()),
            Verdict::Ask(why) => outside = Some(why),
        }
    }
    let label = net(w).me().map_or_else(|| "This machine".to_string(), |m| m.host);
    let session_title = title.map_or_else(|| me.to_string(), |t| xenonet_engine::delivery::policy::sanitize(t, true, 120.0));
    let now = now_ms();
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
        until: now + hold_ms(w),
    };
    let answer = hold(w, me, a, kind, by).map(|(d, by, _)| (d, by));
    let (decision, why) = said(answer.as_ref(), hold_ms(w));
    Ok(answer_json(&decision, &input, &why))
}

/// An answer as the asker hears it: allow or deny, and why a deny.
fn said(answer: Option<&(String, String)>, hold: f64) -> (String, String) {
    match answer {
        Some((d, _)) if d == "allow" => ("allow".to_string(), String::new()),
        Some((_, by)) => ("deny".to_string(), format!("your user denied it, from {by}")),
        None => ("deny".to_string(), format!("nobody answered in {}", minutes(hold))),
    }
}

/// What an ask held here came to: allow or deny, who gave it, and what
/// decide allowed.
type Heard = (String, String, Option<Box<xenonet_authz::Authorized>>);

/// Holds an ask here until it's answered or the hold runs out: in Needs
/// you, here and on every window host, and taken back from them when
/// it's done. Its answer, or None when nobody answered.
fn hold(w: &Runtime, me: &str, a: Ask, kind: xenonet_authz::PromptKind, by: xenonet_authz::Attestation) -> Option<Heard> {
    let here = net(w).me().map(|m| m.host);
    let (id, tool, hold) = (a.id.clone(), a.tool.clone(), a.until - a.at);
    let shown = a.to_json(here.as_deref());
    w.asks.here.locked().insert(id.clone(), Held { ask: a, kind, by, answer: None, allowed: None });
    w.log("info", "ask", &format!("{me} asks to run {tool}: waiting for you"));
    w.emit(From::Engine, "ask", shown.clone());
    let mut out = shown;
    out["op"] = json!("ask");
    to_windows(w, me, &out);
    let side = &w.asks;
    let deadline = Instant::now() + Duration::from_secs_f64(hold / 1000.0);
    let mut held = side.here.locked();
    let answer = loop {
        if let Some(h) = held.get_mut(&id) {
            if let Some((d, by)) = h.answer.clone() {
                break Some((d, by, h.allowed.take()));
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break None;
        }
        held = side.answered.wait_timeout(held, left).map_or_else(|e| e.into_inner().0, |(g, _)| g);
    };
    held.remove(&id);
    drop(held);
    let (decision, why) = said(answer.as_ref().map(|(d, by, _)| (d.clone(), by.clone())).as_ref(), hold);
    let done = json!({ "op": "done", "id": id, "decision": decision });
    w.emit(From::Engine, "ask-done", done.clone());
    to_windows(w, me, &done);
    w.log("info", "ask", &format!("{me}'s ask to run {tool}: {decision}{}", if why.is_empty() { String::new() } else { format!(" ({why})") }));
    answer
}

/// A permission prompt from a session XenoNet holds open over ACP (`me`,
/// whose agent is XenoNet's own child, so its caller is checked): held as
/// a run's ask is, by the ask id the ACP client made, until it's answered
/// or the hold runs out. What decide allowed, for an Allow; None for a
/// Deny, or nobody answering, which the client answers reject-once.
pub fn hold_acp(
    w: &Runtime,
    me: &str,
    title: Option<&str>,
    ask: &xenonet_authz::AskId,
    tool: &str,
    kind: xenonet_authz::PromptKind,
    input: &Value,
) -> Option<Box<xenonet_authz::Authorized>> {
    let label = net(w).me().map_or_else(|| "This machine".to_string(), |m| m.host);
    let session_title = title.map_or_else(|| me.to_string(), |t| xenonet_engine::delivery::policy::sanitize(t, true, 120.0));
    let now = now_ms();
    let a = Ask {
        id: ask.as_str().to_string(),
        host: None,
        session: me.to_string(),
        run: me.to_string(),
        title: clip(&format!("{label} · {session_title}"), 200),
        tool: clip(&xenonet_engine::delivery::policy::sanitize(tool, true, 80.0), 80),
        input: input_words(input, &w.setup.home),
        warn: reads_env(input),
        at: now,
        until: now + hold_ms(w),
    };
    let (decision, _, allowed) = hold(w, me, a, kind, xenonet_authz::Attestation::ProcessChecked)?;
    allowed.filter(|_| decision == "allow")
}

/// Your Allow or Deny for an ask here (`by` says from where: this
/// machine's console, or another window host's). False for one that isn't
/// waiting.
pub fn decide(w: &Runtime, id: &str, decision: &str, by: &str, allowed: Box<xenonet_authz::Authorized>) -> bool {
    let side = &w.asks;
    let mut held = side.here.locked();
    let Some(h) = held.get_mut(id).filter(|h| h.answer.is_none()) else { return false };
    h.answer = Some((if decision == "allow" { "allow" } else { "deny" }.into(), by.to_string()));
    h.allowed = Some(allowed);
    side.answered.notify_all();
    true
}

/// Whether this machine allows the ask `id` itself, under your delegation:
/// decided as the machine's own act, for the ask about to be held.
fn granted(w: &Runtime, id: &str, a: Waiting) -> Result<(), String> {
    let (resource, act) = answer_act(&a.session, id, "allow")?;
    let g = crate::ground::ground(w, Some((id, a))).ok_or("this machine's governance isn't plugged in")?;
    let request = crate::ground::local(&g, xenonet_authz::Principal::Machine { id: g.facts.machine.clone() }, resource, act);
    crate::ground::judge(&g, &request, None)
}

/// An answer's resource and act: the session whose run asked, the ask and
/// the decision, which a presence block for an Allow names.
fn answer_act(session: &str, ask: &str, decision: &str) -> Result<(xenonet_authz::Resource, xenonet_authz::Act), String> {
    let session = xenonet_authz::SessionId::new(session).map_err(|e| e.message)?;
    let ask = xenonet_authz::AskId::new(ask).map_err(|e| e.message)?;
    let decision = if decision == "allow" { xenonet_authz::AnswerDecision::Allow } else { xenonet_authz::AnswerDecision::Deny };
    Ok((xenonet_authz::Resource::Session(session), xenonet_authz::Act::Answer { ask, decision }))
}

/// The asks waiting here, for the ground's facts.
pub fn waiting(w: &Runtime) -> Vec<(String, Waiting)> {
    let held: Vec<(String, String, xenonet_authz::PromptKind, xenonet_authz::Attestation)> =
        w.asks.here.locked().iter().filter(|(_, h)| h.answer.is_none()).map(|(id, h)| (id.clone(), h.ask.session.clone(), h.kind, h.by)).collect();
    held.into_iter()
        .map(|(id, session, kind, by)| {
            let delegated_from = crate::starts::delegated(w, &session).map(|(_, from)| from);
            let delegation = crate::intents::of_session(w, &session).map(|(_, d)| d);
            (id, Waiting { session, kind, by, delegated_from, delegation })
        })
        .collect()
}

/// Your answer from this console, which only the window gives. An Allow
/// asks you once (Touch ID on a Mac): your device's presence for it, which
/// stands for its signature too. A Deny asks nothing: here it's this
/// machine's answer for its window; for another machine's ask, this
/// machine relays it, as a window host, in its signed frame. An ask here is
/// decided and answered at once; for one from another machine, the answer
/// goes there as `network/operator/v2`, for it to decide, and its host
/// comes back (Some). That ask stays until the frame is on its way, so a
/// send that fails leaves it to answer again. Err for one that isn't
/// waiting, or an answer that wasn't allowed.
pub fn answer(w: &Runtime, id: &str, decision: &str) -> Result<Option<String>, String> {
    if !matches!(decision, "allow" | "deny") {
        return Err("the answer is allow or deny".into());
    }
    let allow = decision == "allow";
    let label = net(w).me().map_or_else(|| "this machine's console".to_string(), |m| format!("{}'s console", m.host));
    let g = crate::ground::ground(w, None).ok_or("this machine's governance isn't plugged in, so it answers nothing")?;
    let reason = |tool: &str| format!("allow {tool}");
    let here = w.asks.here.locked().get(id).filter(|h| h.answer.is_none()).map(|h| (h.ask.session.clone(), h.ask.tool.clone()));
    if let Some((session, tool)) = here {
        let (resource, act) = answer_act(&session, id, decision)?;
        let (principal, presence) = if allow {
            let carried = crate::ground::Carried::parse(&crate::ground::authority(&g.facts.identity, true, &resource, &act, &reason(&tool), g.facts.now)?);
            (crate::ground::person(w, &g, None, &carried, &resource, &act)?, carried.presence)
        } else {
            (xenonet_authz::Principal::Machine { id: g.facts.machine.clone() }, None)
        };
        let allowed = crate::ground::judged_by(&g, &crate::ground::local(&g, principal, resource, act), None, presence.as_ref())?;
        if decide(w, id, decision, &label, allowed) {
            return Ok(None);
        }
    }
    let (host, session, tool) = there(w, id)?;
    let authority = if allow {
        let (resource, act) = answer_act(&session, id, decision)?;
        crate::ground::authority(&g.facts.identity, true, &resource, &act, &reason(&tool), g.facts.now)?
    } else {
        json!({})
    };
    send_answer(w, &host, id, &session, decision, authority, g.facts.now)?;
    Ok(Some(host))
}

/// An answer from this machine's actor, by the token it called with. One
/// whose grant has the right to answer denies; it allows only inside its
/// allow scope, here. For another machine's ask it denies through a token
/// minted for the act, which that machine decides; an Allow there would be
/// refused, since the scope is a row only this machine holds, so it's
/// refused here, saying so.
pub fn answer_as_actor(w: &Runtime, token: &str, id: &str, decision: &str) -> Result<Option<String>, String> {
    if !matches!(decision, "allow" | "deny") {
        return Err("the answer is allow or deny".into());
    }
    let g = crate::ground::ground(w, None).ok_or("this machine's governance isn't plugged in, so it answers nothing")?;
    let token = xenonet_authz::Token::parse(token, xenonet_authz::ACTOR_PREFIX, &g, g.facts.now).map_err(|e| e.message)?;
    let bearer = token.bearer().clone();
    if bearer.machine != g.facts.machine {
        return Err("that token isn't one of this machine's actors'".into());
    }
    let principal = xenonet_authz::Principal::Actor { slug: bearer.slug.clone(), machine: bearer.machine.clone() };
    let here = w.asks.here.locked().get(id).filter(|h| h.answer.is_none()).map(|h| h.ask.session.clone());
    if let Some(session) = here {
        let (resource, act) = answer_act(&session, id, decision)?;
        let allowed = crate::ground::judged_by(&g, &crate::ground::local(&g, principal, resource, act), Some(&token), None)?;
        if decide(w, id, decision, &format!("actor:{}", bearer.slug), allowed) {
            return Ok(None);
        }
    }
    let (host, session, _) = there(w, id)?;
    if decision == "allow" {
        return Err(format!("an actor allows only the prompts asked on its own machine, inside its allow scope there: {host}'s is {host}'s to answer"));
    }
    // That machine decides the act token minted for it; the actor's own
    // token must have the right first, as for a start.
    if !token.rights().contains(&xenonet_authz::Action::Answer) {
        return Err("answering prompts needs the answer-prompts capability, which this actor wasn't allowed".into());
    }
    let authority = crate::ground::actor_authority(w, bearer.slug.as_str(), vec![xenonet_authz::Action::Answer])?;
    send_answer(w, &host, id, &session, decision, authority, g.facts.now)?;
    Ok(Some(host))
}

// Another machine's ask waiting on this console: its host, session and tool.
fn there(w: &Runtime, id: &str) -> Result<(String, String, String), String> {
    let now = now_ms();
    let there = w.asks.there.locked();
    let Some(a) = there.iter().find(|a| a.id == id && a.until > now) else { return Err("that ask isn't waiting: it was answered, or its time ran out".into()) };
    Ok((a.host.clone().unwrap_or_default(), a.session.clone(), a.tool.clone()))
}

// An answer to another machine's ask, as `network/operator/v2`, then the ask
// taken off this console.
fn send_answer(w: &Runtime, host: &str, id: &str, session: &str, decision: &str, authority: Value, now: i64) -> Result<(), String> {
    let body = json!({
        "schema": "network/operator/v2",
        "answer": { "ask": id, "session": session, "decision": decision },
        "authority": authority,
        "at": ms(now as f64),
    });
    net(w).send(xenonet_model::ops::Outgoing::new(To::Host(host.to_string()), FrameKind::Operator, body))?;
    answer_sent(w, id, decision);
    Ok(())
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
/// its end. What it asks is information, shown on this console: the
/// answer is decided on the ask's id and the person's presence, never on
/// its text.
pub fn ask_in(w: &Runtime, from: &str, body: &xenonet_authz::Information<String>) {
    let Ok(v) = serde_json::from_str::<Value>(body.read()) else { return };
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

/// An Allow or Deny from another machine (`network/operator/v2`), decided
/// here on this machine's own copies: the person, by their presence for it
/// (an Allow) or their device (a Deny); that machine's actor, by a token it
/// minted for the act; else that machine itself, relaying its window's
/// Deny, which only a window host does.
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
    let Some(mut g) = crate::ground::ground(w, None) else { return refused("this machine's governance isn't plugged in".into()) };
    let (resource, act) = match answer_act(&session, &id, &decision) {
        Ok(x) => x,
        Err(why) => return refused(why),
    };
    let carried = crate::ground::Carried::parse(body.get("authority").unwrap_or(&Value::Null));
    let machine = match crate::ground::machine(from) {
        Ok(m) => m,
        Err(why) => return refused(why),
    };
    let (by, decided) = if carried.person.is_some() {
        let p = crate::ground::person(w, &g, Some(from), &carried, &resource, &act);
        (format!("{from}'s console"), p.and_then(|p| crate::ground::judged_by(&g, &crate::ground::remote(&g, p, resource, act), None, carried.presence.as_ref())))
    } else if carried.token.is_some() {
        match crate::ground::actor(&mut g, &machine, &carried) {
            Ok((p, token)) => {
                let by = match &p {
                    xenonet_authz::Principal::Actor { slug, .. } => format!("actor:{slug} on {from}"),
                    _ => from.to_string(),
                };
                (by, crate::ground::judged_by(&g, &crate::ground::remote(&g, p, resource, act), Some(&token), None))
            }
            Err(why) => (String::new(), Err(why)),
        }
    } else {
        let p = xenonet_authz::Principal::Machine { id: machine };
        (format!("{from}'s console"), crate::ground::judged_by(&g, &crate::ground::remote(&g, p, resource, act), None, None))
    };
    let allowed = match decided {
        Ok(a) => a,
        Err(why) => return refused(why),
    };
    if !decide(w, &id, &decision, &by, allowed) {
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
