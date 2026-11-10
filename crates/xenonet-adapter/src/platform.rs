//! The `Platform` trait: one implementation per app or actor product, in
//! its own crate under adapters/, and the `Registry` of the ones a XenoNet
//! runs with. The core asks the registry; it never matches on an id.
//!
//! An adapter says what it is (`app`), decodes the hooks its app sends
//! (`decode_hook`), brings its doctor rows (`checks`), observes its actor
//! product (`observer`), and, as its code moves out of the core, watches
//! and reconciles its sessions and offers ways to deliver to, start and
//! connect its app.

use crate::apps::App;
use crate::check::{Check, CheckRow};
use crate::connect::{Format, Shared};
use crate::delivery::Ways;
use crate::observe::Observer;
use crate::open::Opener;
use crate::processes::Rules;
use crate::reader::{Lister, ListerEnv, Native, Tailed};
use serde_json::Value;
use std::sync::Arc;
use xenonet_model::actions::HookEvent;

/// A hook an app sent, read into what the engine acts on.
#[derive(Clone, Debug, PartialEq)]
pub struct Hook {
    pub event: HookEvent,
    /// The run's id, as the app names it (the part after the app's id in
    /// a session id).
    pub session: String,
    /// A subagent's id, when the event is a subagent's under its session.
    pub agent: Option<String>,
    pub cwd: Option<String>,
    /// Whether the session bypasses its permission checks, when the hook
    /// says.
    pub bypasses_permissions: Option<bool>,
    /// For a Stop: whether a Stop hook is already running (the app's own
    /// guard against a loop).
    pub stop_hook_active: bool,
    /// For a Stop: why the turn ended, when the app says (Grok Build's
    /// `end_turn`, `channel_closed`, `shutdown`).
    pub reason: Option<String>,
    /// For a tool event: the tool and its input.
    pub tool: Option<(String, Value)>,
}

/// What an adapter provides. Every method but `app` has a default: an
/// adapter brings what its platform has.
pub trait Platform: Send + Sync {
    /// Its registry entry.
    fn app(&self) -> &App;

    /// Its id in the registry.
    fn id(&self) -> &str {
        &self.app().id
    }

    /// A hook's body, as the app sent it, read into a `Hook`; None when
    /// the body names no session or no event XenoNet knows. The default
    /// reads Claude Code's field names, which Codex and Grok Build share.
    fn decode_hook(&self, event: &str, body: &Value) -> Option<Hook> {
        decode_claude_style(event, body)
    }

    /// The doctor checks this adapter brings, in the order they show.
    fn checks(&self) -> Vec<Box<dyn Check>> {
        Vec::new()
    }

    /// How the app's processes are told apart, for an app whose sessions
    /// XenoNet holds to its processes (the registry's `processes`).
    fn processes(&self) -> Option<Rules> {
        None
    }

    /// How live delivery reaches the app's sessions (the registry's
    /// `delivery`).
    fn delivery(&self) -> Option<Ways> {
        None
    }

    /// The app's connect, through its config format (the registry's
    /// `connect`).
    fn connect(&self, _app: &App, _shared: &Shared) -> Option<Box<dyn Format>> {
        None
    }

    /// The observer of its actor product, for an adapter whose product
    /// XenoNet observes (the products table's `observed`).
    fn observer(&self) -> Option<Box<dyn Observer>> {
        None
    }

    /// A reader of its transcripts for the live tailer, new for each
    /// telemetry, for an app whose lines XenoNet tails (the registry's
    /// `lines`).
    fn tailed(&self) -> Option<Box<dyn Tailed>> {
        None
    }

    /// Its reader of its own records, for an app with one (the registry's
    /// `native`).
    fn native(&self) -> Option<Box<dyn Native>> {
        None
    }

    /// Its lister of its sessions and transcripts, new for each map, for
    /// an app whose sessions the map lists.
    fn lister(&self, _env: &ListerEnv<'_>) -> Option<Box<dyn Lister>> {
        None
    }

    /// What opens its sessions and projects in its desktop app, for an app
    /// the registry gives `open`.
    fn opener(&self) -> Option<Opener> {
        None
    }
}

/// A hook in Claude Code's shape (`session_id`, `cwd`, `tool_name`,
/// `tool_input`, `permission_mode`, `agent_id`, `stop_hook_active`,
/// `reason`).
pub fn decode_claude_style(event: &str, body: &Value) -> Option<Hook> {
    let event = HookEvent::parse(event)?;
    let session = body.get("session_id")?.as_str().filter(|s| !s.is_empty())?.to_string();
    let text = |k: &str| body.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    Some(Hook {
        event,
        session,
        agent: text("agent_id"),
        cwd: text("cwd"),
        bypasses_permissions: body.get("permission_mode").and_then(Value::as_str).map(|m| m == "bypassPermissions"),
        stop_hook_active: body.get("stop_hook_active").and_then(Value::as_bool).unwrap_or(false),
        reason: text("reason"),
        tool: text("tool_name").map(|name| (name, body.get("tool_input").cloned().unwrap_or(Value::Null))),
    })
}

/// The adapters a XenoNet runs with, by app id, in the registry's order.
#[derive(Default)]
pub struct Registry {
    platforms: Vec<Arc<dyn Platform>>,
}

impl Registry {
    pub fn new(platforms: Vec<Arc<dyn Platform>>) -> Self {
        let mut r = Registry { platforms };
        r.platforms.sort_by_key(|p| crate::apps::apps().iter().position(|a| a.id == p.id()).unwrap_or(usize::MAX));
        r
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Platform>> {
        self.platforms.iter().find(|p| p.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Platform>> {
        self.platforms.iter()
    }

    pub fn ids(&self) -> Vec<&str> {
        self.platforms.iter().map(|p| p.id()).collect()
    }

    /// Every adapter's doctor checks, in order.
    pub fn checks(&self) -> Vec<Box<dyn Check>> {
        self.platforms.iter().flat_map(|p| p.checks()).collect()
    }

    /// Their rows.
    pub fn check_rows(&self) -> Vec<CheckRow> {
        self.checks().iter().map(|c| c.row()).collect()
    }

    /// The adapters whose apps' processes XenoNet tells apart, with their
    /// rules, in order.
    pub fn processes(&self) -> Vec<(&App, Rules)> {
        self.platforms.iter().filter_map(|p| Some((p.app(), p.processes()?))).collect()
    }

    /// The adapters whose apps live delivery reaches, with their ways in.
    pub fn delivery(&self) -> Vec<(&App, Ways)> {
        self.platforms.iter().filter_map(|p| Some((p.app(), p.delivery()?))).collect()
    }

    /// The observers of the actor products the adapters bring, in order.
    pub fn observers(&self) -> Vec<Box<dyn Observer>> {
        self.platforms.iter().filter_map(|p| p.observer()).collect()
    }

    /// New readers of the tailed apps' transcripts, in order.
    pub fn tailed(&self) -> Vec<Box<dyn Tailed>> {
        self.platforms.iter().filter_map(|p| p.tailed()).collect()
    }

    /// The adapters whose apps have a reader of their own records, with
    /// the reader's row, in order.
    pub fn native(&self) -> Vec<(&App, Box<dyn Native>)> {
        self.platforms.iter().filter_map(|p| Some((p.app(), p.native()?))).collect()
    }

    /// New listers of the apps whose sessions the map lists, in order,
    /// each made for its app's own folders (`env`).
    pub fn listers<'e>(&self, env: impl Fn(&App) -> ListerEnv<'e>) -> Vec<(&App, Box<dyn Lister>)> {
        self.platforms.iter().filter_map(|p| Some((p.app(), p.lister(&env(p.app()))?))).collect()
    }
}

static INSTALLED: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();

/// Puts the adapters this XenoNet runs with in place, once: the composition
/// root (the binary's main) does, before anything asks. A second call
/// changes nothing.
pub fn install(platforms: Vec<Arc<dyn Platform>>) {
    let _ = INSTALLED.set(Registry::new(platforms));
}

/// The adapters in place: none, until `install` is called.
pub fn platforms() -> &'static Registry {
    static EMPTY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    INSTALLED.get().unwrap_or_else(|| EMPTY.get_or_init(Registry::default))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Plain(App);
    impl Platform for Plain {
        fn app(&self) -> &App {
            &self.0
        }
    }

    #[test]
    fn hooks_decode_and_the_registry_keeps_the_apps_order() {
        let h = decode_claude_style(
            "PreToolUse",
            &json!({ "session_id": "abc", "cwd": "/w", "tool_name": "Edit", "tool_input": { "file_path": "/w/a" }, "permission_mode": "bypassPermissions" }),
        )
        .unwrap();
        assert_eq!((h.event, h.session.as_str(), h.cwd.as_deref(), h.bypasses_permissions), (HookEvent::PreToolUse, "abc", Some("/w"), Some(true)));
        assert_eq!(h.tool.as_ref().map(|(n, _)| n.as_str()), Some("Edit"));
        assert!(decode_claude_style("Stop", &json!({ "cwd": "/w" })).is_none(), "no session, no hook");
        assert!(decode_claude_style("Dance", &json!({ "session_id": "abc" })).is_none(), "no such event");
        let codex = crate::apps::app("codex").unwrap().clone();
        let claude = crate::apps::app("claude").unwrap().clone();
        let r = Registry::new(vec![Arc::new(Plain(codex)), Arc::new(Plain(claude))]);
        assert_eq!(r.ids(), ["claude", "codex"]);
        assert!(r.get("grok").is_none() && r.check_rows().is_empty());
    }
}
