//! Grok Build's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its ways in for live delivery, and its
//! connect through Grok Build's config and hooks files.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{Start, Ways};
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

// Grok Build: the grok binary its installer puts in ~/.grok/downloads and
// links from ~/.grok/bin (as grok, and as agent, which Cursor's CLI is
// called too, so that name counts only there).
fn program(p: &str, base: &str) -> bool {
    base == "grok"
        || base == "grok.exe"
        || re!(r"^grok-(macos|linux|windows)-(aarch64|arm64|x86_64|x64)(\.exe)?$").is_match(base)
        || re!(r"/\.grok/(bin|downloads)/[^/]+$").is_match(p)
}

// grok -r <id> --output-format json --prompt-file <file>: Grok Build reads
// a single turn's prompt from a file, which the deliverer writes and adds.
fn resume(id: &str, _: Option<&str>) -> (String, Vec<String>) {
    ("grok".into(), ["-r", id, "--output-format", "json", "--prompt-file"].into_iter().map(str::to_string).collect())
}

// grok --prompt-file <file> -s <id> --output-format json [--permission-mode <m>]:
// the prompt comes from the file XenoNet wrote; Grok Build's word for the
// person's own settings is default.
fn start(s: &Start<'_>) -> (String, Vec<String>) {
    let mut args: Vec<String> = ["--prompt-file", s.prompt_file.unwrap_or(""), "-s", s.id, "--output-format", "json"].into_iter().map(str::to_string).collect();
    if let Some(m) = s.mode {
        args.extend(["--permission-mode".to_string(), if m == "manual" { "default" } else { m }.to_string()]);
    }
    ("grok".into(), args)
}

pub mod checks;
pub mod connect;
pub mod reader;

/// The adapter.
pub struct Adapter(&'static App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        Arc::new(Adapter(app("grok").expect("the registry has grok")))
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
        Some(Rules { program, ..Rules::NONE })
    }

    fn delivery(&self) -> Option<Ways> {
        Some(Ways { resume: Some(resume), start: Some(start), prompt_file: true, marks: Some("GROK_"), ..Ways::default() })
    }

    fn connect(&self, app: &App, shared: &Shared) -> Option<Box<dyn Format>> {
        Some(connect::format(app, shared))
    }

    fn tailed(&self) -> Option<Box<dyn Tailed>> {
        Some(Box::new(reader::Reader::new()))
    }

    fn lister(&self, env: &ListerEnv<'_>) -> Option<Box<dyn Lister>> {
        Some(Box::new(reader::listing::Grok::new(env.root)))
    }
}
