//! Codex's adapter (docs/design/direction.md, "Adapters and surfaces"):
//! its processes, its ways in for live delivery, its connect through
//! Codex's hooks and MCP settings, and what opens its threads in the
//! ChatGPT app, and where its sessions are written, for the watcher.
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::connect::{Format, Shared};
use xenonet_adapter::delivery::{self, Finish, Start, Ways};
use xenonet_adapter::open::Opener;
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

// Sandboxed, a turn (a start or a resume) is
//   codex exec --sandbox workspace-write --ignore-user-config --ignore-rules
//     -c sandbox_mode="workspace-write" -c approval_policy="never"
//     -c sandbox_workspace_write.network_access=false
//     -c sandbox_workspace_write.writable_roots=[] -c notify=[]
//     -c sandbox_workspace_write.exclude_slash_tmp=true
//     -c sandbox_workspace_write.exclude_tmpdir_env_var=true
//     -c mcp_servers.<xenonet>={...} ...
// Commands write only in the folder and have no network, and nothing is
// asked of anyone. The user's config.toml is not loaded (its notify command
// and MCP servers would run outside the sandbox, and what makes a
// repository's own .codex/config.toml, hooks and rules count is a trust the
// config keeps), nor are execpolicy rules; XenoNet's own MCP server is
// given in its place. `codex exec resume` takes no --sandbox of its own, so
// the sandbox is also given through -c, which it takes, after `resume`.
// Codex's workspace-write sandbox reads the whole disk.
fn sandbox_turn(f: &delivery::SandboxFiles<'_>, args: Vec<String>) -> Vec<String> {
    // TOML strings are JSON strings for the characters a path has.
    let text = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let server = format!("mcp_servers.{}={{command={},args=[\"mcp\"],env={{XENONET_PORT={}}}}}", f.server, text(f.plan.bin), text(f.plan.port));
    let overrides = [
        "sandbox_mode=\"workspace-write\"",
        "approval_policy=\"never\"",
        "sandbox_workspace_write.network_access=false",
        "sandbox_workspace_write.writable_roots=[]",
        "sandbox_workspace_write.exclude_slash_tmp=true",
        "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "notify=[]",
        server.as_str(),
    ];
    let common: Vec<String> =
        ["--ignore-user-config", "--ignore-rules"].into_iter().map(str::to_string).chain(overrides.iter().flat_map(|o| ["-c".to_string(), (*o).to_string()])).collect();
    let mut out = args;
    // After `exec`, before its subcommand or the prompt.
    let at = 1.min(out.len());
    let resumes = out.get(at).map(String::as_str) == Some("resume");
    let mut parent = vec!["--sandbox".to_string(), "workspace-write".to_string()];
    parent.extend(common.iter().cloned());
    out.splice(at..at, parent);
    if resumes {
        // Right after `resume`, whose own options these are.
        let after = out.iter().position(|a| a == "resume").map_or(out.len(), |i| i + 1);
        out.splice(after..after, common);
    }
    out
}

// Codex's sandbox can't name the domains a command may reach: with some,
// the start is refused, and with none its network is off. Its sandbox on
// native Windows isn't one XenoNet has checked, so there it is not asked
// for.
fn sandbox_unavailable(p: &delivery::Plan<'_>, windows: bool) -> Option<String> {
    if windows {
        return Some(
            "Codex's sandbox on native Windows isn't one XenoNet has checked, so a sandboxed Codex start isn't made there: run Codex under WSL2, or use Claude Code under WSL2"
                .to_string(),
        );
    }
    (!p.domains.is_empty()).then(|| "Codex's sandbox can't limit the network to named domains: start it with none (no network at all) or use Claude Code".to_string())
}

fn sandbox_words(p: &delivery::Plan<'_>) -> String {
    format!("It runs in Codex's workspace-write sandbox: its commands write only in {} and have no network. Codex's sandbox lets them read the rest of the disk.", p.folder)
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
pub mod watch;

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
            sandbox: Some(delivery::SandboxWays { unavailable: sandbox_unavailable, files: None, words: sandbox_words, turn: sandbox_turn, reads_whole_disk: true }),
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

    fn watch(&self, env: &ListerEnv<'_>) -> Vec<WatchSpec> {
        watch::specs(env)
    }

    fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
        watch::changed(paths, cursor)
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

    fn root_paths(&self, root: &str) -> Option<xenonet_adapter::platform::RootPaths> {
        Some(xenonet_adapter::platform::RootPaths { folder: root.to_string(), signin: xenonet_os::paths::join(root, "auth.json") })
    }

    fn root_candidates(&self, home: &str, look: &mut dyn xenonet_adapter::dirs::Look) -> Vec<String> {
        xenonet_adapter::platform::sibling_roots(home, ".codex", &["sessions", "auth.json", "config.toml"], look)
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

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::delivery::{Plan, SandboxFiles};

    fn files(domains: &[String]) -> SandboxFiles<'_> {
        SandboxFiles { plan: Plan { folder: "/w/atlas", home: "/h", domains, port: "47840", bin: "/usr/bin/xenonet" }, settings: "", mcp: "", server: "xenonet", not_own: false }
    }

    fn has(args: &[String], flags: &[&str]) -> bool {
        flags.iter().all(|f| args.iter().any(|a| a == f))
    }

    #[test]
    fn a_sandboxed_start_closes_what_would_run_outside_the_sandbox_or_widen_it() {
        let args = sandbox_turn(&files(&[]), ["exec", "--json", "-C", "/w/atlas", "-"].map(str::to_string).to_vec());
        assert_eq!(args[..3], ["exec", "--sandbox", "workspace-write"]);
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(
            has(
                &args,
                &[
                    "--ignore-user-config",
                    "--ignore-rules",
                    "sandbox_mode=\"workspace-write\"",
                    "approval_policy=\"never\"",
                    "notify=[]",
                    "sandbox_workspace_write.writable_roots=[]"
                ]
            ),
            "{args:?}"
        );
        assert!(has(&args, &["sandbox_workspace_write.network_access=false", "sandbox_workspace_write.exclude_slash_tmp=true"]), "{args:?}");
        // The user's servers are not loaded; XenoNet's own is given.
        assert!(has(&args, &["mcp_servers.xenonet={command=\"/usr/bin/xenonet\",args=[\"mcp\"],env={XENONET_PORT=\"47840\"}}"]), "{args:?}");
        // No override widens what it was given, and `-c` always has a value.
        assert!(!args.iter().any(|a| a.contains("danger") || a.contains("full-access")), "{args:?}");
        assert!(args.iter().enumerate().all(|(i, a)| a != "-c" || args.get(i + 1).is_some_and(|v| !v.starts_with('-'))));
    }

    #[test]
    fn a_sandboxed_resume_carries_the_sandbox_where_resume_takes_it() {
        let args = sandbox_turn(&files(&[]), ["exec", "resume", "0190d3e0-0000-7000-8000-0000000d0001", "-"].map(str::to_string).to_vec());
        let at = args.iter().position(|a| a == "resume").unwrap();
        // `codex exec resume` has no --sandbox: the sandbox is in -c after it too.
        assert!(!args[at..].iter().any(|a| a == "--sandbox"), "{args:?}");
        assert!(args[at..].windows(2).any(|w| w[0] == "-c" && w[1] == "sandbox_mode=\"workspace-write\""), "{args:?}");
        assert!(args[at..].windows(2).any(|w| w[0] == "-c" && w[1] == "sandbox_workspace_write.network_access=false"), "{args:?}");
        assert!(args[at..].iter().any(|a| a == "--ignore-user-config") && args[at..].iter().any(|a| a == "--ignore-rules"), "{args:?}");
        assert_eq!(args[args.len() - 2..], ["0190d3e0-0000-7000-8000-0000000d0001", "-"]);
        assert_eq!(args[..3], ["exec", "--sandbox", "workspace-write"]);
    }

    #[test]
    fn codex_has_no_sandbox_for_native_windows_and_none_that_names_domains() {
        let some = vec!["github.com".to_string()];
        let plan = |d| Plan { folder: "/w", home: "/h", domains: d, port: "1", bin: "x" };
        assert!(sandbox_unavailable(&plan(&[]), true).is_some_and(|w| w.contains("WSL2")));
        assert!(sandbox_unavailable(&plan(&some), false).is_some_and(|w| w.contains("named domains")));
        assert!(sandbox_unavailable(&plan(&[]), false).is_none());
    }
}
