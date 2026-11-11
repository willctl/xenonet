//! A2A's tasks, in the store: the switch, the agents that called, each
//! one's contexts (a XenoNet thread each), its tasks (one message each, by
//! the message's mid) and the replies that came to them.
//!
//! They hold what agents and sessions wrote, so they're the owner's alone,
//! as the whole store is (0600). A finished task goes a week after it
//! ended, as the engine's messages go after a week, and an agent keeps its
//! latest 1000.

use crate::{Db, R};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::sync::Arc;
/// How long a finished task is kept: as long as the engine keeps messages.
pub const KEEP_MS: f64 = 7.0 * 24.0 * 3600e3;
/// How many tasks an agent keeps at most, its latest.
pub const KEEP_PER_AGENT: i64 = 1000;
/// How long a message that answered no task is kept.
const LOOSE_MS: f64 = KEEP_MS;

/// A2A's task states, as ProtoJSON writes them.
pub const SUBMITTED: &str = "TASK_STATE_SUBMITTED";
pub const WORKING: &str = "TASK_STATE_WORKING";
pub const COMPLETED: &str = "TASK_STATE_COMPLETED";
pub const FAILED: &str = "TASK_STATE_FAILED";
pub const CANCELED: &str = "TASK_STATE_CANCELED";
pub const REJECTED: &str = "TASK_STATE_REJECTED";
pub const INPUT_REQUIRED: &str = "TASK_STATE_INPUT_REQUIRED";
pub const AUTH_REQUIRED: &str = "TASK_STATE_AUTH_REQUIRED";

/// Every state a client may filter by.
pub const STATES: [&str; 8] = [SUBMITTED, WORKING, COMPLETED, FAILED, CANCELED, INPUT_REQUIRED, REJECTED, AUTH_REQUIRED];

pub fn terminal(state: &str) -> bool {
    matches!(state, COMPLETED | FAILED | CANCELED | REJECTED)
}

pub(crate) const TABLES: &[&str] = &[
    // The switch, and the rest of A2A's own settings.
    "CREATE TABLE a2a_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "CREATE TABLE a2a_agents (
        actor TEXT PRIMARY KEY,
        first_at REAL NOT NULL,
        last_at REAL NOT NULL,
        target TEXT
    )",
    "CREATE TABLE a2a_contexts (
        actor TEXT NOT NULL,
        id TEXT NOT NULL,
        target TEXT,
        last_message INTEGER,
        last_sender TEXT,
        created_at REAL NOT NULL,
        updated_at REAL NOT NULL,
        PRIMARY KEY (actor, id)
    )",
    "CREATE TABLE a2a_tasks (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        actor TEXT NOT NULL,
        context TEXT NOT NULL,
        client_message TEXT NOT NULL,
        message INTEGER,
        target TEXT,
        recipients TEXT NOT NULL DEFAULT '[]',
        state TEXT NOT NULL,
        reason TEXT,
        request TEXT NOT NULL,
        created_at REAL NOT NULL,
        updated_at REAL NOT NULL,
        UNIQUE (actor, client_message)
    )",
    "CREATE INDEX a2a_tasks_by_actor ON a2a_tasks (actor, updated_at DESC, seq DESC)",
    "CREATE INDEX a2a_tasks_by_message ON a2a_tasks (message)",
    "CREATE TABLE a2a_replies (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        task TEXT NOT NULL,
        message INTEGER NOT NULL,
        mid TEXT,
        sender TEXT,
        sender_name TEXT,
        text TEXT NOT NULL,
        at REAL NOT NULL,
        UNIQUE (task, message)
    )",
    "CREATE INDEX a2a_replies_by_message ON a2a_replies (message)",
    "CREATE TABLE a2a_loose (
        actor TEXT NOT NULL,
        message INTEGER NOT NULL,
        sender TEXT,
        text TEXT NOT NULL,
        at REAL NOT NULL,
        PRIMARY KEY (actor, message)
    )",
    // Outbound actors, the agents XenoNet calls: what the person approved
    // when they registered one (its card's pin, the address it's called
    // at, the other host and the private host they allowed, who may hand
    // it work and until when). Its bearer is in the credential store.
    "CREATE TABLE a2a_outbound (
        slug TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        card_url TEXT NOT NULL,
        hash TEXT NOT NULL,
        interface_url TEXT NOT NULL,
        approved_host TEXT,
        private_host TEXT,
        agents TEXT NOT NULL,
        projects TEXT NOT NULL,
        until REAL NOT NULL,
        created_at REAL NOT NULL
    )",
    // Work handed to them: one row a delegation, with where its task
    // stands and what the agent last said, which the delegating session
    // reads back.
    "CREATE TABLE a2a_delegations (
        id TEXT PRIMARY KEY,
        slug TEXT NOT NULL,
        agent TEXT NOT NULL,
        task TEXT,
        context TEXT,
        state TEXT NOT NULL,
        text TEXT,
        created_at REAL NOT NULL,
        updated_at REAL NOT NULL
    )",
    "CREATE INDEX a2a_delegations_by_agent ON a2a_delegations (agent, slug, updated_at DESC)",
];

/// An outbound actor as it's kept. `agents` and `projects` are JSON:
/// `"all"`, or a list of ids.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboundRow {
    pub slug: String,
    pub name: String,
    pub card_url: String,
    pub hash: String,
    pub interface_url: String,
    pub approved_host: Option<String>,
    pub private_host: Option<String>,
    pub agents: Value,
    pub projects: Value,
    pub until: f64,
    pub created_at: f64,
}

/// A delegation as it's kept.
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationRow {
    pub id: String,
    pub slug: String,
    pub agent: String,
    pub task: Option<String>,
    pub context: Option<String>,
    /// working, done, asks-you, asks-person, failed, rejected, canceled or
    /// lost.
    pub state: String,
    pub text: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
}

fn outbound_of(r: &Row<'_>) -> rusqlite::Result<OutboundRow> {
    let json = |k: &str| -> rusqlite::Result<Value> { Ok(serde_json::from_str(&r.get::<_, String>(k)?).unwrap_or(Value::Null)) };
    Ok(OutboundRow {
        slug: r.get("slug")?,
        name: r.get("name")?,
        card_url: r.get("card_url")?,
        hash: r.get("hash")?,
        interface_url: r.get("interface_url")?,
        approved_host: r.get("approved_host")?,
        private_host: r.get("private_host")?,
        agents: json("agents")?,
        projects: json("projects")?,
        until: r.get("until")?,
        created_at: r.get("created_at")?,
    })
}

fn delegation_of(r: &Row<'_>) -> rusqlite::Result<DelegationRow> {
    Ok(DelegationRow {
        id: r.get("id")?,
        slug: r.get("slug")?,
        agent: r.get("agent")?,
        task: r.get("task")?,
        context: r.get("context")?,
        state: r.get("state")?,
        text: r.get("text")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

const OUTBOUND_COLUMNS: &str = "slug, name, card_url, hash, interface_url, approved_host, private_host, agents, projects, until, created_at";
const DELEGATION_COLUMNS: &str = "id, slug, agent, task, context, state, text, created_at, updated_at";

/// A task as it's kept.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    pub seq: i64,
    pub id: String,
    pub actor: String,
    pub context: String,
    pub client_message: String,
    pub message: Option<i64>,
    pub target: Option<String>,
    pub recipients: Vec<String>,
    pub state: String,
    pub reason: Option<String>,
    /// The client's message, as it was taken (with its taskId and contextId).
    pub request: Value,
    pub created_at: f64,
    pub updated_at: f64,
}

/// A reply that came to a task.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplyRow {
    pub message: i64,
    pub mid: Option<String>,
    pub sender: Option<String>,
    pub sender_name: Option<String>,
    pub text: String,
    pub at: f64,
}

/// A context: one XenoNet thread of an agent's.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextRow {
    pub id: String,
    pub target: Option<String>,
    pub last_message: Option<i64>,
    pub last_sender: Option<String>,
}

/// What ListTasks asks for.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub context: Option<String>,
    pub state: Option<String>,
    pub after: Option<f64>,
    /// Past this (updated_at, seq), in the list's order.
    pub cursor: Option<(f64, i64)>,
    pub limit: i64,
}

/// A2A's tables in the store.
pub struct A2aStore {
    db: Arc<Db>,
}

fn task_of(r: &Row<'_>) -> rusqlite::Result<TaskRow> {
    let recipients: String = r.get("recipients")?;
    let request: String = r.get("request")?;
    Ok(TaskRow {
        seq: r.get("seq")?,
        id: r.get("id")?,
        actor: r.get("actor")?,
        context: r.get("context")?,
        client_message: r.get("client_message")?,
        message: r.get("message")?,
        target: r.get("target")?,
        recipients: serde_json::from_str::<Vec<String>>(&recipients).unwrap_or_default(),
        state: r.get("state")?,
        reason: r.get("reason")?,
        request: serde_json::from_str(&request).unwrap_or(Value::Null),
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

const TASK_COLUMNS: &str = "seq, id, actor, context, client_message, message, target, recipients, state, reason, request, created_at, updated_at";

impl A2aStore {
    pub fn new(db: Arc<Db>) -> A2aStore {
        A2aStore { db }
    }

    // ---------- the switch ----------

    pub fn setting(&self, key: &str) -> R<Option<String>> {
        self.db.read(|c| Ok(c.query_row("SELECT value FROM a2a_settings WHERE key = ?", [key], |r| r.get(0)).optional()?))
    }

    pub fn set_setting(&self, key: &str, value: &str) -> R<()> {
        self.db.write(|c| {
            c.execute("INSERT INTO a2a_settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2", params![key, value])?;
            Ok(())
        })
    }

    // ---------- agents ----------

    /// An agent's call, written down: the panel lists the agents that called.
    pub fn called(&self, actor: &str, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute("INSERT INTO a2a_agents (actor, first_at, last_at) VALUES (?1, ?2, ?2) ON CONFLICT (actor) DO UPDATE SET last_at = ?2", params![actor, at])?;
            Ok(())
        })
    }

    /// The target an agent's messages go to when they name none.
    pub fn default_target(&self, actor: &str) -> R<Option<String>> {
        self.db.read(|c| Ok(c.query_row("SELECT target FROM a2a_agents WHERE actor = ?", [actor], |r| r.get::<_, Option<String>>(0)).optional()?.flatten()))
    }

    pub fn set_default_target(&self, actor: &str, target: Option<&str>, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute(
                "INSERT INTO a2a_agents (actor, first_at, last_at, target) VALUES (?1, ?2, ?2, ?3) ON CONFLICT (actor) DO UPDATE SET target = ?3",
                params![actor, at, target],
            )?;
            Ok(())
        })
    }

    /// The agents that called, latest first: { id, lastAt, target }.
    pub fn agents(&self) -> R<Vec<Value>> {
        self.db.read(|c| {
            let mut st = c.prepare("SELECT actor, last_at, target FROM a2a_agents ORDER BY last_at DESC LIMIT 200")?;
            let rows = st.query_map([], |r| {
                let (id, at, target): (String, f64, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                Ok(json!({ "id": id, "lastAt": xenonet_text::json::number(at), "target": target }))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Everything kept of an agent: it was removed.
    pub fn forget_agent(&self, actor: &str) -> R<()> {
        self.db.write(|c| {
            c.execute("DELETE FROM a2a_replies WHERE task IN (SELECT id FROM a2a_tasks WHERE actor = ?)", [actor])?;
            for table in ["a2a_tasks", "a2a_contexts", "a2a_loose", "a2a_agents"] {
                c.execute(&format!("DELETE FROM {table} WHERE actor = ?"), [actor])?;
            }
            Ok(())
        })
    }

    // ---------- contexts ----------

    pub fn context(&self, actor: &str, id: &str) -> R<Option<ContextRow>> {
        self.db.read(|c| {
            Ok(c.query_row("SELECT id, target, last_message, last_sender FROM a2a_contexts WHERE actor = ? AND id = ?", [actor, id], |r| {
                Ok(ContextRow { id: r.get(0)?, target: r.get(1)?, last_message: r.get(2)?, last_sender: r.get(3)? })
            })
            .optional()?)
        })
    }

    pub fn ensure_context(&self, actor: &str, id: &str, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute("INSERT OR IGNORE INTO a2a_contexts (actor, id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)", params![actor, id, at])?;
            Ok(())
        })
    }

    /// A message went out in the thread: the next one answers it.
    pub fn context_sent(&self, actor: &str, id: &str, target: &str, message: i64, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute(
                "UPDATE a2a_contexts SET target = COALESCE(target, ?3), last_message = ?4, updated_at = ?5 WHERE actor = ?1 AND id = ?2",
                params![actor, id, target, message, at],
            )?;
            Ok(())
        })
    }

    /// A reply came in the thread: the next message goes back to its sender.
    pub fn context_heard(&self, actor: &str, id: &str, message: i64, sender: Option<&str>, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute(
                "UPDATE a2a_contexts SET last_message = ?3, last_sender = COALESCE(?4, last_sender), updated_at = ?5 WHERE actor = ?1 AND id = ?2",
                params![actor, id, message, sender, at],
            )?;
            Ok(())
        })
    }

    // ---------- tasks ----------

    pub fn insert_task(&self, t: &TaskRow) -> R<i64> {
        self.db.write(|c| {
            c.execute(
                "INSERT INTO a2a_tasks (id, actor, context, client_message, message, target, recipients, state, reason, request, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    t.id,
                    t.actor,
                    t.context,
                    t.client_message,
                    t.message,
                    t.target,
                    serde_json::to_string(&t.recipients).unwrap_or_else(|_| "[]".into()),
                    t.state,
                    t.reason,
                    t.request.to_string(),
                    t.created_at,
                    t.updated_at
                ],
            )?;
            let seq = c.last_insert_rowid();
            prune_in(c, &t.actor, t.updated_at)?;
            Ok(seq)
        })
    }

    /// One of the agent's tasks; another agent's is as good as none.
    pub fn task(&self, actor: &str, id: &str) -> R<Option<TaskRow>> {
        self.db.read(|c| Ok(c.query_row(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ? AND id = ?"), [actor, id], task_of).optional()?))
    }

    /// The agent's task for its own message id, when it sent it before.
    pub fn task_by_client_message(&self, actor: &str, client_message: &str) -> R<Option<TaskRow>> {
        self.db.read(|c| Ok(c.query_row(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ? AND client_message = ?"), [actor, client_message], task_of).optional()?))
    }

    /// The agent's task whose XenoNet message is `message`.
    pub fn task_by_message(&self, actor: &str, message: i64) -> R<Option<TaskRow>> {
        self.db.read(|c| Ok(c.query_row(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ? AND message = ?"), params![actor, message], task_of).optional()?))
    }

    /// The agent's task that a reply with this XenoNet id went to.
    pub fn task_by_reply(&self, actor: &str, message: i64) -> R<Option<TaskRow>> {
        self.db.read(|c| {
            Ok(c.query_row(
                &format!(
                    "SELECT {} FROM a2a_tasks t JOIN a2a_replies r ON r.task = t.id WHERE t.actor = ? AND r.message = ? LIMIT 1",
                    TASK_COLUMNS.split(", ").map(|c| format!("t.{c}")).collect::<Vec<_>>().join(", ")
                ),
                params![actor, message],
                task_of,
            )
            .optional()?)
        })
    }

    /// The agent's newest open task in a context.
    pub fn open_in_context(&self, actor: &str, context: &str) -> R<Option<TaskRow>> {
        self.db.read(|c| {
            Ok(c.query_row(
                &format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ?1 AND context = ?2 AND state IN (?3, ?4) ORDER BY seq DESC LIMIT 1"),
                params![actor, context, SUBMITTED, WORKING],
                task_of,
            )
            .optional()?)
        })
    }

    /// The agent's oldest open task that went to `sender`: a session that
    /// answers without saying which message it answers is answering that.
    pub fn open_to(&self, actor: &str, sender: &str) -> R<Option<TaskRow>> {
        self.db.read(|c| {
            let mut st = c.prepare(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ?1 AND state IN (?2, ?3) AND message IS NOT NULL ORDER BY seq"))?;
            let rows = st.query_map(params![actor, SUBMITTED, WORKING], task_of)?;
            for t in rows {
                let t = t?;
                if t.recipients.iter().any(|r| r == sender) {
                    return Ok(Some(t));
                }
            }
            Ok(None)
        })
    }

    /// The agent's latest task that went to `sender`, whatever its state.
    pub fn latest_to(&self, actor: &str, sender: &str) -> R<Option<TaskRow>> {
        self.db.read(|c| {
            let mut st = c.prepare(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE actor = ?1 AND message IS NOT NULL ORDER BY seq DESC LIMIT 200"))?;
            let rows = st.query_map(params![actor], task_of)?;
            for t in rows {
                let t = t?;
                if t.recipients.iter().any(|r| r == sender) {
                    return Ok(Some(t));
                }
            }
            Ok(None)
        })
    }

    /// A task's state, and why (for a refusal, a failure or a cancel).
    pub fn set_state(&self, id: &str, state: &str, reason: Option<&str>, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute("UPDATE a2a_tasks SET state = ?2, reason = ?3, updated_at = ?4 WHERE id = ?1", params![id, state, reason, at])?;
            Ok(())
        })
    }

    /// The task's message went out: its XenoNet id, what it went to, and
    /// who got it.
    pub fn set_sent(&self, id: &str, message: i64, target: &str, recipients: &[String], at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute(
                "UPDATE a2a_tasks SET message = ?2, target = ?3, recipients = ?4, state = ?5, reason = NULL, updated_at = ?6 WHERE id = ?1",
                params![id, message, target, serde_json::to_string(recipients).unwrap_or_else(|_| "[]".into()), WORKING, at],
            )?;
            Ok(())
        })
    }

    pub fn set_target(&self, id: &str, target: &str) -> R<()> {
        self.db.write(|c| {
            c.execute("UPDATE a2a_tasks SET target = ?2 WHERE id = ?1", params![id, target])?;
            Ok(())
        })
    }

    /// A reply to a task; false when it was there already.
    pub fn add_reply(&self, task: &str, r: &ReplyRow) -> R<bool> {
        self.db.write(|c| {
            let n = c.execute(
                "INSERT OR IGNORE INTO a2a_replies (task, message, mid, sender, sender_name, text, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![task, r.message, r.mid, r.sender, r.sender_name, r.text, r.at],
            )?;
            Ok(n > 0)
        })
    }

    pub fn replies(&self, task: &str) -> R<Vec<ReplyRow>> {
        self.db.read(|c| {
            let mut st = c.prepare("SELECT message, mid, sender, sender_name, text, at FROM a2a_replies WHERE task = ? ORDER BY seq")?;
            let rows = st.query_map([task], |r| Ok(ReplyRow { message: r.get(0)?, mid: r.get(1)?, sender: r.get(2)?, sender_name: r.get(3)?, text: r.get(4)?, at: r.get(5)? }))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// A message to the agent that answered none of its tasks.
    pub fn add_loose(&self, actor: &str, message: i64, sender: Option<&str>, text: &str, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute("INSERT OR IGNORE INTO a2a_loose (actor, message, sender, text, at) VALUES (?1, ?2, ?3, ?4, ?5)", params![actor, message, sender, text, at])?;
            Ok(())
        })
    }

    pub fn loose_count(&self, actor: &str) -> R<i64> {
        self.db.read(|c| Ok(c.query_row("SELECT count(*) FROM a2a_loose WHERE actor = ?", [actor], |r| r.get(0))?))
    }

    /// The agent's tasks, latest change first, and how many match in all.
    pub fn list(&self, actor: &str, f: &Filter) -> R<(Vec<TaskRow>, i64)> {
        self.db.read(|c| {
            let mut clause = String::from("actor = ?1");
            let mut args: Vec<rusqlite::types::Value> = vec![actor.to_string().into()];
            let mut push = |sql: &str, v: rusqlite::types::Value, args: &mut Vec<rusqlite::types::Value>| {
                args.push(v);
                clause.push_str(&sql.replace('#', &format!("?{}", args.len())));
            };
            if let Some(c) = &f.context {
                push(" AND context = #", c.clone().into(), &mut args);
            }
            if let Some(s) = &f.state {
                push(" AND state = #", s.clone().into(), &mut args);
            }
            if let Some(t) = f.after {
                push(" AND updated_at >= #", t.into(), &mut args);
            }
            let total: i64 = c.query_row(&format!("SELECT count(*) FROM a2a_tasks WHERE {clause}"), rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
            let mut page_clause = clause.clone();
            let mut page_args = args.clone();
            if let Some((at, seq)) = f.cursor {
                page_args.push(at.into());
                let a = page_args.len();
                page_args.push(seq.into());
                let s = page_args.len();
                let _ = write!(page_clause, " AND (updated_at < ?{a} OR (updated_at = ?{a} AND seq < ?{s}))");
            }
            page_args.push(f.limit.into());
            let l = page_args.len();
            let mut st = c.prepare(&format!("SELECT {TASK_COLUMNS} FROM a2a_tasks WHERE {page_clause} ORDER BY updated_at DESC, seq DESC LIMIT ?{l}"))?;
            let rows = st.query_map(rusqlite::params_from_iter(page_args.iter()), task_of)?;
            Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
        })
    }

    /// Old finished tasks, and an agent's beyond its latest 1000, go.
    pub fn prune(&self, actor: &str, now: f64) -> R<()> {
        self.db.write(|c| Ok(prune_in(c, actor, now)?))
    }

    // ---------- outbound actors ----------

    /// An outbound actor, registered or registered again.
    pub fn put_outbound(&self, o: &OutboundRow) -> R<()> {
        self.db.write(|c| {
            c.execute(
                &format!("INSERT OR REPLACE INTO a2a_outbound ({OUTBOUND_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
                params![o.slug, o.name, o.card_url, o.hash, o.interface_url, o.approved_host, o.private_host, o.agents.to_string(), o.projects.to_string(), o.until, o.created_at],
            )?;
            Ok(())
        })
    }

    pub fn outbound(&self, slug: &str) -> R<Option<OutboundRow>> {
        self.db.read(|c| Ok(c.query_row(&format!("SELECT {OUTBOUND_COLUMNS} FROM a2a_outbound WHERE slug = ?"), [slug], outbound_of).optional()?))
    }

    pub fn outbounds(&self) -> R<Vec<OutboundRow>> {
        self.db.read(|c| {
            let mut st = c.prepare(&format!("SELECT {OUTBOUND_COLUMNS} FROM a2a_outbound ORDER BY slug"))?;
            let rows = st.query_map([], outbound_of)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }

    /// An outbound actor removed, with its delegations.
    pub fn remove_outbound(&self, slug: &str) -> R<bool> {
        self.db.write(|c| {
            c.execute("DELETE FROM a2a_delegations WHERE slug = ?", [slug])?;
            Ok(c.execute("DELETE FROM a2a_outbound WHERE slug = ?", [slug])? > 0)
        })
    }

    /// A new delegation; ones finished a week ago go.
    pub fn insert_delegation(&self, d: &DelegationRow) -> R<()> {
        self.db.write(|c| {
            c.execute(
                &format!("INSERT INTO a2a_delegations ({DELEGATION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"),
                params![d.id, d.slug, d.agent, d.task, d.context, d.state, d.text, d.created_at, d.updated_at],
            )?;
            c.execute("DELETE FROM a2a_delegations WHERE updated_at < ?1", params![d.created_at - KEEP_MS])?;
            Ok(())
        })
    }

    pub fn delegation(&self, id: &str) -> R<Option<DelegationRow>> {
        self.db.read(|c| Ok(c.query_row(&format!("SELECT {DELEGATION_COLUMNS} FROM a2a_delegations WHERE id = ?"), [id], delegation_of).optional()?))
    }

    /// Where a delegation stands now; a task or context it learned is kept.
    pub fn set_delegation(&self, id: &str, task: Option<&str>, context: Option<&str>, state: &str, text: Option<&str>, at: f64) -> R<()> {
        self.db.write(|c| {
            c.execute(
                "UPDATE a2a_delegations SET task = COALESCE(?2, task), context = COALESCE(?3, context), state = ?4, text = ?5, updated_at = ?6 WHERE id = ?1",
                params![id, task, context, state, text, at],
            )?;
            Ok(())
        })
    }

    /// `agent`'s latest delegation to `slug`.
    pub fn latest_delegation(&self, agent: &str, slug: &str) -> R<Option<DelegationRow>> {
        self.db.read(|c| {
            Ok(c.query_row(
                &format!("SELECT {DELEGATION_COLUMNS} FROM a2a_delegations WHERE agent = ?1 AND slug = ?2 ORDER BY updated_at DESC LIMIT 1"),
                [agent, slug],
                delegation_of,
            )
            .optional()?)
        })
    }

    /// The latest delegations to `slug`, for the window.
    pub fn delegations(&self, slug: &str) -> R<Vec<DelegationRow>> {
        self.db.read(|c| {
            let mut st = c.prepare(&format!("SELECT {DELEGATION_COLUMNS} FROM a2a_delegations WHERE slug = ? ORDER BY updated_at DESC LIMIT 50"))?;
            let rows = st.query_map([slug], delegation_of)?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
    }
}

// Old finished tasks, and an agent's beyond its latest 1000, go: inside
// the write that's open.
fn prune_in(c: &Connection, actor: &str, now: f64) -> rusqlite::Result<()> {
    let old = now - KEEP_MS;
    let done = [COMPLETED, FAILED, CANCELED, REJECTED];
    c.execute(
        "DELETE FROM a2a_replies WHERE task IN (SELECT id FROM a2a_tasks WHERE updated_at < ?1 AND state IN (?2, ?3, ?4, ?5))",
        params![old, done[0], done[1], done[2], done[3]],
    )?;
    c.execute("DELETE FROM a2a_tasks WHERE updated_at < ?1 AND state IN (?2, ?3, ?4, ?5)", params![old, done[0], done[1], done[2], done[3]])?;
    let over = "SELECT id FROM a2a_tasks WHERE actor = ?1 ORDER BY seq DESC LIMIT -1 OFFSET ?2";
    c.execute(&format!("DELETE FROM a2a_replies WHERE task IN ({over})"), params![actor, KEEP_PER_AGENT])?;
    c.execute(&format!("DELETE FROM a2a_tasks WHERE id IN ({over})"), params![actor, KEEP_PER_AGENT])?;
    c.execute("DELETE FROM a2a_contexts WHERE updated_at < ?1", params![old])?;
    c.execute("DELETE FROM a2a_loose WHERE at < ?1", params![now - LOOSE_MS])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, actor: &str, client: &str, at: f64) -> TaskRow {
        TaskRow {
            seq: 0,
            id: id.into(),
            actor: actor.into(),
            context: "ctx_1".into(),
            client_message: client.into(),
            message: None,
            target: None,
            recipients: Vec::new(),
            state: SUBMITTED.into(),
            reason: None,
            request: json!({ "messageId": client }),
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn a_task_is_its_agents_alone() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        s.insert_task(&task("msg_a", "actor:wren", "m1", 1000.0)).unwrap();
        assert!(s.task("actor:wren", "msg_a").unwrap().is_some());
        assert!(s.task("actor:moth", "msg_a").unwrap().is_none(), "another agent's task is as good as none");
        // One message id, one task.
        assert!(s.insert_task(&task("msg_b", "actor:wren", "m1", 1001.0)).is_err());
        assert!(s.insert_task(&task("msg_c", "actor:moth", "m1", 1001.0)).is_ok(), "another agent may use the same message id");
    }

    #[test]
    fn replies_find_their_task_by_message_or_by_sender() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        s.insert_task(&task("msg_a", "actor:wren", "m1", 1000.0)).unwrap();
        s.set_sent("msg_a", 41, "claude:x", &["claude:x".into()], 1001.0).unwrap();
        assert_eq!(s.task_by_message("actor:wren", 41).unwrap().unwrap().state, WORKING);
        assert_eq!(s.open_to("actor:wren", "claude:x").unwrap().unwrap().id, "msg_a");
        assert!(s.open_to("actor:wren", "codex:y").unwrap().is_none());
        let r = ReplyRow { message: 42, mid: Some("msg_r".into()), sender: Some("claude:x".into()), sender_name: None, text: "done".into(), at: 1002.0 };
        assert!(s.add_reply("msg_a", &r).unwrap());
        assert!(!s.add_reply("msg_a", &r).unwrap(), "a reply is kept once");
        assert_eq!(s.task_by_reply("actor:wren", 42).unwrap().unwrap().id, "msg_a");
        assert_eq!(s.replies("msg_a").unwrap(), vec![r]);
    }

    #[test]
    fn lists_go_newest_first_a_page_at_a_time() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        for i in 0..5 {
            s.insert_task(&task(&format!("msg_{i}"), "actor:wren", &format!("m{i}"), 1000.0 + f64::from(i))).unwrap();
        }
        s.insert_task(&task("msg_other", "actor:moth", "m0", 2000.0)).unwrap();
        let (page, total) = s.list("actor:wren", &Filter { limit: 2, ..Filter::default() }).unwrap();
        assert_eq!(total, 5);
        assert_eq!(page.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["msg_4", "msg_3"]);
        let last = page.last().unwrap();
        let (next, _) = s.list("actor:wren", &Filter { limit: 2, cursor: Some((last.updated_at, last.seq)), ..Filter::default() }).unwrap();
        assert_eq!(next.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["msg_2", "msg_1"]);
        let (after, n) = s.list("actor:wren", &Filter { limit: 50, after: Some(1003.0), ..Filter::default() }).unwrap();
        assert_eq!((after.len(), n), (2, 2));
    }

    #[test]
    fn a_removed_agents_tasks_go_with_it() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        s.insert_task(&task("msg_a", "actor:wren", "m1", 1000.0)).unwrap();
        s.add_reply("msg_a", &ReplyRow { message: 2, mid: None, sender: None, sender_name: None, text: "x".into(), at: 1.0 }).unwrap();
        s.set_default_target("actor:wren", Some("claude:x"), 1.0).unwrap();
        s.insert_task(&task("msg_b", "actor:moth", "m1", 1000.0)).unwrap();
        s.forget_agent("actor:wren").unwrap();
        assert!(s.task("actor:wren", "msg_a").unwrap().is_none());
        assert_eq!(s.replies("msg_a").unwrap(), Vec::<ReplyRow>::new());
        assert!(s.default_target("actor:wren").unwrap().is_none());
        assert!(s.task("actor:moth", "msg_b").unwrap().is_some(), "another agent's stay");
    }

    #[test]
    fn old_finished_tasks_go() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        s.insert_task(&task("msg_old", "actor:wren", "m1", 1000.0)).unwrap();
        s.set_state("msg_old", COMPLETED, None, 1000.0).unwrap();
        s.insert_task(&task("msg_open", "actor:wren", "m2", 1000.0)).unwrap();
        s.prune("actor:wren", 1000.0 + KEEP_MS + 1.0).unwrap();
        assert!(s.task("actor:wren", "msg_old").unwrap().is_none());
        assert!(s.task("actor:wren", "msg_open").unwrap().is_some(), "an open task stays");
    }

    #[test]
    fn the_switch_and_an_agents_default_target_are_kept() {
        let dir = std::env::temp_dir().join(format!("xenonet-a2a-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        {
            let s = A2aStore::new(Arc::new(Db::open(&dir).unwrap()));
            s.set_setting("on", "true").unwrap();
            s.set_default_target("actor:wren", Some("claude:x"), 5.0).unwrap();
        }
        let s = A2aStore::new(Arc::new(Db::open(&dir).unwrap()));
        assert_eq!(s.setting("on").unwrap().as_deref(), Some("true"));
        assert_eq!(s.default_target("actor:wren").unwrap().as_deref(), Some("claude:x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outbound_actors_and_their_delegations() {
        let s = A2aStore::new(Arc::new(Db::memory().unwrap()));
        let o = OutboundRow {
            slug: "scout".into(),
            name: "Scout".into(),
            card_url: "https://agents.example.com/.well-known/agent-card.json".into(),
            hash: "ab".repeat(32),
            interface_url: "https://agents.example.com/a2a".into(),
            approved_host: None,
            private_host: None,
            agents: json!(["claude:fix"]),
            projects: json!("all"),
            until: 5000.0,
            created_at: 1000.0,
        };
        s.put_outbound(&o).unwrap();
        assert_eq!(s.outbound("scout").unwrap(), Some(o.clone()));
        assert_eq!(s.outbounds().unwrap().len(), 1);
        let d = DelegationRow {
            id: "dlg_1".into(),
            slug: "scout".into(),
            agent: "claude:fix".into(),
            task: None,
            context: None,
            state: "working".into(),
            text: None,
            created_at: 1000.0,
            updated_at: 1000.0,
        };
        s.insert_delegation(&d).unwrap();
        s.set_delegation("dlg_1", Some("task-1"), Some("ctx-1"), "done", Some("the answer"), 1100.0).unwrap();
        s.set_delegation("dlg_1", None, None, "done", Some("the answer"), 1200.0).unwrap();
        let got = s.latest_delegation("claude:fix", "scout").unwrap().unwrap();
        assert_eq!((got.task.as_deref(), got.context.as_deref(), got.state.as_str()), (Some("task-1"), Some("ctx-1"), "done"), "a task once learned stays");
        assert_eq!(s.delegations("scout").unwrap().len(), 1);
        assert!(s.remove_outbound("scout").unwrap());
        assert!(s.delegation("dlg_1").unwrap().is_none(), "its delegations go with it");
    }
}
