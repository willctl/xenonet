//! A Claude Code session as session/v1 has it, in the order its reader
//! always wrote it.

use serde_json::Value;
use xenonet_sources::record::{tpl, Obj, F};
use xenonet_sources::session::{parent_fields, Parent};

/// A Claude Code session (core/claude.js): the Agent SDK's listing, with
/// what Claude desktop's records say of it. Its id is "claude:" and the
/// Agent SDK's id.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeSession {
    /// The Agent SDK's session id, as it gave it (None when it gave none).
    pub raw_id: Option<Value>,
    pub desktop_id: Value,
    pub in_desktop: bool,
    pub title: Value,
    pub preview: Value,
    pub cwd: Value,
    pub worktree: Value,
    pub created_at: Value,
    pub updated_at: F,
    pub archived: bool,
    pub starred: bool,
    pub model: Value,
    pub effort: Value,
    pub permission_mode: Value,
    pub branch: Value,
    pub tag: Value,
    pub size_bytes: Value,
    pub parent: Option<Parent>,
    pub claude_account: Value,
}

impl ClaudeSession {
    /// The session in Claude Code's order: id, app, rawId, desktopId,
    /// inDesktop, title, preview, cwd, worktree, createdAt, updatedAt,
    /// archived, starred, model, effort, permissionMode, branch, tag,
    /// sizeBytes, parentId, parentKind, claudeAccount.
    pub fn record(self) -> Obj {
        let (parent_id, parent_kind) = parent_fields(self.parent);
        vec![
            ("id", F::s(format!("claude:{}", tpl(self.raw_id.as_ref())))),
            ("app", F::s("claude")),
            ("rawId", F::of(self.raw_id.as_ref())),
            ("desktopId", F::V(self.desktop_id)),
            ("inDesktop", F::V(Value::Bool(self.in_desktop))),
            ("title", F::V(self.title)),
            ("preview", F::V(self.preview)),
            ("cwd", F::V(self.cwd)),
            ("worktree", F::V(self.worktree)),
            ("createdAt", F::V(self.created_at)),
            ("updatedAt", self.updated_at),
            ("archived", F::V(Value::Bool(self.archived))),
            ("starred", F::V(Value::Bool(self.starred))),
            ("model", F::V(self.model)),
            ("effort", F::V(self.effort)),
            ("permissionMode", F::V(self.permission_mode)),
            ("branch", F::V(self.branch)),
            ("tag", F::V(self.tag)),
            ("sizeBytes", F::V(self.size_bytes)),
            ("parentId", parent_id),
            ("parentKind", parent_kind),
            ("claudeAccount", F::V(self.claude_account)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_sources::session::ParentKind;

    fn keys(o: &[(&'static str, F)]) -> String {
        o.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn it_keeps_its_own_order() {
        let claude = ClaudeSession {
            raw_id: Some(json!("c1")),
            desktop_id: Value::Null,
            in_desktop: false,
            title: json!("Refunds"),
            preview: Value::Null,
            cwd: Value::Null,
            worktree: Value::Null,
            created_at: Value::Null,
            updated_at: F::null(),
            archived: false,
            starred: false,
            model: Value::Null,
            effort: Value::Null,
            permission_mode: Value::Null,
            branch: Value::Null,
            tag: Value::Null,
            size_bytes: Value::Null,
            parent: Some(Parent { id: "claude:c0".into(), kind: ParentKind::Spawn }),
            claude_account: Value::Null,
        }
        .record();
        assert_eq!(keys(&claude), "id app rawId desktopId inDesktop title preview cwd worktree createdAt updatedAt archived starred model effort permissionMode branch tag sizeBytes parentId parentKind claudeAccount");
        assert_eq!((&claude[0].1, &claude[19].1, &claude[20].1), (&F::s("claude:c1"), &F::s("claude:c0"), &F::s("spawn")));
    }
}
