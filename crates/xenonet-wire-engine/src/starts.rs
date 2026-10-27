//! Starting sessions (core/delivery/starts.js; docs/guide/network.md), on
//! this machine and on the others in its network.
//!
//! - **Here:** `reach` decides by this machine's start setting, the limits
//!   hold (20 starts an hour, 10 from one machine, and the runner's cap),
//!   the folder is one this machine knows, and the shared runner starts the
//!   app's CLI with the first prompt framed with who asked. A watcher tells
//!   whoever asked when the run ends, and whether Claude Code needs a
//!   sign-in.
//! - **Another machine:** a `message` frame to `xenonet:start` with the
//!   request as its JSON body, sent only while that machine is online and
//!   its presence says its XenoNet takes starts. Its answers are receipts
//!   keyed by the frame's mid: started, refused, then ended. An older
//!   XenoNet refuses the target as it refuses any id it doesn't have,
//!   which reads here as too old. Each answer goes to whoever asked: a
//!   message to a session or an actor, or a line in your console.
//!
//! The requests sent from here are kept in memory: after a restart, a late
//! answer to one is let go.

use crate::js::{now_ms, obj, Locked};
use crate::network::{clip, ms, net, quiet_note, send, sender_kind};
use crate::runs::{Runner, Turn};
use crate::world::World;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_base::ids::YOU;
use xenonet_base::network::{Kind as FrameKind, To};
use xenonet_engine::delivery::reach::{self, Action, Decision, Kind, Sender, Settings, Start, Wake};
use xenonet_engine::delivery::starts::{self, Request, TARGET};
use xenonet_engine::sql::Arg;
use xenonet_service::glue::From;

/// How long Claude Code waits on an MCP tool: a minute past the permission
/// tool's hold, so a prompt waiting for you isn't cut short.
pub(crate) fn mcp_tool_timeout_ms(w: &World) -> u64 {
    (w.setup.permission_hold_ms.unwrap_or(crate::asks::HOLD_MS) + 60e3).round() as u64
}

/// A request sent from here, waiting for its answers.
struct Asked {
    by: Value,
    host: String,
    at: f64,
}

#[derive(Default)]
pub struct StartSide {
    /// Starts here in the last hour: when, and from which machine ("" for
    /// this one).
    history: Mutex<Vec<(f64, String)>>,
    /// Requests sent from here to other machines, by their mid.
    asked: Mutex<HashMap<String, Asked>>,
    /// Requests from other machines already taken, by their mid.
    taken: Mutex<HashMap<String, f64>>,
    /// Copies of sessions XenoNet forked to answer a message, each with the
    /// session it came from: the last 200.
    forks: Mutex<Vec<(String, String)>>,
}

/// One turn: of XENONET_RUNS_BIN's program when it names one (the tests'
/// fake), else of the app's CLI as the runner finds it.
pub fn run_turn(w: &World, runner: &Arc<Runner>, t: Turn<'_>) -> Result<u32, String> {
    match &w.setup.runs_bin {
        Some(bin) => runner.run_program(t, bin),
        None => runner.run(t, &w.setup.home),
    }
}

/// This machine as answers name it: its host id in a network, else "this
/// machine".
pub fn here_name(w: &World) -> String {
    net(w).me().map_or_else(|| "this machine".into(), |m| m.host)
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn kind_of(k: &str) -> Kind {
    match k {
        "you" => Kind::You,
        "actor" => Kind::Actor,
        _ => Kind::Session,
    }
}

/// A plain UUID, for a new session's id.
fn uuid() -> Result<String, String> {
    let id = xenonet_engine::network::new_id("s").map_err(|t| t.message)?;
    Ok(id.split_once('_').map_or(id.clone(), |(_, u)| u.to_string()))
}

/// The folders this machine knows: its projects', folders' and Codex
/// projects' paths by their ids on the map, and the folders its sessions
/// are in.
fn folders(w: &World) -> (Vec<(String, String)>, Vec<String>) {
    let mut known = Vec::new();
    let graph = w.state().graph().ok().flatten().unwrap_or(Value::Null);
    let nodes: Vec<Value> = match graph.get("nodes") {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::Object(o)) => o.values().cloned().collect(),
        _ => Vec::new(),
    };
    for n in &nodes {
        let (Some(id), Some(path)) = (s(n, "id"), s(n, "path")) else { continue };
        if matches!(n.get("kind").and_then(Value::as_str), Some("project" | "zone" | "native")) {
            known.push((id, path));
        }
    }
    let reported = w.state().nodes().unwrap_or_default().iter().filter_map(|n| s(n, "cwd")).collect();
    (known, reported)
}

/// What a start tells whoever asked: started (at once, and again for Codex
/// once its thread has an id), and ended.
pub type Answer = Arc<dyn Fn(&World, &str, Value) + Send + Sync>;

/// A start on this machine, asked for by `by` ({ kind, id, title }) from
/// `from` (another machine, or None for this one), which `sender` says how
/// this machine sees. Its first answer, or why it didn't start; `answer`
/// hears the rest.
pub fn run_here(w: &World, by: &Value, from: Option<&str>, sender: &Sender, r: &Request, answer: Answer) -> Result<Value, String> {
    let reach = net(w).reach();
    let settings = Settings { start: Start::parse(&reach.start), wake: Wake::parse(&reach.wake), ..Settings::default() };
    if let Decision::Refuse(why) = reach::decide(Action::Start, sender, "", &settings) {
        return Err(why);
    }
    if w.setup.demo {
        return Err("XenoNet's demo starts no sessions".into());
    }
    let Some(resumes) = w.router.resume.as_ref() else { return Err("this XenoNet runs no sessions".into()) };
    // Paused, XenoNet starts nothing, whoever asks.
    if w.policy.call("paused", &[]).is_ok_and(|p| p.truthy()) {
        return Err("XenoNet is paused here, so it starts nothing until it's resumed".into());
    }
    let now = now_ms();
    // Ten an hour from each machine, and from each asker here.
    let bucket = from.map_or_else(|| starts::asker(&sender.id), str::to_string);
    {
        let mut h = w.starts.history.locked();
        *h = starts::kept(&h, now);
        if let Some(why) = starts::admit(&h, &bucket, now) {
            return Err(why);
        }
    }
    let (known, reported) = folders(w);
    let cwd = starts::folder(&r.folder, &w.setup.home, &known, &reported, cfg!(windows))?;
    if !std::path::Path::new(&cwd).is_dir() {
        return Err(format!("{} isn't there", r.folder));
    }
    // This machine's limits on messages hold for a start too: too fast
    // from one sender, a repeat of the same prompt.
    let ask =
        obj(vec![("sender", Arg::text(sender.id.clone())), ("recipients", Arg::List(vec![Arg::text(TARGET)])), ("text", Arg::text(r.prompt.clone())), ("depth", Arg::num(0.0))]);
    match w.policy.call("checkSend", &[ask]) {
        Ok(_) => {}
        Err(xenonet_engine::delivery::policy::Fail::Refused { code: "duplicate", .. }) => return Err("the same start was asked for under a minute ago".into()),
        Err(f) => return Err(format!("this machine's limits hold it: {}", crate::delivery::policy_thrown(f).0.message)),
    }
    let id = uuid()?;
    let session = (r.app != "codex").then(|| format!("{}:{id}", r.app));
    let key = session.clone().unwrap_or_else(|| format!("start:{id}"));
    let runs = std::path::Path::new(&w.setup.state_dir).join("runs");
    // Grok Build reads its prompt from a file only its owner can read,
    // taken away when the run ends.
    let text = starts::framed(by, from, &r.prompt);
    let prompt_file = if r.app == "grok" {
        let f = runs.join(format!("{id}.prompt"));
        write_private(&f, &text).map_err(|e| format!("the prompt couldn't be written for Grok Build: {e}"))?;
        Some(f.to_string_lossy().into_owned())
    } else {
        None
    };
    let server = xenonet_engine::delivery::setup::server_name(&Arg::Num(f64::from(w.setup.port)));
    let (app, args) = starts::command(r, &id, &cwd, resumes.settings.as_deref(), &server, prompt_file.as_deref());
    let mut env: Vec<(String, String)> =
        xenonet_engine::delivery::resume::resume_env(std::env::vars().map(|(k, v)| (k, Arg::Text(v))).collect()).into_iter().map(|(k, v)| (k, crate::js::text(&v))).collect();
    if app == "claude" {
        env.retain(|(k, _)| k != "MCP_TOOL_TIMEOUT");
        env.push(("MCP_TOOL_TIMEOUT".into(), mcp_tool_timeout_ms(w).to_string()));
        signed_env(w, &mut env);
    }
    let turn = Turn { key: &key, app: &app, args: &args, cwd: &cwd, env: &env, text: if prompt_file.is_some() { String::new() } else { text } };
    let pid = match run_turn(w, &resumes.runner, turn) {
        Ok(pid) => pid,
        Err(why) => {
            if let Some(f) = &prompt_file {
                let _ = std::fs::remove_file(f);
            }
            return Err(why);
        }
    };
    w.starts.history.locked().push((now, bucket));
    let folder = xenonet_base::paths::tilde(&cwd, &w.setup.home);
    w.log(
        "info",
        "start",
        &format!("started {} in {folder} for {}", session.as_deref().unwrap_or("a Codex session"), from.map_or_else(|| "this machine".to_string(), str::to_string)),
    );
    if let Some(world) = w.arc() {
        let runner = resumes.runner.clone();
        let (key, app, logs) = (key.clone(), app.clone(), runs.clone());
        let (session, folder, prompt_file) = (session.clone(), folder.clone(), prompt_file.clone());
        std::thread::spawn(move || watch(&world, &runner, &key, pid, &app, &logs, session, &folder, prompt_file, &answer));
    }
    Ok(json!({ "session": session, "pid": pid, "folder": folder, "app": app }))
}

// ---------- signing in for unattended runs ----------

fn secrets(w: &World) -> crate::wakes::Secrets {
    crate::wakes::Secrets::new(w.setup.demo)
}

/// The long-lived Claude Code token XenoNet's runs sign in with, from your
/// credential store: for its runs' environment alone, never logged or
/// shown.
pub fn run_token(w: &World) -> Option<String> {
    secrets(w).call("get", &[Arg::text(starts::SIGN_IN_TARGET)]).ok().and_then(|v| v.as_str().map(str::to_string)).filter(|t| !t.is_empty())
}

/// The environment of a Claude Code run with XenoNet's sign-in, when you
/// gave it one: CLAUDE_CODE_OAUTH_TOKEN.
pub fn signed_env(w: &World, env: &mut Vec<(String, String)>) {
    if let Some(t) = run_token(w) {
        env.retain(|(k, _)| k != "CLAUDE_CODE_OAUTH_TOKEN");
        env.push(("CLAUDE_CODE_OAUTH_TOKEN".into(), t));
    }
}

/// `xenonet runs sign-in` and its kin: `status` (whether a token is kept,
/// and where), `sign-in` with the token you pasted, `forget`. The answer
/// never carries the token.
pub fn sign_in(w: &World, action: &str, pasted: &str) -> Result<Value, (u16, String)> {
    let s = secrets(w);
    let kind = s.kind();
    match action {
        "sign-in" => {
            let token = starts::sign_in_token(pasted).map_err(|why| (400, why))?;
            let comment = Arg::Obj(vec![("comment".into(), Arg::text("Claude Code's token for the sessions XenoNet runs, from claude setup-token"))]);
            s.call("set", &[Arg::text(starts::SIGN_IN_TARGET), Arg::text(token), comment]).map_err(|t| (500, format!("your credential store didn't keep it: {}", t.message)))?;
            w.log("info", "runs.sign-in", &format!("a Claude Code token for XenoNet's runs is kept in {kind}"));
        }
        "forget" => {
            let _ = s.call("remove", &[Arg::text(starts::SIGN_IN_TARGET)]);
            w.log("info", "runs.sign-in", "XenoNet forgot its runs' Claude Code token");
        }
        "status" => {}
        other => return Err((400, format!("{other} isn't something xenonet runs does: status, sign-in or forget"))),
    }
    let kept = s.call("has", &[Arg::text(starts::SIGN_IN_TARGET)]).is_ok_and(|v| v.truthy());
    Ok(json!({ "signedIn": kept, "store": kind }))
}

/// The forks XenoNet made, newest last: each copy and the session it was
/// forked from, for the console to link them.
pub fn forks(w: &World) -> Vec<(String, String)> {
    w.starts.forks.locked().clone()
}

/// Grok Build's prompt for a turn, in a file only its owner can read, in
/// the runs folder: its path.
pub fn prompt_file(w: &World, text: &str) -> Result<String, String> {
    let f = std::path::Path::new(&w.setup.state_dir).join("runs").join(format!("{}.prompt", uuid()?));
    write_private(&f, text).map_err(|e| format!("the prompt couldn't be written for Grok Build: {e}"))?;
    Ok(f.to_string_lossy().into_owned())
}

/// A run's log, read as it grows: each read gives the whole lines written
/// since the last one, so a watcher never reads the log from its start
/// again.
struct Tail {
    file: std::path::PathBuf,
    at: u64,
    part: Vec<u8>,
}

impl Tail {
    fn new(file: std::path::PathBuf) -> Tail {
        Tail { file, at: 0, part: Vec::new() }
    }

    /// The bytes written since the last read.
    fn more(&mut self) {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = std::fs::File::open(&self.file) else { return };
        if f.seek(SeekFrom::Start(self.at)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_ok() {
            self.at += buf.len() as u64;
            self.part.extend_from_slice(&buf);
        }
    }

    /// The whole lines written since the last read; a line still being
    /// written waits for the next.
    fn lines(&mut self) -> String {
        self.more();
        let Some(end) = self.part.iter().rposition(|b| *b == b'\n') else { return String::new() };
        let done: Vec<u8> = self.part.drain(..=end).collect();
        String::from_utf8_lossy(&done).into_owned()
    }

    /// Everything not read yet, a last unfinished line included: once the
    /// run has ended.
    fn rest(&mut self) -> String {
        self.more();
        String::from_utf8_lossy(&std::mem::take(&mut self.part)).into_owned()
    }
}

/// After a resumed or forked turn starts: a fork's copy, once its stream
/// names it, linked to the session it came from (the fork event); the
/// prompt file taken away when the turn ends; and a hint for you when
/// Claude Code couldn't sign in for it.
pub fn after_resume(w: &World, runner: &Arc<Runner>, app: &str, key: &str, pid: u32, fork_of: Option<&str>, prompt_file: Option<String>) {
    let claude = app == "claude";
    if fork_of.is_none() && prompt_file.is_none() && !claude {
        return;
    }
    let Some(world) = w.arc() else { return };
    let (runner, key, parent) = (runner.clone(), key.to_string(), fork_of.map(str::to_string));
    let logs = std::path::Path::new(&w.setup.state_dir).join("runs");
    std::thread::spawn(move || {
        let mut named = parent.is_none();
        let mut tail: Option<Tail> = None;
        let mut out = false;
        loop {
            std::thread::sleep(Duration::from_millis(400));
            let Some(rec) = runner.list().into_iter().rev().find(|r| r.key == key && r.pid == Some(f64::from(pid))) else { break };
            let log = tail.get_or_insert_with(|| Tail::new(logs.join(format!("{}.log", rec.id))));
            let new = if rec.end.is_some() { log.rest() } else { log.lines() };
            out = out || (claude && starts::signed_out(&new));
            if !named {
                if let (Some(child), Some(parent)) = (starts::session_of("claude", &new), parent.as_deref()) {
                    named = true;
                    {
                        let mut f = world.starts.forks.locked();
                        f.push((child.clone(), parent.to_string()));
                        let from = f.len().saturating_sub(200);
                        f.drain(..from);
                    }
                    world.log("info", "fork", &format!("{child} answers a message for {parent}, which is open elsewhere"));
                    world.emit(From::Engine, "fork", json!({ "child": child, "parent": parent, "at": ms(now_ms()) }));
                }
            }
            if rec.end.is_some() {
                // A turn Claude Code couldn't sign in for: a hint for you.
                if out {
                    sign_in_hint(&world, &here_name(&world));
                }
                break;
            }
        }
        if let Some(f) = prompt_file {
            let _ = std::fs::remove_file(f);
        }
    });
}

/// A file only its owner can read.
fn write_private(file: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    std::io::Write::write_all(&mut o.open(file)?, text.as_bytes())
}

/// Watches a started run until it ends: Codex's thread id once it says
/// one, then how the run ended and whether Claude Code needs a sign-in.
#[expect(clippy::too_many_arguments, reason = "the run's parts")]
fn watch(
    w: &World,
    runner: &Runner,
    key: &str,
    pid: u32,
    app: &str,
    logs: &std::path::Path,
    mut session: Option<String>,
    folder: &str,
    prompt_file: Option<String>,
    answer: &Answer,
) {
    let mine = |r: &xenonet_engine::delivery::runs::Run| r.key == key && r.pid == Some(f64::from(pid));
    let mut tail: Option<Tail> = None;
    let mut out = false;
    let ended = loop {
        std::thread::sleep(Duration::from_millis(400));
        let Some(rec) = runner.list().into_iter().rev().find(|r| mine(r)) else { break None };
        let log = tail.get_or_insert_with(|| Tail::new(logs.join(format!("{}.log", rec.id))));
        let new = if rec.end.is_some() { log.rest() } else { log.lines() };
        out = out || starts::signed_out(&new);
        if session.is_none() {
            if let Some(found) = starts::session_of(app, &new) {
                session = Some(found.clone());
                answer(w, "started", json!({ "session": found, "pid": pid, "folder": folder, "app": app }));
            }
        }
        if rec.end.is_some() {
            break Some(rec);
        }
    };
    if let Some(f) = prompt_file {
        let _ = std::fs::remove_file(f);
    }
    let Some(rec) = ended else { return };
    let minutes = (rec.ms.unwrap_or(0.0) / 60e3).round();
    let mut body = json!({ "session": session, "exit": rec.exit, "minutes": minutes, "app": app });
    if app == "claude" && out {
        let here = here_name(w);
        body["reason"] = json!(starts::sign_in_words(&here));
        body["signIn"] = json!(true);
        sign_in_hint(w, &here);
    } else if rec.timed_out {
        body["reason"] = json!("it ran half an hour, the most a turn runs");
    }
    answer(w, "ended", body);
}

/// What the requester hears of an answer from `host`: a message to a
/// session or an actor, or a line in your console.
pub fn tell(w: &World, by: &Value, host: &str, state: &str, a: &Value) {
    let words = starts::answer_words(state, host, a);
    // Claude Code there needs a sign-in for XenoNet's runs: a hint in your
    // console, whoever asked.
    if a.get("signIn").and_then(Value::as_bool) == Some(true) {
        sign_in_hint(w, host);
    }
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    if kind == "you" {
        let mut ev = a.clone();
        if !ev.is_object() {
            ev = json!({});
        }
        ev["state"] = json!(state);
        ev["host"] = json!(host);
        ev["words"] = json!(words);
        ev["at"] = ms(now_ms());
        w.emit(From::Engine, "start", ev);
        return;
    }
    // To the session that asked: for its next turn, never a wake of its own.
    if let Some(id) = s(by, "id") {
        quiet_note(w, &id, &id, &words);
    }
}

/// A start asked for through the API (`POST /api/start`, the MCP tool, the
/// CLI): `by` is { kind, id, title }, `req` { machine?, folder?, app?,
/// mode?, prompt }; `strong` says you were checked by the window key (or
/// by process checks on macOS). The first answer, or a status and why.
pub fn ask(w: &World, by: Value, req: &Value, strong: bool) -> Result<Value, (u16, String)> {
    let r = starts::request(req).map_err(|why| (400, why))?;
    let me = net(w).me();
    let machine = s(req, "machine").filter(|m| !m.is_empty() && me.as_ref().is_none_or(|me| me.host != *m));
    let Some(host) = machine else {
        let kind = kind_of(by.get("kind").and_then(Value::as_str).unwrap_or(""));
        let sender = Sender { kind, id: s(&by, "id").unwrap_or_default(), host: None, window: true, strong };
        let who = by.clone();
        let answer: Answer = Arc::new(move |w: &World, state: &str, a: Value| tell(w, &who, "this machine", state, &a));
        let first = run_here(w, &by, None, &sender, &r, answer).map_err(|why| (409, why))?;
        let mut out = first;
        out["state"] = json!("started");
        out["host"] = me.map_or(Value::Null, |m| json!(m.host));
        return Ok(out);
    };
    let Some(_) = me else { return Err((409, "this machine isn't in a XenoNet network: start sessions on another machine once it's in one".into())) };
    let Some(h) = net(w).hosts().into_iter().find(|h| h.id == host) else { return Err((404, format!("{host} isn't a machine in your network"))) };
    // A start is never kept for later: it goes only to a machine that's
    // there, and new enough to take it.
    if !h.online {
        return Err((409, format!("{host} is offline, so nothing was started: a start is never kept for later")));
    }
    if !net(w).host_can(&host, "start") {
        return Err((409, format!("{host}'s XenoNet is too old to start sessions: update it there")));
    }
    let mid = xenonet_engine::network::new_id("msg").map_err(|t| (500, t.message))?;
    let id = s(&by, "id").unwrap_or_else(|| YOU.into());
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    let sender = if kind == "you" { YOU.to_string() } else { id.clone() };
    let mut request = r.to_json();
    request["request"] = json!(mid);
    request["by"] = json!({ "kind": kind, "id": sender, "title": by.get("title").and_then(Value::as_str).map(|t| clip(t, 200)) });
    let text = request.to_string();
    if text.chars().count() > 16000 {
        return Err((400, "that prompt is too long to send to another machine: keep it under 15,000 characters".into()));
    }
    let method = if kind == "you" {
        if strong {
            "window"
        } else {
            "person"
        }
    } else if kind == "actor" {
        "token"
    } else {
        "session"
    };
    let body = json!({
        "schema": "network/message/v1",
        "mid": mid,
        "sender": sender,
        "senderKind": sender_kind(&sender),
        "auth": { "method": method, "strength": if strong { "strong" } else { "weak" } },
        "target": TARGET,
        "body": text,
        "replyTo": null,
        "scope": "direct",
        "kind": "message",
        "hops": 0,
        "at": ms(now_ms()),
    });
    w.starts.asked.locked().insert(mid.clone(), Asked { by: by.clone(), host: host.clone(), at: now_ms() });
    send(w, To::Host(host.clone()), FrameKind::Message, body);
    Ok(
        json!({ "request": mid, "host": host, "state": "sent", "way": format!("sent to {host}: the answer comes as a message, started or refused, and again when the session ends") }),
    )
}

fn start_receipt(w: &World, to: &str, mid: &str, state: &str, extra: &Value) {
    let mut body = json!({ "schema": "network/receipt/v1", "mid": mid, "agent": TARGET, "state": state, "reason": null, "at": ms(now_ms()) });
    for (k, v) in extra.as_object().into_iter().flatten() {
        body[k.as_str()] = if k == "reason" { v.as_str().map_or(Value::Null, |r| json!(clip(r, 280))) } else { v.clone() };
    }
    send(w, To::Host(to.to_string()), FrameKind::Receipt, body);
}

/// A start request from another machine (`message_in` routes the target
/// here): `kind` and `sender` as its frame says, `strong` whether that
/// machine checked it strongly, `window` whether it's a window host, `at`
/// when it was sent, by this clock.
#[expect(clippy::too_many_arguments, reason = "the frame's parts")]
pub fn start_in(w: &World, from: &str, mid: &str, kind: &str, sender: &str, body: &str, strong: bool, window: bool, at: f64) {
    let refuse = |why: &str| start_receipt(w, from, mid, "refused", &json!({ "reason": why }));
    let now = now_ms();
    {
        let mut taken = w.starts.taken.locked();
        taken.retain(|_, t| now - *t < 2.0 * starts::FRESH_MS);
        if taken.insert(format!("{from}|{mid}"), now).is_some() {
            return;
        }
    }
    if now - at > starts::FRESH_MS {
        return refuse("it waited too long on its way to be started: a start is never kept for later");
    }
    let Ok(v) = serde_json::from_str::<Value>(body) else { return refuse("that isn't a start request") };
    let r = match starts::request(&v) {
        Ok(r) => r,
        Err(why) => return refuse(&why),
    };
    let k = match kind {
        "you" => Kind::You,
        "actor" => Kind::Actor,
        _ => Kind::Session,
    };
    let qualified = if k == Kind::You { YOU.to_string() } else { format!("@{from}:{sender}") };
    let by = json!({ "kind": k.as_str(), "id": qualified, "title": v.get("by").and_then(|b| b.get("title")).and_then(Value::as_str) });
    let who = Sender { kind: k, id: qualified.clone(), host: Some(from.to_string()), window, strong };
    let (to, m) = (from.to_string(), mid.to_string());
    let answer: Answer = Arc::new(move |w: &World, state: &str, a: Value| start_receipt(w, &to, &m, state, &a));
    match run_here(w, &by, Some(from), &who, &r, answer) {
        Ok(first) => start_receipt(w, from, mid, "started", &first),
        Err(why) => {
            w.log("info", "start", &format!("a start from {from} was refused: {why}"));
            refuse(&why);
        }
    }
}

/// A receipt for a start request sent from here: true when it was one.
pub fn answer_in(w: &World, from: &str, mid: &str, b: &Value) -> bool {
    let state = b.get("state").and_then(Value::as_str).unwrap_or("");
    let (by, host) = {
        let mut asked = w.starts.asked.locked();
        let Some(a) = asked.get(mid) else { return false };
        if a.host != from {
            return true;
        }
        let out = (a.by.clone(), a.host.clone());
        if matches!(state, "refused" | "ended") {
            asked.remove(mid);
        }
        // A week on, an answer that never came is let go.
        asked.retain(|_, a| now_ms() - a.at < 7.0 * 24.0 * 3600e3);
        out
    };
    let mut a = b.clone();
    // An older XenoNet refuses the target as an id it doesn't have.
    if state == "refused" && b.get("reason").and_then(Value::as_str) == Some("no session or actor here has that id") {
        a["reason"] = json!(format!("{host}'s XenoNet is too old to start sessions: update it there"));
    }
    tell(w, &by, &host, state, &a);
    true
}

/// Claude Code on `host` needs a sign-in for XenoNet's runs: a Needs-you
/// hint in the console.
pub fn sign_in_hint(w: &World, host: &str) {
    w.emit(From::Engine, "sign-in", json!({ "host": host, "app": "claude", "words": format!("Claude Code on {host} needs a sign-in for XenoNet's runs"), "fix": starts::sign_in_words(host), "at": ms(now_ms()) }));
}

#[cfg(test)]
mod tests {
    use super::Tail;

    #[test]
    fn a_log_is_read_as_it_grows_by_whole_lines() {
        let dir = std::env::temp_dir().join(format!("xenonet-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("run.log");
        let mut t = Tail::new(file.clone());
        assert_eq!(t.lines(), "", "no log yet");
        std::fs::write(&file, "{\"a\":1}\n{\"session_").unwrap();
        assert_eq!(t.lines(), "{\"a\":1}\n");
        assert_eq!(t.lines(), "", "the half line waits");
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, b"id\":\"x\"}\nNot logged in").unwrap();
        assert_eq!(t.lines(), "{\"session_id\":\"x\"}\n", "only what's new, the line made whole");
        assert_eq!(t.rest(), "Not logged in", "at the end, the last unfinished line too");
        assert_eq!(t.rest(), "");
        let _ = std::fs::remove_dir_all(dir);
    }
}
