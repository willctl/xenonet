//! Codex's adapter (docs/design/direction.md, "Adapters and surfaces"):
//! its processes, its ways in for live delivery, and its connect through
//! Codex's hooks and MCP settings.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{Start, Ways};
use xenonet_adapter::processes::Rules;
use xenonet_adapter::reader::Tailed;
use xenonet_adapter::{Check, Platform};

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($p).expect("a pattern that compiles"))
    }};
}

// Codex: the CLI (codex.exe on Windows), or the one the ChatGPT app
// bundles.
fn program(_: &str, base: &str) -> bool {
    base == "codex" || base == "codex.exe" || re!(r"^codex-(aarch64|x86_64)").is_match(base)
}

fn script(args: &str) -> bool {
    args.contains("@openai/codex/")
}

// codex exec resume <id> -
fn resume(id: &str, _: Option<&str>) -> (String, Vec<String>) {
    ("codex".into(), vec!["exec".into(), "resume".into(), id.to_string(), "-".into()])
}

// codex exec --json -C <folder> -: the prompt goes on standard input, and
// Codex names the new thread in its first event.
fn start(s: &Start<'_>) -> (String, Vec<String>) {
    ("codex".into(), ["exec", "--json", "-C", s.cwd, "-"].into_iter().map(str::to_string).collect())
}

pub mod checks;
pub mod connect;
pub mod reader;

/// The adapter.
pub struct Adapter(&'static App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        Arc::new(Adapter(app("codex").expect("the registry has codex")))
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
        Some(Rules { program, script: Some(script), ..Rules::NONE })
    }

    fn delivery(&self) -> Option<Ways> {
        Some(Ways { resume: Some(resume), start: Some(start), marks: Some("CODEX_"), ..Ways::default() })
    }

    fn connect(&self, app: &App, shared: &Shared) -> Option<Box<dyn Format>> {
        Some(connect::format(app, shared))
    }

    fn tailed(&self) -> Option<Box<dyn Tailed>> {
        Some(Box::new(reader::Reader::new()))
    }
}
