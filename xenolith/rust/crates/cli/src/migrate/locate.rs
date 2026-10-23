//! Where agent-wiki kept its files, and where Xenolith keeps its own, for one OS. Everything is
//! computed from an environment map, an OS name and a home folder, never from the running OS, so
//! the Windows layouts are tested on any machine. Mirrors agent-wiki's paths.rs (1.3 and later)
//! and its installer's places.rs, plus the pre-1.3 ~/.agent-wiki.

use std::path::{Path, PathBuf};
use xenolith_core::paths::{AppPaths, Env};

/// agent-wiki's five kinds of folders.
#[derive(Clone, Debug, PartialEq)]
pub struct AwDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub log: PathBuf,
    pub cache: PathBuf,
}

impl AwDirs {
    pub fn all(&self) -> [(&'static str, &Path); 5] {
        [("config", &self.config), ("data", &self.data), ("state", &self.state), ("logs", &self.log), ("cache", &self.cache)]
    }
}

const OVERRIDES: [&str; 6] = ["AGENT_WIKI_HOME", "AGENT_WIKI_CONFIG_DIR", "AGENT_WIKI_DATA_DIR", "AGENT_WIKI_STATE_DIR", "AGENT_WIKI_LOG_DIR", "AGENT_WIKI_CACHE_DIR"];

fn var<'a>(env: &'a Env, k: &str) -> Option<&'a str> {
    env.get(k).map(String::as_str).filter(|v| !v.is_empty())
}

/// An absolute XDG variable (the spec says to ignore a relative one).
fn xdg(env: &Env, k: &str) -> Option<PathBuf> {
    var(env, k).filter(|v| Path::new(v).is_absolute()).map(PathBuf::from)
}

fn resolve(p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() { path } else { std::env::current_dir().map(|d| d.join(&path)).unwrap_or(path) }
}

/// agent-wiki's folders as its programs found them. `overrides`: honour AGENT_WIKI_HOME and
/// AGENT_WIKI_*_DIR (what the programs did); without, the standard folders its installer used.
pub fn aw_dirs(env: &Env, os: &str, home: &Path, overrides: bool) -> AwDirs {
    let portable = var(env, "AGENT_WIKI_HOME").filter(|_| overrides).map(resolve);
    let mut d = if let Some(h) = &portable {
        AwDirs { config: h.clone(), data: h.clone(), state: h.clone(), log: h.join("logs"), cache: h.clone() }
    } else if os == "windows" {
        let roaming = var(env, "APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"));
        let local = var(env, "LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
        let data = local.join("AgentWiki");
        AwDirs { config: roaming.join("AgentWiki"), state: data.join("state"), log: data.join("logs"), cache: data.clone(), data }
    } else if os == "macos" {
        let support = home.join("Library").join("Application Support").join("Agent Wiki");
        let state = xdg(env, "XDG_STATE_HOME").map(|p| p.join("agent-wiki"));
        AwDirs {
            config: xdg(env, "XDG_CONFIG_HOME").map(|p| p.join("agent-wiki")).unwrap_or_else(|| support.clone()),
            data: xdg(env, "XDG_DATA_HOME").map(|p| p.join("agent-wiki")).unwrap_or_else(|| support.clone()),
            log: state.as_ref().map(|s| s.join("logs")).unwrap_or_else(|| home.join("Library").join("Logs").join("Agent Wiki")),
            state: state.unwrap_or_else(|| support.join("state")),
            cache: xdg(env, "XDG_CACHE_HOME").map(|p| p.join("agent-wiki")).unwrap_or_else(|| home.join("Library").join("Caches").join("Agent Wiki")),
        }
    } else {
        let state = xdg(env, "XDG_STATE_HOME").unwrap_or_else(|| home.join(".local").join("state")).join("agent-wiki");
        AwDirs {
            config: xdg(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("agent-wiki"),
            data: xdg(env, "XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("agent-wiki"),
            log: state.join("logs"),
            state,
            cache: xdg(env, "XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")).join("agent-wiki"),
        }
    };
    if overrides {
        for (k, slot) in [
            ("AGENT_WIKI_CONFIG_DIR", &mut d.config),
            ("AGENT_WIKI_DATA_DIR", &mut d.data),
            ("AGENT_WIKI_STATE_DIR", &mut d.state),
            ("AGENT_WIKI_LOG_DIR", &mut d.log),
            ("AGENT_WIKI_CACHE_DIR", &mut d.cache),
        ] {
            if let Some(v) = var(env, k) {
                *slot = resolve(v);
            }
        }
    }
    d
}

/// Every place this migration reads or changes, for one OS.
pub struct Layout {
    pub os: String,
    pub home: PathBuf,
    /// The standard folders agent-wiki's installer used.
    pub standard: AwDirs,
    /// The folders AGENT_WIKI_* variables in this environment point to, when they differ.
    pub overridden: Option<AwDirs>,
    /// The AGENT_WIKI_* variables that are set (names only).
    pub aw_vars: Vec<String>,
    /// ~/.agent-wiki: every version before 1.3 kept everything here.
    pub legacy: PathBuf,
    /// Windows: copies of agent-wiki's folders that an app package captured (MSIX LocalCache).
    pub captured: Vec<PathBuf>,
    pub claude_dir: PathBuf,
    pub claude_md: PathBuf,
    pub claude_settings: PathBuf,
    pub codex_agents: PathBuf,
    pub codex_config: PathBuf,
    pub personal_market: PathBuf,
    /// claude_desktop_config.json files Claude desktop reads (only where the app is installed).
    pub desktop_configs: Vec<PathBuf>,
    pub launch_agents: PathBuf,
    pub systemd_user: PathBuf,
    pub autostart: PathBuf,
    /// Xenolith's own folders, as its installer computes them.
    pub xen: AppPaths,
    pub xen_default_wiki: PathBuf,
    /// Where this migration keeps its journal, backups and log.
    pub work: PathBuf,
}

impl Layout {
    pub fn new(env: &Env, os: &str) -> Layout {
        let home = home_for(env, os);
        let standard = aw_dirs(env, os, &home, false);
        let aw_vars: Vec<String> = OVERRIDES.iter().chain(["AGENT_WIKI_DIR"].iter()).filter(|k| var(env, k).is_some()).map(|k| k.to_string()).collect();
        let over = aw_dirs(env, os, &home, true);
        let overridden = (over != standard).then_some(over);
        let codex_home = var(env, "CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
        let claude_dir = home.join(".claude");
        let config_base = xdg(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        // Xenolith's installer ignores XENOLITH_* overrides; so does the migration.
        let xen_env: Env = env.iter().filter(|(k, _)| !k.starts_with("XENOLITH_")).map(|(k, v)| (k.clone(), v.clone())).collect();
        let xen = AppPaths::for_platform(&xen_env, os, &home);
        Layout {
            captured: captured_copies(env, os, &home),
            desktop_configs: desktop_configs(env, os, &home),
            claude_md: claude_dir.join("CLAUDE.md"),
            claude_settings: claude_dir.join("settings.json"),
            codex_agents: codex_home.join("AGENTS.md"),
            codex_config: codex_home.join("config.toml"),
            personal_market: home.join(".agents").join("plugins").join("marketplace.json"),
            launch_agents: home.join("Library").join("LaunchAgents"),
            // agent-wiki's installer put its units beside its config folder's parent, which is
            // XDG_CONFIG_HOME or ~/.config; its tray's autostart entry went to the same base.
            systemd_user: standard.config.parent().map(|p| p.join("systemd").join("user")).unwrap_or_else(|| config_base.join("systemd").join("user")),
            autostart: config_base.join("autostart"),
            legacy: home.join(".agent-wiki"),
            work: xen.state_dir.join("migrate-agent-wiki"),
            xen_default_wiki: home.join("Xenolith"),
            xen,
            standard,
            overridden,
            aw_vars,
            claude_dir,
            home,
            os: os.to_string(),
        }
    }

    pub fn exe(&self, name: &str) -> String {
        if self.os == "windows" { format!("{name}.exe") } else { name.to_string() }
    }

    /// The folders that may hold agent-wiki's programs and settings, most recent layout first:
    /// the overridden ones, the standard ones, the pre-1.3 folder, the captured copies.
    pub fn data_candidates(&self) -> Vec<PathBuf> {
        let mut out = vec![];
        if let Some(o) = &self.overridden {
            out.push(o.data.clone());
        }
        out.push(self.standard.data.clone());
        out.push(self.legacy.clone());
        out.extend(self.captured.iter().cloned());
        dedup(out)
    }

    pub fn config_candidates(&self) -> Vec<PathBuf> {
        let mut out = vec![];
        if let Some(o) = &self.overridden {
            out.push(o.config.join("config.json"));
        }
        out.push(self.standard.config.join("config.json"));
        out.push(self.legacy.join("config.json"));
        out.extend(self.captured.iter().map(|c| c.join("config.json")));
        dedup(out)
    }

    pub fn state_candidates(&self) -> Vec<PathBuf> {
        let mut out = vec![];
        if let Some(o) = &self.overridden {
            out.push(o.state.join("install-state.json"));
        }
        out.push(self.standard.state.join("install-state.json"));
        out.push(self.legacy.join("install-state.json"));
        out.extend(self.captured.iter().map(|c| c.join("state").join("install-state.json")));
        dedup(out)
    }

    /// Every folder of agent-wiki's own that exists, with what it is (for the plan and --finish).
    pub fn aw_folders(&self) -> Vec<(String, PathBuf)> {
        let mut out: Vec<(String, PathBuf)> = vec![];
        let mut push = |kind: String, p: PathBuf| {
            if p.is_dir() && !out.iter().any(|(_, q)| q == &p) {
                out.push((kind, p));
            }
        };
        if let Some(o) = &self.overridden {
            for (k, p) in o.all() {
                push(format!("{k} (AGENT_WIKI_*)"), p.to_path_buf());
            }
        }
        for (k, p) in self.standard.all() {
            push(k.to_string(), p.to_path_buf());
        }
        push("before 1.3".into(), self.legacy.clone());
        for c in &self.captured {
            push("captured by an app package".into(), c.clone());
        }
        out
    }
}

/// The home folder as xenolith_core::paths::home_dir finds it, for `os` rather than this OS.
pub fn home_for(env: &Env, os: &str) -> PathBuf {
    let k = if os == "windows" { "USERPROFILE" } else { "HOME" };
    if let Some(h) = var(env, k) {
        return PathBuf::from(h);
    }
    match (os, var(env, "HOMEDRIVE"), var(env, "HOMEPATH")) {
        ("windows", Some(d), Some(p)) => PathBuf::from(format!("{d}{p}")),
        _ => PathBuf::from("."),
    }
}

fn dedup(v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for p in v {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Windows: %LOCALAPPDATA%\Packages\<pkg>\LocalCache\{Local,Roaming}\AgentWiki. A process inside an
/// app package (Claude desktop's terminal, Store PowerShell) that wrote under AppData wrote there.
fn captured_copies(env: &Env, os: &str, home: &Path) -> Vec<PathBuf> {
    if os != "windows" {
        return vec![];
    }
    let local = var(env, "LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
    let mut names: Vec<String> = std::fs::read_dir(local.join("Packages")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    let mut out = vec![];
    for n in names {
        for side in ["Local", "Roaming"] {
            let d = local.join("Packages").join(&n).join("LocalCache").join(side).join("AgentWiki");
            if d.is_dir() {
                out.push(d);
            }
        }
    }
    out
}

/// claude_desktop_config.json files, as Xenolith's installer finds them (install::places), for `os`.
fn desktop_configs(env: &Env, os: &str, home: &Path) -> Vec<PathBuf> {
    let mut found = vec![];
    if os == "windows" {
        let local = var(env, "LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
        let mut names: Vec<String> = std::fs::read_dir(local.join("Packages"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.to_lowercase().starts_with("claude_"))
            .collect();
        names.sort();
        for n in names {
            let dir = local.join("Packages").join(n).join("LocalCache").join("Roaming").join("Claude");
            if dir.is_dir() {
                found.push(dir.join("claude_desktop_config.json"));
            }
        }
        let appdata = var(env, "APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming")).join("Claude");
        if found.is_empty() && appdata.is_dir() {
            found.push(appdata.join("claude_desktop_config.json"));
        }
    } else {
        let dir = if os == "macos" { home.join("Library").join("Application Support").join("Claude") } else { home.join(".config").join("Claude") };
        if dir.is_dir() {
            found.push(dir.join("claude_desktop_config.json"));
        }
    }
    found
}
