// A made-up Hermes Agent install for the observer's tests and the
// benchmark: a root folder laid out as Hermes lays out ~/.hermes, its files
// written as Hermes writes them, and a session database in its current
// layout (schemas 31 and 32) or an older one. The content columns are filled too,
// with made-up text, so tests can check none of it is read. Nothing here
// comes from anyone's real install.
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

// Text that must never leave Hermes's files: tests look for it in
// everything XenoNet keeps or sends.
export const SECRET = "made-up-secret-do-not-read";

const write = (file, text) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const before = fs.existsSync(file) ? fs.statSync(file).mtimeMs : null;
  fs.writeFileSync(file, text);
  // Fixtures change faster than the real files. Give each rewrite its
  // own timestamp so the observer's cache sees it on every filesystem.
  if (before != null) {
    const at = new Date(Math.max(Date.now(), before + 1000));
    fs.utimesSync(file, at, at);
  }
};
const json = (file, value) => write(file, JSON.stringify(value, null, 2) + "\n");

// The root, with what's asked for in it: `stamp` (a baseVersion, or null
// for none), the spawn ledger's entries, and files for the default
// profile. Returns helpers to change it as Hermes would.
export function hermesRoot(dir, { stamp = "0.21.5", ledger = null } = {}) {
  fs.mkdirSync(dir, { recursive: true });
  if (stamp) json(path.join(dir, "hermes-agent", "install-stamp.json"), { schemaVersion: 2, commit: "0a1b2c3d4e", branch: "main", builtAt: "2026-10-05T12:00:00+00:00", dirty: false, source: "git", distribution: null, updateMechanism: "self", baseVersion: stamp, displayVersion: `${stamp}+12.g0a1b2c3`, distance: 12, payload: "bootstrap", tag: null });
  // Secrets Hermes keeps, which XenoNet never opens.
  write(path.join(dir, ".env"), `OPENROUTER_API_KEY=${SECRET}\n`);
  json(path.join(dir, "auth.json"), { token: SECRET });
  if (ledger) setLedger(dir, ledger);
  return dir;
}

// The spawn ledger, as Hermes writes it: argv and hermes_home included.
export function setLedger(dir, entries) {
  json(
    path.join(dir, "spawn-ledger.json"),
    entries.map((e) => ({ install: "a1b2c3d4e5f6", spawner_pid: null, spawner_create: null, registered_at: e.createTime ?? 0, argv: `hermes ${e.purpose} ${SECRET}`, host: "127.0.0.1", port: null, profile: "default", hermes_home: dir, isolated: false, ...e, create_time: e.createTime ?? null, createTime: undefined })),
  );
}

// A profile's home: the root for the default one.
export const profileHome = (root, profile = "default") => (profile === "default" ? root : path.join(root, "profiles", profile));

export function setGateway(home, { state = "running", pid, startTime = null, updatedAt, activeAgents = 0, platforms = {} }) {
  json(path.join(home, "gateway_state.json"), {
    pid,
    kind: "hermes-gateway",
    argv: ["hermes", "gateway", "run", SECRET],
    start_time: startTime,
    hermes_home: home,
    gateway_state: state,
    exit_reason: null,
    restart_requested: false,
    active_agents: activeAgents,
    platforms: Object.fromEntries(Object.entries(platforms).map(([name, s]) => [name, { state: s, error_message: SECRET, updated_at: new Date(updatedAt).toISOString() }])),
    updated_at: new Date(updatedAt).toISOString(),
    code_version: "0.21.5",
  });
}

export function setTicker(home, { at, pid, lastSuccess = at }) {
  write(path.join(home, "cron", "ticker_heartbeat"), `${at / 1000} ${pid}`);
  write(path.join(home, "cron", "ticker_last_success"), `${lastSuccess / 1000}`);
}

export function setProcesses(home, entries) {
  json(path.join(home, "processes.json"), entries.map((e, i) => ({ session_id: `proc_${i}`, command: `codex exec '${SECRET}'`, pid_scope: "host", task_id: `t${i}`, owner_task_id: `t${i}`, session_key: SECRET, watch_patterns: [], ...e, host_start_time: e.startTime ?? null, started_at: (e.startedAt ?? 0) / 1000, parent_session_id: e.session ?? null, startTime: undefined, startedAt: undefined, session: undefined })));
}

export function setJobs(home, jobs) {
  json(path.join(home, "cron", "jobs.json"), {
    jobs: jobs.map((j) => ({ name: SECRET, prompt: SECRET, deliver: "origin", schedule: { kind: "cron", expr: "0 3 * * *" }, enabled: true, state: "scheduled", last_status: null, last_run_at: null, next_run_at: null, workdir: null, ...j })),
    updated_at: "2026-10-05T03:00:01+00:00",
  });
}

const CURRENT = `
  CREATE TABLE schema_version (version INTEGER NOT NULL);
  CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, user_id TEXT, chat_id TEXT, model TEXT, system_prompt TEXT, parent_session_id TEXT,
    started_at REAL NOT NULL, ended_at REAL, end_reason TEXT, message_count INTEGER DEFAULT 0, tool_call_count INTEGER DEFAULT 0, cwd TEXT, git_branch TEXT,
    git_repo_root TEXT, title TEXT, archived INTEGER DEFAULT 0, display_name TEXT, origin_json TEXT, last_activity_at REAL, last_activity_description TEXT,
    profile_name TEXT, hidden INTEGER DEFAULT 0);
  CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT,
    tool_name TEXT, timestamp REAL NOT NULL, reasoning TEXT, active INTEGER DEFAULT 1, api_content TEXT);
  CREATE TABLE gateway_heartbeats (backend_id TEXT, pid INTEGER, started_at REAL, last_heartbeat REAL, profile TEXT, host TEXT);
  CREATE TABLE state_meta (key TEXT PRIMARY KEY, value TEXT);`;
// The layout of July's 0.18.0: no activity stamps, no heartbeats table.
const OLD = `
  CREATE TABLE schema_version (version INTEGER NOT NULL);
  CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, user_id TEXT, chat_id TEXT, system_prompt TEXT, parent_session_id TEXT,
    started_at REAL NOT NULL, ended_at REAL, end_reason TEXT, cwd TEXT, git_repo_root TEXT, title TEXT, archived INTEGER DEFAULT 0);
  CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT,
    tool_name TEXT, timestamp REAL NOT NULL, active INTEGER DEFAULT 1, observed INTEGER, compacted INTEGER);`;

// A session database at `file`, in the current layout or the old one, in
// WAL mode as Hermes keeps it. Its helpers take times in epoch
// milliseconds and write them as Hermes does, in seconds.
export function stateDb(file, { layout = "current", version = layout === "current" ? 32 : 12 } = {}) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const db = new DatabaseSync(file);
  db.exec("PRAGMA journal_mode = WAL");
  db.exec(layout === "current" ? CURRENT : OLD);
  db.prepare("INSERT INTO schema_version (version) VALUES (?)").run(version);
  if (layout === "current") db.prepare("INSERT INTO state_meta (key, value) VALUES (?, ?)").run("heartbeat:x", SECRET);
  const s = (ms) => (ms == null ? null : ms / 1000);
  const current = layout === "current";
  return {
    db,
    session({ id, source = "tui", cwd = null, repo = null, parent = null, startedAt, endedAt = null, endReason = null, activityAt = null, activity = null, archived = 0, hidden = 0 }) {
      if (current)
        db.prepare("INSERT INTO sessions (id, source, user_id, chat_id, system_prompt, parent_session_id, started_at, ended_at, end_reason, cwd, git_repo_root, title, archived, display_name, origin_json, last_activity_at, last_activity_description, hidden) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)").run(
          id, source, SECRET, SECRET, SECRET, parent, s(startedAt), s(endedAt), endReason, cwd, repo, SECRET, archived, SECRET, SECRET, s(activityAt), activity, hidden,
        );
      else db.prepare("INSERT INTO sessions (id, source, user_id, chat_id, system_prompt, parent_session_id, started_at, ended_at, end_reason, cwd, git_repo_root, title, archived) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)").run(id, source, SECRET, SECRET, SECRET, parent, s(startedAt), s(endedAt), endReason, cwd, repo, SECRET, archived);
    },
    update(id, { endedAt, endReason, activityAt, activity } = {}) {
      if (endedAt !== undefined) db.prepare("UPDATE sessions SET ended_at = ?, end_reason = ? WHERE id = ?").run(s(endedAt), endReason ?? null, id);
      if (current && activityAt !== undefined) db.prepare("UPDATE sessions SET last_activity_at = ?, last_activity_description = ? WHERE id = ?").run(s(activityAt), activity ?? null, id);
    },
    // A message: an assistant's with tool calls ([{ name, args }]), a tool's
    // result (tool), or plain text, all with made-up content.
    message({ session, role = "assistant", at, tool = null, calls = null, active = 1, rawCalls = null }) {
      const toolCalls = rawCalls ?? (calls ? JSON.stringify(calls.map((c, i) => ({ id: `call_${i}`, type: "function", function: { name: c.name, arguments: typeof c.args === "string" ? c.args : JSON.stringify({ ...c.args, content: SECRET }) } }))) : null);
      if (current) db.prepare("INSERT INTO messages (session_id, role, content, tool_calls, tool_name, timestamp, reasoning, active, api_content) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)").run(session, role, SECRET, toolCalls, tool, s(at), SECRET, active, SECRET);
      else db.prepare("INSERT INTO messages (session_id, role, content, tool_calls, tool_name, timestamp, active) VALUES (?, ?, ?, ?, ?, ?, ?)").run(session, role, SECRET, toolCalls, tool, s(at), active);
    },
    heartbeat({ pid, startedAt, at, profile = "default" }) {
      if (current) db.prepare("INSERT INTO gateway_heartbeats (backend_id, pid, started_at, last_heartbeat, profile, host) VALUES (?, ?, ?, ?, ?, ?)").run(`b${pid}`, pid, s(startedAt), s(at), profile, SECRET);
    },
    close: () => db.close(),
  };
}
