//! core/apps.js: the apps XenoNet knows sessions of, and what's fixed about
//! each (its numbat names, vendor products, folder, context window, account
//! and plan limits, signing team, hook events, edit tools and capabilities),
//! as app/v1 describes an entry.
//!
//! The JS is the source. Its data is src/apps.json, written from it by
//! `node scripts/apps.mjs` (and checked with `--check`), embedded here and
//! read once into typed entries.

use serde_json::{Map, Value};
use std::sync::OnceLock;

/// An app, as its entry in core/apps.js has it. What an entry leaves out is
/// empty or None.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// The names numbat records its runs under.
    pub numbat: Vec<String>,
    /// Its vendor products, `<vendor>/<product>`.
    pub products: Vec<String>,
    pub roots: Option<Roots>,
    /// Its context window, until a transcript says otherwise.
    pub context: Option<f64>,
    /// The kind of account it runs under (identity/v1).
    pub identity: Option<String>,
    /// Its plan limits, as live/snapshot/v1's limits name them.
    pub limits: Option<String>,
    /// The developer team that signs it (codesign's TeamIdentifier).
    pub team: Option<String>,
    pub connect: Option<Connect>,
    /// For a native reader, the tools whose calls are edits.
    pub edit_tools: Vec<String>,
    /// What XenoNet can do with it at most: lines, sessions, native,
    /// connect, delivery, processes, caller, open.
    pub has: Vec<String>,
}

/// The variable that moves an app's folder, and the folder otherwise (~
/// for the home folder); `elsewhere`, the files it moves too that are
/// elsewhere otherwise, each with its folder then.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roots {
    pub env: String,
    pub default: String,
    pub elsewhere: Vec<(String, String)>,
}

/// How XenoNet connects an app: its config format, and the hook events it
/// adds, in order, each with its matcher when it takes one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connect {
    pub format: String,
    pub events: Vec<(String, Option<String>)>,
}

impl App {
    /// Whether it has a capability.
    pub fn has(&self, capability: &str) -> bool {
        self.has.iter().any(|c| c == capability)
    }

    /// Whether a tool's calls are edits, for a native reader.
    pub fn edits_with(&self, tool: &str) -> bool {
        self.edit_tools.iter().any(|t| t == tool)
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

fn texts(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| text(Some(x))).collect()).unwrap_or_default()
}

fn roots(v: &Map<String, Value>) -> Roots {
    let elsewhere = v.get("elsewhere").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), text(Some(x))?))).collect()).unwrap_or_default();
    Roots { env: text(v.get("env")).unwrap_or_default(), default: text(v.get("default")).unwrap_or_default(), elsewhere }
}

fn connect(v: &Map<String, Value>) -> Connect {
    let events = v.get("events").and_then(Value::as_array).map(|a| a.iter().filter_map(|e| Some((text(e.get(0))?, text(e.get(1))))).collect()).unwrap_or_default();
    Connect { format: text(v.get("format")).unwrap_or_default(), events }
}

fn entry(v: &Value) -> Option<App> {
    let o = v.as_object()?;
    Some(App {
        id: text(o.get("id"))?,
        name: text(o.get("name"))?,
        numbat: texts(o.get("numbat")),
        products: texts(o.get("products")),
        roots: o.get("roots").and_then(Value::as_object).map(roots),
        context: o.get("context").and_then(Value::as_f64),
        identity: text(o.get("identity")),
        limits: text(o.get("limits")),
        team: text(o.get("team")),
        connect: o.get("connect").and_then(Value::as_object).map(connect),
        edit_tools: texts(o.get("editTools")),
        has: texts(o.get("has")),
    })
}

/// Every app, in core/apps.js's order.
pub fn apps() -> &'static [App] {
    static A: OnceLock<Vec<App>> = OnceLock::new();
    A.get_or_init(|| match crate::json::parse(include_str!("apps.json")) {
        Ok(Value::Array(list)) => list.iter().filter_map(entry).collect(),
        _ => Vec::new(),
    })
}

/// The app with this id.
pub fn app(id: &str) -> Option<&'static App> {
    apps().iter().find(|a| a.id == id)
}

/// The apps that have a capability, in order.
pub fn with(capability: &str) -> impl Iterator<Item = &'static App> + '_ {
    apps().iter().filter(move |a| a.has(capability))
}

/// The tailed app (one with `lines`: Claude Code, Codex or Grok Build) `app`
/// names, or `fallback` for any other. The places that read a transcript by
/// its app read an app that isn't tailed as one of them, and `fallback` says
/// which, as each always has.
pub fn tailed_app(app: &str, fallback: &'static str) -> &'static str {
    with("lines").find(|a| a.id == app).map_or(fallback, |a| a.id.as_str())
}

/// The apps XenoNet asks a question of or hands a session to (xenonet
/// diagnose --ask, a handoff).
pub const ASKED: [&str; 2] = ["claude", "codex"];

/// The asked app `app` names, or `fallback` for any other, as it always has.
pub fn asked_app(app: &str, fallback: &'static str) -> &'static str {
    ASKED.into_iter().find(|a| *a == app).unwrap_or(fallback)
}

/// An app's hook events, with their matchers; none for an app XenoNet
/// doesn't connect.
pub fn hook_events(id: &str) -> &'static [(String, Option<String>)] {
    app(id).and_then(|a| a.connect.as_ref()).map_or(&[], |c| c.events.as_slice())
}

/// The tools whose calls are edits, for an app with a native reader; none
/// for another.
pub fn edit_tools(id: &str) -> &'static [String] {
    app(id).map_or(&[], |a| a.edit_tools.as_slice())
}

/// The developer team that signs an app, by its id.
pub fn team(id: &str) -> Option<&'static str> {
    app(id).and_then(|a| a.team.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_reads_whole() {
        let ids: Vec<&str> = apps().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.len(), 26, "every entry reads");
        assert_eq!(&ids[..4], ["claude", "codex", "grok", "gemini-cli"]);
        assert_eq!(team("grok"), Some("5Y6N3AJ54S"));
        assert_eq!(app("grok").and_then(|a| a.roots.as_ref()).map(|r| r.env.as_str()), Some("GROK_HOME"));
        let claude = app("claude").unwrap();
        assert_eq!(claude.name, "Claude Code");
        assert_eq!(claude.numbat, ["claude-code"]);
        assert_eq!(claude.context, Some(200000.0));
        assert_eq!(claude.roots.as_ref().unwrap().elsewhere, [(".claude.json".to_string(), "~".to_string())]);
        assert_eq!(app("codex").unwrap().context, Some(258400.0));
        assert_eq!(team("codex"), Some("2DC432GLL2"));
        assert_eq!(team("opencode"), None);
        assert_eq!(hook_events("codex")[2], ("PreToolUse".to_string(), Some("apply_patch|Edit|Write".to_string())));
        assert_eq!(hook_events("gemini-cli"), []);
        assert!(app("opencode").unwrap().edits_with("multiedit"));
        assert_eq!(edit_tools("gemini-cli"), ["write_file", "replace"]);
        assert_eq!((edit_tools("claude"), edit_tools("nope")), (&[][..], &[][..]));
        let observed = app("amp").unwrap();
        assert_eq!((observed.numbat.as_slice(), observed.has.is_empty()), (["amp".to_string()].as_slice(), true));
    }

    #[test]
    fn the_fallbacks_read_any_other_app_as_one_they_know() {
        assert_eq!(with("lines").map(|a| a.id.as_str()).collect::<Vec<_>>(), ["claude", "codex", "grok"]);
        for (app, fallback, read) in [
            ("claude", "codex", "claude"),
            ("codex", "claude", "codex"),
            ("grok", "claude", "grok"),
            ("gemini-cli", "claude", "claude"),
            ("opencode", "codex", "codex"),
            ("", "claude", "claude"),
            ("Codex", "claude", "claude"),
        ] {
            assert_eq!(tailed_app(app, fallback), read, "{app}");
        }
        for (app, fallback, read) in [("claude", "codex", "claude"), ("codex", "claude", "codex"), ("grok", "codex", "codex"), ("grok", "claude", "claude"), ("", "codex", "codex")]
        {
            assert_eq!(asked_app(app, fallback), read, "{app}");
        }
    }
}
