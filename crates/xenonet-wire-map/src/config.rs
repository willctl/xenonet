//! The settings the map reads (core/config.js's, through xenonet-app's
//! config): its folders, the projects scan's limits, the poll, the
//! readers that are on, identities and the windows a session counts as
//! active in. Worked out once, after the demo's world has moved the
//! folders.

use serde_json::{json, Map, Value};
use std::collections::HashSet;
use xenonet_app::config::{config, Field, Finds};
use xenonet_base::text::js_string;

#[derive(Clone, Debug)]
pub struct MapConfig {
    pub demo: bool,
    pub host: String,
    pub port: Value,
    pub home_dir: String,
    /// The projects folder XENONET_PROJECTS_DIR or ~/Projects names; the
    /// one in use can be another you chose (workspace.js).
    pub projects_dir: String,
    pub project_limits: Value,
    pub codex_home: String,
    pub claude_desktop_dir: Option<String>,
    pub poll_ms: f64,
    pub claude_usage: bool,
    pub claude_projects_dir: String,
    pub identities: bool,
    pub numbat_file: Option<String>,
    pub readers: HashSet<String>,
    pub hide_identities: bool,
    pub claude_config_file: String,
    pub gh_config_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
    pub wiki_dir: Option<String>,
    pub wiki_server: Value,
    pub hidden_resources: Vec<String>,
    /// activeWindowMs, recentWindowMs and projectActiveMs, as buildGraph
    /// reads them from the config.
    pub windows: Value,
    pub platform: String,
    /// XENONET_PROJECTS_DIR, which locks the projects folder (never in the
    /// demo).
    pub env_projects_dir: Option<String>,
    /// The environment the service runs in.
    pub env: Map<String, Value>,
}

struct Found<'a> {
    world: Option<&'a Value>,
    env: &'a Map<String, Value>,
    home: &'a str,
    platform: &'a str,
}

struct Disk;
impl xenonet_base::dirs::Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        std::path::Path::new(p).exists()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, xenonet_base::dirs::Unreadable> {
        let names = std::fs::read_dir(p).map_err(|_| xenonet_base::dirs::Unreadable)?;
        let mut out: Vec<String> = names.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
        out.sort();
        Ok(out)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)
    }
}

fn var(env: &Map<String, Value>, k: &str) -> Option<String> {
    env.get(k).and_then(Value::as_str).map(str::to_string)
}

/// The environment the desktop apps' folders depend on (dirs.js's envOf).
pub fn dirs_env(env: &Map<String, Value>) -> xenonet_base::dirs::Env {
    xenonet_base::dirs::Env {
        claude_desktop_dir: var(env, "XENONET_CLAUDE_DESKTOP_DIR").filter(|d| !d.is_empty()),
        local_app_data: var(env, "LOCALAPPDATA"),
        app_data: var(env, "APPDATA"),
        xdg_config_home: var(env, "XDG_CONFIG_HOME"),
    }
}

impl Finds for Found<'_> {
    fn world(&mut self) -> Value {
        self.world.cloned().unwrap_or(Value::Null)
    }
    fn context_dir(&mut self) -> Option<String> {
        let env: xenonet_sources::xenolith::Env = self.env.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect();
        xenonet_sources::xenolith::find_context_dir(&env, self.home, self.platform).map(|(d, _)| d)
    }
    fn context_server(&mut self, dir: Option<&str>) -> Value {
        dir.map_or(Value::Null, |d| Value::String(xenonet_sources::xenolith::context_server_of(d).into()))
    }
    fn claude_desktop_dir(&mut self) -> Option<String> {
        xenonet_base::dirs::find_claude_desktop_dir(&dirs_env(self.env), self.platform, self.home, &mut Disk)
    }
}

/// The platform as process.platform names it.
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    }
}

/// The environment as process.env has it.
pub fn process_env() -> Map<String, Value> {
    std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, Value::String(v.into_string().ok()?)))).collect()
}

impl MapConfig {
    /// The map's settings for this command line and environment, with the
    /// demo's world when there is one. `home` is your home folder.
    pub fn new(demo: bool, dev: bool, env: Map<String, Value>, home: &str, world: Option<&Value>) -> MapConfig {
        let platform = platform();
        let mut argv = Vec::new();
        if demo {
            argv.push("--demo".to_string());
        }
        if dev {
            argv.push("--dev".to_string());
        }
        let mut finds = Found { world, env: &env, home, platform };
        let c = config(&argv, &env, home, platform, &mut finds);
        let json = |k: &str| match c.get(k) {
            Some(Field::Json(v)) => v.clone(),
            Some(Field::Num(n)) => xenonet_base::json::number(*n),
            Some(Field::Set(list)) => json!(list),
            Some(Field::Nums(ns)) => Value::Object(ns.iter().map(|(k, v)| (k.to_string(), xenonet_base::json::number(*v))).collect()),
            None => Value::Null,
        };
        let text = |k: &str| match json(k) {
            Value::String(s) => s,
            Value::Null => String::new(),
            v => js_string(&v),
        };
        let maybe = |k: &str| match json(k) {
            Value::String(s) => Some(s),
            _ => None,
        };
        let set = |k: &str| match c.get(k) {
            Some(Field::Set(list)) => list.clone(),
            _ => Vec::new(),
        };
        let numf = |k: &str| match c.get(k) {
            Some(Field::Num(n)) => *n,
            _ => f64::NAN,
        };
        let flag = |k: &str| json(k) == Value::Bool(true);
        let env_projects_dir = if demo { None } else { var(&env, "XENONET_PROJECTS_DIR").filter(|d| !d.is_empty()) };
        MapConfig {
            demo: flag("demo"),
            host: text("host"),
            port: json("port"),
            home_dir: text("homeDir"),
            projects_dir: text("projectsDir"),
            project_limits: json("projectLimits"),
            codex_home: text("codexHome"),
            claude_desktop_dir: maybe("claudeDesktopDir"),
            poll_ms: numf("pollMs"),
            claude_usage: flag("claudeUsage"),
            claude_projects_dir: text("claudeProjectsDir"),
            identities: flag("identities"),
            numbat_file: maybe("numbatFile"),
            readers: set("readers").into_iter().collect(),
            hide_identities: flag("hideIdentities"),
            claude_config_file: text("claudeConfigFile"),
            gh_config_dir: text("ghConfigDir"),
            state_dir: text("stateDir"),
            cache_dir: text("cacheDir"),
            wiki_dir: maybe("wikiDir"),
            wiki_server: json("wikiServer"),
            hidden_resources: set("hiddenResources"),
            windows: json!({ "activeWindowMs": json("activeWindowMs"), "projectActiveMs": json("projectActiveMs"), "recentWindowMs": json("recentWindowMs") }),
            platform: platform.into(),
            env_projects_dir,
            env,
        }
    }

    pub fn hidden(&self) -> HashSet<String> {
        self.hidden_resources.iter().cloned().collect()
    }

    /// The codex binary XenoNet runs (dirs.js's findCodexBinary): the
    /// one XENONET_CODEX_BIN names, else ChatGPT's own, else codex.
    pub fn codex_binary(&self) -> String {
        if let Some(b) = var(&self.env, "XENONET_CODEX_BIN").filter(|b| !b.is_empty()) {
            return b;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        xenonet_base::dirs::bundled_codex_binaries(&dirs_env(&self.env), &self.platform, &home, &mut Disk).into_iter().next().unwrap_or_else(|| "codex".into())
    }

    /// The poll's period: XENONET_POLL_MS, 8 seconds by default; never
    /// under a second, so a mistyped one can't spin.
    pub fn poll(&self) -> std::time::Duration {
        let ms = if self.poll_ms.is_finite() && self.poll_ms > 0.0 { self.poll_ms.max(1000.0) } else { 8000.0 };
        std::time::Duration::from_millis(ms as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_reads_its_world() {
        let world = json!({ "home": "/w/home", "projectsDir": "/w/home/Projects", "codexHome": "/w/home/.codex", "claudeRoot": "/w/home/.claude/projects", "stateDir": "/w/state", "cacheDir": "/w/cache", "wikiDir": "/w/home/wiki", "claudeDesktopDir": "/w/home/claude-desktop" });
        let c = MapConfig::new(true, true, Map::new(), "/home/nova", Some(&world));
        assert!(c.demo && !c.claude_usage);
        assert_eq!((c.home_dir.as_str(), c.projects_dir.as_str(), c.claude_projects_dir.as_str()), ("/w/home", "/w/home/Projects", "/w/home/.claude/projects"));
        assert_eq!(c.claude_config_file, "/w/home/.claude.json");
        assert_eq!(c.wiki_server, "agent-wiki");
        assert_eq!(c.windows["activeWindowMs"], 600000);
        assert!(c.readers.is_empty() && c.numbat_file.is_none());
        let mut env = Map::new();
        env.insert("XENONET_READERS".into(), json!("opencode, cursor"));
        env.insert("XENONET_POLL_MS".into(), json!("10"));
        let c = MapConfig::new(false, false, env, "/home/nova", None);
        assert!(c.readers.contains("opencode") && c.readers.contains("cursor"));
        assert_eq!(c.poll().as_millis(), 1000, "never under a second");
        assert_eq!(c.projects_dir, "/home/nova/Projects");
    }
}
