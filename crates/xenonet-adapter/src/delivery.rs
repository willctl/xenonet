//! How live delivery reaches an app's sessions: the ways in an adapter
//! gives for its app, which the engine's router picks from.

use serde_json::Value;

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
    /// The command for one turn of a copy of a session that a process
    /// holds (its whole context under a new id, which the run's output
    /// names), for an app that copies one.
    pub fork: Option<Resume>,
    /// The arguments that have a turn XenoNet runs ask its permission
    /// prompts of you through the xenonet MCP server of this name, for an
    /// app that takes a permission prompt tool.
    pub permission_tool: Option<fn(&str) -> Vec<String>>,
    /// The variable that says how long the app waits on an MCP tool call,
    /// for an app that cuts one short: a run XenoNet starts waits past the
    /// permission tool's hold.
    pub tool_timeout: Option<&'static str>,
    /// For an app whose unattended runs sign in with the token you gave
    /// XenoNet's runs (`xenonet runs sign-in`): the variable that hands it
    /// to them, and whether a run's output says the app isn't signed in.
    pub sign_in: Option<SignIn>,
    /// The app picks a new session's id itself, which its run's output
    /// names (`session_of`), rather than taking the one XenoNet picks.
    pub picks_id: bool,
    /// Why a start of the app can't ask for a permission mode, for an app
    /// that takes its own.
    pub no_mode: Option<&'static str>,
    /// The session a run's output names, by the app's own id for it.
    pub session_of: Option<fn(&str) -> Option<String>>,
    /// The last turn that ended in these lines of a run's output, if any
    /// ended.
    pub finish: Option<fn(&str) -> Option<Finish>>,
    /// Whether the session called XenoNet's send tool in these lines of
    /// its output.
    pub reported: Option<Says>,
    /// How full the app's plan is, from its own reading in the live
    /// snapshot's limits (the registry's `limits` names it): the percents
    /// of its windows that haven't reset by `now`, or None when the
    /// reading isn't there or isn't fresh.
    pub plan_used: Option<PlanUsed>,
    /// The program and arguments that ask the app's version, for the live
    /// setup.
    pub version: Option<(&'static str, &'static [&'static str])>,
    /// The command that starts a session with XenoNet's MCP server (by its
    /// name) as a channel, for an app with channels.
    pub channel_command: Option<fn(&str) -> String>,
    /// The command that starts a session listening after each turn with
    /// a settings file (its path, quoted for a shell).
    pub listen_command: Option<fn(&str) -> String>,
    /// The alias that starts the app so that XenoNet reaches its server,
    /// for an app whose own server takes messages.
    pub server_alias: Option<&'static str>,
}

/// Whether lines of a run's output say something.
pub type Says = fn(&str) -> bool;

/// See `Ways::plan_used`.
pub type PlanUsed = fn(Option<&Value>, f64) -> Option<Vec<f64>>;

/// How an app's unattended runs sign in.
#[derive(Clone, Copy, Debug)]
pub struct SignIn {
    /// The variable that hands a run the token.
    pub variable: &'static str,
    /// Whether a run's output says the app isn't signed in.
    pub signed_out: fn(&str) -> bool,
}

/// How the end of one turn of a started run reads in its output: what the
/// session said last, whether it failed, and the tools it was denied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub text: String,
    pub error: bool,
    pub denied: Vec<String>,
}

// ---------- reading a run's output, as more than one app writes it ----------

/// Whether text is a UUID, as a run's output names a session.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The first UUID under `key` in a run's lines of JSON.
pub fn id_under(key: &str, log: &str) -> Option<String> {
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        if let Some(id) = v.get(key).and_then(Value::as_str).filter(|id| is_uuid(id)) {
            return Some(id.to_string());
        }
    }
    None
}

/// A text field of a run's event, trimmed; "" for one that isn't text.
pub fn clean(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

/// The last turn that ended in a run's output in Claude Code's shape: a
/// stream's `result` event, or with `bare`, any line whose `result` is
/// text (Grok Build prints one).
pub fn finish_claude_style(log: &str, bare: bool) -> Option<Finish> {
    let mut out = None;
    for line in log.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "result" || (bare && v.get("result").is_some_and(Value::is_string)) {
            let denied = v
                .get("permission_denials")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|d| d.get("tool_name").and_then(Value::as_str)).map(str::to_string).collect())
                .unwrap_or_default();
            let error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false) || v.get("subtype").and_then(Value::as_str).is_some_and(|t| t.starts_with("error"));
            out = Some(Finish { text: clean(&v["result"]), error, denied });
        }
    }
    out
}

/// Whether a run's output in Claude Code's shape has the session call
/// XenoNet's send tool (`mcp__xenonet…__send`).
pub fn reported_claude_style(log: &str) -> bool {
    log.lines().any(|line| {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { return false };
        v["message"]["content"]
            .as_array()
            .is_some_and(|c| c.iter().any(|b| b["type"] == "tool_use" && b["name"].as_str().is_some_and(|n| n.starts_with("mcp__xenonet") && n.ends_with("__send"))))
    })
}

/// Number(x) of a plan reading's field; NaN for one that isn't there.
pub fn reading_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(v @ Value::String(_)) => xenonet_text::text::to_number(Some(v)),
        Some(_) => f64::NAN,
    }
}

/// `x && x <= now`: a plan window's reset that's passed.
pub fn reset_passed(x: Option<&Value>, now: f64) -> bool {
    xenonet_text::text::truthy(x) && reading_number(x) <= now
}

/// A plan reading's percent, when it's a finite number.
pub fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}
