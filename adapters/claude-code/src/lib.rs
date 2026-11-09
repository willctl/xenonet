//! Claude Code's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its ways in for live delivery, and its
//! connect through Claude Code's settings.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::Ways;
use xenonet_adapter::processes::Rules;
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

pub mod checks;
pub mod connect;

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

    fn delivery(&self) -> Option<Ways> {
        Some(Ways { resume: Some(resume), marks: Some("CLAUDE"), channel: true, server: false })
    }

    fn connect(&self, app: &App, shared: &Shared) -> Option<Box<dyn Format>> {
        Some(connect::format(app, shared))
    }
}
