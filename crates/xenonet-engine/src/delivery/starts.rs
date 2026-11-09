//! Starting a session: core/delivery/starts.js. You, a session or an
//! actor asks a machine for a new session of Claude Code, Codex or Grok
//! Build, in a folder there, with a first prompt. Whether the machine
//! takes it is `reach`'s (its start setting); what a request may say, how
//! its first prompt is framed, the limits and what the answers say are
//! here. The command that starts each app is its adapter's (`apps`, the
//! `start` way in). Running it is the shared runner's (`runs`), and
//! carrying it to another machine the network's: a message frame to
//! `TARGET`, answered by receipts keyed by its id.
//!
//! The prompt goes on standard input (or in a file of XenoNet's, for an
//! app that reads it from one), never as an argument. A session never
//! starts bypassing its permission checks.

use super::apps::{self, Start};
use serde_json::{json, Value};

/// The reserved target a start request goes to on another machine.
pub const TARGET: &str = "xenonet:start";
pub const APPS: [&str; 3] = ["claude", "codex", "grok"];
/// Permission modes a request may ask for; manual is the person's own
/// settings. bypassPermissions is never taken.
pub const MODES: [&str; 5] = ["manual", "plan", "acceptEdits", "auto", "dontAsk"];
pub const PROMPT_MAX: usize = 16000;
/// Starts an hour, on one machine and from one sending machine.
pub const PER_HOUR: usize = 20;
pub const PER_HOST_HOUR: usize = 10;
const HOUR_MS: f64 = 3600e3;
/// A start request older than this when it arrives isn't started: a start
/// is never kept for later.
pub const FRESH_MS: f64 = 120e3;

/// What a request asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// A project or folder id from the machine's map, a path it reports,
    /// or ~.
    pub folder: String,
    pub app: String,
    /// None: the person's own settings.
    pub mode: Option<String>,
    pub prompt: String,
    /// Asked as delegated work: the machine starts it only when it's set to
    /// delegate (a session or actor asking; you asking is never refused).
    pub delegate: bool,
}

impl Request {
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "folder": self.folder, "app": self.app, "mode": self.mode, "prompt": self.prompt });
        if self.delegate {
            v["delegate"] = json!(true);
        }
        v
    }
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// A request as it came (`{ folder?, app?, mode?, prompt, delegate? }`), or why it
/// can't be started anywhere.
pub fn request(v: &Value) -> Result<Request, String> {
    let app = text(v, "app").filter(|a| !a.is_empty()).unwrap_or_else(|| "claude".into());
    if !APPS.contains(&app.as_str()) {
        return Err(format!("XenoNet starts Claude Code (claude), Codex (codex) or Grok Build (grok), not {app}"));
    }
    let mode = text(v, "mode").filter(|m| !m.is_empty() && m != "manual" && m != "default");
    if let Some(m) = &mode {
        if m == "bypassPermissions" {
            return Err("XenoNet never starts a session that bypasses its permission checks".into());
        }
        if !MODES.contains(&m.as_str()) {
            return Err(format!("the mode is one of {}, not {m}", MODES.join(", ")));
        }
        if app == "codex" {
            return Err("Codex runs with its own config.toml's sandbox and approvals: leave the mode out".into());
        }
    }
    let prompt = text(v, "prompt").map(|p| p.trim().to_string()).unwrap_or_default();
    if prompt.is_empty() {
        return Err("say what the new session should do: a prompt".into());
    }
    if prompt.chars().count() > PROMPT_MAX {
        return Err(format!("a first prompt is at most {PROMPT_MAX} characters"));
    }
    let folder = text(v, "folder").map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).unwrap_or_else(|| "~".into());
    if folder.chars().count() > 1000 {
        return Err("that folder is too long to be one".into());
    }
    let delegate = v.get("delegate").and_then(Value::as_bool).unwrap_or(false);
    Ok(Request { folder, app, mode, prompt, delegate })
}

/// One-line text for a title, without what could break the framing.
fn title_of(t: &str) -> String {
    let one = super::policy::sanitize(t, true, 120.0);
    one.replace(['«', '»'], "\"")
}

/// What a delegated session is told after who asked: what XenoNet does
/// for it, and what stays off limits whatever the delegation.
pub const DELEGATED_RULES: &str = "Within that delegation XenoNet answers your permission prompts for reading, for editing files under this folder and for routine commands, and sends anything else to your user's console as a question, so you needn't ask first. Never delete data, print or move secrets, or spend money.";

/// The first prompt: who asked, then the prompt. `by` is `{ kind, id,
/// title }`; `from` is the machine it came from, None for this one.
/// `delegation` is the machine that put this one on standing delegation
/// and the day (a session or actor starts under it): the prompt then says
/// the asker's authority is your user's, checked here, not claimed.
pub fn framed(by: &Value, from: Option<&str>, prompt: &str, delegation: Option<(&str, &str)>) -> String {
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    let mut rules = String::new();
    let line = if kind == "you" {
        match from {
            Some(h) => format!("[XenoNet] Started by you, at {h}'s console."),
            None => "[XenoNet] Started by you, at this machine's console.".to_string(),
        }
    } else {
        let id = super::policy::sanitize(by.get("id").and_then(Value::as_str).unwrap_or(""), true, 200.0);
        let title = title_of(by.get("title").and_then(Value::as_str).unwrap_or(&id));
        let what = if kind == "actor" { "the actor" } else { "the session" };
        let tail = match delegation {
            Some((host, date)) => {
                rules = format!("{DELEGATED_RULES}\n\n");
                format!(
                    " Your user delegated it: they put this machine on standing delegation at {host}'s console on {date}, which XenoNet here checked; the asker can't claim it."
                )
            }
            None => " It came from an agent, not from your user.".to_string(),
        };
        match from {
            Some(h) => format!("[XenoNet] {h} says this was asked by {what} «{title}» ({id}).{tail}"),
            None => format!("[XenoNet] This was asked by {what} «{title}» ({id}) on this machine.{tail}"),
        }
    };
    format!("{line}\n\n{rules}{prompt}")
}

/// Whether the app reads a turn's prompt from a file XenoNet writes, as its
/// adapter says, rather than from standard input.
pub fn prompt_in_file(app: &str) -> bool {
    apps::of(app).is_some_and(|w| w.prompt_file)
}

/// The app and arguments that start a new session `id` (a UUID XenoNet
/// picks, so the answer can name it at once), in `cwd`, as the app's
/// adapter gives them. `settings` is the listen file a session waits for
/// messages with after each turn; `server` the xenonet MCP server's name
/// in its settings, whose permission tool answers its prompts;
/// `prompt_file` where the prompt was written for an app that reads it
/// from a file. Or why there's no such command: the app's adapter has no
/// start.
pub fn command(r: &Request, id: &str, cwd: &str, settings: Option<&str>, server: &str, prompt_file: Option<&str>) -> Result<(String, Vec<String>), String> {
    let start = apps::of(&r.app).and_then(|w| w.start).ok_or_else(|| format!("XenoNet has no way to start a {} session here", r.app))?;
    Ok(start(&Start { id, cwd, mode: r.mode.as_deref(), settings, server, prompt_file }))
}

/// Who asked a start here, in the starts' history: one asker on this
/// machine (a session, an actor, you) by its id, apart from the others.
pub fn asker(id: &str) -> String {
    format!("here:{id}")
}

/// Whether a start may go now, by the starts in the last hour (when, and
/// who asked: another machine by its name, or one asker here, `asker`,
/// each with its own ten): None, or why not.
pub fn admit(history: &[(f64, String)], from: &str, now: f64) -> Option<String> {
    let recent: Vec<&(f64, String)> = history.iter().filter(|(at, _)| now - at < HOUR_MS).collect();
    if recent.len() >= PER_HOUR {
        return Some(format!("this machine starts at most {PER_HOUR} sessions an hour"));
    }
    if recent.iter().filter(|(_, h)| h == from).count() < PER_HOST_HOUR {
        return None;
    }
    let who = match from.strip_prefix("here:") {
        Some("you") => "you ask".to_string(),
        Some(id) => format!("{id} asks"),
        None if from.is_empty() => "this machine asks".to_string(),
        None => format!("{from} asks"),
    };
    Some(format!("{who} for at most {PER_HOST_HOUR} sessions an hour here"))
}

/// The history worth keeping: the last hour's.
pub fn kept(history: &[(f64, String)], now: f64) -> Vec<(f64, String)> {
    history.iter().filter(|(at, _)| now - at < HOUR_MS).cloned().collect()
}

fn trim_end(p: &str) -> &str {
    let t = p.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        p
    } else {
        t
    }
}

/// The folder a request names, as a path on this machine: ~ (home), a
/// project or folder by its id on the map (`known`: [id, path]), or a path
/// the machine reports (a project's or folder's path, or a session's
/// folder, in `reported`), with ~ for home. On Windows (`windows`) a path
/// takes its separator, and paths compare as its file system does, either
/// slash and any case alike. Or why it's none of those.
pub fn folder(asked: &str, home: &str, known: &[(String, String)], reported: &[String], windows: bool) -> Result<String, String> {
    let asked = asked.trim();
    if asked.is_empty() || asked == "~" {
        return Ok(home.to_string());
    }
    if let Some((_, path)) = known.iter().find(|(id, _)| id == asked) {
        return Ok(path.clone());
    }
    let expand = |p: &str| match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) if windows => format!("{}\\{}", trim_end(home), rest.replace('/', "\\")),
        Some(rest) => format!("{}/{rest}", trim_end(home)),
        None => p.to_string(),
    };
    let key = |p: &str| {
        let t = trim_end(&expand(p)).to_string();
        if windows {
            t.replace('/', "\\").to_lowercase()
        } else {
            t
        }
    };
    let want = key(asked);
    let same = |p: &str| key(p) == want;
    if let Some((_, path)) = known.iter().find(|(_, p)| same(p)) {
        return Ok(path.clone());
    }
    if let Some(p) = reported.iter().find(|p| same(p)) {
        return Ok(expand(p));
    }
    Err(format!("{asked} isn't a project or folder on this machine's map: name one by its id or path, or ~"))
}

/// Whether a run's output says Claude Code isn't signed in.
pub fn signed_out(log: &str) -> bool {
    crate::re!(r"(?i)not logged in|oauth (session|token) (has )?expired|please run /login|invalid api key|authentication_error|token has expired|login required").is_match(log)
}

fn uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The session a run's output names: Claude Code's session_id (its first
/// stream-json event, or its result), Codex's thread_id (thread.started).
pub fn session_of(app: &str, log: &str) -> Option<String> {
    let key = if app == "codex" { "thread_id" } else { "session_id" };
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if let Some(id) = v.get(key).and_then(Value::as_str).filter(|id| uuid(id)) {
            return Some(format!("{app}:{id}"));
        }
    }
    None
}

/// How the end of one turn of a started run reads in its output: what the
/// session said last, whether it failed, and the tools it was denied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub text: String,
    pub error: bool,
    pub denied: Vec<String>,
}

fn clean(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

/// The last turn that ended in these lines of a run's output (a Claude Code
/// stream's result, Codex's last message at a turn.completed, or the
/// result Grok Build prints), if any ended.
pub fn finish(app: &str, log: &str) -> Option<Finish> {
    let mut last_message = String::new();
    let mut out = None;
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        if app == "codex" {
            match kind {
                "item.completed" if v["item"]["type"] == "agent_message" => last_message = clean(&v["item"]["text"]),
                "turn.completed" => out = Some(Finish { text: std::mem::take(&mut last_message), error: false, denied: Vec::new() }),
                "turn.failed" | "error" => {
                    let why = v.pointer("/error/message").or_else(|| v.get("message")).map(clean).unwrap_or_default();
                    out = Some(Finish { text: if why.is_empty() { std::mem::take(&mut last_message) } else { why }, error: true, denied: Vec::new() });
                }
                _ => {}
            }
        } else if kind == "result" || (app == "grok" && v.get("result").is_some_and(Value::is_string)) {
            let denied = v
                .get("permission_denials")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|d| d.get("tool_name").and_then(Value::as_str)).map(str::to_string).collect())
                .unwrap_or_default();
            let error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false) || v.get("subtype").and_then(Value::as_str).is_some_and(|t| t.starts_with("error"));
            out = Some(Finish { text: clean(&v["result"]), error, denied });
        }
    }
    out
}

/// Whether the session called XenoNet's send tool in these lines of its
/// output: it told someone what it found itself.
pub fn reported(app: &str, log: &str) -> bool {
    log.lines().any(|line| {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return false };
        if app == "codex" {
            return v["item"]["type"] == "mcp_tool_call" && v["item"]["tool"] == "send" && v["item"]["server"].as_str().is_some_and(|s| s.starts_with("xenonet"));
        }
        v["message"]["content"]
            .as_array()
            .is_some_and(|c| c.iter().any(|b| b["type"] == "tool_use" && b["name"].as_str().is_some_and(|n| n.starts_with("mcp__xenonet") && n.ends_with("__send"))))
    })
}

/// Whether a final message asks something and waits: a question in its last
/// lines, or an offer to go on when told to.
fn asks(text: &str) -> bool {
    let tail: String = text.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
    tail.contains('?') || crate::re!(r"(?i)\b(want me to|shall i|should i|let me know|say so and i|tell me (if|whether|which))\b").is_match(&tail)
}

/// What a turn came to, from its end: failed, denied a tool, asked
/// something, finished (and reported back itself, or only said so in its
/// last message), or silent when it left nothing.
pub fn outcome(f: Option<&Finish>, reported: bool) -> &'static str {
    let Some(f) = f else { return "silent" };
    if f.error {
        "failed"
    } else if !f.denied.is_empty() {
        "denied"
    } else if f.text.is_empty() {
        "silent"
    } else if asks(&f.text) {
        "asked"
    } else if reported {
        "reported"
    } else {
        "finished"
    }
}

/// The most of a session's last words a notice carries.
pub const LAST_MAX: usize = 1200;

/// The words clipped for a notice, to whole characters.
pub fn clipped(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= LAST_MAX {
        return t.to_string();
    }
    let head: String = t.chars().take(LAST_MAX).collect();
    format!("{}…", head.trim_end())
}

/// What the requester is told of an answer: started, refused or ended, on
/// `host` ("this machine" for this one). `a` is the receipt's body. An end
/// says what the session did (`outcome`: finished, reported, asked,
/// denied, failed or silent) and quotes its last words (`last`), never
/// only that a process exited.
pub fn answer_words(state: &str, host: &str, a: &Value) -> String {
    let session = a.get("session").and_then(Value::as_str);
    let reason = a.get("reason").and_then(Value::as_str).unwrap_or("");
    let on = if host == "this machine" { String::new() } else { format!(" on {host}") };
    match state {
        "started" => {
            let folder = a.get("folder").and_then(Value::as_str).map(|f| format!(", in {f}")).unwrap_or_default();
            match session {
                Some(s) => format!("[XenoNet] Started {s}{on}{folder}. Message it as you would any session."),
                None => format!("[XenoNet] Started a session{on}{folder}."),
            }
        }
        "ended" => {
            let who = format!("{}{on}", session.unwrap_or("The session"));
            let minutes = a.get("minutes").and_then(Value::as_f64).unwrap_or(0.0);
            let after = format!(" after {minutes} min");
            let last = a.get("last").and_then(Value::as_str).unwrap_or("").trim();
            let quote = if last.is_empty() { String::new() } else { format!("\nIts last words: «{last}»") };
            let tools = a.get("denied").and_then(Value::as_array).map(|d| d.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let because = if reason.is_empty() { String::new() } else { format!(": {reason}") };
            let exit = a.get("exit").and_then(Value::as_f64).map_or_else(|| "stopped".to_string(), |e| format!("exit {e}"));
            let head = match a.get("outcome").and_then(Value::as_str) {
                Some("asked") => format!("{who} stopped to ask something, and has done nothing more{after}. Answer it by messaging it: it takes up from there.{quote}"),
                Some("denied") => format!("{who} stopped{after} for want of permission to use {tools}, which it was denied.{quote}"),
                Some("failed") => format!("{who} failed{after} ({exit}){because}.{quote}"),
                Some("reported") => format!("{who} finished{after} and reported back itself.{quote}"),
                Some("finished") => format!("{who} finished{after}, but sent no report of its own.{quote}"),
                Some("silent") => format!("{who} ended{after} without a word: it sent no report and left no last message ({exit}){because}."),
                _ => format!("{who} ended{after} ({exit}){because}. That machine's XenoNet is older and can't say whether it finished or stopped to ask something, so don't take it for done: update XenoNet there (xenonet update tools, then xenonet update service)."),
            };
            format!("[XenoNet] {head}")
        }
        _ => format!("[XenoNet] No session was started{on}: {reason}."),
    }
}

/// What the Allow card says of a call that may read the run's environment:
/// Claude Code passes its own environment to its tools, so a token given
/// to XenoNet's runs (`xenonet runs sign-in`) is readable there.
pub const ENV_WARNING: &str =
    "This may read the run's environment, which holds the Claude Code sign-in token you gave XenoNet's runs, if you gave one. Deny it unless the session may see that token.";

/// Where your computer's credential store keeps the long-lived Claude Code
/// token XenoNet's runs sign in with (`claude setup-token`'s), which goes
/// to them alone as CLAUDE_CODE_OAUTH_TOKEN. The store takes names without
/// quotes or backslashes (`secret_store::TARGET`).
pub const SIGN_IN_TARGET: &str = "XenoNet/Claude Code runs";

/// A token as you pasted it, trimmed, or why it isn't one: what `claude
/// setup-token` prints, one line with no spaces.
pub fn sign_in_token(pasted: &str) -> Result<String, String> {
    let t = pasted.trim();
    if t.is_empty() {
        return Err("paste the token claude setup-token printed".into());
    }
    if !t.starts_with("sk-ant-") || t.len() < 20 || t.len() > 400 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("that isn't a Claude Code token: run claude setup-token, and paste the line it prints".into());
    }
    Ok(t.to_string())
}

/// Why a start that ended or was refused needs a sign-in, in words.
pub fn sign_in_words(host: &str) -> String {
    format!("Claude Code on {host} needs a sign-in for XenoNet's runs: run xenonet runs sign-in there")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_says_its_app_mode_folder_and_prompt() {
        let r = request(&json!({ "prompt": "  Fix the login form  " })).unwrap();
        assert_eq!(r, Request { folder: "~".into(), app: "claude".into(), mode: None, prompt: "Fix the login form".into(), delegate: false });
        assert_eq!(request(&json!({ "app": "grok", "mode": "plan", "folder": "project:atlas", "prompt": "x" })).unwrap().mode.as_deref(), Some("plan"));
        assert_eq!(request(&json!({ "mode": "manual", "prompt": "x" })).unwrap().mode, None, "manual is the person's own settings");
        assert_eq!(request(&json!({ "mode": "bypassPermissions", "prompt": "x" })), Err("XenoNet never starts a session that bypasses its permission checks".into()));
        assert!(request(&json!({ "mode": "yolo", "prompt": "x" })).is_err());
        assert!(request(&json!({ "app": "codex", "mode": "plan", "prompt": "x" })).is_err(), "Codex takes its own config's");
        assert!(request(&json!({ "app": "opencode", "prompt": "x" })).is_err());
        assert_eq!(request(&json!({ "prompt": " " })), Err("say what the new session should do: a prompt".into()));
        assert!(request(&json!({ "prompt": "x".repeat(PROMPT_MAX + 1) })).is_err());
        let back = request(&r.to_json()).unwrap();
        assert_eq!(back, r);
        // Delegated work says so, and a plain request doesn't carry the word.
        let d = request(&json!({ "prompt": "x", "delegate": true })).unwrap();
        assert!(d.delegate && d.to_json()["delegate"] == json!(true));
        assert!(!r.delegate && r.to_json().get("delegate").is_none());
        assert_eq!(request(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn the_first_prompt_says_who_asked() {
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(framed(&you, Some("inix"), "Go.", None), "[XenoNet] Started by you, at inix's console.\n\nGo.");
        assert_eq!(framed(&you, None, "Go.", None), "[XenoNet] Started by you, at this machine's console.\n\nGo.");
        let s = json!({ "kind": "session", "id": "claude:aaaaaaaa-0001", "title": "Fix «it»\n\u{1b}[31mnow" });
        assert_eq!(
            framed(&s, Some("inix"), "Go.", None),
            "[XenoNet] inix says this was asked by the session «Fix \"it\" now» (claude:aaaaaaaa-0001). It came from an agent, not from your user.\n\nGo."
        );
        let a = json!({ "kind": "actor", "id": "actor:muse" });
        assert!(framed(&a, None, "Go.", None).starts_with("[XenoNet] This was asked by the actor «actor:muse» (actor:muse) on this machine."));
    }

    #[test]
    fn a_delegated_start_says_your_user_delegated_it_and_what_stays_off_limits() {
        let s = json!({ "kind": "session", "id": "claude:aaaaaaaa-0001", "title": "Update" });
        let text = framed(&s, Some("inix"), "Go.", Some(("inix", "2026-11-01")));
        assert!(text.starts_with("[XenoNet] inix says this was asked by the session «Update» (claude:aaaaaaaa-0001). Your user delegated it: they put this machine on standing delegation at inix's console on 2026-11-01, which XenoNet here checked; the asker can't claim it.\n\n"));
        assert!(text.ends_with(&format!("{DELEGATED_RULES}\n\nGo.")));
        assert!(!text.contains("not from your user"), "the agent line is gone: the delegation is your user's");
        // A start by you says what it said, delegation or not.
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(framed(&you, Some("inix"), "Go.", Some(("inix", "2026-11-01"))), "[XenoNet] Started by you, at inix's console.\n\nGo.");
    }

    #[test]
    fn each_app_starts_its_own_way_and_the_prompt_never_goes_as_an_argument() {
        xenonet_platforms::install();
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let r = |app: &str, mode: Option<&str>| Request { folder: "~".into(), app: app.into(), mode: mode.map(str::to_string), prompt: "-rm -rf".into(), delegate: false };
        let (app, args) = command(&r("claude", Some("plan")), id, "/w/atlas", Some("/s/listen.json"), "xenonet", None).unwrap();
        assert_eq!(app, "claude");
        assert_eq!(
            args,
            [
                "-p",
                "--session-id",
                id,
                "--output-format",
                "stream-json",
                "--verbose",
                "--settings",
                "/s/listen.json",
                "--permission-mode",
                "plan",
                "--permission-prompt-tool",
                "mcp__xenonet__permission"
            ]
        );
        assert_eq!(
            command(&r("claude", None), id, "/w", None, "xenonet-dev", None).unwrap().1,
            ["-p", "--session-id", id, "--output-format", "stream-json", "--verbose", "--permission-prompt-tool", "mcp__xenonet-dev__permission"]
        );
        assert_eq!(command(&r("codex", None), id, "/w/atlas", None, "xenonet", None).unwrap().1, ["exec", "--json", "-C", "/w/atlas", "-"]);
        assert_eq!(
            command(&r("grok", Some("manual")), id, "/w", None, "xenonet", Some("/s/runs/p.txt")).unwrap().1,
            ["--prompt-file", "/s/runs/p.txt", "-s", id, "--output-format", "json", "--permission-mode", "default"]
        );
        for app in APPS {
            assert!(!command(&r(app, None), id, "/w", None, "xenonet", Some("/p")).unwrap().1.iter().any(|a| a.contains("rm -rf")), "{app}");
        }
        assert!(prompt_in_file("grok") && !prompt_in_file("claude") && !prompt_in_file("codex"));
        assert!(command(&r("opencode", None), id, "/w", None, "xenonet", None).is_err(), "no start in its adapter");
    }

    #[test]
    fn twenty_starts_an_hour_here_and_ten_from_one_machine() {
        let now = 10_000_000.0;
        let mut h: Vec<(f64, String)> = (0..10).map(|i| (now - f64::from(i) * 1000.0, "inix".to_string())).collect();
        assert_eq!(admit(&h, "inix", now).as_deref(), Some("inix asks for at most 10 sessions an hour here"));
        assert_eq!(admit(&h, "", now), None, "this machine has its own ten");
        h.extend((0..10).map(|_| (now - 5.0, String::new())));
        assert_eq!(admit(&h, "studio", now).as_deref(), Some("this machine starts at most 20 sessions an hour"));
        assert_eq!(admit(&h, "studio", now + HOUR_MS), None, "an hour on");
        assert_eq!(kept(&h, now + HOUR_MS - 6000.0).len(), 16, "six of inix's and this machine's ten");
        // Each asker here has its own ten: one session's don't hold
        // another's, or yours.
        let a = asker("claude:a");
        let mine: Vec<(f64, String)> = (0..10).map(|_| (now, a.clone())).collect();
        assert_eq!(admit(&mine, &a, now).as_deref(), Some("claude:a asks for at most 10 sessions an hour here"));
        assert_eq!(admit(&mine, &asker("claude:b"), now), None);
        assert_eq!(admit(&mine, &asker("you"), now), None);
        let yours: Vec<(f64, String)> = (0..10).map(|_| (now, asker("you"))).collect();
        assert_eq!(admit(&yours, &asker("you"), now).as_deref(), Some("you ask for at most 10 sessions an hour here"));
    }

    #[test]
    fn a_folder_is_home_a_map_id_or_a_path_the_machine_reports() {
        let known = vec![("project:atlas".to_string(), "/home/jo/src/atlas".to_string()), ("zone:notes".to_string(), "/home/jo/notes".to_string())];
        let reported = vec!["~/work/ledger".to_string()];
        let f = |a: &str| folder(a, "/home/jo", &known, &reported, false);
        assert_eq!(f("~"), Ok("/home/jo".into()));
        assert_eq!(f(""), Ok("/home/jo".into()));
        assert_eq!(f("project:atlas"), Ok("/home/jo/src/atlas".into()));
        assert_eq!(f("~/src/atlas/"), Ok("/home/jo/src/atlas".into()));
        assert_eq!(f("/home/jo/notes"), Ok("/home/jo/notes".into()));
        assert_eq!(f("~/work/ledger"), Ok("/home/jo/work/ledger".into()));
        assert_eq!(f("/home/jo/work/ledger"), Ok("/home/jo/work/ledger".into()));
        assert_eq!(f("/etc"), Err("/etc isn't a project or folder on this machine's map: name one by its id or path, or ~".into()));
        assert!(f("~/src").is_err(), "a folder above a project isn't one");
        assert!(f("/HOME/jo/notes").is_err(), "case counts, off Windows");
        // On Windows: its separator, and either slash and any case alike.
        let known = vec![("project:atlas".to_string(), "C:\\Users\\jo\\src\\Atlas".to_string())];
        let reported = vec!["~\\work\\Ledger".to_string()];
        let w = |a: &str| folder(a, "C:\\Users\\jo", &known, &reported, true);
        assert_eq!(w("~/work/ledger"), Ok("C:\\Users\\jo\\work\\Ledger".into()));
        assert_eq!(w("c:/users/jo/src/atlas/"), Ok("C:\\Users\\jo\\src\\Atlas".into()));
        assert_eq!(w("~\\SRC\\atlas"), Ok("C:\\Users\\jo\\src\\Atlas".into()));
        assert!(w("~/src").is_err());
    }

    #[test]
    fn a_run_says_its_session_and_whether_claude_code_is_signed_out() {
        let claude = "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"0190d3e0-0000-7000-8000-00000000ab02\"}\n{\"type\":\"result\"}";
        assert_eq!(session_of("claude", claude).as_deref(), Some("claude:0190d3e0-0000-7000-8000-00000000ab02"));
        let codex = "{\"type\":\"thread.started\",\"thread_id\":\"019a0000-aaaa-bbbb-cccc-000000000001\"}";
        assert_eq!(session_of("codex", codex).as_deref(), Some("codex:019a0000-aaaa-bbbb-cccc-000000000001"));
        assert_eq!(session_of("claude", "not json\n{\"session_id\":\"../x\"}"), None);
        assert!(signed_out("Error: OAuth session expired. Please run /login"));
        assert!(signed_out("Not logged in · Please run /login"));
        assert!(!signed_out("{\"type\":\"result\",\"subtype\":\"success\"}"));
    }

    #[test]
    fn the_answers_in_words() {
        let a = json!({ "session": "claude:0190d3e0-0000-7000-8000-00000000ab01", "folder": "~/src/atlas" });
        assert_eq!(
            answer_words("started", "neo-rex", &a),
            "[XenoNet] Started claude:0190d3e0-0000-7000-8000-00000000ab01 on neo-rex, in ~/src/atlas. Message it as you would any session."
        );
        assert_eq!(
            answer_words("refused", "neo-rex", &json!({ "reason": "only you start sessions here" })),
            "[XenoNet] No session was started on neo-rex: only you start sessions here."
        );
        let ended = json!({ "session": "claude:x", "exit": 0, "minutes": 4 });
        assert_eq!(
            answer_words("ended", "this machine", &ended),
            "[XenoNet] claude:x ended after 4 min (exit 0). That machine's XenoNet is older and can't say whether it finished or stopped to ask something, so don't take it for done: update XenoNet there (xenonet update tools, then xenonet update service)."
        );
        let ended = |outcome: &str, extra: Value| {
            let mut a = json!({ "session": "claude:x", "exit": 0, "minutes": 5, "outcome": outcome, "last": "All set." });
            a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            answer_words("ended", "neo-rex", &a)
        };
        assert_eq!(ended("asked", json!({ "last": "Want me to go ahead?" })), "[XenoNet] claude:x on neo-rex stopped to ask something, and has done nothing more after 5 min. Answer it by messaging it: it takes up from there.\nIts last words: «Want me to go ahead?»");
        assert_eq!(ended("finished", json!({})), "[XenoNet] claude:x on neo-rex finished after 5 min, but sent no report of its own.\nIts last words: «All set.»");
        assert_eq!(ended("reported", json!({})), "[XenoNet] claude:x on neo-rex finished after 5 min and reported back itself.\nIts last words: «All set.»");
        assert_eq!(
            ended("denied", json!({ "denied": ["Bash", "Edit"] })),
            "[XenoNet] claude:x on neo-rex stopped after 5 min for want of permission to use Bash, Edit, which it was denied.\nIts last words: «All set.»"
        );
        assert_eq!(
            ended("failed", json!({ "exit": 1, "reason": "Claude Code needs a sign-in" })),
            "[XenoNet] claude:x on neo-rex failed after 5 min (exit 1): Claude Code needs a sign-in.\nIts last words: «All set.»"
        );
        assert_eq!(ended("silent", json!({ "last": "" })), "[XenoNet] claude:x on neo-rex ended after 5 min without a word: it sent no report and left no last message (exit 0).");
        assert_eq!(sign_in_words("neo-rex"), "Claude Code on neo-rex needs a sign-in for XenoNet's runs: run xenonet runs sign-in there");
    }

    #[test]
    fn a_sign_in_token_is_what_claude_setup_token_prints() {
        let token = format!("sk-ant-oat01-{}", "a".repeat(80));
        assert_eq!(sign_in_token(&format!("  {token}\n")), Ok(token.clone()));
        assert_eq!(sign_in_token(""), Err("paste the token claude setup-token printed".into()));
        assert!(sign_in_token("hunter2").is_err());
        assert!(sign_in_token(&format!("{token} extra")).is_err(), "one line, no spaces");
    }

    #[test]
    fn a_turns_end_is_read_from_the_runs_output() {
        let claude = r#"{"type":"system","subtype":"init","session_id":"s"}
{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Want me to go ahead? I'll stop here.","permission_denials":[]}
"#;
        let f = finish("claude", claude).unwrap();
        assert_eq!(f, Finish { text: "Want me to go ahead? I'll stop here.".into(), error: false, denied: vec![] });
        assert_eq!(outcome(Some(&f), false), "asked");
        assert!(finish("claude", "{\"type\":\"assistant\"}\n").is_none(), "no end yet");
        let denied = r#"{"type":"result","subtype":"success","is_error":false,"result":"I could not run it.","permission_denials":[{"tool_name":"Bash"},{"tool_name":"Edit"}]}"#;
        let d = finish("claude", denied).unwrap();
        assert_eq!((d.denied.clone(), outcome(Some(&d), true)), (vec!["Bash".to_string(), "Edit".to_string()], "denied"));
        let failed = finish("claude", r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":""}"#).unwrap();
        assert_eq!(outcome(Some(&failed), false), "failed");
        let done = Finish { text: "Updated to 0.6.1.".into(), error: false, denied: vec![] };
        assert_eq!((outcome(Some(&done), false), outcome(Some(&done), true)), ("finished", "reported"));
        assert_eq!(outcome(Some(&Finish { text: String::new(), error: false, denied: vec![] }), false), "silent");
        assert_eq!(outcome(None, true), "silent");
        for q in ["Let me know which one.", "Shall I continue", "ok, say so and I'll do it"] {
            assert!(asks(q), "{q}");
        }
        assert!(!asks("Done. The service restarted."));
    }

    #[test]
    fn codex_ends_a_turn_with_its_last_message() {
        let log = r#"{"type":"thread.started","thread_id":"t"}
{"type":"item.completed","item":{"id":"1","type":"agent_message","text":"Which folder?"}}
{"type":"turn.completed"}
"#;
        assert_eq!(finish("codex", log), Some(Finish { text: "Which folder?".into(), error: false, denied: vec![] }));
        assert_eq!(finish("codex", r#"{"type":"turn.failed","error":{"message":"rate limited"}}"#).map(|f| (f.text, f.error)), Some(("rate limited".into(), true)));
    }

    #[test]
    fn reporting_back_means_calling_the_send_tool() {
        let sent = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__xenonet__send","input":{"to":"claude:a","text":"done"}}]}}"#;
        assert!(reported("claude", sent));
        assert!(reported("claude", &sent.replace("mcp__xenonet__send", "mcp__xenonet-dev__send")));
        assert!(!reported("claude", &sent.replace("__send", "__inbox")));
        assert!(!reported("claude", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"mcp__xenonet__send"}]}}"#));
        assert!(reported("codex", r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"xenonet","tool":"send"}}"#));
    }

    #[test]
    fn last_words_are_clipped_to_whole_characters() {
        assert_eq!(clipped("  hi  "), "hi");
        let long = "é".repeat(LAST_MAX + 5);
        assert_eq!(clipped(&long).chars().count(), LAST_MAX + 1);
    }
}
