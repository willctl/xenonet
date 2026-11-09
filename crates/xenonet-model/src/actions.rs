//! The action vocabulary: what an agent, an actor or the person can ask
//! of XenoNet, as one closed set. Every surface (the MCP tools, the HTTP
//! API, the command) names one of these, so that who may do what is
//! decided over a value, never a route's or a tool's string.

use serde::{Deserialize, Serialize};

/// An action on the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Who am I, as the engine sees me.
    Whoami,
    /// The sessions and actors at work around me.
    Peers,
    /// Who has a hand in a file.
    WhoHas,
    /// A message to a session, an actor, a repo or everyone.
    Send,
    /// My unread messages.
    Inbox,
    /// Files I'm about to work on.
    Claim,
    /// Letting claims go.
    Release,
    /// A line about what I'm doing.
    Focus,
    /// Work handed over, or taken.
    Handoff,
    /// A new session, here or on another machine.
    StartSession,
    /// Standing delegation on a machine.
    Delegate,
    /// A run's permission prompt, held for the person.
    Permission,
    /// What an actor's instances report of themselves.
    ActorHeartbeat,
    /// What an actor did.
    ActorEvents,
    /// The nodes an actor acts on.
    ActorTargets,
    /// A gateway token for an actor's app, started for the person to finish.
    AskGatewayToken,
    /// Whether XenoNet is set up right.
    Doctor,
}

impl Action {
    pub const ALL: [Self; 17] = [
        Self::Whoami,
        Self::Peers,
        Self::WhoHas,
        Self::Send,
        Self::Inbox,
        Self::Claim,
        Self::Release,
        Self::Focus,
        Self::Handoff,
        Self::StartSession,
        Self::Delegate,
        Self::Permission,
        Self::ActorHeartbeat,
        Self::ActorEvents,
        Self::ActorTargets,
        Self::AskGatewayToken,
        Self::Doctor,
    ];

    /// The action's name, as the MCP tools are called.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Whoami => "whoami",
            Self::Peers => "peers",
            Self::WhoHas => "who_has",
            Self::Send => "send",
            Self::Inbox => "inbox",
            Self::Claim => "claim",
            Self::Release => "release",
            Self::Focus => "focus",
            Self::Handoff => "handoff",
            Self::StartSession => "start_session",
            Self::Delegate => "delegate",
            Self::Permission => "permission",
            Self::ActorHeartbeat => "actor_heartbeat",
            Self::ActorEvents => "actor_events",
            Self::ActorTargets => "actor_targets",
            Self::AskGatewayToken => "ask_gateway_token",
            Self::Doctor => "doctor",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == name)
    }

    /// Whether the action changes anything, or only reads.
    pub fn writes(self) -> bool {
        !matches!(self, Self::Whoami | Self::Peers | Self::WhoHas | Self::Inbox | Self::Doctor)
    }

    /// Whether the action is the person's alone to allow: it starts work,
    /// widens what a machine lets others do, or answers for them.
    pub fn needs_person(self) -> bool {
        matches!(self, Self::StartSession | Self::Delegate | Self::Permission | Self::AskGatewayToken)
    }
}

/// A moment in a harness session that its hooks report to XenoNet, by the
/// names the platforms use in their settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    StopCancelled,
    StopFailure,
    Notification,
    SessionEnd,
}

impl HookEvent {
    pub const ALL: [Self; 10] = [
        Self::SessionStart,
        Self::UserPromptSubmit,
        Self::PreToolUse,
        Self::PostToolUse,
        Self::PostToolUseFailure,
        Self::Stop,
        Self::StopCancelled,
        Self::StopFailure,
        Self::Notification,
        Self::SessionEnd,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::Stop => "Stop",
            Self::StopCancelled => "StopCancelled",
            Self::StopFailure => "StopFailure",
            Self::Notification => "Notification",
            Self::SessionEnd => "SessionEnd",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == name)
    }

    /// Whether the session's turn has ended, so mail can be put in front of
    /// it now.
    pub fn ends_turn(self) -> bool {
        matches!(self, Self::Stop | Self::StopCancelled | Self::StopFailure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::parse(a.as_str()), Some(a));
            assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{}\"", a.as_str()));
        }
        for e in HookEvent::ALL {
            assert_eq!(HookEvent::parse(e.as_str()), Some(e));
            assert_eq!(serde_json::to_string(&e).unwrap(), format!("\"{}\"", e.as_str()));
        }
        assert!(!Action::Peers.writes() && Action::Send.writes());
        assert!(Action::StartSession.needs_person() && !Action::Claim.needs_person());
        assert!(HookEvent::Stop.ends_turn() && !HookEvent::PreToolUse.ends_turn());
    }
}
