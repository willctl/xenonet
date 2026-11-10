//! session/v1 (core/schema/session.js) as the readers write it, in the
//! order they always have. The order is part of what XenoNet writes (its
//! answers, state.json, the golden files), so each builder keeps it byte
//! for byte.
//!
//! The runs of the native readers, numbat and the gateway are answers as
//! the native readers give them (`native::Out`), and share one order, here;
//! Claude Code's and Codex's sessions are objects for JS (`record::Obj`),
//! each built by its adapter in its own order, with the parent fields here.

use crate::native::Out;
use crate::record::F;

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

/// parentId and parentKind, both null for a session with no parent.
pub fn parent_fields(p: Option<Parent>) -> (F, F) {
    match p {
        Some(p) => (F::s(p.id), F::s(p.kind.as_str())),
        None => (F::null(), F::null()),
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
    /// What the app's own records say the session used (`usage`), after the
    /// rest.
    pub usage: Option<Out>,
    pub via: Via<'a>,
}

impl RunSession<'_> {
    /// The session in the runs' order: id, app, rawId, title, preview
    /// (null), cwd, createdAt, updatedAt, archived, model, branch,
    /// parentId, parentKind, then how XenoNet knows of it, then the usage when it has one.
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
        if let Some(usage) = self.usage {
            o.push(("usage", usage));
        }
        Out::Obj(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_with_usage_carries_it_last() {
        let run = |usage| {
            RunSession {
                id: "opencode:s1",
                app: "opencode",
                raw_id: "s1",
                title: "OpenCode".into(),
                cwd: None,
                created_at: Out::Num(1.0),
                updated_at: Out::Num(2.0),
                archived: false,
                model: None,
                branch: None,
                subagent_of: None,
                usage,
                via: Via::Reader,
            }
            .record()
        };
        let keys = |o: Out| match o {
            Out::Obj(fs) => fs.into_iter().map(|(k, _)| k).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        assert_eq!(keys(run(None)).last(), Some(&"reader"));
        assert_eq!(keys(run(Some(Out::Obj(vec![])))).last(), Some(&"usage"));
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
                usage: None,
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
