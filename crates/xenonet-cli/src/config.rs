//! The parts of core/config.js the command reads: whether it stands for
//! the demo, and XenoNet's folders, as config.js finds them in the
//! command's own process. The demo's are a made-up world's
//! (core/demo.js), which isn't in Rust yet.

use crate::out::Fail;
use crate::pending;
use crate::sys;

/// The folders, and the home folder they're shown under.
pub struct Config {
    pub state_dir: String,
    pub cache_dir: String,
    pub home_dir: String,
}

fn argv_has(flag: &str) -> bool {
    std::env::args().skip(1).any(|a| a == flag)
}

/// config.demo: --demo on the command line, or XENONET_DEMO=1.
pub fn demo() -> bool {
    argv_has("--demo") || std::env::var("XENONET_DEMO").is_ok_and(|v| v == "1")
}

/// The config, or the pending demo world's for --demo.
pub fn config() -> Result<Config, Fail> {
    if demo() {
        return Err(pending::create_world());
    }
    // The command with --dev, and no XENONET_PORT, is XenoNet Dev's.
    let server_dev = argv_has("--dev") && sys::var("XENONET_PORT").is_none();
    let dir = |kind: &str, var: &str| match std::env::var(var) {
        Ok(set) => sys::resolve(&set),
        Err(_) => sys::resolve(&sys::app_dir(kind, if server_dev { "XenoNet Dev" } else { "XenoNet" })),
    };
    Ok(Config { state_dir: dir("state", "XENONET_STATE_DIR"), cache_dir: dir("cache", "XENONET_CACHE_DIR"), home_dir: sys::home() })
}

/// config.claudeProjectsDir: Claude Code's transcripts.
pub fn claude_projects_dir() -> String {
    let base = std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| sys::join(&sys::home(), ".claude"));
    sys::join(&base, "projects")
}

/// config.codexHome: Codex's own folder.
pub fn codex_home() -> String {
    sys::resolve(&std::env::var("CODEX_HOME").unwrap_or_else(|_| sys::join(&sys::home(), ".codex")))
}

/// instanceDir(kind, name): the folder an instance keeps its "state" or
/// "cache" in, or the one XENONET_STATE_DIR or XENONET_CACHE_DIR says
/// (core/workspace.js).
pub fn instance_dir(kind: &str, name: &str) -> String {
    match sys::var(if kind == "state" { "XENONET_STATE_DIR" } else { "XENONET_CACHE_DIR" }) {
        Some(set) => sys::resolve(&set),
        None => sys::app_dir(kind, name),
    }
}

/// standsFor(flags): the instance a command that finds no XenoNet running
/// stands in for, when it isn't the installed XenoNet: XenoNet Dev, when
/// it's the one asked about (core/workspace.js).
pub fn stands_for(pick: &xenonet_base::client::Pick) -> Option<&'static str> {
    let asked = xenonet_base::client::candidates(pick);
    (asked.len() == 1 && asked[0] == f64::from(xenonet_base::client::DEV_PORT)).then_some("XenoNet Dev")
}
