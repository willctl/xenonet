//! How an app's processes are told apart: the rules an adapter gives for
//! its app, which the engine's process checks apply (which program is the
//! app's, whether Node or Bun running some arguments runs it, and how its
//! sessions are held to processes).

/// An app's rules.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Whether a program is one of the app's own, from its path (with /
    /// for \) and its file's name.
    pub program: fn(&str, &str) -> bool,
    /// Whether Node or Bun running these arguments runs the app's package.
    pub script: Option<fn(&str) -> bool>,
    /// The app sends its hooks from its own process, which its maker signs,
    /// so they pin a session to that process.
    pub pins_by_hook: bool,
    /// Any of the app's processes may have any of its sessions open, so
    /// they aren't pinned to one.
    pub any_process: bool,
}

impl Rules {
    /// Rules that match nothing: a start for an adapter's own.
    pub const NONE: Rules = Rules { program: |_, _| false, script: None, pins_by_hook: false, any_process: false };
}
