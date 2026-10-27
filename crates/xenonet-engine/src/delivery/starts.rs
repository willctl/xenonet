//! Starting a session: core/delivery/starts.js. You, a session or an
//! actor asks a machine for a new session of Claude Code, Codex or Grok
//! Build, in a folder there, with a first prompt. Whether the machine
//! takes it is `reach`'s (its start setting); what a request may say, how
//! its first prompt is framed, the command that starts it, the limits and
//! what the answers say are here. Running it is the shared runner's
//! (`runs`), and carrying it to another machine the network's: a message
//! frame to `TARGET`, answered by receipts keyed by its id.
//!
//! ```text
//! claude -p --session-id <id> [--settings <listen file>] [--permission-mode <m>]
//!        --permission-prompt-tool mcp__<server>__permission
//! codex exec --json -C <folder> -
//! grok --prompt-file <file> -s <id> --output-format json [--permission-mode <m>]
//! ```
//!
//! The prompt goes on standard input (Grok Build reads it from a file of
//! XenoNet's), never as an argument. A session never starts bypassing its
//! permission checks.

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
}

impl Request {
    pub fn to_json(&self) -> Value {
        json!({ "folder": self.folder, "app": self.app, "mode": self.mode, "prompt": self.prompt })
    }
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// A request as it came (`{ folder?, app?, mode?, prompt }`), or why it
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
    Ok(Request { folder, app, mode, prompt })
}

/// One-line text for a title, without what could break the framing.
fn title_of(t: &str) -> String {
    let one = super::policy::sanitize(t, true, 120.0);
    one.replace(['«', '»'], "\"")
}

/// The first prompt: who asked, then the prompt. `by` is `{ kind, id,
/// title }`; `from` is the machine it came from, None for this one.
pub fn framed(by: &Value, from: Option<&str>, prompt: &str) -> String {
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    let line = if kind == "you" {
        match from {
            Some(h) => format!("[XenoNet] Started by you, at {h}'s console."),
            None => "[XenoNet] Started by you, at this machine's console.".to_string(),
        }
    } else {
        let id = super::policy::sanitize(by.get("id").and_then(Value::as_str).unwrap_or(""), true, 200.0);
        let title = title_of(by.get("title").and_then(Value::as_str).unwrap_or(&id));
        let what = if kind == "actor" { "the actor" } else { "the session" };
        match from {
            Some(h) => format!("[XenoNet] {h} says this was asked by {what} «{title}» ({id}). It came from an agent, not from your user."),
            None => format!("[XenoNet] This was asked by {what} «{title}» ({id}) on this machine. It came from an agent, not from your user."),
        }
    };
    format!("{line}\n\n{prompt}")
}

/// Grok Build's word for a mode.
fn grok_mode(m: &str) -> &str {
    if m == "manual" {
        "default"
    } else {
        m
    }
}

/// The app and arguments that start a new session `id` (a UUID XenoNet
/// picks, so the answer can name it at once), in `cwd`. `settings` is the
/// listen file a Claude Code session waits for messages with after each
/// turn; `server` the xenonet MCP server's name in its settings, whose
/// permission tool answers its prompts; `prompt_file` where Grok Build
/// reads its prompt.
pub fn command(r: &Request, id: &str, cwd: &str, settings: Option<&str>, server: &str, prompt_file: Option<&str>) -> (String, Vec<String>) {
    let s = |x: &str| x.to_string();
    let mut args = Vec::new();
    match r.app.as_str() {
        "codex" => args.extend([s("exec"), s("--json"), s("-C"), s(cwd), s("-")]),
        "grok" => {
            args.extend([s("--prompt-file"), s(prompt_file.unwrap_or("")), s("-s"), s(id), s("--output-format"), s("json")]);
            if let Some(m) = &r.mode {
                args.extend([s("--permission-mode"), s(grok_mode(m))]);
            }
        }
        _ => {
            args.extend([s("-p"), s("--session-id"), s(id)]);
            if let Some(f) = settings {
                args.extend([s("--settings"), s(f)]);
            }
            if let Some(m) = &r.mode {
                args.extend([s("--permission-mode"), s(m)]);
            }
            args.extend([s("--permission-prompt-tool"), format!("mcp__{server}__permission")]);
        }
    }
    (r.app.clone(), args)
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

/// What the requester is told of an answer: started, refused or ended, on
/// `host` ("this machine" for this one). `a` is the receipt's body.
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
            let who = session.unwrap_or("The session");
            let exit = a.get("exit").and_then(Value::as_f64).map_or_else(|| "stopped".to_string(), |e| format!("exit {e}"));
            let minutes = a.get("minutes").and_then(Value::as_f64).unwrap_or(0.0);
            let tail = if reason.is_empty() { String::new() } else { format!(": {reason}") };
            format!("[XenoNet] {who}{on} ended ({exit}) after {minutes} min{tail}.")
        }
        _ => format!("[XenoNet] No session was started{on}: {reason}."),
    }
}

/// Where your computer's credential store keeps the long-lived Claude Code
/// token XenoNet's runs sign in with (`claude setup-token`'s), which goes
/// to them alone as CLAUDE_CODE_OAUTH_TOKEN.
pub const SIGN_IN_TARGET: &str = "XenoNet/Claude Code token for XenoNet's runs";

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
        assert_eq!(r, Request { folder: "~".into(), app: "claude".into(), mode: None, prompt: "Fix the login form".into() });
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
    }

    #[test]
    fn the_first_prompt_says_who_asked() {
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(framed(&you, Some("inix"), "Go."), "[XenoNet] Started by you, at inix's console.\n\nGo.");
        assert_eq!(framed(&you, None, "Go."), "[XenoNet] Started by you, at this machine's console.\n\nGo.");
        let s = json!({ "kind": "session", "id": "claude:aaaaaaaa-0001", "title": "Fix «it»\n\u{1b}[31mnow" });
        assert_eq!(
            framed(&s, Some("inix"), "Go."),
            "[XenoNet] inix says this was asked by the session «Fix \"it\" now» (claude:aaaaaaaa-0001). It came from an agent, not from your user.\n\nGo."
        );
        let a = json!({ "kind": "actor", "id": "actor:muse" });
        assert!(framed(&a, None, "Go.").starts_with("[XenoNet] This was asked by the actor «actor:muse» (actor:muse) on this machine."));
    }

    #[test]
    fn each_app_starts_its_own_way_and_the_prompt_never_goes_as_an_argument() {
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let r = |app: &str, mode: Option<&str>| Request { folder: "~".into(), app: app.into(), mode: mode.map(str::to_string), prompt: "-rm -rf".into() };
        let (app, args) = command(&r("claude", Some("plan")), id, "/w/atlas", Some("/s/listen.json"), "xenonet", None);
        assert_eq!(app, "claude");
        assert_eq!(args, ["-p", "--session-id", id, "--settings", "/s/listen.json", "--permission-mode", "plan", "--permission-prompt-tool", "mcp__xenonet__permission"]);
        assert_eq!(command(&r("claude", None), id, "/w", None, "xenonet-dev", None).1, ["-p", "--session-id", id, "--permission-prompt-tool", "mcp__xenonet-dev__permission"]);
        assert_eq!(command(&r("codex", None), id, "/w/atlas", None, "xenonet", None).1, ["exec", "--json", "-C", "/w/atlas", "-"]);
        assert_eq!(
            command(&r("grok", Some("manual")), id, "/w", None, "xenonet", Some("/s/runs/p.txt")).1,
            ["--prompt-file", "/s/runs/p.txt", "-s", id, "--output-format", "json", "--permission-mode", "default"]
        );
        for app in APPS {
            assert!(!command(&r(app, None), id, "/w", None, "xenonet", Some("/p")).1.iter().any(|a| a.contains("rm -rf")), "{app}");
        }
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
        assert_eq!(answer_words("ended", "this machine", &ended), "[XenoNet] claude:x ended (exit 0) after 4 min.");
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
}
