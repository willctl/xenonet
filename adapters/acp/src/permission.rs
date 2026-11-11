//! What an agent's `session/request_permission` becomes: a XenoNet prompt
//! the host holds until it's answered, and the one option its answer
//! picks.

use crate::schema::{PermissionOption, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest, SelectedPermissionOutcome, SessionId, ToolKind};
use serde_json::Value;
use xenonet_authz::{act_hash, Act, Action, AnswerDecision, AskId, Authority, Authorized, Information, PromptKind};

/// A permission prompt from an agent's session, for the host to hold and
/// ask: the ask's id (XenoNet's, made here, which an Allow must name), the
/// session, the tool call, its kind, and the agent's words for it, which
/// inform and never authorize.
#[derive(Debug)]
pub struct Prompt {
    pub ask: AskId,
    pub session: SessionId,
    pub tool: String,
    /// The kind the agent gives its tool call, as XenoNet sorts prompts.
    /// It's the agent's word, as a hook's tool name is.
    pub kind: PromptKind,
    pub title: Information<String>,
    pub input: Information<Value>,
}

impl Prompt {
    pub(crate) fn of(req: &RequestPermissionRequest) -> Prompt {
        let hex = xenonet_os::random_uuid().replace('-', "");
        let f = &req.tool_call.fields;
        Prompt {
            ask: AskId::new(&format!("ask_{hex}")).expect("ask_ and hex digits are an id"),
            session: req.session_id.clone(),
            tool: req.tool_call.tool_call_id.0.to_string(),
            kind: f.kind.map_or(PromptKind::Tool, prompt_kind),
            title: Information::new(f.title.clone().unwrap_or_default()),
            input: Information::new(f.raw_input.clone().unwrap_or(Value::Null)),
        }
    }
}

/// An ACP tool call's kind, as XenoNet sorts prompts.
pub fn prompt_kind(kind: ToolKind) -> PromptKind {
    match kind {
        ToolKind::Read | ToolKind::Search => PromptKind::Read,
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => PromptKind::Edit,
        ToolKind::Execute => PromptKind::Command,
        ToolKind::Fetch => PromptKind::Web,
        _ => PromptKind::Tool,
    }
}

/// The host's answer to a prompt. An Allow carries what
/// `xenonet_authz::decide` allowed, which nothing else makes; a Deny needs
/// nothing, since it only narrows (and no answer by the hold's end is one).
#[derive(Debug)]
pub enum Answer {
    Allow(Box<Authorized>),
    Deny,
}

/// Whether `ok` allows the ask `ask`: an answer that allows, decided for
/// this very ask (its act's hash covers the ask's id and the Allow), on an
/// authority an Allow may rest on: the person's presence, an actor's allow
/// scope, or the person's delegation.
pub fn allows(ok: &Authorized, ask: &AskId) -> bool {
    let act = Act::Answer { ask: ask.clone(), decision: AnswerDecision::Allow };
    ok.action() == Action::Answer
        && *ok.act_hash() == act_hash(ok.resource(), &act)
        && matches!(ok.authority(), Authority::Presence { .. } | Authority::AllowScope { .. } | Authority::StandingGrant)
}

/// The option an answer picks: allow-once for an Allow that `allows` this
/// ask, reject-once for anything else. Never an always option, which the
/// agent would keep: one answer covers one call. When the agent offers no
/// such option, the request is answered as cancelled, which allows
/// nothing.
pub fn choose(options: &[PermissionOption], ask: &AskId, answer: &Answer) -> RequestPermissionOutcome {
    let allowed = matches!(answer, Answer::Allow(ok) if allows(ok, ask));
    let want = if allowed { PermissionOptionKind::AllowOnce } else { PermissionOptionKind::RejectOnce };
    options.iter().find(|o| o.kind == want).map_or(RequestPermissionOutcome::Cancelled, |o| RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(kinds: &[PermissionOptionKind]) -> Vec<PermissionOption> {
        kinds.iter().enumerate().map(|(i, k)| PermissionOption::new(format!("o{i}"), format!("option {i}"), *k)).collect()
    }

    fn picked(o: &RequestPermissionOutcome) -> Option<String> {
        match o {
            RequestPermissionOutcome::Selected(s) => Some(s.option_id.0.to_string()),
            _ => None,
        }
    }

    #[test]
    fn a_deny_picks_reject_once_and_never_an_always() {
        use PermissionOptionKind::*;
        let ask = AskId::new("ask_1").unwrap();
        let all = options(&[AllowAlways, RejectAlways, AllowOnce, RejectOnce]);
        assert_eq!(picked(&choose(&all, &ask, &Answer::Deny)).as_deref(), Some("o3"));
        // Only the always options: nothing is picked.
        let always = options(&[AllowAlways, RejectAlways]);
        assert_eq!(choose(&always, &ask, &Answer::Deny), RequestPermissionOutcome::Cancelled);
    }

    #[test]
    fn kinds_sort_as_xenonet_sorts_prompts() {
        assert_eq!(prompt_kind(ToolKind::Search), PromptKind::Read);
        assert_eq!(prompt_kind(ToolKind::Move), PromptKind::Edit);
        assert_eq!(prompt_kind(ToolKind::Execute), PromptKind::Command);
        assert_eq!(prompt_kind(ToolKind::Fetch), PromptKind::Web);
        assert_eq!(prompt_kind(ToolKind::Think), PromptKind::Tool);
    }
}
