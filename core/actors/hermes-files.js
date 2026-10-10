// Hermes Agent's state on this machine, read for metadata only
// (docs/design/actors/hermes.md, sections 2 and 5): its build stamp, its
// spawn ledger, and for each profile (the default one in ~/.hermes, a named
// one in ~/.hermes/profiles/<name>) its gateway's state, its scheduler's
// ticker, its background processes, its scheduled jobs and its session
// database. Each source is read against its vendor schema (core/schema/
// vendor/nous/hermes-agent.js) through the lenient readers, so a change in
// Hermes's formats shows as drift rather than as a quiet observer.
//
// What's never read: .env, auth.json, pairing/, mcp-tokens/, the desktop
// app's connection and token files, logs, memories, cron output, and in
// state.db anything that holds content or who's on the other end. The
// database is opened read only for a moment, with named columns in every
// query; the one thing taken from a tool call's arguments is the path of a
// file it wrote, which SQLite takes out (json_extract), so nothing else in
// the call leaves the database.
//
// Every reader answers { state, ... }: ok, missing (not there, which is
// usually fine: no gateway, no jobs), denied (macOS privacy or
// permissions) or unreadable (not what XenoNet reads).
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { reader } from "../vendor.js";
import { cleanVersion } from "../drift.js";

export const SOURCE = {
  stamp: "nous/hermes-agent/install-stamp",
  ledger: "nous/hermes-agent/spawn-ledger",
  gateway: "nous/hermes-agent/gateway-state",
  processes: "nous/hermes-agent/processes",
  jobs: "nous/hermes-agent/cron-jobs",
  ticker: "nous/hermes-agent/cron-ticker",
  stateDb: "nous/hermes-agent/state-db",
};

// What a failed read says, by the error's code.
const stateOf = (err) => (err?.code === "ENOENT" || err?.code === "ENOTDIR" ? "missing" : err?.code === "EACCES" || err?.code === "EPERM" ? "denied" : "unreadable");

// Hermes's times, epoch seconds (floats) or ISO 8601, as epoch
// milliseconds; null for anything else.
export function msOf(v) {
  if (typeof v === "number" && Number.isFinite(v) && v > 0) return v > 1e11 ? Math.round(v) : Math.round(v * 1000);
  if (typeof v === "string" && v) {
    const t = Date.parse(v);
    return Number.isFinite(t) ? t : null;
  }
  return null;
}

// Whether a process start Hermes recorded is the one the process table
// says (to the second): psutil's epoch seconds in the ledger, centiseconds
// of the epoch in the gateway's and the background processes' records on
// macOS. A pid used again by another process starts at another time.
export function sameStart(recorded, tableStart, slackMs = 2000) {
  if (typeof recorded !== "number" || !Number.isFinite(recorded) || !tableStart) return false;
  return [recorded * 1000, recorded * 10, recorded].some((ms) => Math.abs(ms - tableStart) <= slackMs);
}

function readText(file, files) {
  try {
    return { state: "ok", text: files.readFileSync(file, "utf8") };
  } catch (err) {
    return { state: stateOf(err), code: err.code ?? null };
  }
}

// A JSON file read against its source's schema, with `version` (or one
// the record says itself, `versionOf`): { state, value }.
function readJson(file, source, { version = null, versionOf = null, files = fs } = {}) {
  const r = readText(file, files);
  if (r.state !== "ok") return r;
  let value;
  try {
    // Hermes reads its files as utf-8-sig, so one may start with a BOM.
    value = JSON.parse(r.text.replace(/^\uFEFF/, ""));
  } catch {
    reader(source).unparsable(version);
    return { state: "unreadable", code: "not JSON" };
  }
  const v = versionOf ? versionOf(value) : version;
  const checked = reader(source).check(value, { version: v });
  return checked == null ? { state: "unreadable", code: "not what XenoNet reads" } : { state: "ok", value: checked };
}

const num = (v) => (typeof v === "number" && Number.isFinite(v) ? v : null);
const text = (v) => (typeof v === "string" && v ? v : null);

// ---------- the install ----------

// Hermes's root folder: HERMES_HOME when XenoNet's own environment names
// it, else ~/.hermes. Each named profile has the same layout below it.
export const hermesRoot = (home, env = process.env) => (env.HERMES_HOME ? path.resolve(env.HERMES_HOME) : path.join(home, ".hermes"));

// The profiles, by name, with their folders: the default one is the root.
export function listProfiles(root, { files = fs } = {}) {
  const out = [{ name: "default", home: root }];
  let names = [];
  try {
    names = files.readdirSync(path.join(root, "profiles"), { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name);
  } catch {}
  for (const name of names.sort()) if (name !== "default" && !name.startsWith(".")) out.push({ name, home: path.join(root, "profiles", name) });
  return out;
}

// Whether Hermes is installed here: its root, with its checkout or its
// session database in it.
export const installed = (root, { files = fs } = {}) => ["hermes-agent", "state.db"].some((f) => files.existsSync(path.join(root, f)));

// The build stamp: the release this build is of (baseVersion), which the
// other sources are read against, or null.
export function readStamp(root, { files = fs } = {}) {
  const r = readJson(path.join(root, "hermes-agent", "install-stamp.json"), SOURCE.stamp, { files, versionOf: (v) => cleanVersion(num(v?.schemaVersion) != null ? String(v.schemaVersion) : null) });
  return { state: r.state, version: r.state === "ok" ? cleanVersion(text(r.value.baseVersion)) : null };
}

// ---------- spawn-ledger.json (the root's only) ----------

// Hermes's long-lived processes as it keeps them: { pid, createTime (epoch
// seconds), purpose, port, profile, spawnerPid }. Never argv.
export function readLedger(root, { version = null, files = fs } = {}) {
  const r = readJson(path.join(root, "spawn-ledger.json"), SOURCE.ledger, { version, files });
  if (r.state !== "ok") return { state: r.state, entries: [] };
  const entries = (Array.isArray(r.value) ? r.value : [])
    .filter((e) => e && num(e.pid) && text(e.purpose))
    .map((e) => ({ pid: e.pid, createTime: num(e.create_time), purpose: e.purpose, port: num(e.port), profile: text(e.profile), spawnerPid: num(e.spawner_pid) }));
  return { state: "ok", entries };
}

// ---------- a profile's gateway_state.json ----------

// The gateway as its state file says: { state, pid, startTime, updatedAt
// (ms), activeAgents, exitReason, platforms: [{ name, state }] }, or null
// with no file.
export function readGateway(home, { version = null, files = fs } = {}) {
  const r = readJson(path.join(home, "gateway_state.json"), SOURCE.gateway, { version, files });
  if (r.state !== "ok") return { state: r.state, gateway: null };
  const g = r.value;
  const platforms = g.platforms && typeof g.platforms === "object" ? Object.entries(g.platforms).filter(([, p]) => p && typeof p === "object").map(([name, p]) => ({ name, state: text(p.state) })) : [];
  return {
    state: "ok",
    gateway: { state: text(g.gateway_state), pid: num(g.pid), startTime: num(g.start_time), updatedAt: msOf(g.updated_at), activeAgents: num(g.active_agents) ?? 0, exitReason: text(g.exit_reason), platforms },
  };
}

// ---------- a profile's cron/ticker_heartbeat and ticker_last_success ----------

// The scheduler's ticker: { at (ms), pid, lastSuccess (ms) }, or null.
// Each file is one line; one that doesn't read is unparsable.
export function readTicker(home, { version = null, files = fs } = {}) {
  const beat = readText(path.join(home, "cron", "ticker_heartbeat"), files);
  const success = readText(path.join(home, "cron", "ticker_last_success"), files);
  const failed = [beat, success].find((r) => r.state !== "ok" && r.state !== "missing");
  if (failed) return { state: failed.state, ticker: null };
  if (beat.state === "missing" && success.state === "missing") return { state: "missing", ticker: null };
  // "<epoch seconds> <pid>" and "<epoch seconds>".
  const words = (r) => (r.state === "ok" ? r.text.trim().split(/\s+/) : []);
  const number = (w) => (/^\d+(\.\d+)?$/.test(w ?? "") ? Number(w) : null);
  const [at, pid] = words(beat);
  const [last] = words(success);
  if ((beat.state === "ok" && number(at) == null) || (success.state === "ok" && number(last) == null)) {
    reader(SOURCE.ticker).unparsable(version);
    return { state: "unreadable", ticker: null };
  }
  const record = {
    heartbeat: beat.state === "ok" ? { at: number(at), ...(pid != null ? { pid: number(pid) ?? pid } : {}) } : null,
    last_success: success.state === "ok" ? { at: number(last) } : null,
  };
  const checked = reader(SOURCE.ticker).check(record, { version }) ?? {};
  return { state: "ok", ticker: { at: msOf(checked.heartbeat?.at), pid: num(checked.heartbeat?.pid), lastSuccess: msOf(checked.last_success?.at) } };
}

// ---------- a profile's processes.json ----------

// The background processes Hermes's terminal tool runs: { pid, startTime,
// cwd, startedAt (ms), session (the Hermes session that started it) }.
// Never their commands.
export function readProcesses(home, { version = null, files = fs } = {}) {
  const r = readJson(path.join(home, "processes.json"), SOURCE.processes, { version, files });
  if (r.state !== "ok") return { state: r.state, entries: [] };
  const entries = (Array.isArray(r.value) ? r.value : [])
    .filter((e) => e && num(e.pid) && (e.pid_scope == null || e.pid_scope === "host"))
    .map((e) => ({ pid: e.pid, startTime: num(e.host_start_time), cwd: text(e.cwd), startedAt: msOf(e.started_at), session: text(e.parent_session_id) }));
  return { state: "ok", entries };
}

// ---------- a profile's cron/jobs.json ----------

// Scheduled jobs: { id, status (the last run's), state, enabled, workdir,
// lastRunAt, nextRunAt (ms) }. Never names, prompts or destinations.
export function readJobs(home, { version = null, files = fs } = {}) {
  const r = readJson(path.join(home, "cron", "jobs.json"), SOURCE.jobs, { version, files });
  if (r.state !== "ok") return { state: r.state, jobs: [] };
  const jobs = (Array.isArray(r.value?.jobs) ? r.value.jobs : [])
    .filter((j) => j && text(j.id))
    .map((j) => ({ id: j.id, status: text(j.last_status), state: text(j.state), enabled: j.enabled !== false, workdir: text(j.workdir), lastRunAt: msOf(j.last_run_at), nextRunAt: msOf(j.next_run_at) }));
  return { state: "ok", jobs };
}

// ---------- a profile's state.db ----------

const SESSION_COLUMNS = ["id", "source", "parent_session_id", "started_at", "ended_at", "end_reason", "cwd", "git_repo_root", "last_activity_at", "last_activity_description", "archived", "hidden"];
const MESSAGE_COLUMNS = ["id", "session_id", "role", "tool_name", "timestamp", "active"];
const HEARTBEAT_COLUMNS = ["pid", "started_at", "last_heartbeat", "profile"];
// The columns XenoNet can't do without: one that's gone is drift.
const NEEDED = { sessions: ["id", "source", "started_at", "ended_at", "cwd"], messages: ["id", "session_id", "role", "tool_name", "timestamp", "tool_calls"] };
// New messages read at once at most; the rest wait for the next look.
const MESSAGES_MAX = 5000;
// On a first read, how far back to start, at most.
const FIRST_ROWS = 5000;

// The tool names whose `path` argument is a file Hermes changed.
export const EDIT_TOOLS = ["write_file", "patch"];

// The path argument of each write_file and patch call in messages
// (after, upTo], taken out by SQLite: a call's other arguments, and its
// message's content, never leave the database. A tool_calls that isn't a
// JSON array, a call in it that isn't an object, or a call whose arguments
// aren't JSON, is passed over. A call that's a string would make
// json_extract fail the whole read, and the same row would fail every read
// after it, so only objects are opened, by json_each's own type, and in a
// CASE, which SQLite evaluates in order (a WHERE's terms it may not).
const toolPathsSql = (withTime) => `
  WITH calls AS (
    SELECT m.id, m.session_id, m.timestamp, tc.type AS kind, tc.value AS call
    FROM messages m, json_each(CASE WHEN json_valid(m.tool_calls) THEN CASE WHEN json_type(m.tool_calls) = 'array' THEN m.tool_calls END END) tc
    WHERE m.id > ? AND m.id <= ?${withTime ? " AND m.timestamp > ?" : ""} AND m.role = 'assistant' AND m.tool_calls IS NOT NULL
  ), named AS (
    SELECT id, session_id, timestamp,
      CASE WHEN kind = 'object' THEN json_extract(call, '$.function.name') END AS tool,
      CASE WHEN kind = 'object' THEN json_extract(call, '$.function.arguments') END AS args
    FROM calls
  )
  SELECT id, session_id, timestamp, tool,
    CASE WHEN json_valid(args) THEN CASE WHEN json_type(args, '$.path') = 'text' THEN json_extract(args, '$.path') END END AS path
  FROM named WHERE tool IN (${EDIT_TOOLS.map((t) => `'${t}'`).join(", ")}) ORDER BY id`;

const plain = (rows) => rows.map((r) => ({ ...r }));

// What's new in a profile's session database since the last read: the
// messages after `afterId` (on a first read, those of the last `sinceMs`),
// the edit tools' paths among them, the sessions they belong to, the ones
// still open that were active since `sinceMs` or ended since then, and the
// backends' heartbeats. { state, version, sessions, messages, toolPaths,
// heartbeats, lastId }. `open` opens the file read only (tests give their
// own).
export function readStateDb(file, { afterId = null, sinceMs, sessionIds = [], open = (f) => new DatabaseSync(f, { readOnly: true }), files = fs } = {}) {
  const empty = { version: null, sessions: [], messages: [], toolPaths: [], heartbeats: [], lastId: afterId };
  if (!files.existsSync(file)) return { state: "missing", ...empty };
  let db;
  try {
    db = open(file);
  } catch (err) {
    return { state: /permission|EACCES|EPERM|authoriz/i.test(String(err?.message)) ? "denied" : "unreadable", code: err?.code ?? null, ...empty };
  }
  const stateDb = reader(SOURCE.stateDb);
  try {
    const columns = (table) => new Set(db.prepare(`PRAGMA table_info(${table})`).all().map((c) => c.name));
    const has = { sessions: columns("sessions"), messages: columns("messages"), gateway_heartbeats: columns("gateway_heartbeats"), schema_version: columns("schema_version") };
    const version = has.schema_version.has("version") ? cleanVersion(String(db.prepare("SELECT max(version) AS v FROM schema_version").get()?.v ?? "")) : null;
    for (const [table, needed] of Object.entries(NEEDED)) {
      if (!has[table].size) stateDb.missing(table, version);
      else for (const c of needed) if (!has[table].has(c)) stateDb.missing(`${table}[].${c}`, version);
    }
    if (!has.sessions.has("id") || !has.messages.has("id")) return { state: "unreadable", code: "not Hermes's layout", ...empty, version };
    const since = (sinceMs ?? 0) / 1000;
    const pick = (wanted, table) => wanted.filter((c) => has[table].has(c));

    // The messages since the last read: from the last one read, or on a
    // first read (or after the file was made again) the recent ones
    // among the last few thousand.
    const maxId = db.prepare("SELECT max(id) AS n FROM messages").get()?.n ?? 0;
    const from = afterId != null && afterId <= maxId ? afterId : Math.max(0, maxId - FIRST_ROWS);
    // Whether a row is a tool call (so not a reply to anyone) is all that's
    // taken of its tool_calls here.
    const msgCols = [...pick(MESSAGE_COLUMNS, "messages"), ...(has.messages.has("tool_calls") ? ["(tool_calls IS NOT NULL) AS calls"] : [])];
    const time = has.messages.has("timestamp") ? " AND timestamp > ?" : "";
    const messages = plain(db.prepare(`SELECT ${msgCols.join(", ")} FROM messages WHERE id > ?${time} ORDER BY id LIMIT ${MESSAGES_MAX}`).all(...[from, ...(time ? [since] : [])]));
    const lastId = messages.length === MESSAGES_MAX ? messages.at(-1).id : maxId;
    const toolPaths = has.messages.has("tool_calls") && has.messages.has("role") ? plain(db.prepare(toolPathsSql(!!time)).all(...[from, lastId, ...(time ? [since] : [])])) : [];

    // The sessions those belong to, the ones open and active since then,
    // and the ones that ended since then.
    const ids = [...new Set([...sessionIds, ...messages.map((m) => m.session_id), ...toolPaths.map((t) => t.session_id)])].filter((id) => typeof id === "string");
    const active = has.sessions.has("last_activity_at") ? "COALESCE(last_activity_at, started_at)" : "started_at";
    const ended = has.sessions.has("ended_at");
    const where = [`id IN (SELECT value FROM json_each(?))`, ...(ended ? [`(ended_at IS NULL AND ${active} > ?)`, `ended_at > ?`] : [`${active} > ?`])];
    const sessions = plain(db.prepare(`SELECT ${pick(SESSION_COLUMNS, "sessions").join(", ")} FROM sessions WHERE ${where.join(" OR ")} ORDER BY started_at DESC LIMIT 500`).all(JSON.stringify(ids), since, ...(ended ? [since] : [])));
    const heartbeats = has.gateway_heartbeats.has("pid") ? plain(db.prepare(`SELECT ${pick(HEARTBEAT_COLUMNS, "gateway_heartbeats").join(", ")} FROM gateway_heartbeats`).all()) : [];

    const checked = stateDb.check({ schema_version: version != null ? [{ version: Number(version) }] : [], sessions, messages, tool_paths: toolPaths, gateway_heartbeats: heartbeats }, { version }) ?? {};
    return {
      state: "ok",
      version,
      sessions: checked.sessions ?? [],
      // Archived compaction generations are history, not new work.
      messages: (checked.messages ?? []).filter((m) => m.active !== 0),
      toolPaths: checked.tool_paths ?? [],
      heartbeats: checked.gateway_heartbeats ?? [],
      lastId,
    };
  } catch (err) {
    return { state: /SQLITE_BUSY|locked/i.test(String(err?.message)) ? "busy" : "unreadable", code: err?.code ?? null, ...empty };
  } finally {
    try {
      db.close();
    } catch {}
  }
}
