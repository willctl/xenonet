//! You at the console, actor:you (core/actors/you.js;
//! docs/design/actors/interface.md, section 6). XenoNet's window alone
//! reports you, with the key only it holds: your presence, from input in
//! the focused window, and what you did there, as node ids and action
//! names. XenoNet writes the words from its own map. The desktop app adds
//! whether the window shows. Nothing you type is ever sent, and you're
//! never named.
//!
//! What becomes an event:
//!
//!   looked      a selection that held 3 s   status   the node, watches
//!   read        a preview, or stats         status   the session, reviews
//!   opened      a session in its app        status   the session, reviews
//!   terminal    a terminal in a folder      status   the place, works-on
//!   started     a new session in a place    spawned  the place, works-on
//!   handed-off  a session to another app    spawned  its place, works-on
//!
//! The calls are the registry's (`Actors::you_report` and the rest); this
//! is what it keeps of you.

/// The page reports every minute while the window shows, so two missed
/// are away and ten are offline.
pub const YOU_HEARTBEAT_MS: f64 = 60e3;
pub const DAY_MS: f64 = 24.0 * 3600e3;
pub const AHEAD_MS: f64 = 60e3;

/// The desktop app's word for the window (shown, hidden or closed), and the
/// presence the page last reported. The window's word counts only once the
/// page has said something, and once you turn yourself off (`off`),
/// nothing more is said of you until the page reports again.
#[derive(Clone, Debug)]
pub struct You {
    pub shown: String,
    pub page: Option<String>,
    pub off: bool,
}

impl Default for You {
    fn default() -> Self {
        You { shown: "shown".into(), page: None, off: false }
    }
}

impl You {
    pub fn presence(&self) -> Option<String> {
        let page = self.page.as_ref()?;
        Some(match self.shown.as_str() {
            "closed" => "offline".into(),
            "hidden" => "away".into(),
            _ => page.clone(),
        })
    }
}

/// Whether an action may be done to a node of this kind.
pub fn on(what: &str, kind: &str) -> bool {
    let place = matches!(kind, "project" | "zone" | "native");
    match what {
        "looked" | "terminal" => place || kind == "session",
        "read" | "opened" | "handed-off" => kind == "session",
        "started" => place,
        _ => false,
    }
}

pub fn known_action(what: &str) -> bool {
    matches!(what, "looked" | "read" | "opened" | "terminal" | "started" | "handed-off")
}

pub fn relation(what: &str) -> Option<&'static str> {
    match what {
        "looked" => Some("watches"),
        "read" | "opened" => Some("reviews"),
        "terminal" => Some("works-on"),
        _ => None,
    }
}
