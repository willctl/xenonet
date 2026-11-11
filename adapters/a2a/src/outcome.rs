//! What an outside agent's task states mean for XenoNet (revision 26),
//! from a2a-lf's stream events:
//!
//! ```text
//!   submitted, working   Working: under way, with what it said of it
//!   (an artifact)        Artifact: a piece of the result
//!   completed            Done: the result
//!   input-required       AsksAgent: a question for the agent that delegated
//!   auth-required        AsksPerson: a question for the person; XenoNet never
//!                        sends a credential for it
//!   failed, rejected,    Ended, with why
//!   canceled
//! ```
//!
//! A plain Message in answer, with no task, is the result. Every word is
//! `Information`, with the bearer taken out.

use crate::bearer::{self, Bearer};
use ::a2a::{Part, PartContent, StreamResponse, TaskState, TaskStatus};
use xenonet_authz::Information;

/// The most of one text XenoNet passes on.
pub const TEXT_MAX: usize = 64 << 10;

/// How a task ended without a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Failed,
    Rejected,
    Canceled,
}

impl Ended {
    pub fn as_str(self) -> &'static str {
        match self {
            Ended::Failed => "failed",
            Ended::Rejected => "rejected",
            Ended::Canceled => "canceled",
        }
    }
}

/// One thing a delegated task did.
#[derive(Debug)]
pub enum Update {
    Working { task: String, context: String, note: Option<Information<String>> },
    Artifact { task: String, name: Option<String>, text: Information<String> },
    Done { task: Option<String>, text: Information<String> },
    AsksAgent { task: String, question: Information<String> },
    AsksPerson { task: String, question: Information<String> },
    Ended { task: String, how: Ended, why: Information<String> },
}

impl Update {
    /// Whether the task is over, or waits on someone: the stream ends there.
    pub fn stops(&self) -> bool {
        !matches!(self, Update::Working { .. } | Update::Artifact { .. })
    }

    /// The task it's about, when it names one.
    pub fn task(&self) -> Option<&str> {
        match self {
            Update::Working { task, .. } | Update::Artifact { task, .. } | Update::AsksAgent { task, .. } | Update::AsksPerson { task, .. } | Update::Ended { task, .. } => {
                Some(task)
            }
            Update::Done { task, .. } => task.as_deref(),
        }
    }
}

/// `text`, cut to `TEXT_MAX` on a character boundary.
fn cut(text: &str) -> &str {
    if text.len() <= TEXT_MAX {
        return text;
    }
    let mut end = TEXT_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// What remote words become: Information, without the bearer, cut short.
pub(crate) fn info(bearer: Option<&Bearer>, text: &str) -> Information<String> {
    Information::new(bearer::redact(bearer, cut(text)))
}

/// A message's or an artifact's parts as text: the text parts, and a word
/// for each part XenoNet doesn't pass on.
pub(crate) fn text_of(parts: &[Part]) -> String {
    let words: Vec<String> = parts
        .iter()
        .map(|p| match &p.content {
            PartContent::Text(t) => t.clone(),
            PartContent::Raw(_) | PartContent::Url(_) => "(a file, which XenoNet doesn't pass on)".to_string(),
            PartContent::Data(_) => "(structured data, which XenoNet doesn't pass on)".to_string(),
        })
        .collect();
    words.join("\n")
}

fn said(status: &TaskStatus) -> Option<String> {
    status.message.as_ref().map(|m| text_of(&m.parts)).filter(|t| !t.trim().is_empty())
}

fn by_state(task: String, context: String, status: &TaskStatus, artifacts: &str, bearer: Option<&Bearer>) -> Update {
    let note = said(status);
    let words = |default: &str| info(bearer, note.as_deref().unwrap_or(default));
    match status.state {
        TaskState::Unspecified | TaskState::Submitted | TaskState::Working => Update::Working { task, context, note: note.as_deref().map(|n| info(bearer, n)) },
        TaskState::Completed => Update::Done { task: Some(task), text: info(bearer, note.as_deref().unwrap_or(artifacts)) },
        TaskState::InputRequired => Update::AsksAgent { task, question: words("The agent needs more from you to go on.") },
        TaskState::AuthRequired => {
            Update::AsksPerson { task, question: words("The agent asks to be signed in. XenoNet never sends a credential for it: sign in where the agent says.") }
        }
        TaskState::Failed => Update::Ended { task, how: Ended::Failed, why: words("The agent's task failed.") },
        TaskState::Rejected => Update::Ended { task, how: Ended::Rejected, why: words("The agent turned the task down.") },
        TaskState::Canceled => Update::Ended { task, how: Ended::Canceled, why: words("The task was canceled.") },
    }
}

/// One stream event as an update. `artifacts` is the text of the
/// artifacts seen so far, the result when a completed status says none.
pub fn of(event: StreamResponse, artifacts: &str, bearer: Option<&Bearer>) -> Update {
    match event {
        StreamResponse::Task(t) => {
            let own: Vec<String> = t.artifacts.iter().flatten().map(|a| text_of(&a.parts)).collect();
            let all = if own.is_empty() { artifacts.to_string() } else { own.join("\n\n") };
            by_state(t.id, t.context_id, &t.status, &all, bearer)
        }
        StreamResponse::Message(m) => Update::Done { task: m.task_id.clone(), text: info(bearer, &text_of(&m.parts)) },
        StreamResponse::StatusUpdate(e) => by_state(e.task_id, e.context_id, &e.status, artifacts, bearer),
        StreamResponse::ArtifactUpdate(e) => Update::Artifact { task: e.task_id, name: e.artifact.name.clone(), text: info(bearer, &text_of(&e.artifact.parts)) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::a2a::{Message, Role, TaskStatusUpdateEvent};

    fn status(state: TaskState, text: Option<&str>) -> StreamResponse {
        let message = text.map(|t| Message::new(Role::Agent, vec![Part::text(t)]));
        StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: "t1".into(),
            context_id: "c1".into(),
            status: TaskStatus { state, message, timestamp: None },
            metadata: None,
        })
    }

    #[test]
    fn states_map() {
        assert!(matches!(of(status(TaskState::Submitted, None), "", None), Update::Working { .. }));
        assert!(matches!(of(status(TaskState::Working, Some("reading")), "", None), Update::Working { note: Some(_), .. }));
        let Update::Done { text, .. } = of(status(TaskState::Completed, None), "the answer", None) else { panic!() };
        assert_eq!(text.read(), "the answer", "a completed status with no words: the artifacts");
        let Update::AsksAgent { question, .. } = of(status(TaskState::InputRequired, Some("Which branch?")), "", None) else { panic!() };
        assert_eq!(question.read(), "Which branch?");
        let Update::AsksPerson { question, .. } = of(status(TaskState::AuthRequired, None), "", None) else { panic!() };
        assert!(question.read().contains("never sends a credential"));
        for (state, how) in [(TaskState::Failed, Ended::Failed), (TaskState::Rejected, Ended::Rejected), (TaskState::Canceled, Ended::Canceled)] {
            let u = of(status(state, None), "", None);
            assert!(u.stops());
            assert!(matches!(u, Update::Ended { how: h, .. } if h == how));
        }
    }

    #[test]
    fn the_bearer_never_comes_back() {
        let b = Bearer::new("sk-secret-0123456789".into()).unwrap();
        let Update::Ended { why, .. } = of(status(TaskState::Failed, Some("bad token sk-secret-0123456789")), "", Some(&b)) else { panic!() };
        assert_eq!(why.read(), "bad token [token]");
    }

    #[test]
    fn only_text_passes() {
        let parts = vec![Part::text("see"), Part::url("https://x/a.pdf"), Part::data(serde_json::json!({ "a": 1 }))];
        assert_eq!(text_of(&parts), "see\n(a file, which XenoNet doesn't pass on)\n(structured data, which XenoNet doesn't pass on)");
        let long = "é".repeat(TEXT_MAX);
        assert!(info(None, &long).read().len() <= TEXT_MAX);
    }
}
