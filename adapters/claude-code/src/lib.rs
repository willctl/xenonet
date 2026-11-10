//! Claude Code's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its ways in for live delivery, its connect
//! through Claude Code's settings, and what opens its sessions in Claude
//! desktop.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{Start, Ways};
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

pub mod checks;
pub mod connect;
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
        Some(Ways { resume: Some(resume), start: Some(start), probe: Some(probe), marks: Some("CLAUDE"), channel: true, ..Ways::default() })
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
}
