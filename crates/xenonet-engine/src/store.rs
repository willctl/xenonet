//! What the engine remembers, in engine.sqlite in the state dir
//! (core/engine/store.js): messages between agents (and which of their
//! recipients have seen them), the files agents have claimed, what each
//! says it's doing, when XenoNet last heard from each agent directly, and
//! which process holds each agent's session (pins; see `verify`). Actors
//! keep their rows, events and targets here too, with registrations
//! waiting for you and revoked tokens, and remote actors' connections and
//! grants, with the bounded log of what they read. One writer, the
//! service; agents and actors never touch the file. The store block
//! (xenonet_blocks::sqlite) opens it and keeps its version, and copies a
//! file with data in it before taking it a step up (`engine.v<n>.sqlite`
//! beside it).
//!
//! Every call binds and reads as node:sqlite does (`sql`), with the
//! arguments as JS gave them, so the store behaves the same behind its JS
//! twin as in the binary.

use crate::sql::{self, Arg, Cell, Changes, Result, Row, SqlError};
use rusqlite::Connection;
use serde_json::{json, Value};
use xenonet_base::ids::YOU;
use xenonet_base::sync::Locked;
use xenonet_blocks::sqlite::{SqliteStore, Step, Version};

const DAY_MS: f64 = 24.0 * 3600e3;
const WEEK_MS: f64 = 7.0 * DAY_MS;
/// Events kept for an actor, whatever their age; the oldest go first.
pub const ACTOR_EVENTS_KEPT: f64 = 10000.0;
/// Remote reads logged (assistant_audit), for a week and at most this
/// many, the oldest going first.
pub const AUDIT_KEPT: f64 = 20000.0;

fn you_slug() -> &'static str {
    &YOU["actor:".len()..]
}

const STEP_ONE: &str = r"
      CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY, sender TEXT NOT NULL, target TEXT NOT NULL,
        body TEXT NOT NULL, reply_to INTEGER, created_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS deliveries (
        message_id INTEGER NOT NULL, agent TEXT NOT NULL, seen_at INTEGER,
        PRIMARY KEY (message_id, agent));
      CREATE INDEX IF NOT EXISTS deliveries_unseen ON deliveries (agent, seen_at);
      CREATE TABLE IF NOT EXISTS claims (
        id INTEGER PRIMARY KEY, agent TEXT NOT NULL, pattern TEXT NOT NULL,
        note TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS focus (agent TEXT PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS contacts (
        agent TEXT PRIMARY KEY, app TEXT NOT NULL, cwd TEXT, via TEXT NOT NULL,
        at INTEGER NOT NULL, ended_at INTEGER);
      CREATE TABLE IF NOT EXISTS pins (
        agent TEXT NOT NULL, pid INTEGER NOT NULL, start INTEGER NOT NULL,
        sure INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (agent, pid, start));
    ";
const STEP_FOUR: &str = r"
      CREATE TABLE IF NOT EXISTS actors (
        slug TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL,
        product TEXT, via TEXT NOT NULL, capabilities TEXT NOT NULL,
        heartbeat_ms INTEGER, identity TEXT,
        token_hash TEXT UNIQUE, token_at INTEGER,
        enabled INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL, allowed_at INTEGER, last_seen INTEGER,
        presence TEXT NOT NULL DEFAULT 'unknown', presence_at INTEGER,
        status TEXT, status_at INTEGER,
        seq INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS actor_requests (
        id TEXT PRIMARY KEY, slug TEXT NOT NULL, ask TEXT NOT NULL,
        token_hash TEXT NOT NULL UNIQUE, asker TEXT,
        created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
        decision TEXT, decided_at INTEGER);
      CREATE TABLE IF NOT EXISTS actor_events (
        id TEXT PRIMARY KEY, actor TEXT NOT NULL, at INTEGER NOT NULL,
        kind TEXT NOT NULL, target TEXT, detail TEXT, via TEXT NOT NULL,
        facts TEXT, ref TEXT);
      CREATE INDEX IF NOT EXISTS actor_events_by_actor ON actor_events (actor, at);
      CREATE INDEX IF NOT EXISTS actor_events_by_time ON actor_events (at);
      CREATE INDEX IF NOT EXISTS actor_events_by_ref ON actor_events (actor, ref) WHERE ref IS NOT NULL;
      CREATE TABLE IF NOT EXISTS actor_targets (
        actor TEXT NOT NULL, target TEXT NOT NULL, relation TEXT NOT NULL,
        since INTEGER NOT NULL, last_at INTEGER NOT NULL,
        PRIMARY KEY (actor, target, relation));
      CREATE TABLE IF NOT EXISTS actor_revoked (
        token_hash TEXT PRIMARY KEY, actor TEXT NOT NULL, reason TEXT NOT NULL,
        at INTEGER NOT NULL);
    ";
const STEP_FIVE: &str = r"
      CREATE TABLE IF NOT EXISTS actor_connections (
        connection_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        adapter_id TEXT NOT NULL, provider TEXT NOT NULL,
        transport_kind TEXT NOT NULL, transport_binding TEXT,
        label TEXT NOT NULL, device_alias TEXT,
        audience_description TEXT NOT NULL, audience_verification TEXT NOT NULL,
        audience_scope TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'active',
        created_at INTEGER NOT NULL, last_used_at INTEGER, revoked_at INTEGER);
      CREATE INDEX IF NOT EXISTS actor_connections_by_actor ON actor_connections (actor_slug);
      CREATE TABLE IF NOT EXISTS actor_grants (
        grant_id TEXT PRIMARY KEY,
        connection_id TEXT NOT NULL REFERENCES actor_connections (connection_id) ON DELETE CASCADE,
        device_id TEXT NOT NULL, permissions TEXT NOT NULL,
        project_allowlist TEXT NOT NULL, field_policy TEXT NOT NULL,
        data_mode TEXT NOT NULL, lookback_ms INTEGER NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1, token_hash TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
        revoked_at INTEGER, revoked_reason TEXT, last_used_at INTEGER);
      CREATE INDEX IF NOT EXISTS actor_grants_by_connection ON actor_grants (connection_id);
      CREATE TABLE IF NOT EXISTS actor_account_links (
        link_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        identity_id TEXT, issuer TEXT, provider TEXT NOT NULL, workspace TEXT,
        proof_status TEXT NOT NULL, proof_method TEXT, subject_binding TEXT,
        checked_at INTEGER, expires_at INTEGER, created_at INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS actor_account_links_by_actor ON actor_account_links (actor_slug);
      CREATE TABLE IF NOT EXISTS actor_external_refs (
        external_ref_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        adapter_id TEXT NOT NULL, issuer TEXT NOT NULL, account_binding TEXT NOT NULL,
        external_actor_id TEXT NOT NULL, verified_at INTEGER NOT NULL,
        UNIQUE (adapter_id, issuer, account_binding, external_actor_id));
      CREATE TABLE IF NOT EXISTS assistant_places (
        place TEXT PRIMARY KEY, project_id TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS assistant_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS assistant_audit (
        id INTEGER PRIMARY KEY, at INTEGER NOT NULL, kind TEXT NOT NULL,
        request_id TEXT, grant_id TEXT, grant_revision INTEGER, actor TEXT,
        connection_id TEXT, operation TEXT NOT NULL, result TEXT NOT NULL,
        latency_ms INTEGER, bytes INTEGER, projects INTEGER);
      CREATE INDEX IF NOT EXISTS assistant_audit_by_time ON assistant_audit (at);
    ";

const STEP_EIGHT: &str = r"
      CREATE INDEX IF NOT EXISTS messages_by_mid ON messages (mid) WHERE mid IS NOT NULL;
      CREATE TABLE IF NOT EXISTS net_presence (
        host TEXT PRIMARY KEY, counter INTEGER NOT NULL, at INTEGER NOT NULL,
        you INTEGER NOT NULL DEFAULT 0, roster TEXT, agents TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS net_claims (
        host TEXT NOT NULL, repo_key TEXT NOT NULL, path TEXT NOT NULL,
        agent TEXT NOT NULL, branch TEXT, kind TEXT NOT NULL, until INTEGER,
        at INTEGER NOT NULL, PRIMARY KEY (host, repo_key, path, agent, kind));
      CREATE TABLE IF NOT EXISTS handoffs (
        id TEXT PRIMARY KEY, host TEXT, maker TEXT NOT NULL,
        to_agent TEXT, to_repo TEXT, to_scope TEXT, to_host TEXT,
        goal TEXT, done TEXT, next TEXT, files TEXT NOT NULL,
        branch TEXT, commit_sha TEXT,
        state TEXT NOT NULL DEFAULT 'open', taken_by TEXT, taken_at INTEGER,
        asked_by TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
    ";

/// The columns step 8 adds, each to its table.
const STEP_EIGHT_COLUMNS: [(&str, &str, &str); 6] = [
    ("messages", "host", "ALTER TABLE messages ADD COLUMN host TEXT"),
    ("messages", "mid", "ALTER TABLE messages ADD COLUMN mid TEXT"),
    ("deliveries", "host", "ALTER TABLE deliveries ADD COLUMN host TEXT"),
    ("deliveries", "receipt", "ALTER TABLE deliveries ADD COLUMN receipt TEXT"),
    ("deliveries", "receipt_at", "ALTER TABLE deliveries ADD COLUMN receipt_at INTEGER"),
    ("deliveries", "frame", "ALTER TABLE deliveries ADD COLUMN frame TEXT"),
];

/// The steps the database's shape takes; PRAGMA user_version counts those
/// taken.
pub const ENGINE_VERSION: usize = 8;

/// The statements step `i` (0 for the first) runs, given whether a table
/// has a column already: a database from before versions were counted can
/// have any of the first three already, so those look before they change.
pub fn engine_step<E>(i: usize, has_column: &mut dyn FnMut(&str, &str) -> std::result::Result<bool, E>) -> std::result::Result<Vec<&'static str>, E> {
    Ok(match i {
        // 1: the tables as they first were.
        0 => vec![STEP_ONE],
        // 2: a message's repo (its scope), for messages to everyone working there.
        1 => {
            if has_column("messages", "scope")? {
                vec![]
            } else {
                vec!["ALTER TABLE messages ADD COLUMN scope TEXT"]
            }
        }
        // 3: an agent's process, to tell when it's gone.
        2 => {
            if has_column("contacts", "pid")? {
                vec![]
            } else {
                vec!["ALTER TABLE contacts ADD COLUMN pid INTEGER"]
            }
        }
        // 4: actors, their registrations, events, targets and revoked tokens.
        3 => vec![STEP_FOUR],
        // 5: remote actors' connections, grants, account links and refs,
        // and the log of what was read.
        4 => vec![STEP_FIVE],
        // 6: how XenoNet knows who sent a message (attest): what it checked.
        5 => {
            if has_column("messages", "envelope")? {
                vec![]
            } else {
                vec!["ALTER TABLE messages ADD COLUMN envelope TEXT"]
            }
        }
        // 7: an actor's gateway token, apart from its own.
        6 => {
            let mut out = Vec::new();
            if !has_column("actors", "gateway_hash")? {
                out.push("ALTER TABLE actors ADD COLUMN gateway_hash TEXT");
            }
            if !has_column("actors", "gateway_at")? {
                out.push("ALTER TABLE actors ADD COLUMN gateway_at INTEGER");
            }
            out
        }
        // 8: a network across machines (docs/design/network.md): where a
        // message came from and its id across the network, which host each
        // delivery is on and its receipt, the latest presence of each other
        // host, their claims, and handoffs.
        7 => {
            let mut out = Vec::new();
            for (table, column, sql) in STEP_EIGHT_COLUMNS {
                if !has_column(table, column)? {
                    out.push(sql);
                }
            }
            out.push(STEP_EIGHT);
            out
        }
        _ => vec![],
    })
}

/// Whether a table has a column, as PRAGMA table_info says.
pub fn has_column(db: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(sql::all(db, &format!("PRAGMA table_info({table})"), &[])?.iter().any(|c| c.text("name") == Some(column)))
}

/// The steps, for the store block.
pub fn engine_steps() -> Vec<Step> {
    (0..ENGINE_VERSION)
        .map(|i| -> Step {
            Box::new(move |db: &Connection| {
                let stmts =
                    engine_step(i, &mut |t, c| db.query_row(&format!("SELECT count(*) FROM pragma_table_info('{t}') WHERE name = ?"), [c], |r| r.get::<_, i64>(0)).map(|n| n > 0))?;
                for s in stmts {
                    db.execute_batch(s)?;
                }
                Ok(())
            })
        })
        .collect()
}

// The columns of an actor's row that the actors write, and their values
// when they leave one out.
fn actor_defaults() -> Vec<(&'static str, Arg)> {
    vec![
        ("slug", Arg::Null),
        ("kind", Arg::Null),
        ("name", Arg::Null),
        ("product", Arg::Null),
        ("via", Arg::text("[]")),
        ("capabilities", Arg::text("[]")),
        ("heartbeat_ms", Arg::Null),
        ("identity", Arg::Null),
        ("enabled", Arg::Num(1.0)),
        ("created_at", Arg::Null),
        ("allowed_at", Arg::Null),
        ("last_seen", Arg::Null),
        ("presence", Arg::text("unknown")),
        ("presence_at", Arg::Null),
        ("status", Arg::Null),
        ("status_at", Arg::Null),
        ("seq", Arg::Num(0.0)),
    ]
}

/// A value given, or `d` when it's undefined (a JS default).
pub fn def(v: Arg, d: Arg) -> Arg {
    if v == Arg::Undef {
        d
    } else {
        v
    }
}

fn n(x: f64) -> Arg {
    Arg::Num(x)
}

fn items(v: &Arg) -> Result<Vec<Arg>> {
    match v {
        Arg::List(a) => Ok(a.clone()),
        Arg::Text(s) => Ok(s.chars().map(|c| Arg::Text(c.to_string())).collect()),
        _ => Err(SqlError::Other("that isn't a list".into())),
    }
}

fn num(c: &Cell) -> Arg {
    c.as_f64().map_or(Arg::Null, Arg::Num)
}

pub struct EngineStore {
    pub block: SqliteStore,
}

impl EngineStore {
    /// Opens `file` (":memory:" for one of its own), brings it up to date
    /// and prunes it.
    pub fn open(file: &str, now: f64) -> std::result::Result<Self, String> {
        // Foreign keys on, so a connection goes with its actor and a grant
        // with its connection; the tables from before 5 declare none.
        let mut block = SqliteStore::new("engine", file, engine_steps(), vec!["journal_mode = WAL".into(), "foreign_keys = ON".into()], true, now);
        block.open(now)?;
        if let Some(db) = &block.db {
            db.set_prepared_statement_cache_capacity(256);
        }
        let mut store = EngineStore { block };
        store.prune(now).map_err(|e| e.to_string())?;
        Ok(store)
    }

    pub fn version(&self) -> Option<Version> {
        self.block.version
    }

    /// The connection, while it's open.
    pub fn db(&self) -> Result<&Connection> {
        self.block.db.as_ref().ok_or_else(|| SqlError::State("database is not open".into()))
    }

    pub fn is_open(&self) -> bool {
        self.block.db.is_some()
    }

    pub fn close(&mut self, now: f64) {
        self.block.close(now);
    }

    fn run(&self, q: &str, args: &[Arg]) -> Result<Changes> {
        sql::run(self.db()?, q, args)
    }
    fn get(&self, q: &str, args: &[Arg]) -> Result<Option<Row>> {
        sql::get(self.db()?, q, args)
    }
    fn all(&self, q: &str, args: &[Arg]) -> Result<Vec<Row>> {
        sql::all(self.db()?, q, args)
    }

    /// `f` in a transaction of its own, unless one is under way already.
    pub fn transaction<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        sql::transaction(self.db()?, f)
    }

    pub fn in_transaction(&self) -> bool {
        self.block.db.as_ref().is_some_and(|d| !d.is_autocommit())
    }

    /// What's kept, at start and every hour: messages and pins for a week,
    /// claims until they expire. Actors' events for a week and at most
    /// ACTOR_EVENTS_KEPT each, their targets for a week after they were
    /// last touched, registrations a day, revoked tokens a week; the person
    /// at the console's events, targets and status for a day. Other hosts'
    /// claims until they lapse, and handoffs a week past theirs. A copy
    /// made before a migration goes a week after it was made.
    pub fn prune(&mut self, now: f64) -> Result<()> {
        self.block.prune_backups(now);
        self.transaction(|| {
            self.run("DELETE FROM deliveries WHERE message_id IN (SELECT id FROM messages WHERE created_at < ?)", &[n(now - WEEK_MS)])?;
            self.run("DELETE FROM messages WHERE created_at < ?", &[n(now - WEEK_MS)])?;
            self.run("DELETE FROM claims WHERE expires_at < ?", &[n(now)])?;
            self.run("DELETE FROM pins WHERE at < ?", &[n(now - WEEK_MS)])?;
            self.run("DELETE FROM actor_events WHERE at < ? OR (actor = ? AND at < ?)", &[n(now - WEEK_MS), Arg::text(YOU), n(now - DAY_MS)])?;
            for r in self.all("SELECT actor FROM actor_events GROUP BY actor HAVING count(*) > ?", &[n(ACTOR_EVENTS_KEPT)])? {
                let actor = r.get("actor").arg();
                if let Some(last) =
                    self.get("SELECT at, rowid FROM actor_events WHERE actor = ? ORDER BY at DESC, rowid DESC LIMIT 1 OFFSET ?", &[actor.clone(), n(ACTOR_EVENTS_KEPT - 1.0)])?
                {
                    self.run(
                        "DELETE FROM actor_events WHERE actor = ? AND (at < ? OR (at = ? AND rowid < ?))",
                        &[actor, num(last.get("at")), num(last.get("at")), num(last.get("rowid"))],
                    )?;
                }
            }
            self.run("DELETE FROM actor_targets WHERE last_at < ? OR (actor = ? AND last_at < ?)", &[n(now - WEEK_MS), Arg::text(YOU), n(now - DAY_MS)])?;
            self.run("DELETE FROM actor_requests WHERE created_at < ?", &[n(now - DAY_MS)])?;
            self.run("DELETE FROM actor_revoked WHERE at < ?", &[n(now - WEEK_MS)])?;
            self.run("UPDATE actors SET status = NULL, status_at = NULL WHERE slug = ? AND status_at < ?", &[Arg::text(you_slug()), n(now - DAY_MS)])?;
            self.run("DELETE FROM net_claims WHERE until < ?", &[n(now)])?;
            self.run("DELETE FROM handoffs WHERE expires_at < ?", &[n(now - WEEK_MS)])?;
            self.prune_audit(now)
        })
    }

    // ---------- the network (step 8) ----------
    //
    // Only the binary calls these, and only while this machine is in a
    // network, except handoffs, which work on one machine too.

    /// Another host's presence: its row, or None.
    pub fn net_presence(&self, host: &str) -> Result<Option<Row>> {
        self.get("SELECT * FROM net_presence WHERE host = ?", &[Arg::text(host)])
    }
    /// Every other host's latest presence, by host.
    pub fn net_presences(&self) -> Result<Vec<Row>> {
        self.all("SELECT * FROM net_presence ORDER BY host", &[])
    }
    /// A host's presence, kept in place of the one before it. `agents` is
    /// the JSON of its agents, merged already.
    pub fn set_net_presence(&self, host: &str, counter: f64, at: f64, you: bool, roster: &str, agents: &str) -> Result<()> {
        self.run(
            "INSERT INTO net_presence (host, counter, at, you, roster, agents) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(host) DO UPDATE SET counter = excluded.counter, at = excluded.at, you = excluded.you, roster = excluded.roster, agents = excluded.agents",
            &[Arg::text(host), n(counter), n(at), n(if you { 1.0 } else { 0.0 }), Arg::text(roster), Arg::text(agents)],
        )?;
        Ok(())
    }
    /// A host removed from the network: its presence, its claims and the
    /// handoffs it made that wait.
    pub fn drop_net_host(&self, host: &str) -> Result<()> {
        let h = Arg::text(host);
        self.transaction(|| {
            self.run("DELETE FROM net_presence WHERE host = ?", std::slice::from_ref(&h))?;
            self.run("DELETE FROM net_claims WHERE host = ?", std::slice::from_ref(&h))?;
            self.run("DELETE FROM handoffs WHERE host = ? AND state = 'open'", std::slice::from_ref(&h))?;
            Ok(())
        })
    }
    /// Other hosts' claims and edits that still hold.
    pub fn net_claims(&self, now: f64) -> Result<Vec<Row>> {
        self.all("SELECT * FROM net_claims WHERE until IS NULL OR until >= ? ORDER BY host, repo_key, path", &[n(now)])
    }
    /// Other hosts' live claims in clones of one repo.
    pub fn net_claims_in(&self, repo_key: &str, now: f64) -> Result<Vec<Row>> {
        self.all("SELECT * FROM net_claims WHERE repo_key = ? AND (until IS NULL OR until >= ?) ORDER BY host, path", &[Arg::text(repo_key), n(now)])
    }
    /// How many claims a host has here.
    pub fn net_claim_count(&self, host: &str) -> Result<f64> {
        Ok(self.get("SELECT COUNT(*) AS n FROM net_claims WHERE host = ?", &[Arg::text(host)])?.and_then(|r| r.num("n")).unwrap_or(0.0))
    }
    /// Whether a host's claim is here already.
    pub fn has_net_claim(&self, host: &str, repo_key: &str, path: &str, agent: &str, kind: &str) -> Result<bool> {
        Ok(self
            .get(
                "SELECT 1 AS one FROM net_claims WHERE host = ? AND repo_key = ? AND path = ? AND agent = ? AND kind = ?",
                &[Arg::text(host), Arg::text(repo_key), Arg::text(path), Arg::text(agent), Arg::text(kind)],
            )?
            .is_some())
    }
    /// A host's whole set of claims, in place of what it had.
    pub fn clear_net_claims(&self, host: &str) -> Result<()> {
        self.run("DELETE FROM net_claims WHERE host = ?", &[Arg::text(host)])?;
        Ok(())
    }
    #[expect(clippy::too_many_arguments, reason = "one row's columns")]
    pub fn put_net_claim(&self, host: &str, repo_key: &str, path: &str, agent: &str, branch: Option<&str>, kind: &str, until: Option<f64>, at: f64) -> Result<()> {
        self.run(
            "INSERT INTO net_claims (host, repo_key, path, agent, branch, kind, until, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(host, repo_key, path, agent, kind) DO UPDATE SET branch = excluded.branch, until = excluded.until, at = excluded.at",
            &[Arg::text(host), Arg::text(repo_key), Arg::text(path), Arg::text(agent), Arg::opt_text(branch), Arg::text(kind), Arg::opt_num(until), n(at)],
        )?;
        Ok(())
    }
    pub fn drop_net_claim(&self, host: &str, repo_key: &str, path: &str, agent: &str) -> Result<()> {
        self.run("DELETE FROM net_claims WHERE host = ? AND repo_key = ? AND path = ? AND agent = ?", &[Arg::text(host), Arg::text(repo_key), Arg::text(path), Arg::text(agent)])?;
        Ok(())
    }

    /// Where a message came from (null: made here) and its id across the
    /// network.
    pub fn set_message_net(&self, id: f64, host: Option<&str>, mid: &str) -> Result<()> {
        self.run("UPDATE messages SET host = ?, mid = ? WHERE id = ?", &[Arg::opt_text(host), Arg::text(mid), n(id)])?;
        Ok(())
    }
    /// A message by its id across the network: one made here (`host`
    /// None), or one that came from `host`.
    pub fn message_by_mid(&self, mid: &str, host: Option<&str>) -> Result<Option<Row>> {
        match host {
            None => self.get("SELECT * FROM messages WHERE mid = ? AND host IS NULL ORDER BY id LIMIT 1", &[Arg::text(mid)]),
            Some(h) => self.get("SELECT * FROM messages WHERE mid = ? AND host = ? ORDER BY id LIMIT 1", &[Arg::text(mid), Arg::text(h)]),
        }
    }
    /// A message's delivery to an agent on another host, with the frame
    /// that carries it, which waits for its stored receipt.
    pub fn set_delivery_net(&self, id: f64, agent: &str, host: &str, frame: &str) -> Result<()> {
        self.run("UPDATE deliveries SET host = ?, frame = ? WHERE message_id = ? AND agent = ?", &[Arg::text(host), Arg::text(frame), n(id), Arg::text(agent)])?;
        Ok(())
    }
    pub fn delivery(&self, id: f64, agent: &str) -> Result<Option<Row>> {
        self.get("SELECT * FROM deliveries WHERE message_id = ? AND agent = ?", &[n(id), Arg::text(agent)])
    }
    /// What the other host said of a delivery. A receipt never goes back
    /// to an earlier state, and seen marks it seen.
    pub fn set_receipt(&self, id: f64, agent: &str, state: &str, at: f64) -> Result<bool> {
        let rank = |s: &str| match s {
            "stored" => 1,
            "held" => 2,
            "delivered" => 3,
            "seen" => 4,
            "refused" => 5,
            _ => 0,
        };
        let Some(had) = self.delivery(id, agent)? else { return Ok(false) };
        if rank(had.text("receipt").unwrap_or("")) >= rank(state) {
            return Ok(false);
        }
        self.run("UPDATE deliveries SET receipt = ?, receipt_at = ? WHERE message_id = ? AND agent = ?", &[Arg::text(state), n(at), n(id), Arg::text(agent)])?;
        if state == "seen" {
            self.run("UPDATE deliveries SET seen_at = COALESCE(seen_at, ?) WHERE message_id = ? AND agent = ?", &[n(at), n(id), Arg::text(agent)])?;
        }
        Ok(true)
    }
    /// What waits for a stored receipt: deliveries to other hosts since
    /// `since`, oldest first, with their frames.
    pub fn outbox(&self, since: f64) -> Result<Vec<Row>> {
        self.all(
            "SELECT d.message_id, d.agent, d.host, d.frame FROM deliveries d JOIN messages m ON m.id = d.message_id WHERE d.host IS NOT NULL AND d.receipt IS NULL AND d.frame IS NOT NULL AND m.created_at >= ? ORDER BY d.message_id",
            &[n(since)],
        )
    }
    /// Each delivery of a message with its host and receipt, for the
    /// console.
    pub fn net_deliveries(&self, id: f64) -> Result<Vec<Row>> {
        self.all("SELECT agent, seen_at, host, receipt FROM deliveries WHERE message_id = ?", &[n(id)])
    }

    /// A handoff, made here (`host` null) or by another host.
    pub fn add_handoff(&self, h: &Arg) -> Result<bool> {
        let f = |k: &str| def(h.field(k), Arg::Null);
        Ok(self
            .run(
                "INSERT OR IGNORE INTO handoffs (id, host, maker, to_agent, to_repo, to_scope, to_host, goal, done, next, files, branch, commit_sha, state, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?)",
                &[
                    f("id"),
                    f("host"),
                    f("maker"),
                    f("to_agent"),
                    f("to_repo"),
                    f("to_scope"),
                    f("to_host"),
                    f("goal"),
                    f("done"),
                    f("next"),
                    def(h.field("files"), Arg::text("[]")),
                    f("branch"),
                    f("commit_sha"),
                    f("created_at"),
                    f("expires_at"),
                ],
            )?
            .changes
            > 0)
    }
    pub fn handoff(&self, id: &str) -> Result<Option<Row>> {
        self.get("SELECT * FROM handoffs WHERE id = ?", &[Arg::text(id)])
    }
    /// The handoffs no one has taken that haven't lapsed.
    /// How many handoffs made on `host` are open here.
    pub fn open_handoffs_from(&self, host: &str, now: f64) -> Result<f64> {
        Ok(self.get("SELECT COUNT(*) AS n FROM handoffs WHERE host = ? AND state = 'open' AND expires_at > ?", &[Arg::text(host), n(now)])?.and_then(|r| r.num("n")).unwrap_or(0.0))
    }
    pub fn open_handoffs(&self, now: f64) -> Result<Vec<Row>> {
        self.all("SELECT * FROM handoffs WHERE state = 'open' AND expires_at > ? ORDER BY created_at", &[n(now)])
    }
    /// Handoffs no one took in time.
    pub fn lapsing_handoffs(&self, now: f64) -> Result<Vec<Row>> {
        self.all("SELECT * FROM handoffs WHERE state = 'open' AND expires_at <= ? ORDER BY created_at", &[n(now)])
    }
    /// Taken by `by`, if it's still open: whether this was the take.
    pub fn take_handoff(&self, id: &str, by: &str, at: f64) -> Result<bool> {
        Ok(self.run("UPDATE handoffs SET state = 'taken', taken_by = ?, taken_at = ? WHERE id = ? AND state = 'open'", &[Arg::text(by), n(at), Arg::text(id)])?.changes > 0)
    }
    /// What the making host decided, on another host's copy.
    pub fn settle_handoff(&self, id: &str, state: &str, by: Option<&str>, at: f64) -> Result<()> {
        self.run("UPDATE handoffs SET state = ?, taken_by = COALESCE(?, taken_by), taken_at = ? WHERE id = ?", &[Arg::text(state), Arg::opt_text(by), n(at), Arg::text(id)])?;
        Ok(())
    }
    /// A session here asked the making host for it.
    pub fn ask_handoff(&self, id: &str, by: &str) -> Result<()> {
        self.run("UPDATE handoffs SET asked_by = ? WHERE id = ?", &[Arg::text(by), Arg::text(id)])?;
        Ok(())
    }

    /// The remote reads logged: a week of them, and at most AUDIT_KEPT.
    pub fn prune_audit(&self, now: f64) -> Result<()> {
        self.run("DELETE FROM assistant_audit WHERE at < ?", &[n(now - WEEK_MS)])?;
        if let Some(last) = self.get("SELECT id FROM assistant_audit ORDER BY id DESC LIMIT 1 OFFSET ?", &[n(AUDIT_KEPT - 1.0)])? {
            self.run("DELETE FROM assistant_audit WHERE id < ?", &[num(last.get("id"))])?;
        }
        Ok(())
    }

    // ---------- contacts ----------

    pub fn touch(&self, agent: Arg, o: &Arg, now: Arg) -> Result<()> {
        self.run(
            "INSERT INTO contacts (agent, app, cwd, via, at, pid) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(agent) DO UPDATE SET app = excluded.app, cwd = COALESCE(excluded.cwd, contacts.cwd), via = excluded.via, at = excluded.at, pid = COALESCE(excluded.pid, contacts.pid), ended_at = NULL",
            &[agent, o.field("app"), def(o.field("cwd"), Arg::Null), o.field("via"), now, def(o.field("pid"), Arg::Null)],
        )?;
        Ok(())
    }
    pub fn ended(&self, agent: Arg, now: Arg) -> Result<()> {
        self.run("UPDATE contacts SET ended_at = ? WHERE agent = ?", &[now, agent])?;
        Ok(())
    }
    pub fn contact(&self, agent: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM contacts WHERE agent = ?", &[agent])
    }
    pub fn contacts(&self, since: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM contacts WHERE at >= ?", &[since])
    }

    // ---------- pins: the process (pid and start time) holding a session ----------

    pub fn pins(&self, agent: Arg) -> Result<Vec<Row>> {
        self.all("SELECT pid, start, sure, at FROM pins WHERE agent = ?", &[agent])
    }
    /// The sessions pinned to a process.
    pub fn pinned_to(&self, pid: Arg) -> Result<Vec<Row>> {
        self.all("SELECT agent, start, sure, at FROM pins WHERE pid = ?", &[pid])
    }
    pub fn pin(&self, agent: Arg, p: &Arg, sure: Arg, now: Arg) -> Result<()> {
        self.run(
            "INSERT INTO pins (agent, pid, start, sure, at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(agent, pid, start) DO UPDATE SET sure = MAX(pins.sure, excluded.sure), at = excluded.at",
            &[agent, p.field("pid"), p.field("start"), n(if sure.truthy() { 1.0 } else { 0.0 }), now],
        )?;
        Ok(())
    }
    /// Takes back the guesses for an agent once its own process has spoken.
    pub fn drop_guesses(&self, agent: Arg) -> Result<()> {
        self.run("DELETE FROM pins WHERE agent = ? AND sure = 0", &[agent])?;
        Ok(())
    }
    pub fn unpin(&self, agent: Arg, p: &Arg) -> Result<()> {
        self.run("DELETE FROM pins WHERE agent = ? AND pid = ? AND start = ?", &[agent, p.field("pid"), p.field("start")])?;
        Ok(())
    }

    // ---------- messages ----------

    /// A message, delivered to its recipients: its id.
    pub fn send(&self, m: &Arg, now: Arg) -> Result<f64> {
        let envelope = def(m.field("envelope"), Arg::Null);
        let envelope = if envelope.truthy() { Arg::Text(xenonet_base::json::stringify(&envelope.to_json(), 0)) } else { Arg::Null };
        let c = self.run(
            "INSERT INTO messages (sender, target, scope, body, reply_to, created_at, envelope) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[m.field("sender"), m.field("target"), def(m.field("scope"), Arg::Null), m.field("body"), def(m.field("replyTo"), Arg::Null), now, envelope],
        )?;
        let id = c.last_insert_rowid as f64;
        for r in items(&m.field("recipients"))? {
            self.deliver(n(id), r)?;
        }
        Ok(id)
    }
    pub fn deliver(&self, id: Arg, agent: Arg) -> Result<()> {
        self.run("INSERT OR IGNORE INTO deliveries (message_id, agent) VALUES (?, ?)", &[id, agent])?;
        Ok(())
    }
    /// Messages to everyone in a repo since `since`, for an agent that
    /// joins it.
    pub fn scoped(&self, scope: Arg, since: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM messages WHERE scope = ? AND created_at >= ? ORDER BY id", &[scope, since])
    }
    pub fn message(&self, id: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM messages WHERE id = ?", &[id])
    }
    pub fn unseen(&self, agent: Arg) -> Result<Vec<Row>> {
        self.all("SELECT m.* FROM deliveries d JOIN messages m ON m.id = d.message_id WHERE d.agent = ? AND d.seen_at IS NULL ORDER BY m.id", &[agent])
    }
    pub fn mark_seen(&self, agent: Arg, ids: &Arg, now: Arg) -> Result<()> {
        for id in items(ids)? {
            self.run("UPDATE deliveries SET seen_at = ? WHERE agent = ? AND message_id = ? AND seen_at IS NULL", &[now.clone(), agent.clone(), id])?;
        }
        Ok(())
    }
    /// A message pushed into a session that never showed it got there waits
    /// for the session's hooks again.
    pub fn mark_unseen(&self, agent: Arg, ids: &Arg) -> Result<()> {
        for id in items(ids)? {
            self.run("UPDATE deliveries SET seen_at = NULL WHERE agent = ? AND message_id = ?", &[agent.clone(), id])?;
        }
        Ok(())
    }
    /// Who a message went to, and who has seen it.
    pub fn deliveries(&self, id: Arg) -> Result<Vec<Row>> {
        self.all("SELECT agent, seen_at FROM deliveries WHERE message_id = ?", &[id])
    }
    pub fn recent(&self, since: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM messages WHERE created_at >= ? ORDER BY id", &[since])
    }

    // ---------- claims ----------

    pub fn claim(&self, agent: Arg, pattern: Arg, note: Arg, expires_at: Arg, now: Arg) -> Result<()> {
        self.run("DELETE FROM claims WHERE agent = ? AND pattern = ?", &[agent.clone(), pattern.clone()])?;
        self.run("INSERT INTO claims (agent, pattern, note, created_at, expires_at) VALUES (?, ?, ?, ?, ?)", &[agent, pattern, note.or(Arg::Null), now, expires_at])?;
        Ok(())
    }
    /// The claims let go: all of an agent's, or those of `patterns`.
    pub fn release(&self, agent: Arg, patterns: &Arg) -> Result<f64> {
        if !patterns.truthy() {
            return Ok(self.run("DELETE FROM claims WHERE agent = ?", &[agent])?.changes as f64);
        }
        let mut count = 0.0;
        for p in items(patterns)? {
            count += self.run("DELETE FROM claims WHERE agent = ? AND pattern = ?", &[agent.clone(), p])?.changes as f64;
        }
        Ok(count)
    }
    pub fn claims(&self, now: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM claims WHERE expires_at >= ? ORDER BY created_at", &[now])
    }

    // ---------- focus ----------

    /// Sets what an agent says it's doing; a falsy text takes it away (and
    /// says what that changed, as JS returned the run's result).
    pub fn set_focus(&self, agent: Arg, text: Arg, now: Arg) -> Result<Option<Changes>> {
        if !text.truthy() {
            return self.run("DELETE FROM focus WHERE agent = ?", &[agent]).map(Some);
        }
        self.run("INSERT INTO focus (agent, text, at) VALUES (?, ?, ?) ON CONFLICT(agent) DO UPDATE SET text = excluded.text, at = excluded.at", &[agent, text, now])?;
        Ok(None)
    }
    pub fn focus(&self, agent: Arg) -> Result<Option<Row>> {
        self.get("SELECT text, at FROM focus WHERE agent = ?", &[agent])
    }

    // ---------- actors ----------

    pub fn actor_rows(&self) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actors ORDER BY created_at, rowid", &[])
    }
    pub fn actor_row(&self, slug: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actors WHERE slug = ?", &[slug])
    }
    /// A row as the actors keep it: everything but the token, which only
    /// registering, allowing and rotating touch. Columns left out take
    /// their defaults on a new row, and are cleared on one that's there.
    pub fn save_actor(&self, row: &Arg) -> Result<()> {
        let values: Vec<(String, Arg)> = actor_defaults().into_iter().map(|(k, d)| (k.to_string(), def(row.field(k), d))).collect();
        self.run(
            "INSERT INTO actors (slug, kind, name, product, via, capabilities, heartbeat_ms, identity, enabled, created_at, allowed_at, last_seen, presence, presence_at, status, status_at, seq)
         VALUES (:slug, :kind, :name, :product, :via, :capabilities, :heartbeat_ms, :identity, :enabled, :created_at, :allowed_at, :last_seen, :presence, :presence_at, :status, :status_at, :seq)
         ON CONFLICT(slug) DO UPDATE SET kind = excluded.kind, name = excluded.name, product = excluded.product, via = excluded.via, capabilities = excluded.capabilities,
           heartbeat_ms = excluded.heartbeat_ms, identity = excluded.identity, enabled = excluded.enabled, allowed_at = excluded.allowed_at, last_seen = excluded.last_seen,
           presence = excluded.presence, presence_at = excluded.presence_at, status = excluded.status, status_at = excluded.status_at, seq = excluded.seq",
            &[Arg::Obj(values)],
        )?;
        Ok(())
    }
    /// An actor gone: its row, events and targets.
    pub fn remove_actor(&self, slug: Arg) -> Result<()> {
        let id = Arg::Text(format!("actor:{}", js_text(&slug)));
        self.transaction(|| {
            self.run("DELETE FROM actors WHERE slug = ?", std::slice::from_ref(&slug))?;
            self.run("DELETE FROM actor_events WHERE actor = ?", std::slice::from_ref(&id))?;
            self.run("DELETE FROM actor_targets WHERE actor = ?", std::slice::from_ref(&id))?;
            Ok(())
        })
    }
    /// An actor's record let go and the actor kept: its events and targets.
    pub fn forget_actor(&self, slug: Arg) -> Result<()> {
        let id = Arg::Text(format!("actor:{}", js_text(&slug)));
        self.transaction(|| {
            self.run("DELETE FROM actor_events WHERE actor = ?", std::slice::from_ref(&id))?;
            self.run("DELETE FROM actor_targets WHERE actor = ?", std::slice::from_ref(&id))?;
            Ok(())
        })
    }
    pub fn add_actor_event(&self, e: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_events (id, actor, at, kind, target, detail, via, facts, ref) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                e.field("id"),
                e.field("actor"),
                e.field("at"),
                e.field("kind"),
                def(e.field("target"), Arg::Null),
                def(e.field("detail"), Arg::Null),
                e.field("via"),
                def(e.field("facts"), Arg::Null),
                def(e.field("ref"), Arg::Null),
            ],
        )?;
        Ok(())
    }
    /// An actor's last `limit` events, newest last.
    pub fn last_actor_events(&self, actor: Arg, limit: Arg) -> Result<Vec<Row>> {
        let mut rows = self.all("SELECT * FROM actor_events WHERE actor = ? ORDER BY at DESC, rowid DESC LIMIT ?", &[actor, def(limit, n(20.0))])?;
        rows.reverse();
        Ok(rows)
    }
    /// Events since `since`, oldest first: only `only`'s, or all but
    /// `except`'s.
    pub fn actor_events_since(&self, since: Arg, o: &Arg) -> Result<Vec<Row>> {
        let limit = def(o.field("limit"), n(1000.0));
        let only = def(o.field("only"), Arg::Null);
        if only.truthy() {
            self.all("SELECT * FROM actor_events WHERE actor = ? AND at >= ? ORDER BY at, rowid LIMIT ?", &[only, since, limit])
        } else {
            self.all("SELECT * FROM actor_events WHERE at >= ? AND actor IS NOT ? ORDER BY at, rowid LIMIT ?", &[since, def(o.field("except"), Arg::Null), limit])
        }
    }
    /// How many events of each kind each actor has had since `since`.
    pub fn actor_event_counts(&self, since: Arg) -> Result<Vec<Row>> {
        self.all("SELECT actor, kind, count(*) AS n FROM actor_events WHERE at >= ? GROUP BY actor, kind", &[since])
    }
    /// The targets of each actor's spawned events since `since`, the first
    /// time from each source.
    pub fn actor_spawns(&self, since: Arg) -> Result<Vec<Row>> {
        self.all("SELECT actor, target, via, MIN(at) AS at FROM actor_events WHERE kind = 'spawned' AND target IS NOT NULL AND at >= ? GROUP BY actor, target, via", &[since])
    }
    /// Whether an actor reported an event with this `ref` since `since`.
    pub fn has_actor_ref(&self, actor: Arg, r: Arg, since: Arg) -> Result<bool> {
        Ok(self.get("SELECT 1 FROM actor_events WHERE actor = ? AND ref = ? AND at >= ? LIMIT 1", &[actor, r, since])?.is_some())
    }
    pub fn actor_targets(&self) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_targets", &[])
    }
    pub fn save_actor_target(&self, t: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_targets (actor, target, relation, since, last_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(actor, target, relation) DO UPDATE SET since = MIN(actor_targets.since, excluded.since), last_at = MAX(actor_targets.last_at, excluded.last_at)",
            &[t.field("actor"), t.field("target"), t.field("relation"), t.field("since"), t.field("last_at")],
        )?;
        Ok(())
    }
    pub fn drop_actor_target(&self, actor: Arg, target: Arg, relation: Arg) -> Result<()> {
        self.run("DELETE FROM actor_targets WHERE actor = ? AND target = ? AND relation = ?", &[actor, target, relation])?;
        Ok(())
    }
    /// What an actor's events of one kind said as facts since `since`,
    /// newest first, with the source that said it.
    pub fn actor_facts_since(&self, actor: Arg, kind: Arg, since: Arg) -> Result<Vec<Row>> {
        self.all(
            "SELECT at, facts, via FROM actor_events WHERE actor = ? AND kind = ? AND at >= ? AND facts IS NOT NULL ORDER BY at DESC, rowid DESC LIMIT 200",
            &[actor, kind, since],
        )
    }

    // ---------- actors' tokens, registrations, revoked tokens ----------

    pub fn set_actor_token(&self, slug: Arg, hash: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actors SET token_hash = ?, token_at = ? WHERE slug = ?", &[hash, at, slug])?;
        Ok(())
    }
    pub fn drop_actor_token(&self, slug: Arg) -> Result<()> {
        self.run("UPDATE actors SET token_hash = NULL, token_at = NULL WHERE slug = ?", &[slug])?;
        Ok(())
    }
    pub fn touch_actor_token(&self, slug: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actors SET token_at = ? WHERE slug = ?", &[at, slug])?;
        Ok(())
    }
    pub fn actor_by_token(&self, hash: Arg) -> Result<Option<Row>> {
        self.get("SELECT slug, token_hash, token_at FROM actors WHERE token_hash = ?", &[hash])
    }
    /// Each actor's token as XenoNet's window may see it: whether it has
    /// one, and when it was last used (or made). Never the hash.
    pub fn actor_token_times(&self) -> Result<Vec<Value>> {
        Ok(self
            .all("SELECT slug, token_hash IS NOT NULL AS has_token, token_at, gateway_hash IS NOT NULL AS has_gateway, gateway_at FROM actors", &[])?
            .iter()
            .map(|r| json!({ "slug": r.get("slug").to_json(), "hasToken": truthy(r.get("has_token")), "at": r.get("token_at").to_json(), "hasGateway": truthy(r.get("has_gateway")), "gatewayAt": r.get("gateway_at").to_json() }))
            .collect())
    }
    pub fn actor_token(&self, slug: Arg) -> Result<Option<Row>> {
        self.get("SELECT token_hash, token_at FROM actors WHERE slug = ? AND token_hash IS NOT NULL", &[slug])
    }
    pub fn stale_actor_tokens(&self, before: Arg) -> Result<Vec<Row>> {
        self.all("SELECT slug, token_hash, token_at FROM actors WHERE token_hash IS NOT NULL AND token_at < ?", &[before])
    }
    /// Actors you allowed whose token is gone (unused for 30 days): their
    /// slugs and names, never a hash. One that reports only through the
    /// gateway isn't listed while its gateway token stands.
    pub fn expired_actor_tokens(&self) -> Result<Vec<Value>> {
        Ok(self
            .all("SELECT slug, name FROM actors WHERE allowed_at IS NOT NULL AND token_hash IS NULL AND gateway_hash IS NULL ORDER BY allowed_at, rowid", &[])?
            .iter()
            .map(|r| json!({ "slug": r.get("slug").to_json(), "name": r.get("name").to_json() }))
            .collect())
    }
    /// The hashes of the tokens that work now: each actor's own, its
    /// gateway token, and those of the registrations still waiting. The
    /// doctor holds the token-shaped strings it finds in repos to these,
    /// so a made-up one in a test fixture isn't taken for a leak. Never
    /// shown.
    pub fn issued_token_hashes(&self) -> Result<Vec<String>> {
        Ok(self
            .all(
                "SELECT token_hash AS hash FROM actors WHERE token_hash IS NOT NULL
                 UNION SELECT gateway_hash FROM actors WHERE gateway_hash IS NOT NULL
                 UNION SELECT token_hash FROM actor_requests WHERE decision IS NULL",
                &[],
            )?
            .iter()
            .filter_map(|r| r.text("hash").map(str::to_string))
            .collect())
    }
    /// A registration waiting for you.
    pub fn add_actor_request(&self, r: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_requests (id, slug, ask, token_hash, asker, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[r.field("id"), r.field("slug"), r.field("ask"), r.field("token_hash"), def(r.field("asker"), Arg::Null), r.field("created_at"), r.field("expires_at")],
        )?;
        Ok(())
    }
    pub fn actor_request(&self, id: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actor_requests WHERE id = ?", &[id])
    }
    pub fn actor_request_by_token(&self, hash: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actor_requests WHERE token_hash = ?", &[hash])
    }
    pub fn waiting_actor_requests(&self, now: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_requests WHERE decision IS NULL AND expires_at > ? ORDER BY created_at, rowid", &[now])
    }
    pub fn lapsed_actor_requests(&self, now: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_requests WHERE decision IS NULL AND expires_at <= ?", &[now])
    }
    /// Answers a waiting registration; whether it was still waiting.
    pub fn decide_actor_request(&self, id: Arg, decision: Arg, at: Arg) -> Result<bool> {
        Ok(self.run("UPDATE actor_requests SET decision = ?, decided_at = ? WHERE id = ? AND decision IS NULL", &[decision, at, id])?.changes > 0)
    }
    /// When each registration since `since` was made, oldest first.
    pub fn actor_request_times(&self, since: Arg) -> Result<Vec<Value>> {
        Ok(self.all("SELECT created_at FROM actor_requests WHERE created_at >= ? ORDER BY created_at", &[since])?.iter().map(|r| r.get("created_at").to_json()).collect())
    }
    pub fn set_gateway_token(&self, slug: Arg, hash: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actors SET gateway_hash = ?, gateway_at = ? WHERE slug = ?", &[hash, at, slug])?;
        Ok(())
    }
    pub fn drop_gateway_token(&self, slug: Arg) -> Result<()> {
        self.run("UPDATE actors SET gateway_hash = NULL, gateway_at = NULL WHERE slug = ?", &[slug])?;
        Ok(())
    }
    pub fn touch_gateway_token(&self, slug: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actors SET gateway_at = ? WHERE slug = ?", &[at, slug])?;
        Ok(())
    }
    pub fn actor_by_gateway_token(&self, hash: Arg) -> Result<Option<Row>> {
        self.get("SELECT slug, gateway_hash, gateway_at FROM actors WHERE gateway_hash = ?", &[hash])
    }
    pub fn gateway_token(&self, slug: Arg) -> Result<Option<Row>> {
        self.get("SELECT gateway_hash, gateway_at FROM actors WHERE slug = ? AND gateway_hash IS NOT NULL", &[slug])
    }
    pub fn stale_gateway_tokens(&self, before: Arg) -> Result<Vec<Row>> {
        self.all("SELECT slug, gateway_hash, gateway_at FROM actors WHERE gateway_hash IS NOT NULL AND gateway_at < ?", &[before])
    }
    /// A token that stopped working, and why. The first reason stands.
    pub fn revoke_actor_token(&self, r: &Arg) -> Result<()> {
        self.run(
            "INSERT OR IGNORE INTO actor_revoked (token_hash, actor, reason, at) VALUES (?, ?, ?, ?)",
            &[r.field("token_hash"), r.field("actor"), r.field("reason"), r.field("at")],
        )?;
        Ok(())
    }
    pub fn revoked_actor_token(&self, hash: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actor_revoked WHERE token_hash = ?", &[hash])
    }

    // ---------- remote connections ----------

    pub fn meta(&self, key: Arg) -> Result<Value> {
        Ok(self.get("SELECT value FROM assistant_meta WHERE key = ?", &[key])?.map_or(Value::Null, |r| match r.get("value") {
            Cell::Null => Value::Null,
            c => c.to_json(),
        }))
    }
    pub fn set_meta(&self, key: Arg, value: Arg) -> Result<()> {
        self.run("INSERT INTO assistant_meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", &[key, Arg::Text(js_text(&value))])?;
        Ok(())
    }
    /// A place's own id for remote readers (`prj_<uuid>`), made the first
    /// time it's asked for; `make` makes one.
    pub fn project_id_of<E: From<SqlError>>(&self, place: Arg, make: &mut dyn FnMut() -> std::result::Result<Arg, E>, now: Arg) -> std::result::Result<Value, E> {
        if let Some(had) = self.get("SELECT project_id FROM assistant_places WHERE place = ?", std::slice::from_ref(&place))? {
            return Ok(had.get("project_id").to_json());
        }
        let id = make()?;
        let out = id.to_json();
        self.run("INSERT INTO assistant_places (place, project_id, created_at) VALUES (?, ?, ?)", &[place, id, now])?;
        Ok(out)
    }
    pub fn project_id_for(&self, place: Arg) -> Result<Value> {
        Ok(self.get("SELECT project_id FROM assistant_places WHERE place = ?", &[place])?.map_or(Value::Null, |r| null_if_missing(r.get("project_id"))))
    }
    pub fn place_of_project(&self, project_id: Arg) -> Result<Value> {
        Ok(self.get("SELECT place FROM assistant_places WHERE project_id = ?", &[project_id])?.map_or(Value::Null, |r| null_if_missing(r.get("place"))))
    }
    pub fn add_connection(&self, row: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_connections (connection_id, actor_slug, adapter_id, provider, transport_kind, transport_binding, label, device_alias, audience_description, audience_verification, audience_scope, state, created_at)
         VALUES (:connection_id, :actor_slug, :adapter_id, :provider, :transport_kind, :transport_binding, :label, :device_alias, :audience_description, :audience_verification, :audience_scope, 'active', :created_at)",
            std::slice::from_ref(row),
        )?;
        Ok(())
    }
    pub fn connection(&self, id: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actor_connections WHERE connection_id = ?", &[id])
    }
    pub fn connections_of(&self, slug: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_connections WHERE actor_slug = ? ORDER BY created_at, rowid", &[slug])
    }
    pub fn connections(&self) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_connections ORDER BY created_at, rowid", &[])
    }
    /// How a connection's transport is set up, as JSON; never a secret.
    pub fn set_transport_binding(&self, id: Arg, binding: Arg) -> Result<()> {
        self.run("UPDATE actor_connections SET transport_binding = ? WHERE connection_id = ?", &[binding, id])?;
        Ok(())
    }
    pub fn end_connection(&self, id: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actor_connections SET state = 'revoked', revoked_at = COALESCE(revoked_at, ?) WHERE connection_id = ?", &[at, id])?;
        Ok(())
    }
    pub fn add_grant(&self, row: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_grants (grant_id, connection_id, device_id, permissions, project_allowlist, field_policy, data_mode, lookback_ms, revision, token_hash, created_at, expires_at)
         VALUES (:grant_id, :connection_id, :device_id, :permissions, :project_allowlist, :field_policy, :data_mode, :lookback_ms, 1, :token_hash, :created_at, :expires_at)",
            std::slice::from_ref(row),
        )?;
        Ok(())
    }
    pub fn grant(&self, id: Arg) -> Result<Option<Row>> {
        self.get("SELECT * FROM actor_grants WHERE grant_id = ?", &[id])
    }
    pub fn grants(&self) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_grants ORDER BY created_at, rowid", &[])
    }
    /// A grant's scope changed: its revision moves on. Whether it was
    /// changed (it isn't once revoked).
    pub fn change_grant(&self, id: Arg, c: &Arg) -> Result<bool> {
        let sets: Vec<(&str, Arg)> =
            ["project_allowlist", "field_policy", "data_mode", "token_hash", "expires_at"].into_iter().map(|k| (k, c.field(k))).filter(|(_, v)| *v != Arg::Undef).collect();
        let mut parts: Vec<String> = sets.iter().map(|(k, _)| format!("{k} = ?")).collect();
        parts.push("revision = revision + 1".into());
        let q = format!("UPDATE actor_grants SET {} WHERE grant_id = ? AND revoked_at IS NULL", parts.join(", "));
        let mut args: Vec<Arg> = sets.into_iter().map(|(_, v)| v).collect();
        args.push(id);
        Ok(self.run(&q, &args)?.changes > 0)
    }
    /// Revoked, for good: the first reason stands, and the revision moves on.
    pub fn revoke_grant(&self, id: Arg, reason: Arg, at: Arg) -> Result<bool> {
        Ok(self.run("UPDATE actor_grants SET revoked_at = ?, revoked_reason = ?, revision = revision + 1 WHERE grant_id = ? AND revoked_at IS NULL", &[at, reason, id])?.changes
            > 0)
    }
    pub fn touch_grant(&self, id: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actor_grants SET last_used_at = ? WHERE grant_id = ?", &[at.clone(), id.clone()])?;
        self.run("UPDATE actor_connections SET last_used_at = ? WHERE connection_id = (SELECT connection_id FROM actor_grants WHERE grant_id = ?)", &[at, id])?;
        Ok(())
    }
    pub fn add_account_link(&self, row: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_account_links (link_id, actor_slug, identity_id, issuer, provider, workspace, proof_status, proof_method, subject_binding, checked_at, expires_at, created_at)
         VALUES (:link_id, :actor_slug, :identity_id, :issuer, :provider, :workspace, :proof_status, :proof_method, :subject_binding, :checked_at, :expires_at, :created_at)",
            std::slice::from_ref(row),
        )?;
        Ok(())
    }
    pub fn account_links(&self, slug: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_account_links WHERE actor_slug = ? ORDER BY created_at, rowid", &[slug])
    }
    pub fn drop_account_link(&self, id: Arg) -> Result<()> {
        self.run("DELETE FROM actor_account_links WHERE link_id = ?", &[id])?;
        Ok(())
    }
    /// A verified account link's lapse, when the account changed.
    pub fn expire_account_link(&self, id: Arg, at: Arg) -> Result<()> {
        self.run("UPDATE actor_account_links SET expires_at = ? WHERE link_id = ?", &[at, id])?;
        Ok(())
    }
    pub fn add_external_ref(&self, row: &Arg) -> Result<()> {
        self.run(
            "INSERT INTO actor_external_refs (external_ref_id, actor_slug, adapter_id, issuer, account_binding, external_actor_id, verified_at)
         VALUES (:external_ref_id, :actor_slug, :adapter_id, :issuer, :account_binding, :external_actor_id, :verified_at)",
            std::slice::from_ref(row),
        )?;
        Ok(())
    }
    pub fn external_ref(&self, k: &Arg) -> Result<Option<Row>> {
        self.get(
            "SELECT * FROM actor_external_refs WHERE adapter_id = ? AND issuer = ? AND account_binding = ? AND external_actor_id = ?",
            &[k.field("adapter_id"), k.field("issuer"), k.field("account_binding"), k.field("external_actor_id")],
        )
    }
    pub fn external_refs(&self, slug: Arg) -> Result<Vec<Row>> {
        self.all("SELECT * FROM actor_external_refs WHERE actor_slug = ?", &[slug])
    }
    pub fn add_audit(&self, a: &Arg) -> Result<()> {
        let f = |k: &str| def(a.field(k), Arg::Null);
        self.run(
            "INSERT INTO assistant_audit (at, kind, request_id, grant_id, grant_revision, actor, connection_id, operation, result, latency_ms, bytes, projects) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[a.field("at"), a.field("kind"), f("request_id"), f("grant_id"), f("grant_revision"), f("actor"), f("connection_id"), a.field("operation"), a.field("result"), f("latency_ms"), f("bytes"), f("projects")],
        )?;
        Ok(())
    }
    /// The newest `limit` records since `since`, newest first.
    pub fn audit(&self, o: &Arg) -> Result<Vec<Row>> {
        let since = def(o.field("since"), n(0.0));
        let limit = def(o.field("limit"), n(200.0));
        let grant = def(o.field("grant"), Arg::Null);
        if grant.truthy() {
            self.all("SELECT * FROM assistant_audit WHERE at >= ? AND grant_id = ? ORDER BY id DESC LIMIT ?", &[since, grant, limit])
        } else {
            self.all("SELECT * FROM assistant_audit WHERE at >= ? ORDER BY id DESC LIMIT ?", &[since, limit])
        }
    }
}

fn truthy(c: &Cell) -> bool {
    match c {
        Cell::Null => false,
        Cell::Int(i) => *i != 0,
        Cell::Real(f) => *f != 0.0 && !f.is_nan(),
        Cell::Text(s) => !s.is_empty(),
        Cell::Blob(_) => true,
    }
}

fn null_if_missing(c: &Cell) -> Value {
    c.to_json()
}

/// A value as JS's template literal or String() writes it.
pub fn js_text(v: &Arg) -> String {
    match v {
        Arg::Undef => "undefined".into(),
        Arg::Other => "[object Object]".into(),
        other => crate::js::template(Some(&other.to_json())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(fields: &[(&str, Arg)]) -> Arg {
        Arg::Obj(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn a_new_store_takes_every_step_and_keeps_messages() {
        let s = EngineStore::open(":memory:", 1000.0).unwrap();
        assert_eq!(s.version().unwrap(), Version { from: 0, version: 8, newer: false });
        let id = s
            .send(&obj(&[("sender", "claude:a".into()), ("target", "codex:b".into()), ("body", "hi".into()), ("recipients", Arg::List(vec!["codex:b".into()]))]), n(1000.0))
            .unwrap();
        assert_eq!(id, 1.0);
        let unseen = s.unseen("codex:b".into()).unwrap();
        assert_eq!(unseen.len(), 1);
        assert_eq!(unseen[0].get("created_at"), &Cell::Int(1000));
        s.touch("claude:a".into(), &obj(&[("app", "claude".into()), ("via", "mcp".into())]), n(5.0)).unwrap();
        assert_eq!(s.contact("claude:a".into()).unwrap().unwrap().get("pid"), &Cell::Null);
        assert!(matches!(s.touch("claude:a".into(), &obj(&[("via", "mcp".into())]), n(5.0)), Err(SqlError::Bind(_))));
    }

    #[test]
    fn the_network_keeps_presence_receipts_claims_and_handoffs() {
        let s = EngineStore::open(":memory:", 1000.0).unwrap();
        s.set_net_presence("laptop", 5.0, 1000.0, true, "", "[]").unwrap();
        s.set_net_presence("laptop", 6.0, 1001.0, false, "", "[{\"id\":\"claude:x\"}]").unwrap();
        assert_eq!(s.net_presence("laptop").unwrap().unwrap().num("counter"), Some(6.0));
        let id = s
            .send(
                &obj(&[("sender", "claude:a".into()), ("target", "@laptop:claude:x".into()), ("body", "hi".into()), ("recipients", Arg::List(vec!["@laptop:claude:x".into()]))]),
                n(1000.0),
            )
            .unwrap();
        s.set_message_net(id, None, "msg_1").unwrap();
        s.set_delivery_net(id, "@laptop:claude:x", "laptop", "{}").unwrap();
        assert_eq!(s.outbox(0.0).unwrap().len(), 1, "it waits for its receipt");
        assert!(s.set_receipt(id, "@laptop:claude:x", "seen", 1002.0).unwrap());
        assert!(!s.set_receipt(id, "@laptop:claude:x", "stored", 1003.0).unwrap(), "never back to an earlier state");
        assert_eq!(s.outbox(0.0).unwrap().len(), 0);
        assert_eq!(s.message_by_mid("msg_1", None).unwrap().unwrap().num("id"), Some(id));
        assert!(s.message_by_mid("msg_1", Some("laptop")).unwrap().is_none());
        s.put_net_claim("laptop", "github.com/x/y", "src/a.rs", "claude:x", Some("main"), "edit", None, 1000.0).unwrap();
        assert_eq!(s.net_claims(1000.0).unwrap().len(), 1);
        let h = obj(&[("id", "hof_1".into()), ("host", "laptop".into()), ("maker", "claude:x".into()), ("created_at", n(1000.0)), ("expires_at", n(2000.0))]);
        assert!(s.add_handoff(&h).unwrap());
        assert!(!s.add_handoff(&h).unwrap(), "once");
        assert!(s.take_handoff("hof_1", "claude:a", 1500.0).unwrap());
        assert!(!s.take_handoff("hof_1", "claude:b", 1501.0).unwrap(), "the first take wins");
        s.drop_net_host("laptop").unwrap();
        assert!(s.net_presence("laptop").unwrap().is_none());
        assert_eq!(s.net_claims(1000.0).unwrap().len(), 0);
    }

    #[test]
    fn the_issued_tokens_are_the_ones_that_work_now() {
        let s = EngineStore::open(":memory:", 1000.0).unwrap();
        assert_eq!(s.issued_token_hashes().unwrap(), Vec::<String>::new());
        for (slug, token, gateway) in [("muse", Some("h-muse"), Some("g-muse")), ("wren", Some("h-wren"), None), ("moth", None, None)] {
            s.run(
                "INSERT INTO actors (slug, kind, name, via, capabilities, created_at, token_hash, gateway_hash) VALUES (?, 'agent', ?, '[]', '[]', 1, ?, ?)",
                &[Arg::text(slug), Arg::text(slug), token.map_or(Arg::Null, Arg::text), gateway.map_or(Arg::Null, Arg::text)],
            )
            .unwrap();
        }
        for (id, hash, decision) in [("r1", "h-waiting", None), ("r2", "h-denied", Some("deny")), ("r3", "h-wren", Some("allow"))] {
            s.run(
                "INSERT INTO actor_requests (id, slug, ask, token_hash, created_at, expires_at, decision) VALUES (?, 'heron', '{}', ?, 1, 2, ?)",
                &[Arg::text(id), Arg::text(hash), decision.map_or(Arg::Null, Arg::text)],
            )
            .unwrap();
        }
        s.revoke_actor_token(&obj(&[("token_hash", "h-old".into()), ("actor", "actor:muse".into()), ("reason", "rotated".into()), ("at", n(5.0))])).unwrap();
        let mut hashes = s.issued_token_hashes().unwrap();
        hashes.sort();
        assert_eq!(hashes, ["g-muse", "h-muse", "h-waiting", "h-wren"], "a revoked token, or a refused registration's, no longer works");
    }
}

/// A store shared by the parts that use it (the actors, the twin's own
/// calls): each call holds it only while it runs SQL, so a call may come
/// back to it from within another's transaction.
#[derive(Clone)]
pub struct Shared(pub std::sync::Arc<std::sync::Mutex<EngineStore>>);

impl Shared {
    pub fn new(store: EngineStore) -> Self {
        Shared(std::sync::Arc::new(std::sync::Mutex::new(store)))
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut EngineStore) -> T) -> T {
        f(&mut self.0.locked())
    }

    /// `f` in a transaction of its own, unless one is under way already,
    /// without holding the store while `f` runs.
    pub fn transaction<T, E: From<SqlError>>(&self, f: impl FnOnce() -> std::result::Result<T, E>) -> std::result::Result<T, E> {
        if self.with(|s| s.in_transaction()) {
            return f();
        }
        self.with(|s| s.db().and_then(|db| sql::exec(db, "BEGIN")))?;
        let out = f();
        match out {
            Ok(v) => match self.with(|s| s.db().and_then(|db| sql::exec(db, "COMMIT"))) {
                Ok(()) => Ok(v),
                Err(e) => {
                    let _ = self.with(|s| s.db().and_then(|db| sql::exec(db, "ROLLBACK")));
                    Err(e.into())
                }
            },
            Err(e) => {
                let _ = self.with(|s| s.db().and_then(|db| sql::exec(db, "ROLLBACK")));
                Err(e)
            }
        }
    }
}
