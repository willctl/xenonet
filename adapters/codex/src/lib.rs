//! Codex's adapter (docs/design/direction.md, "Adapters and surfaces"):
//! its processes, its ways in for live delivery, its connect through
//! Codex's hooks and MCP settings, and what opens its threads in the
//! ChatGPT app.
use serde_json::Value;
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{self, Finish, Start, Ways};
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

// Codex names the new thread in thread.started's thread_id.
fn session_of(log: &str) -> Option<String> {
    delivery::id_under("thread_id", log)
}

// Codex's last agent message at a turn.completed, or why a turn failed.
fn finish(log: &str) -> Option<Finish> {
    let mut last_message = String::new();
    let mut out = None;
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "item.completed" if v["item"]["type"] == "agent_message" => last_message = delivery::clean(&v["item"]["text"]),
            "turn.completed" => out = Some(Finish { text: std::mem::take(&mut last_message), error: false, denied: Vec::new() }),
            "turn.failed" | "error" => {
                let why = v.pointer("/error/message").or_else(|| v.get("message")).map(delivery::clean).unwrap_or_default();
                out = Some(Finish { text: if why.is_empty() { std::mem::take(&mut last_message) } else { why }, error: true, denied: Vec::new() });
            }
            _ => {}
        }
    }
    out
}

// A call of a xenonet MCP server's send tool.
fn reported(log: &str) -> bool {
    log.lines().any(|line| {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return false };
        v["item"]["type"] == "mcp_tool_call" && v["item"]["tool"] == "send" && v["item"]["server"].as_str().is_some_and(|s| s.starts_with("xenonet"))
    })
}

// Codex's limits reading: its primary and secondary windows, each with its
// reset; none when it was observed more than six hours ago.
fn plan_used(c: Option<&Value>, now: f64) -> Option<Vec<f64>> {
    const FRESH_MS: f64 = 6.0 * 3600e3;
    if !xenonet_text::text::truthy(c) || now - c.and_then(|c| c.get("observedAt")).filter(|v| !v.is_null()).map_or(0.0, |v| delivery::reading_number(Some(v))) > FRESH_MS {
        return None;
    }
    let mut pcts = Vec::new();
    for w in ["primary", "secondary"] {
        let w = c.and_then(|c| c.get(w));
        if let Some(p) = delivery::finite(w.and_then(|w| w.get("pct"))) {
            if !delivery::reset_passed(w.and_then(|w| w.get("resetsAt")), now) {
                pcts.push(p);
            }
        }
    }
    Some(pcts)
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

    fn opener(&self) -> Option<Opener> {
        Some(open::opener())
    }

    fn delivery(&self) -> Option<Ways> {
        Some(Ways {
            resume: Some(resume),
            start: Some(start),
            marks: Some("CODEX_"),
            picks_id: true,
            no_mode: Some("Codex runs with its own config.toml's sandbox and approvals: leave the mode out"),
            session_of: Some(session_of),
            finish: Some(finish),
            reported: Some(reported),
            plan_used: Some(plan_used),
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
        Some(Box::new(reader::app_server::Codex::new(env)))
    }

    fn ask(&self, a: &xenonet_adapter::platform::Ask<'_>) -> Option<(String, Vec<String>)> {
        Some(asked::ask(a))
    }

    fn doctor_facts(&self, home: &str, env: &xenonet_adapter::dirs::Env, look: &mut dyn xenonet_adapter::dirs::Look) -> Vec<(&'static str, Value)> {
        asked::facts(home, env, look)
    }

    fn presence(&self) -> Option<Box<dyn xenonet_adapter::platform::Presence>> {
        Some(Box::new(asked::Installed))
    }

    fn counted_words(&self, count: &Value) -> Option<String> {
        asked::counted(count)
    }

    fn scratch(&self) -> Option<xenonet_adapter::platform::Scratch> {
        Some(asked::scratch())
    }

    // The map's folder, transcripts, threads' list, renames and sign-in,
    // and the program XenoNet runs its own turns with.

    fn folder_setting(&self) -> Option<&'static str> {
        Some("codexHome")
    }

    fn transcript_dirs(&self, folder: &str) -> Vec<String> {
        ["sessions", "archived_sessions"].map(|d| std::path::Path::new(folder).join(d).to_string_lossy().into_owned()).to_vec()
    }

    fn session_files(&self) -> Option<Box<dyn xenonet_adapter::platform::SessionFiles>> {
        Some(Box::<map::Files>::default())
    }

    fn sessions_gatherer(&self) -> String {
        "codex-threads".into()
    }

    fn keeps_projects(&self) -> bool {
        true
    }

    fn renames(&self) -> bool {
        true
    }

    fn program(&self, env: &xenonet_adapter::dirs::Env, platform: &str, home: &str, look: &mut dyn xenonet_adapter::dirs::Look) -> Option<String> {
        Some(map::program(env, platform, home, look))
    }

    fn signin(&self) -> Option<xenonet_adapter::platform::Signin> {
        Some(map::signin())
    }
}
