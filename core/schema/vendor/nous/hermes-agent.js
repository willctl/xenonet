// Hermes Agent's data, as XenoNet's observer reads it (core/actors/hermes.js;
// docs/design/actors/hermes.md, "The vendor schemas XenoNet needs"): the
// metadata in its session database, its spawn ledger, its gateway's state
// file, its checkpoint of background processes, its scheduled jobs and
// their ticker, and its build stamp. Each declares only the fields XenoNet
// reads. The ones that hold content, secrets or who's on the other end
// (argv, hermes_home, commands, prompts, titles, chat ids, the machine's
// name, error texts) are never read, so they aren't declared either.
//
// Hermes's own files carry no version, so they're read against the version
// in its build stamp (install-stamp.json's baseVersion). The session
// database says its layout's version itself (schema_version), and the
// stamp its own format's (schemaVersion).
import { vendor, open, variants, maybe, num, str, z } from "../common.js";

// The versions of Hermes these were made for: 0.18.0, the first install
// here, to 0.21.5 and the main branch past it.
const HERMES = "<1.0.0";
// Hermes writes times as epoch seconds (floats) or ISO 8601.
const when = z.union([num, str]);

// ---------- state.db ----------

const session = open({
  id: str,
  source: maybe(str).describe("cli, oneshot, desktop, tui, a messaging platform, cron, subagent… an open set"),
  parent_session_id: maybe(str),
  started_at: num.describe("Epoch seconds"),
  ended_at: maybe(num),
  end_reason: maybe(str),
  cwd: maybe(str),
  git_repo_root: maybe(str),
  last_activity_at: maybe(num).describe("Epoch seconds; the current layout only"),
  last_activity_description: maybe(str).describe("A machine phrase from fixed templates (executing tool: terminal), matched and put into XenoNet's own words, never shown"),
  archived: maybe(num),
  hidden: maybe(num),
});

export const StateDb = vendor(
  "nous/hermes-agent/state-db",
  1,
  open({
    schema_version: z.array(open({ version: num })).describe("select version from schema_version"),
    sessions: z.array(session).describe("select id, source, parent_session_id, started_at, ended_at, end_reason, cwd, git_repo_root, last_activity_at, last_activity_description, archived, hidden from sessions, the columns that are there; never title, system_prompt, user_id, chat_id, display_name or origin_json"),
    messages: z
      .array(open({ id: num, session_id: str, role: str, tool_name: maybe(str), timestamp: num, active: maybe(num), calls: maybe(num).describe("1 when the row holds tool calls (so isn't a reply), from tool_calls IS NOT NULL") }))
      .describe("select id, session_id, role, tool_name, timestamp, active, (tool_calls IS NOT NULL) AS calls from messages: what happened and when, never content"),
    tool_paths: z
      .array(open({ id: num, session_id: str, timestamp: num, tool: str, path: maybe(str) }))
      .describe("The path argument of write_file and patch calls in messages.tool_calls, taken out by SQLite (json_extract), so nothing else in a call leaves the database"),
    gateway_heartbeats: z.array(open({ pid: num, started_at: maybe(num), last_heartbeat: num, profile: maybe(str) })).describe("select pid, started_at, last_heartbeat, profile from gateway_heartbeats; never host, which is the machine's name"),
  }),
  {
    title: "Hermes Agent's session database",
    description:
      "The metadata XenoNet reads from ~/.hermes/state.db (and each profile's), opened read only for a moment, with named columns. The version is the database's own schema_version: through 32, read by the columns PRAGMA table_info lists. Version 32 records additive structures introduced under 31; the metadata columns read here are unchanged.",
    versions: "<=32",
  },
);

// ---------- spawn-ledger.json ----------

const PURPOSES = ["cli", "serve", "dashboard", "gateway", "mcp-helper"];
const ledgerEntry = (purpose) =>
  open({
    purpose: z.literal(purpose),
    pid: num,
    create_time: maybe(num).describe("The process's start, in epoch seconds, to tell a live entry from a pid used again"),
    install: maybe(str),
    spawner_pid: maybe(num),
    spawner_create: maybe(num),
    registered_at: maybe(num),
    port: maybe(num).describe("Where a serve or dashboard listens, on 127.0.0.1"),
    profile: maybe(str),
    isolated: maybe(z.boolean()),
  });

export const SpawnLedger = vendor("nous/hermes-agent/spawn-ledger", 1, z.array(variants("purpose", PURPOSES.map(ledgerEntry))), {
  title: "Hermes Agent's spawn ledger",
  description:
    "~/.hermes/spawn-ledger.json: every long-lived Hermes process of the install (the classic CLI, a desktop backend, the dashboard, the gateway, MCP helpers), as Hermes keeps it for itself. Dead entries stay until the next one is added, so each is checked against the process table. argv is never read: it can hold a prompt.",
  versions: HERMES,
});

// ---------- gateway_state.json ----------

const GATEWAY_STATES = ["starting", "running", "degraded", "draining", "stopped", "startup_failed"];
const platform = open({ state: maybe(str), needs_attention: maybe(z.boolean()), updated_at: maybe(when) });
const gatewayState = (state) =>
  open({
    gateway_state: z.literal(state),
    kind: maybe(str),
    pid: maybe(num),
    start_time: maybe(num).describe("The process's start: centiseconds of the epoch on macOS"),
    updated_at: maybe(when).describe("The heartbeat, stamped again every 60 s and stale after 120 s: ISO 8601, or epoch seconds from older gateways"),
    exit_reason: maybe(str),
    active_agents: maybe(num),
    served_profiles: maybe(z.array(str)),
    code_version: maybe(str),
    platforms: maybe(z.record(str, platform).describe("By platform: its state only, never its error text or addresses")),
  });

export const GatewayState = vendor("nous/hermes-agent/gateway-state", 1, variants("gateway_state", GATEWAY_STATES.map(gatewayState)), {
  title: "Hermes Agent's gateway state",
  description: "gateway_state.json in ~/.hermes (and each profile's): the always-on gateway's state, heartbeat and messaging platforms. argv, hermes_home and each platform's error_message, ingress_url and listener_base are never read.",
  versions: HERMES,
});

// ---------- processes.json ----------

export const Processes = vendor(
  "nous/hermes-agent/processes",
  1,
  z.array(
    open({
      pid: num,
      pid_scope: maybe(str),
      host_start_time: maybe(num),
      cwd: maybe(str),
      started_at: maybe(num).describe("Epoch seconds"),
      parent_session_id: maybe(str).describe("The Hermes session that started it"),
      task_id: maybe(str),
    }),
  ),
  {
    title: "Hermes Agent's background processes",
    description: "processes.json in ~/.hermes (and each profile's): the processes Hermes's terminal tool started in the background and still runs, its checkpoint for after a crash. Commands, watch patterns and session keys are never read.",
    versions: HERMES,
  },
);

// ---------- cron/jobs.json ----------

const JOB_STATUSES = ["ok", "error", "delivery_failed", "blocked_config", null];
const job = (status) =>
  open({
    id: str,
    last_status: z.literal(status).describe("null until a job first runs"),
    state: maybe(str).describe("scheduled, paused, completed or error"),
    enabled: maybe(z.boolean()),
    schedule: maybe(open({ kind: maybe(str) })),
    next_run_at: maybe(when),
    last_run_at: maybe(when),
    workdir: maybe(str),
  });

export const CronJobs = vendor("nous/hermes-agent/cron-jobs", 1, open({ jobs: z.array(variants("last_status", JOB_STATUSES.map(job))), updated_at: maybe(when) }), {
  title: "Hermes Agent's scheduled jobs",
  description: "cron/jobs.json in ~/.hermes (and each profile's): when each job runs and how its last run went, and the folder it works in. Names, prompts, scripts, skills, destinations and error texts are never read.",
  versions: HERMES,
});

// ---------- cron/ticker_heartbeat, cron/ticker_last_success ----------

export const CronTicker = vendor(
  "nous/hermes-agent/cron-ticker",
  1,
  open({
    heartbeat: maybe(open({ at: num.describe("Epoch seconds"), pid: maybe(num) })).describe("ticker_heartbeat: one line, <epoch seconds> <pid>"),
    last_success: maybe(open({ at: num })).describe("ticker_last_success: one line, <epoch seconds>"),
  }),
  {
    title: "Hermes Agent's scheduler ticker",
    description: "The scheduler's two heartbeat files in ~/.hermes/cron (and each profile's), rewritten on every 60 s tick by the gateway or a desktop backend. They aren't JSON: XenoNet reads each line into this shape, and a line that doesn't read is unparsable.",
    versions: HERMES,
  },
);

// ---------- hermes-agent/install-stamp.json ----------

export const InstallStamp = vendor(
  "nous/hermes-agent/install-stamp",
  1,
  open({
    schemaVersion: maybe(num),
    baseVersion: str.describe("The release this build is of, such as 0.21.5: the version every other Hermes source is read against"),
    displayVersion: maybe(str),
    source: maybe(str),
    payload: maybe(str),
    branch: maybe(str),
  }),
  {
    title: "Hermes Agent's build stamp",
    description: "~/.hermes/hermes-agent/install-stamp.json, rewritten on every update. The version is the stamp's own format, its schemaVersion.",
    versions: "<=2",
  },
);
