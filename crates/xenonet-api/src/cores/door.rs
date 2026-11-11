//! The local door: whether a caller on this machine may do what a route
//! does, decided before the route's handler runs (the HTTP routes' rules
//! table). The composition root decides through xenonet-authz; with
//! nothing wired in, every act is refused.

/// How an agent was checked: its process seen and passed, or not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attestation {
    Unchecked,
    ProcessChecked,
}

/// Who calls, as this machine's local door sees them: this machine itself
/// (its window, its own reads, you at your terminal), or an agent with how
/// it was checked. An agent's id is empty when it didn't say who it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    Machine,
    Agent(String, Attestation),
}

/// What a read takes: the map, what sessions said, a search of it, or this
/// machine's messages, deliveries and network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadScope {
    Map,
    Transcripts,
    Search,
    State,
}

/// The acts the local door decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Read(ReadScope),
    Message,
    Claim,
    Handoff,
}

pub trait LocalDoor: Send + Sync {
    /// Ok when `caller` may do `act` here at `now` (ms), or why not.
    fn decide(&self, caller: &Caller, act: Act, now: f64) -> Result<(), String> {
        let _ = (caller, act, now);
        Err("this XenoNet has no authorizer at its local door, so it allows nothing".into())
    }
}

/// What the routes ask of the app registry: which ids are sessions', the
/// apps XenoNet asks, and which app wrote a hook. The composition root
/// answers from the adapters; with nothing wired in, no app is known.
pub trait Apps: Send + Sync {
    /// Whether `id` is a session's, `<app>:<run>`, of an app XenoNet knows.
    fn is_session_id(&self, id: &str) -> bool {
        let _ = id;
        false
    }

    /// XenoNet's id for a session named as a person or an app names it
    /// (OpenCode's `ses_…` is `opencode:ses-…`); any other id as it came.
    fn session_id_of(&self, named: &str) -> String {
        xenonet_text::text::trim(named).to_string()
    }

    /// The app XenoNet asks (runs a turn with) that `app` names, or
    /// `fallback` when it names none; for another app, the refusal's words
    /// (`what` is the field it came in).
    fn asked(&self, what: &str, app: Option<&serde_json::Value>, fallback: &'static str) -> Result<&'static str, String> {
        let _ = (app, fallback);
        Err(format!("{what} names no app this XenoNet knows"))
    }

    /// Which other app wrote a Claude Code hook's payload (apps that run
    /// Claude Code's hooks with payloads of their own), or None.
    fn hook_written_by(&self, body: &serde_json::Value) -> Option<&'static str> {
        let _ = body;
        None
    }
}

impl LocalDoor for super::Pending {}
impl Apps for super::Pending {}
