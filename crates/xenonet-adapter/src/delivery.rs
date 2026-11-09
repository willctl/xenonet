//! How live delivery reaches an app's sessions: the ways in an adapter
//! gives for its app, which the engine's router picks from.

/// The command for one headless turn of a session whose process has gone,
/// given the session's id and the settings file the turn listens with (if
/// any): the program and its arguments.
pub type Resume = fn(&str, Option<&str>) -> (String, Vec<String>);

/// An app's ways in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ways {
    /// Its headless resume, if it has one.
    pub resume: Option<Resume>,
    /// The start of the variables that mark a process as one of the app's
    /// sessions, which a resumed turn leaves out; where the app keeps its
    /// sessions (the registry's roots.env) stays.
    pub marks: Option<&'static str>,
    /// A session started with XenoNet as a channel takes messages pushed
    /// into it.
    pub channel: bool,
    /// The app's own server takes a message for a session, which the
    /// router reaches through the app's server adapter.
    pub server: bool,
}
