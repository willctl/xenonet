//! How live delivery reaches an app's sessions: the ways in an adapter
//! gives for its app, which the engine's router picks from.

/// The command for one headless turn of a session whose process has gone,
/// given the session's id and the settings file the turn listens with (if
/// any): the program and its arguments.
pub type Resume = fn(&str, Option<&str>) -> (String, Vec<String>);

/// What a new session is started with: what its adapter's `Starter` has
/// to make the command from.
#[derive(Clone, Copy, Debug)]
pub struct Start<'a> {
    /// The session's id, a UUID XenoNet picks so the answer can name it at
    /// once.
    pub id: &'a str,
    /// The folder it starts in.
    pub cwd: &'a str,
    /// The permission mode asked for; None is the person's own settings.
    pub mode: Option<&'a str>,
    /// The settings file the session listens for messages with after each
    /// turn, for an app that takes one.
    pub settings: Option<&'a str>,
    /// The xenonet MCP server's name in the app's settings, whose
    /// permission tool answers the session's prompts.
    pub server: &'a str,
    /// Where the first prompt was written, for an app that reads it from a
    /// file (`Ways::prompt_file`).
    pub prompt_file: Option<&'a str>,
}

/// The command that starts a new session with a first prompt: the program
/// and its arguments. The prompt goes on standard input, or in
/// `Start::prompt_file`, never as an argument.
pub type Starter = fn(&Start<'_>) -> (String, Vec<String>);

/// Whether these arguments to the app's CLI only ask its version: a probe
/// a standing delegation covers without a question.
pub type Probe = fn(&[String]) -> bool;

/// An app's ways in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ways {
    /// Its headless resume, if it has one.
    pub resume: Option<Resume>,
    /// How a new session of it is started, if XenoNet starts them.
    pub start: Option<Starter>,
    /// The app reads a turn's prompt from a file XenoNet writes, not from
    /// standard input.
    pub prompt_file: bool,
    /// Its version probe, if a delegation covers one.
    pub probe: Option<Probe>,
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
