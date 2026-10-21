//! session/v1 (core/schema/session.js) as each reader writes it: a builder
//! per app that names the app's fields and writes them in the order its
//! reader always has. The order is part of what XenoNet writes (its answers,
//! state.json, the golden files), so each builder keeps it byte for byte.
//!
//! Claude Code's and Codex's sessions are objects for JS (`record::Obj`);
//! the runs of the native readers, numbat and the gateway are answers as
//! the native readers give them (`native::Out`), and share one order.

use crate::codex::tpl;
use crate::native::Out;
use crate::record::{Obj, F};
use serde_json::Value;

/// How a session came from another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentKind {
    /// Claude desktop spawned it from another session.
    Spawn,
    /// A Codex thread forked from another.
    Fork,
    /// A subagent's run, or a Codex sub-agent's thread.
    Subagent,
}

impl ParentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ParentKind::Spawn => "spawn",
            ParentKind::Fork => "fork",
            ParentKind::Subagent => "subagent",
        }
    }
}

/// The session one came from: its id, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parent {
    pub id: String,
    pub kind: ParentKind,
}

// parentId and parentKind, both null for a session with no parent.
fn parent_fields(p: Option<Parent>) -> (F, F) {
    match p {
        Some(p) => (F::s(p.id), F::s(p.kind.as_str())),
        None => (F::null(), F::null()),
    }
}

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

/// How XenoNet knows of a run it has no transcript reader for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via<'a> {
    /// numbat's event records (`observed: "numbat"`).
    Numbat,
    /// An actor's report through the gateway (`observed: "gateway"`, and
    /// the actor in `reportedBy`).
    Gateway { actor: &'a str },
    /// An experimental reader of the harness's own records, core/native
    /// (`reader: "experimental"`).
    Reader,
}

/// A run of another harness: a native reader's (OpenCode, Gemini CLI,
/// Copilot CLI, Cursor), numbat's or a reported one. They share one order.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSession<'a> {
    pub id: &'a str,
    pub app: &'a str,
    pub raw_id: &'a str,
    pub title: String,
    pub cwd: Option<&'a str>,
    pub created_at: Out,
    pub updated_at: Out,
    pub archived: bool,
    pub model: Option<&'a str>,
    pub branch: Option<&'a str>,
    /// The session this run is a subagent of.
    pub subagent_of: Option<&'a str>,
    pub via: Via<'a>,
}

impl RunSession<'_> {
    /// The session in the runs' order: id, app, rawId, title, preview
    /// (null), cwd, createdAt, updatedAt, archived, model, branch,
    /// parentId, parentKind, then how XenoNet knows of it.
    pub fn record(self) -> Out {
        let mut o = vec![
            ("id", Out::str(self.id)),
            ("app", Out::str(self.app)),
            ("rawId", Out::str(self.raw_id)),
            ("title", Out::str(&self.title)),
            ("preview", Out::null()),
            ("cwd", Out::opt_str(self.cwd)),
            ("createdAt", self.created_at),
            ("updatedAt", self.updated_at),
            ("archived", Out::bool(self.archived)),
            ("model", Out::opt_str(self.model)),
            ("branch", Out::opt_str(self.branch)),
            ("parentId", Out::opt_str(self.subagent_of)),
            ("parentKind", if self.subagent_of.is_some() { Out::str(ParentKind::Subagent.as_str()) } else { Out::null() }),
        ];
        match self.via {
            Via::Numbat => o.push(("observed", Out::str("numbat"))),
            Via::Gateway { actor } => {
                o.push(("observed", Out::str("gateway")));
                o.push(("reportedBy", Out::str(actor)));
            }
            Via::Reader => o.push(("reader", Out::str("experimental"))),
        }
        Out::Obj(o)
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
    fn each_app_keeps_its_own_order() {
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

    #[test]
    fn runs_say_how_xenonet_knows_of_them() {
        let run = |via| {
            RunSession {
                id: "gemini-cli:r1",
                app: "gemini-cli",
                raw_id: "r1",
                title: "Gemini CLI".into(),
                cwd: None,
                created_at: Out::Num(1.0),
                updated_at: Out::Num(2.0),
                archived: false,
                model: None,
                branch: None,
                subagent_of: Some("gemini-cli:r0"),
                via,
            }
            .record()
        };
        let tail = |o: Out| match o {
            Out::Obj(fs) => fs.into_iter().skip(11).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(tail(run(Via::Reader)), [("parentId", Out::str("gemini-cli:r0")), ("parentKind", Out::str("subagent")), ("reader", Out::str("experimental"))]);
        assert_eq!(tail(run(Via::Numbat))[2], ("observed", Out::str("numbat")));
        assert_eq!(tail(run(Via::Gateway { actor: "actor:kite" }))[2..], [("observed", Out::str("gateway")), ("reportedBy", Out::str("actor:kite"))]);
    }
}
