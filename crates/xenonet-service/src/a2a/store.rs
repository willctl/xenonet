//! The A2A task table, in `<state>/a2a.sqlite`: its own file, so nothing
//! here touches engine.sqlite. It keeps the switch, the agents that called,
//! each one's contexts (a XenoNet thread each), its tasks (one message each,
//! by the message's mid) and the replies that came to them.
//!
//! The file holds what agents and sessions wrote, so it's the owner's alone
//! (0600). A finished task goes a week after it ended, as the engine's
//! messages go after a week, and an agent keeps its latest 1000.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::Path;

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

/// Each step brings the file up one version (PRAGMA user_version).
const STEPS: [&str; 1] = ["
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE agents (
        actor TEXT PRIMARY KEY,
        first_at REAL NOT NULL,
        last_at REAL NOT NULL,
        target TEXT
    );
    CREATE TABLE contexts (
        actor TEXT NOT NULL,
        id TEXT NOT NULL,
        target TEXT,
        last_message INTEGER,
        last_sender TEXT,
        created_at REAL NOT NULL,
        updated_at REAL NOT NULL,
        PRIMARY KEY (actor, id)
    );
    CREATE TABLE tasks (
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
    );
    CREATE INDEX tasks_by_actor ON tasks (actor, updated_at DESC, seq DESC);
    CREATE INDEX tasks_by_message ON tasks (message);
    CREATE TABLE replies (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        task TEXT NOT NULL,
        message INTEGER NOT NULL,
        mid TEXT,
        sender TEXT,
        sender_name TEXT,
        text TEXT NOT NULL,
        at REAL NOT NULL,
        UNIQUE (task, message)
    );
    CREATE INDEX replies_by_message ON replies (message);
    CREATE TABLE loose (
        actor TEXT NOT NULL,
        message INTEGER NOT NULL,
        sender TEXT,
        text TEXT NOT NULL,
        at REAL NOT NULL,
        PRIMARY KEY (actor, message)
    );
"];

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

pub struct Store {
    db: Connection,
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

impl Store {
    /// The file at `path`, made and brought up to date; or an in-memory one
    /// (the demo's, and tests').
    pub fn open(path: Option<&Path>) -> rusqlite::Result<Store> {
        let db = match path {
            Some(p) => {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                // What agents and sessions wrote: the owner's alone, from
                // the start, so SQLite's own files beside it are too.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    let _ = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(p);
                }
                let db = Connection::open(p)?;
                let _ = xenonet_base::private_file::chmod_600(p);
                db.execute_batch("PRAGMA journal_mode = WAL")?;
                db
            }
            None => Connection::open_in_memory()?,
        };
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        let from: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        for (i, step) in STEPS.iter().enumerate().skip(usize::try_from(from).unwrap_or(usize::MAX)) {
            db.execute_batch("BEGIN")?;
            let done = db.execute_batch(step).and_then(|()| db.execute_batch(&format!("PRAGMA user_version = {}", i + 1)));
            match done {
                Ok(()) => db.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = db.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        Ok(Store { db })
    }

    // ---------- the switch ----------

    pub fn setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.db.query_row("SELECT value FROM settings WHERE key = ?", [key], |r| r.get(0)).optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.db.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2", params![key, value])?;
        Ok(())
    }

    // ---------- agents ----------

    /// An agent's call, written down: the panel lists the agents that called.
    pub fn called(&self, actor: &str, at: f64) -> rusqlite::Result<()> {
        self.db.execute("INSERT INTO agents (actor, first_at, last_at) VALUES (?1, ?2, ?2) ON CONFLICT (actor) DO UPDATE SET last_at = ?2", params![actor, at])?;
        Ok(())
    }

    /// The target an agent's messages go to when they name none.
    pub fn default_target(&self, actor: &str) -> rusqlite::Result<Option<String>> {
        Ok(self.db.query_row("SELECT target FROM agents WHERE actor = ?", [actor], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
    }

    pub fn set_default_target(&self, actor: &str, target: Option<&str>, at: f64) -> rusqlite::Result<()> {
        self.db
            .execute("INSERT INTO agents (actor, first_at, last_at, target) VALUES (?1, ?2, ?2, ?3) ON CONFLICT (actor) DO UPDATE SET target = ?3", params![actor, at, target])?;
        Ok(())
    }

    /// The agents that called, latest first: { id, lastAt, target }.
    pub fn agents(&self) -> rusqlite::Result<Vec<Value>> {
        let mut st = self.db.prepare("SELECT actor, last_at, target FROM agents ORDER BY last_at DESC LIMIT 200")?;
        let rows = st.query_map([], |r| {
            let (id, at, target): (String, f64, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?);
            Ok(json!({ "id": id, "lastAt": xenonet_base::json::number(at), "target": target }))
        })?;
        rows.collect()
    }

    /// Everything kept of an agent: it was removed.
    pub fn forget_agent(&self, actor: &str) -> rusqlite::Result<()> {
        self.db.execute("DELETE FROM replies WHERE task IN (SELECT id FROM tasks WHERE actor = ?)", [actor])?;
        for table in ["tasks", "contexts", "loose", "agents"] {
            self.db.execute(&format!("DELETE FROM {table} WHERE actor = ?"), [actor])?;
        }
        Ok(())
    }

    // ---------- contexts ----------

    pub fn context(&self, actor: &str, id: &str) -> rusqlite::Result<Option<ContextRow>> {
        self.db
            .query_row("SELECT id, target, last_message, last_sender FROM contexts WHERE actor = ? AND id = ?", [actor, id], |r| {
                Ok(ContextRow { id: r.get(0)?, target: r.get(1)?, last_message: r.get(2)?, last_sender: r.get(3)? })
            })
            .optional()
    }

    pub fn ensure_context(&self, actor: &str, id: &str, at: f64) -> rusqlite::Result<()> {
        self.db.execute("INSERT OR IGNORE INTO contexts (actor, id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)", params![actor, id, at])?;
        Ok(())
    }

    /// A message went out in the thread: the next one answers it.
    pub fn context_sent(&self, actor: &str, id: &str, target: &str, message: i64, at: f64) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE contexts SET target = COALESCE(target, ?3), last_message = ?4, updated_at = ?5 WHERE actor = ?1 AND id = ?2",
            params![actor, id, target, message, at],
        )?;
        Ok(())
    }

    /// A reply came in the thread: the next message goes back to its sender.
    pub fn context_heard(&self, actor: &str, id: &str, message: i64, sender: Option<&str>, at: f64) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE contexts SET last_message = ?3, last_sender = COALESCE(?4, last_sender), updated_at = ?5 WHERE actor = ?1 AND id = ?2",
            params![actor, id, message, sender, at],
        )?;
        Ok(())
    }

    // ---------- tasks ----------

    pub fn insert_task(&self, t: &TaskRow) -> rusqlite::Result<i64> {
        self.db.execute(
            "INSERT INTO tasks (id, actor, context, client_message, message, target, recipients, state, reason, request, created_at, updated_at)
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
        let seq = self.db.last_insert_rowid();
        self.prune(&t.actor, t.updated_at)?;
        Ok(seq)
    }

    /// One of the agent's tasks; another agent's is as good as none.
    pub fn task(&self, actor: &str, id: &str) -> rusqlite::Result<Option<TaskRow>> {
        self.db.query_row(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ? AND id = ?"), [actor, id], task_of).optional()
    }

    /// The agent's task for its own message id, when it sent it before.
    pub fn task_by_client_message(&self, actor: &str, client_message: &str) -> rusqlite::Result<Option<TaskRow>> {
        self.db.query_row(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ? AND client_message = ?"), [actor, client_message], task_of).optional()
    }

    /// The agent's task whose XenoNet message is `message`.
    pub fn task_by_message(&self, actor: &str, message: i64) -> rusqlite::Result<Option<TaskRow>> {
        self.db.query_row(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ? AND message = ?"), params![actor, message], task_of).optional()
    }

    /// The agent's task that a reply with this XenoNet id went to.
    pub fn task_by_reply(&self, actor: &str, message: i64) -> rusqlite::Result<Option<TaskRow>> {
        self.db
            .query_row(
                &format!(
                    "SELECT {} FROM tasks t JOIN replies r ON r.task = t.id WHERE t.actor = ? AND r.message = ? LIMIT 1",
                    TASK_COLUMNS.split(", ").map(|c| format!("t.{c}")).collect::<Vec<_>>().join(", ")
                ),
                params![actor, message],
                task_of,
            )
            .optional()
    }

    /// The agent's newest open task in a context.
    pub fn open_in_context(&self, actor: &str, context: &str) -> rusqlite::Result<Option<TaskRow>> {
        self.db
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ?1 AND context = ?2 AND state IN (?3, ?4) ORDER BY seq DESC LIMIT 1"),
                params![actor, context, SUBMITTED, WORKING],
                task_of,
            )
            .optional()
    }

    /// The agent's oldest open task that went to `sender`: a session that
    /// answers without saying which message it answers is answering that.
    pub fn open_to(&self, actor: &str, sender: &str) -> rusqlite::Result<Option<TaskRow>> {
        let mut st = self.db.prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ?1 AND state IN (?2, ?3) AND message IS NOT NULL ORDER BY seq"))?;
        let rows = st.query_map(params![actor, SUBMITTED, WORKING], task_of)?;
        for t in rows {
            let t = t?;
            if t.recipients.iter().any(|r| r == sender) {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// The agent's latest task that went to `sender`, whatever its state.
    pub fn latest_to(&self, actor: &str, sender: &str) -> rusqlite::Result<Option<TaskRow>> {
        let mut st = self.db.prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE actor = ?1 AND message IS NOT NULL ORDER BY seq DESC LIMIT 200"))?;
        let rows = st.query_map(params![actor], task_of)?;
        for t in rows {
            let t = t?;
            if t.recipients.iter().any(|r| r == sender) {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// A task's state, and why (for a refusal, a failure or a cancel).
    pub fn set_state(&self, id: &str, state: &str, reason: Option<&str>, at: f64) -> rusqlite::Result<()> {
        self.db.execute("UPDATE tasks SET state = ?2, reason = ?3, updated_at = ?4 WHERE id = ?1", params![id, state, reason, at])?;
        Ok(())
    }

    /// The task's message went out: its XenoNet id, what it went to, and
    /// who got it.
    pub fn set_sent(&self, id: &str, message: i64, target: &str, recipients: &[String], at: f64) -> rusqlite::Result<()> {
        self.db.execute(
            "UPDATE tasks SET message = ?2, target = ?3, recipients = ?4, state = ?5, reason = NULL, updated_at = ?6 WHERE id = ?1",
            params![id, message, target, serde_json::to_string(recipients).unwrap_or_else(|_| "[]".into()), WORKING, at],
        )?;
        Ok(())
    }

    pub fn set_target(&self, id: &str, target: &str) -> rusqlite::Result<()> {
        self.db.execute("UPDATE tasks SET target = ?2 WHERE id = ?1", params![id, target])?;
        Ok(())
    }

    /// A reply to a task; false when it was there already.
    pub fn add_reply(&self, task: &str, r: &ReplyRow) -> rusqlite::Result<bool> {
        let n = self.db.execute(
            "INSERT OR IGNORE INTO replies (task, message, mid, sender, sender_name, text, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![task, r.message, r.mid, r.sender, r.sender_name, r.text, r.at],
        )?;
        Ok(n > 0)
    }

    pub fn replies(&self, task: &str) -> rusqlite::Result<Vec<ReplyRow>> {
        let mut st = self.db.prepare("SELECT message, mid, sender, sender_name, text, at FROM replies WHERE task = ? ORDER BY seq")?;
        let rows = st.query_map([task], |r| Ok(ReplyRow { message: r.get(0)?, mid: r.get(1)?, sender: r.get(2)?, sender_name: r.get(3)?, text: r.get(4)?, at: r.get(5)? }))?;
        rows.collect()
    }

    /// A message to the agent that answered none of its tasks.
    pub fn add_loose(&self, actor: &str, message: i64, sender: Option<&str>, text: &str, at: f64) -> rusqlite::Result<()> {
        self.db.execute("INSERT OR IGNORE INTO loose (actor, message, sender, text, at) VALUES (?1, ?2, ?3, ?4, ?5)", params![actor, message, sender, text, at])?;
        Ok(())
    }

    pub fn loose_count(&self, actor: &str) -> rusqlite::Result<i64> {
        self.db.query_row("SELECT count(*) FROM loose WHERE actor = ?", [actor], |r| r.get(0))
    }

    /// The agent's tasks, latest change first, and how many match in all.
    pub fn list(&self, actor: &str, f: &Filter) -> rusqlite::Result<(Vec<TaskRow>, i64)> {
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
        let total: i64 = self.db.query_row(&format!("SELECT count(*) FROM tasks WHERE {clause}"), rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
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
        let mut st = self.db.prepare(&format!("SELECT {TASK_COLUMNS} FROM tasks WHERE {page_clause} ORDER BY updated_at DESC, seq DESC LIMIT ?{l}"))?;
        let rows = st.query_map(rusqlite::params_from_iter(page_args.iter()), task_of)?;
        Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
    }

    /// Old finished tasks, and an agent's beyond its latest 1000, go.
    pub fn prune(&self, actor: &str, now: f64) -> rusqlite::Result<()> {
        let old = now - KEEP_MS;
        let done = [COMPLETED, FAILED, CANCELED, REJECTED];
        self.db.execute(
            "DELETE FROM replies WHERE task IN (SELECT id FROM tasks WHERE updated_at < ?1 AND state IN (?2, ?3, ?4, ?5))",
            params![old, done[0], done[1], done[2], done[3]],
        )?;
        self.db.execute("DELETE FROM tasks WHERE updated_at < ?1 AND state IN (?2, ?3, ?4, ?5)", params![old, done[0], done[1], done[2], done[3]])?;
        let over = "SELECT id FROM tasks WHERE actor = ?1 ORDER BY seq DESC LIMIT -1 OFFSET ?2";
        self.db.execute(&format!("DELETE FROM replies WHERE task IN ({over})"), params![actor, KEEP_PER_AGENT])?;
        self.db.execute(&format!("DELETE FROM tasks WHERE id IN ({over})"), params![actor, KEEP_PER_AGENT])?;
        self.db.execute("DELETE FROM contexts WHERE updated_at < ?1", params![old])?;
        self.db.execute("DELETE FROM loose WHERE at < ?1", params![now - LOOSE_MS])?;
        Ok(())
    }
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
        let s = Store::open(None).unwrap();
        s.insert_task(&task("msg_a", "actor:wren", "m1", 1000.0)).unwrap();
        assert!(s.task("actor:wren", "msg_a").unwrap().is_some());
        assert!(s.task("actor:moth", "msg_a").unwrap().is_none(), "another agent's task is as good as none");
        // One message id, one task.
        assert!(s.insert_task(&task("msg_b", "actor:wren", "m1", 1001.0)).is_err());
        assert!(s.insert_task(&task("msg_c", "actor:moth", "m1", 1001.0)).is_ok(), "another agent may use the same message id");
    }

    #[test]
    fn replies_find_their_task_by_message_or_by_sender() {
        let s = Store::open(None).unwrap();
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
        let s = Store::open(None).unwrap();
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
        let s = Store::open(None).unwrap();
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
        let s = Store::open(None).unwrap();
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
        let file = dir.join("a2a.sqlite");
        {
            let s = Store::open(Some(&file)).unwrap();
            s.set_setting("on", "true").unwrap();
            s.set_default_target("actor:wren", Some("claude:x"), 5.0).unwrap();
        }
        let s = Store::open(Some(&file)).unwrap();
        assert_eq!(s.setting("on").unwrap().as_deref(), Some("true"));
        assert_eq!(s.default_target("actor:wren").unwrap().as_deref(), Some("claude:x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_the_owners_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xenonet-a2a-mode-{}", std::process::id()));
        let file = dir.join("a2a.sqlite");
        let s = Store::open(Some(&file)).unwrap();
        s.set_setting("on", "true").unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        let wal = dir.join("a2a.sqlite-wal");
        assert_eq!(std::fs::metadata(&wal).unwrap().permissions().mode() & 0o777, 0o600, "and its write-ahead log");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
