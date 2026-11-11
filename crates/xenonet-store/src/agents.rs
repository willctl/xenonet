//! Agents (docs/design/direction.md, "Domain model"): a harness session as
//! the store knows it, by an `agt_` id minted the first time XenoNet sees
//! it, with the stored mapping to the id its app gave it, so the id stays
//! the same however the app names the session.
//!
//! - **First sight.** The map lists a session, or it gets in touch through
//!   its MCP server or a hook: its row is made then (`agent_of`). A session
//!   XenoNet starts itself gets a pending row before its app has named it
//!   (`pending`), with the process it started and who asked; the session's
//!   first touch from that process takes the row over, so the agent the
//!   start answered with is the session's.
//! - **In touch.** `via` and `last_seen` say how and when an agent was last
//!   heard from directly, which means messages reach it; a row the map
//!   made has neither until then. `ended_at` is when its SessionEnd hook
//!   said it ended.
//! - **Other machines' agents.** A row with another machine's signing key
//!   id as its `machine` is that machine's agent: the first machine to name
//!   an id keeps it, and an id another machine names is refused
//!   (`remote`), so no machine can speak as another's agent.
//!
//! This machine's own rows have `machine` = `projects::LOCAL`.

use crate::projects::LOCAL;
use crate::{Db, StoreError, R};
use rusqlite::{params, Connection, OptionalExtension, Row};
use xenonet_model::ids::{AgentId, Ids};

pub(crate) const TABLES: &[&str] = &[
    // Not STRICT: the engine binds its times as REALs, which the columns'
    // affinity keeps as integers.
    "CREATE TABLE agents (
        id TEXT PRIMARY KEY,
        machine TEXT NOT NULL,
        app TEXT NOT NULL,
        vendor_id TEXT,
        checkout TEXT,
        cwd TEXT,
        via TEXT,
        pid INTEGER,
        started_by TEXT,
        digest TEXT,
        first_seen INTEGER NOT NULL,
        last_seen INTEGER,
        ended_at INTEGER,
        UNIQUE (machine, app, vendor_id)
    )",
    "CREATE INDEX agents_pending ON agents (machine, app, pid) WHERE vendor_id IS NULL",
];

/// An agent as the store keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub id: AgentId,
    pub machine: String,
    pub app: String,
    /// The id its app gave the session; None while a start waits for it.
    pub vendor_id: Option<String>,
    pub checkout: Option<String>,
    pub cwd: Option<String>,
    /// How it was last in touch directly (mcp, a hook), if it was.
    pub via: Option<String>,
    pub pid: Option<i64>,
    /// Who started it, for a session XenoNet started.
    pub started_by: Option<String>,
    pub digest: Option<String>,
    pub first_seen: i64,
    pub last_seen: Option<i64>,
    pub ended_at: Option<i64>,
}

impl Agent {
    /// The session as its app names it here, `<app>:<vendor id>`.
    pub fn session(&self) -> Option<String> {
        self.vendor_id.as_ref().map(|v| format!("{}:{v}", self.app))
    }
}

fn agent_of_row(r: &Row<'_>) -> rusqlite::Result<Agent> {
    let int = |k: &str| -> rusqlite::Result<Option<i64>> { Ok(r.get::<_, Option<f64>>(k)?.map(|x| x as i64)) };
    Ok(Agent {
        id: AgentId::parse(&r.get::<_, String>("id")?).map_err(|e| rusqlite::Error::InvalidColumnName(e.0))?,
        machine: r.get("machine")?,
        app: r.get("app")?,
        vendor_id: r.get("vendor_id")?,
        checkout: r.get("checkout")?,
        cwd: r.get("cwd")?,
        via: r.get("via")?,
        pid: int("pid")?,
        started_by: r.get("started_by")?,
        digest: r.get("digest")?,
        first_seen: int("first_seen")?.unwrap_or_default(),
        last_seen: int("last_seen")?,
        ended_at: int("ended_at")?,
    })
}

/// `<app>:<vendor id>` as its parts; None for text that isn't one.
pub fn split_session(session: &str) -> Option<(&str, &str)> {
    session.split_once(':').filter(|(a, v)| !a.is_empty() && !v.is_empty())
}

fn by_session(c: &Connection, machine: &str, app: &str, vendor: &str) -> R<Option<Agent>> {
    Ok(c.prepare_cached("SELECT * FROM agents WHERE machine = ?1 AND app = ?2 AND vendor_id = ?3")?.query_row(params![machine, app, vendor], agent_of_row).optional()?)
}

fn by_id(c: &Connection, id: &str) -> R<Option<Agent>> {
    Ok(c.prepare_cached("SELECT * FROM agents WHERE id = ?1")?.query_row(params![id], agent_of_row).optional()?)
}

/// The local session's agent, made when there's none: a pending row of
/// its app started as `pid` takes it, else a new id.
pub(crate) fn mint(c: &Connection, ids: &Ids, session: &str, pid: Option<i64>, now: i64) -> R<AgentId> {
    let Some((app, vendor)) = split_session(session) else {
        return Err(StoreError::new("bad-entry", format!("{session} isn't a session's id")));
    };
    if let Some(a) = by_session(c, LOCAL, app, vendor)? {
        return Ok(a.id);
    }
    if let Some(pid) = pid {
        let pending: Option<String> = c
            .prepare_cached("SELECT id FROM agents WHERE machine = ?1 AND app = ?2 AND vendor_id IS NULL AND pid = ?3 ORDER BY first_seen LIMIT 1")?
            .query_row(params![LOCAL, app, pid], |r| r.get(0))
            .optional()?;
        if let Some(id) = pending {
            c.execute("UPDATE agents SET vendor_id = ?2 WHERE id = ?1", params![id, vendor])?;
            return AgentId::parse(&id).map_err(|e| StoreError::new("bad-entry", e.to_string()));
        }
    }
    let id = ids.agent();
    c.execute("INSERT INTO agents (id, machine, app, vendor_id, first_seen) VALUES (?1, ?2, ?3, ?4, ?5)", params![id.as_str(), LOCAL, app, vendor, now])?;
    Ok(id)
}

impl Db {
    /// The agent of this machine's session `<app>:<vendor id>`, made the
    /// first time it's seen.
    pub fn agent_of(&self, session: &str, pid: Option<i64>, now: i64) -> R<AgentId> {
        let ids = self.ids();
        self.write(|c| mint(c, &ids, session, pid, now))
    }

    /// The agents of these sessions of this machine's, each made the first
    /// time it's seen: what the map does as it lists them.
    pub fn agents_of(&self, sessions: &[String], now: i64) -> R<Vec<(String, AgentId)>> {
        let ids = self.ids();
        self.write(|c| {
            let mut out = Vec::new();
            for s in sessions {
                if split_session(s).is_some() {
                    out.push((s.clone(), mint(c, &ids, s, None, now)?));
                }
            }
            Ok(out)
        })
    }

    /// This machine's session's agent, when it has one; nothing is made.
    pub fn agent_known(&self, session: &str) -> R<Option<AgentId>> {
        let Some((app, vendor)) = split_session(session) else { return Ok(None) };
        self.read(|c| Ok(by_session(c, LOCAL, app, vendor)?.map(|a| a.id)))
    }

    pub fn agent(&self, id: &AgentId) -> R<Option<Agent>> {
        self.read(|c| by_id(c, id.as_str()))
    }

    /// The session an agent of this machine's is, `<app>:<vendor id>`;
    /// None for another machine's, an unknown one, or one still pending.
    pub fn session_of(&self, id: &AgentId) -> R<Option<String>> {
        Ok(self.agent(id)?.filter(|a| a.machine == LOCAL).and_then(|a| a.session()))
    }

    /// A row for a session XenoNet starts, before its app names it: the
    /// process started and who asked. The session's first touch from that
    /// process takes it over (`agent_of`).
    pub fn pending_agent(&self, app: &str, pid: Option<i64>, started_by: Option<&str>, cwd: Option<&str>, now: i64) -> R<AgentId> {
        let id = self.ids().agent();
        self.write(|c| {
            c.execute(
                "INSERT INTO agents (id, machine, app, cwd, pid, started_by, first_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![id.as_str(), LOCAL, app, cwd, pid, started_by, now],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Another machine's agent, as an entry or presence from `machine`
    /// names it: kept the first time, and Ok(false) when `machine` isn't the
    /// one that first named the id (another machine can't speak as it).
    pub fn remote_agent(&self, id: &AgentId, machine: &str, session: Option<&str>, now: i64) -> R<bool> {
        if machine == LOCAL {
            return Ok(false);
        }
        self.write(|c| {
            if let Some(a) = by_id(c, id.as_str())? {
                if a.machine != machine {
                    return Ok(false);
                }
                c.execute("UPDATE agents SET last_seen = ?2 WHERE id = ?1", params![id.as_str(), now])?;
                return Ok(true);
            }
            let (app, vendor) = session.and_then(split_session).map_or((String::new(), None), |(a, v)| (a.to_string(), Some(v.to_string())));
            c.execute("INSERT INTO agents (id, machine, app, vendor_id, first_seen, last_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?5)", params![id.as_str(), machine, app, vendor, now])?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        let db = Db::memory().unwrap();
        db.use_ids(Ids::seeded("agents"));
        db
    }

    #[test]
    fn a_session_gets_one_agent_the_first_time_its_seen() {
        let db = db();
        let a = db.agent_of("claude:4f1c2a3b", None, 1).unwrap();
        assert!(a.as_str().starts_with("agt_"));
        assert_eq!(db.agent_of("claude:4f1c2a3b", Some(9), 2).unwrap(), a, "the stored mapping, every time");
        assert_eq!(db.agent_known("claude:4f1c2a3b").unwrap(), Some(a.clone()));
        assert_eq!(db.session_of(&a).unwrap().as_deref(), Some("claude:4f1c2a3b"));
        assert_eq!(db.agent_known("codex:other").unwrap(), None, "a lookup makes nothing");
        let many = db.agents_of(&["claude:4f1c2a3b".into(), "codex:0199abcdef".into(), "nope".into()], 3).unwrap();
        assert_eq!(many.len(), 2);
        assert_eq!(many[0].1, a);
        assert!(db.agent_of("not a session", None, 1).is_err());
    }

    #[test]
    fn a_started_session_takes_its_pending_row_by_its_process() {
        let db = db();
        let started = db.pending_agent("codex", Some(4242), Some("actor:you"), Some("/w/a"), 1).unwrap();
        assert_eq!(db.session_of(&started).unwrap(), None, "no session yet");
        assert_ne!(db.agent_of("codex:0199aaaa", Some(1), 2).unwrap(), started, "another process isn't it");
        assert_eq!(db.agent_of("codex:0199bbbb", Some(4242), 3).unwrap(), started, "its own process is");
        let row = db.agent(&started).unwrap().unwrap();
        assert_eq!((row.started_by.as_deref(), row.session().as_deref()), (Some("actor:you"), Some("codex:0199bbbb")));
    }

    #[test]
    fn another_machines_agent_is_that_machines_alone() {
        let db = db();
        let theirs = AgentId::from_bytes(&[5; 20]);
        assert!(db.remote_agent(&theirs, "xns1_laptop", Some("claude:abc1234"), 1).unwrap());
        assert!(db.remote_agent(&theirs, "xns1_laptop", None, 2).unwrap(), "the same machine again");
        assert!(!db.remote_agent(&theirs, "xns1_other", None, 3).unwrap(), "another machine can't speak as it");
        let mine = db.agent_of("claude:mine1234", None, 1).unwrap();
        assert!(!db.remote_agent(&mine, "xns1_laptop", None, 4).unwrap(), "nor as this machine's");
        assert!(!db.remote_agent(&AgentId::from_bytes(&[6; 20]), LOCAL, None, 5).unwrap(), "a remote agent is another machine's");
        assert_eq!(db.session_of(&theirs).unwrap(), None, "another machine's agent is no session here");
    }
}
