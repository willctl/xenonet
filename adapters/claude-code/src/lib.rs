//! Claude Code's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its ways in for live delivery, its connect
//! through Claude Code's settings, and what opens its sessions in Claude
//! desktop.
use serde_json::Value;
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{self, Finish, SignIn, Start, Ways};
use xenonet_adapter::open::Opener;
use xenonet_adapter::processes::Rules;
use xenonet_adapter::reader::{Lister, ListerEnv, Tailed};
use xenonet_adapter::{Check, Platform};

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($p).expect("a pattern that compiles"))
    }};
}

// Claude Code: the CLI, the native install under
// ~/.local/share/claude/versions, or the one Claude desktop runs from its
// claude.app; on Windows claude.exe, but for Claude desktop's own main
// executable, which is called that too: AnthropicClaude\app-<version>\
// claude.exe. The Claude Code Claude desktop runs, from elsewhere in that
// folder, is Claude Code.
fn program(p: &str, base: &str) -> bool {
    base == "claude" || (base == "claude.exe" && !claude_desktop(p)) || re!(r"/claude/versions/[^/]+$").is_match(p)
}

/// Whether a path (with / for \) is Claude desktop's own main executable
/// on Windows.
fn claude_desktop(p: &str) -> bool {
    re!(r"(?i)/AnthropicClaude/app-[^/]+/claude\.exe$").is_match(p)
}

fn script(args: &str) -> bool {
    args.contains("@anthropic-ai/claude-code/cli.js")
}

// claude -p --resume <id> [--settings <listen file>]
fn resume(id: &str, settings: Option<&str>) -> (String, Vec<String>) {
    let mut args = vec!["-p".to_string(), "--resume".to_string(), id.to_string()];
    if let Some(s) = settings {
        args.push("--settings".to_string());
        args.push(s.to_string());
    }
    ("claude".into(), args)
}

// claude -p --session-id <id> --output-format stream-json --verbose
//        [--settings <listen file>] [--permission-mode <m>]
//        --permission-prompt-tool mcp__<server>__permission
// The prompt goes on standard input.
fn start(s: &Start<'_>) -> (String, Vec<String>) {
    let mut args: Vec<String> = ["-p", "--session-id", s.id, "--output-format", "stream-json", "--verbose"].into_iter().map(str::to_string).collect();
    if let Some(f) = s.settings {
        args.extend(["--settings".to_string(), f.to_string()]);
    }
    if let Some(m) = s.mode {
        args.extend(["--permission-mode".to_string(), m.to_string()]);
    }
    args.extend(["--permission-prompt-tool".to_string(), format!("mcp__{}__permission", s.server)]);
    ("claude".into(), args)
}

// claude --version (or -v) only says which Claude Code is installed.
fn probe(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("--version" | "-v"))
}

// A copy of a session a process holds: its whole context under a new id,
// which the stream's first event names.
// claude -p --resume <id> --fork-session --output-format stream-json --verbose [--settings <listen file>]
fn fork(id: &str, settings: Option<&str>) -> (String, Vec<String>) {
    let mut args: Vec<String> = ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose"].into_iter().map(str::to_string).collect();
    if let Some(s) = settings.filter(|s| !s.is_empty()) {
        args.push("--settings".into());
        args.push(s.to_string());
    }
    ("claude".into(), args)
}

// A turn XenoNet runs asks its permission prompts of you through the
// xenonet MCP server's permission tool, as a started session does.
fn permission_tool(server: &str) -> Vec<String> {
    vec!["--permission-prompt-tool".into(), format!("mcp__{server}__permission")]
}

// Whether a run's output says Claude Code isn't signed in.
fn signed_out(log: &str) -> bool {
    re!(r"(?i)not logged in|oauth (session|token) (has )?expired|please run /login|invalid api key|authentication_error|token has expired|login required").is_match(log)
}

// Its session_id: the stream's first event, or its result.
fn session_of(log: &str) -> Option<String> {
    delivery::id_under("session_id", log)
}

// A stream's result event.
fn finish(log: &str) -> Option<Finish> {
    delivery::finish_claude_style(log, false)
}

// Claude Code's limits reading: its five-hour and seven-day percents, each
// with its reset; none while the reading is stale.
fn plan_used(c: Option<&Value>, now: f64) -> Option<Vec<f64>> {
    if !xenonet_text::text::truthy(c) || xenonet_text::text::truthy(c.and_then(|c| c.get("stale"))) {
        return None;
    }
    let mut pcts = Vec::new();
    for k in ["fiveHour", "sevenDay"] {
        if let Some(p) = delivery::finite(c.and_then(|c| c.get(k))) {
            if !delivery::reset_passed(c.and_then(|c| c.get("resets")).and_then(|r| r.get(k)), now) {
                pcts.push(p);
            }
        }
    }
    Some(pcts)
}

// claude --dangerously-load-development-channels server:<name>
fn channel_command(server: &str) -> String {
    format!("claude --dangerously-load-development-channels server:{server}")
}

// claude --settings <listen file>
fn listen_command(file: &str) -> String {
    format!("claude --settings {file}")
}

mod asked;
pub mod checks;
pub mod connect;
pub mod map;
mod open;
pub mod reader;

/// The adapter.
pub struct Adapter(&'static App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        Arc::new(Adapter(app("claude").expect("the registry has claude")))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        self.0
    }

    fn checks(&self) -> Vec<Box<dyn Check>> {
        checks::all()
    }

    fn processes(&self) -> Option<Rules> {
        Some(Rules { program, script: Some(script), pins_by_hook: true, ..Rules::NONE })
    }

    fn opener(&self) -> Option<Opener> {
        Some(open::opener())
    }

    fn delivery(&self) -> Option<Ways> {
        Some(Ways {
            resume: Some(resume),
            start: Some(start),
            probe: Some(probe),
            marks: Some("CLAUDE"),
            channel: true,
            fork: Some(fork),
            permission_tool: Some(permission_tool),
            tool_timeout: Some("MCP_TOOL_TIMEOUT"),
            sign_in: Some(SignIn { variable: "CLAUDE_CODE_OAUTH_TOKEN", signed_out }),
            session_of: Some(session_of),
            finish: Some(finish),
            reported: Some(delivery::reported_claude_style),
            plan_used: Some(plan_used),
            version: Some(("claude", &["--version"])),
            channel_command: Some(channel_command),
            listen_command: Some(listen_command),
            ..Ways::default()
        })
    }

    fn connect(&self, app: &App, shared: &Shared) -> Option<Box<dyn Format>> {
        Some(connect::format(app, shared))
    }

    fn tailed(&self) -> Option<Box<dyn Tailed>> {
        Some(Box::new(reader::Reader::new()))
    }

    fn lister(&self, env: &ListerEnv<'_>) -> Option<Box<dyn Lister>> {
        Some(Box::new(reader::listing::Claude::new(env.root, env.desktop_dir)))
    }

    fn ask(&self, a: &xenonet_adapter::platform::Ask<'_>) -> Option<(String, Vec<String>)> {
        Some(asked::ask(a))
    }

    fn counted_words(&self, count: &Value) -> Option<String> {
        asked::counted(count)
    }

    fn scratch(&self) -> Option<xenonet_adapter::platform::Scratch> {
        Some(asked::scratch())
    }

    // The map's folder, transcripts, renames and sign-in.

    fn folder_setting(&self) -> Option<&'static str> {
        Some("claudeProjectsDir")
    }

    fn transcript_dirs(&self, folder: &str) -> Vec<String> {
        vec![folder.to_string()]
    }

    fn session_files(&self) -> Option<Box<dyn xenonet_adapter::platform::SessionFiles>> {
        Some(Box::new(map::Files))
    }

    fn renames(&self) -> bool {
        true
    }

    fn signin(&self) -> Option<xenonet_adapter::platform::Signin> {
        Some(map::signin())
    }
}
