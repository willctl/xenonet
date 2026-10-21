//! core/config.js: the settings XenoNet's service runs with, worked out
//! from its command line and environment once, when it starts.
//!
//! What only the JS around it can find is asked for through `Finds`: the
//! made-up world of the demo, Xenolith's (or Agent Wiki's) folder and
//! server, and Claude desktop's folder. Each is asked only where the JS
//! asked it.

use crate::nodepath;
use serde_json::{Map, Value};
use xenonet_base::dirs;
use xenonet_base::text::{js_number, js_trim};

/// The variables of the environment, as process.env has them.
pub type Env = Map<String, Value>;

pub trait Finds {
    /// createWorld() from core/demo.js.
    fn world(&mut self) -> Value;
    /// findContextDir()?.dir from core/xenolith.js.
    fn context_dir(&mut self) -> Option<String>;
    /// contextServerOf(dir) from core/xenolith.js.
    fn context_server(&mut self, dir: Option<&str>) -> Value;
    /// findClaudeDesktopDir() from core/dirs.js.
    fn claude_desktop_dir(&mut self) -> Option<String>;
}

/// A number field, which may be NaN as Number() reads a variable, and the
/// other fields as JSON.
pub enum Field {
    Num(f64),
    Json(Value),
    /// A Set of these strings.
    Set(Vec<String>),
    /// An object of number fields.
    Nums(Vec<(&'static str, f64)>),
}

pub struct Config {
    pub fields: Vec<(&'static str, Field)>,
}

impl Config {
    pub fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, f)| f)
    }
}

pub const DEFAULT_HIDDEN_RESOURCES: &str = "node_repl,cua_repl,codex_app,codex_apps,visualize,terminal,mcp-registry,scheduled-tasks";

fn var<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    // process.env holds only strings; anything else is as if unset.
    env.get(name).and_then(Value::as_str)
}

// `env.X || fallback`: set and not empty.
fn set<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    var(env, name).filter(|v| !v.is_empty())
}

fn number(s: &str) -> f64 {
    js_number(Some(&Value::String(s.into())))
}

// A world's field as `world?.field ?? fallback` reads it.
fn world_str(world: Option<&Value>, key: &str) -> Option<Value> {
    world.and_then(|w| w.get(key)).filter(|v| !v.is_null()).cloned()
}

fn list(text: &str) -> Vec<String> {
    text.split(',').map(|s| js_trim(s).to_string()).filter(|s| !s.is_empty()).collect()
}

/// The settings for this command line (process.argv), environment, home
/// folder and platform.
pub fn config(argv: &[String], env: &Env, home: &str, platform: &str, finds: &mut dyn Finds) -> Config {
    let has = |flag: &str| argv.iter().any(|a| a == flag);
    let demo = has("--demo") || var(env, "XENONET_DEMO") == Some("1");
    let world = if demo { Some(finds.world()) } else { None };
    let world = world.as_ref();
    let server_dev = !demo && has("--dev") && set(env, "XENONET_PORT").is_none();
    let app_dir = |kind: &str, name: &str| {
        let xdg = var(env, if kind == "state" { "XDG_STATE_HOME" } else { "XDG_CACHE_HOME" });
        dirs::app_dir(kind, xdg, var(env, "LOCALAPPDATA"), platform, home, name)
    };
    let dev_dir = |kind: &str| server_dev.then(|| app_dir(kind, "XenoNet Dev"));
    let path = nodepath::NATIVE;

    let wiki_dir: Value = match world {
        Some(w) => w.get("wikiDir").cloned().unwrap_or(Value::Null),
        None => match set(env, "XENONET_WIKI_DIR") {
            Some(d) => Value::String(d.into()),
            None => finds.context_dir().map_or(Value::Null, Value::String),
        },
    };

    let port = number(var(env, "XENONET_PORT").unwrap_or(if demo {
        "47841"
    } else if server_dev {
        "47842"
    } else {
        "47840"
    }));
    let home_dir = world_str(world, "home").unwrap_or_else(|| Value::String(home.into()));
    let projects_dir = world_str(world, "projectsDir")
        .unwrap_or_else(|| Value::String(path.resolve(&[&set(env, "XENONET_PROJECTS_DIR").map_or_else(|| path.join(&[home, "Projects"]), String::from)])));
    let limit = |name: &str, fallback: f64| {
        let n = var(env, name).map_or(f64::NAN, number);
        let n = if n == 0.0 || n.is_nan() { fallback } else { n };
        n.max(1.0)
    };
    let codex_home =
        world_str(world, "codexHome").unwrap_or_else(|| Value::String(path.resolve(&[&var(env, "CODEX_HOME").map_or_else(|| path.join(&[home, ".codex"]), String::from)])));
    let grok_home =
        world_str(world, "grokHome").unwrap_or_else(|| Value::String(path.resolve(&[&var(env, "GROK_HOME").map_or_else(|| path.join(&[home, ".grok"]), String::from)])));
    let claude_desktop_dir = match world {
        Some(w) => w.get("claudeDesktopDir").cloned().unwrap_or(Value::Null),
        None => finds.claude_desktop_dir().map_or(Value::Null, Value::String),
    };
    let poll_ms = var(env, "XENONET_POLL_MS").map_or(8000.0, number);
    let claude_config_dir = var(env, "CLAUDE_CONFIG_DIR");
    let claude_projects_dir =
        world_str(world, "claudeRoot").unwrap_or_else(|| Value::String(path.join(&[&claude_config_dir.map_or_else(|| path.join(&[home, ".claude"]), String::from), "projects"])));
    let numbat_file = match set(env, "XENONET_NUMBAT_FILE") {
        Some(f) if !demo => Value::String(path.resolve(&[f])),
        _ => Value::Null,
    };
    let readers = if demo { Vec::new() } else { list(var(env, "XENONET_READERS").unwrap_or("")) };
    let world_home = world.and_then(|w| w.get("home")).map(xenonet_base::text::js_string);
    let claude_config_file = match &world_home {
        Some(h) => path.join(&[h, ".claude.json"]),
        None => path.join(&[claude_config_dir.unwrap_or(home), ".claude.json"]),
    };
    let gh_config_dir = match world_home {
        Some(h) => path.join(&[&h, ".config", "gh"]),
        None => match var(env, "GH_CONFIG_DIR") {
            Some(d) => d.to_string(),
            None => match (set(env, "XDG_CONFIG_HOME"), set(env, "APPDATA")) {
                (Some(x), _) => path.join(&[x, "gh"]),
                (None, Some(a)) if platform == "win32" => path.join(&[a, "GitHub CLI"]),
                _ => path.join(&[home, ".config", "gh"]),
            },
        },
    };
    let folder = |key: &str, var_name: &str, kind: &str| {
        world_str(world, key).unwrap_or_else(|| {
            let chosen = var(env, var_name).map(String::from).or_else(|| dev_dir(kind)).unwrap_or_else(|| app_dir(kind, "XenoNet"));
            Value::String(path.resolve(&[&chosen]))
        })
    };
    let state_dir = folder("stateDir", "XENONET_STATE_DIR", "state");
    let cache_dir = folder("cacheDir", "XENONET_CACHE_DIR", "cache");
    let wiki_server = if world.is_some() { Value::String("agent-wiki".into()) } else { finds.context_server(wiki_dir.as_str()) };
    let hidden = list(var(env, "XENONET_HIDE_RESOURCES").unwrap_or(DEFAULT_HIDDEN_RESOURCES));
    let active_minutes = var(env, "XENONET_ACTIVE_MINUTES").map_or(60.0, number);

    let off = |name: &str| var(env, name) == Some("off");
    Config {
        fields: vec![
            ("demo", Field::Json(Value::Bool(demo))),
            ("host", Field::Json(Value::String("127.0.0.1".into()))),
            ("port", Field::Num(port)),
            ("homeDir", Field::Json(home_dir)),
            ("projectsDir", Field::Json(projects_dir)),
            ("projectLimits", Field::Nums(vec![("depth", limit("XENONET_PROJECTS_DEPTH", 3.0)), ("maxDirs", limit("XENONET_PROJECTS_MAX_DIRS", 2000.0))])),
            ("codexHome", Field::Json(codex_home)),
            ("grokHome", Field::Json(grok_home)),
            ("claudeDesktopDir", Field::Json(claude_desktop_dir)),
            ("pollMs", Field::Num(poll_ms)),
            ("claudeUsage", Field::Json(Value::Bool(!demo && !off("XENONET_CLAUDE_USAGE")))),
            ("claudeProjectsDir", Field::Json(claude_projects_dir)),
            ("identities", Field::Json(Value::Bool(!off("XENONET_IDENTITIES")))),
            ("actors", Field::Json(Value::Bool(!off("XENONET_ACTORS")))),
            ("numbatFile", Field::Json(numbat_file)),
            ("readers", Field::Set(readers)),
            ("hideIdentities", Field::Json(Value::Bool(var(env, "XENONET_HIDE_IDENTITIES") == Some("1")))),
            ("claudeConfigFile", Field::Json(Value::String(claude_config_file))),
            ("ghConfigDir", Field::Json(Value::String(gh_config_dir))),
            ("stateDir", Field::Json(state_dir)),
            ("cacheDir", Field::Json(cache_dir)),
            ("wikiDir", Field::Json(wiki_dir)),
            ("wikiServer", Field::Json(wiki_server)),
            ("hiddenResources", Field::Set(hidden)),
            ("activeWindowMs", Field::Num(10.0 * 60.0 * 1000.0)),
            ("projectActiveMs", Field::Num(active_minutes * 60.0 * 1000.0)),
            ("recentWindowMs", Field::Num(24.0 * 60.0 * 60.0 * 1000.0)),
        ],
    }
}

// The paths here are spelled as on a unix host; on Windows the JS tests run
// against the Rust (test:rust) check the same settings. (Two attributes:
// clippy takes only a plain cfg(test) for test code.)
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fake {
        asked: Vec<&'static str>,
    }

    impl Finds for Fake {
        fn world(&mut self) -> Value {
            self.asked.push("world");
            json!({ "home": "/w/home", "projectsDir": "/w/projects", "codexHome": "/w/codex", "claudeRoot": "/w/claude", "stateDir": "/w/state", "cacheDir": "/w/cache", "wikiDir": "/w/wiki", "claudeDesktopDir": "/w/desktop" })
        }
        fn context_dir(&mut self) -> Option<String> {
            self.asked.push("context");
            Some("/ctx".into())
        }
        fn context_server(&mut self, dir: Option<&str>) -> Value {
            self.asked.push("server");
            Value::String(format!("server:{}", dir.unwrap_or("-")))
        }
        fn claude_desktop_dir(&mut self) -> Option<String> {
            self.asked.push("desktop");
            None
        }
    }

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| (k.to_string(), Value::String(v.to_string()))).collect()
    }

    fn json_of<'a>(c: &'a Config, key: &str) -> &'a Value {
        match c.get(key) {
            Some(Field::Json(v)) => v,
            _ => panic!("{key} isn't JSON"),
        }
    }

    fn num_of(c: &Config, key: &str) -> f64 {
        match c.get(key) {
            Some(Field::Num(n)) => *n,
            _ => panic!("{key} isn't a number"),
        }
    }

    #[test]
    fn plain_settings_come_from_the_environment() {
        let mut f = Fake { asked: vec![] };
        let c = config(
            &["node".into(), "server/index.js".into()],
            &env(&[("XENONET_PROJECTS_DEPTH", "0"), ("XENONET_POLL_MS", "x"), ("XENONET_READERS", " opencode, ,cursor"), ("XDG_STATE_HOME", "/xdg")]),
            "/home/me",
            "linux",
            &mut f,
        );
        assert_eq!(f.asked, ["context", "desktop", "server"]);
        assert_eq!(num_of(&c, "port"), 47840.0);
        assert!(num_of(&c, "pollMs").is_nan());
        assert_eq!(json_of(&c, "projectsDir"), "/home/me/Projects");
        assert!(matches!(c.get("projectLimits"), Some(Field::Nums(n)) if n == &[("depth", 3.0), ("maxDirs", 2000.0)]));
        assert_eq!(json_of(&c, "wikiDir"), "/ctx");
        assert_eq!(json_of(&c, "wikiServer"), "server:/ctx");
        assert_eq!(json_of(&c, "stateDir"), "/xdg/xenonet");
        assert_eq!(json_of(&c, "ghConfigDir"), "/home/me/.config/gh");
        assert!(matches!(c.get("readers"), Some(Field::Set(r)) if r == &["opencode", "cursor"]));
    }

    #[test]
    fn the_demo_reads_its_made_up_world() {
        let mut f = Fake { asked: vec![] };
        let c = config(&["--demo".into()], &env(&[("XENONET_READERS", "cursor")]), "/home/me", "darwin", &mut f);
        assert_eq!(f.asked, ["world"]);
        assert_eq!(num_of(&c, "port"), 47841.0);
        assert_eq!(json_of(&c, "homeDir"), "/w/home");
        assert_eq!(json_of(&c, "claudeConfigFile"), "/w/home/.claude.json");
        assert_eq!(json_of(&c, "wikiServer"), "agent-wiki");
        assert_eq!(json_of(&c, "claudeUsage"), false);
        assert!(matches!(c.get("readers"), Some(Field::Set(r)) if r.is_empty()));
    }

    #[test]
    fn the_server_alone_from_a_checkout_is_xenonet_devs() {
        let mut f = Fake { asked: vec![] };
        let c = config(&["--dev".into()], &env(&[("XENONET_WIKI_DIR", "/wiki")]), "/Users/me", "darwin", &mut f);
        assert_eq!(f.asked, ["desktop", "server"]);
        assert_eq!(num_of(&c, "port"), 47842.0);
        assert_eq!(json_of(&c, "stateDir"), "/Users/me/Library/Application Support/XenoNet Dev");
    }
}
