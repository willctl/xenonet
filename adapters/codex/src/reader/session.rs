//! A Codex thread as session/v1 has it, in the order its reader always
//! wrote it.

use serde_json::Value;
use xenonet_sources::record::{tpl, Obj, F};
use xenonet_sources::session::{parent_fields, Parent};

/// A Codex thread (core/codex.js), as `codex app-server` lists it. Its id
/// is "codex:" and the thread's id.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexSession {
    /// The thread's id, as Codex gave it (None when it gave none).
    pub raw_id: Option<Value>,
    pub title: Value,
    pub preview: Value,
    pub cwd: Value,
    pub created_at: F,
    pub updated_at: F,
    pub archived: bool,
    pub status: Value,
    pub model: Value,
    pub provider: Value,
    pub effort: Value,
    pub branch: Value,
    pub origin: Value,
    pub source: F,
    pub originator: Value,
    pub native_project_id: Value,
    pub parent: Option<Parent>,
    pub agent_nickname: Value,
    pub agent_kind: Value,
    pub background: bool,
}

impl CodexSession {
    /// The session in Codex's order: id, app, rawId, title, preview, cwd,
    /// createdAt, updatedAt, archived, status, model, provider, effort,
    /// branch, origin, source, originator, nativeProjectId, parentId,
    /// parentKind, agentNickname, agentKind, background.
    pub fn record(self) -> Obj {
        let (parent_id, parent_kind) = parent_fields(self.parent);
        vec![
            ("id", F::s(format!("codex:{}", tpl(self.raw_id.as_ref())))),
            ("app", F::s("codex")),
            ("rawId", F::of(self.raw_id.as_ref())),
            ("title", F::V(self.title)),
            ("preview", F::V(self.preview)),
            ("cwd", F::V(self.cwd)),
            ("createdAt", self.created_at),
            ("updatedAt", self.updated_at),
            ("archived", F::V(Value::Bool(self.archived))),
            ("status", F::V(self.status)),
            ("model", F::V(self.model)),
            ("provider", F::V(self.provider)),
            ("effort", F::V(self.effort)),
            ("branch", F::V(self.branch)),
            ("origin", F::V(self.origin)),
            ("source", self.source),
            ("originator", F::V(self.originator)),
            ("nativeProjectId", F::V(self.native_project_id)),
            ("parentId", parent_id),
            ("parentKind", parent_kind),
            ("agentNickname", F::V(self.agent_nickname)),
            ("agentKind", F::V(self.agent_kind)),
            ("background", F::V(Value::Bool(self.background))),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(o: &[(&'static str, F)]) -> String {
        o.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn it_keeps_its_own_order() {
        let codex = CodexSession {
            raw_id: None,
            title: json!("(untitled thread)"),
            preview: Value::Null,
            cwd: Value::Null,
            created_at: F::Num(f64::NAN),
            updated_at: F::null(),
            archived: true,
            status: Value::Null,
            model: Value::Null,
            provider: Value::Null,
            effort: Value::Null,
            branch: Value::Null,
            origin: Value::Null,
            source: F::Undef,
            originator: Value::Null,
            native_project_id: Value::Null,
            parent: None,
            agent_nickname: Value::Null,
            agent_kind: Value::Null,
            background: false,
        }
        .record();
        assert_eq!(keys(&codex), "id app rawId title preview cwd createdAt updatedAt archived status model provider effort branch origin source originator nativeProjectId parentId parentKind agentNickname agentKind background");
        // A thread with no id is "codex:undefined", as JS's template wrote it.
        assert_eq!((&codex[0].1, &codex[2].1, &codex[18].1), (&F::s("codex:undefined"), &F::Undef, &F::null()));
    }
}
