//! Grok Build's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its ways in for live delivery, and its
//! connect through Grok Build's config and hooks files, and where its
//! sessions are written, for the watcher.
use std::path::PathBuf;
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{self, Finish, Start, Ways};
use xenonet_adapter::processes::Rules;
use xenonet_adapter::reader::{Lister, ListerEnv, Tailed};
use xenonet_adapter::watch::{Cursor, SessionEvent, WatchSpec};
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

// Its session_id, as Claude Code's stream names one.
fn session_of(log: &str) -> Option<String> {
    delivery::id_under("session_id", log)
}

// A result event, or the line with a result in text that Grok Build prints.
fn finish(log: &str) -> Option<Finish> {
    delivery::finish_claude_style(log, true)
}

mod asked;
pub mod checks;
pub mod connect;
pub mod map;
pub mod reader;
pub mod watch;

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
        Some(Ways {
            resume: Some(resume),
            start: Some(start),
            prompt_file: true,
            marks: Some("GROK_"),
            session_of: Some(session_of),
            finish: Some(finish),
            reported: Some(delivery::reported_claude_style),
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
        Some(Box::new(reader::listing::Grok::new(env.root)))
    }

    fn watch(&self, env: &ListerEnv<'_>) -> Vec<WatchSpec> {
        watch::specs(env)
    }

    fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
        watch::changed(paths, cursor)
    }

    fn counted_words(&self, count: &serde_json::Value) -> Option<String> {
        asked::counted(count)
    }

    // The map's folder and transcripts.

    fn folder_setting(&self) -> Option<&'static str> {
        Some("grokHome")
    }

    fn transcript_dirs(&self, folder: &str) -> Vec<String> {
        vec![std::path::Path::new(folder).join("sessions").to_string_lossy().into_owned()]
    }

    fn session_files(&self) -> Option<Box<dyn xenonet_adapter::platform::SessionFiles>> {
        Some(Box::new(map::Files))
    }
}
