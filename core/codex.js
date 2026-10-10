// Codex adapter. Talks to a private `codex app-server` over stdio JSON-RPC,
// the same protocol the ChatGPT desktop app is built on. It lists and reads
// threads, renames one when you ask, and never starts turns. Its answers,
// and the state database's rows, are read leniently against their schemas
// (vendor.js), with Codex's version from the app-server's user agent. A
// thread the app-server can't list the items of is read from its rollout
// file.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { config } from "./config.js";
import { excerpt } from "./units.js";
import { findCodexBinary } from "./dirs.js";
import { VERSION } from "./version.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, codexVersionOf } from "./vendor.js";
import { readLines, readNewLines } from "./lines.js";
import { commandText } from "./live.js";

const answers = reader("openai/codex/app-server");
const stateDb = reader("openai/codex/state-db");
// The state database XenoNet reads, and its format's version.
const STATE_DB = 5;

const SOURCE_KINDS = ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther"];
const REQUEST_TIMEOUT_MS = 20000;

// The client is a building block (core/blocks/lifecycle.js): it starts
// the app-server on the first request, and again after it exits. Its
// health: running once the app-server answers, degraded after a request it
// refused or didn't answer in time, failed when it couldn't start or
// exited, stopped when XenoNet stopped it. `spawn` and `bin` stand in for
// the real ones in tests.
export class AppServerClient extends Block {
  constructor({ spawn: spawnFn = spawn, bin = findCodexBinary, timeoutMs = REQUEST_TIMEOUT_MS, clock } = {}) {
    super({ name: "codex-app-server", kind: "client", clock });
    this.spawn = spawnFn;
    this.findBin = bin;
    this.timeoutMs = timeoutMs;
    this.proc = null;
    this.ready = null;
    this.nextId = 1;
    this.pending = new Map();
    this.stderrTail = [];
    this.info = null;
    // Codex's version, from initialize's user agent.
    this.version = null;
    this.counters = { errors: 0, starts: 0, requests: 0, timeouts: 0 };
  }

  // Starts the app-server, unless it's running: the answer to initialize.
  connect() {
    if (this.ready) return this.ready;
    const bin = this.findBin();
    this.bin = bin;
    this.count("starts");
    const proc = this.spawn(bin, ["app-server"], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
    this.proc = proc;
    let buf = "";
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => {
      buf += chunk;
      let nl;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl).trim();
        buf = buf.slice(nl + 1);
        if (line) this.#onMessage(line);
      }
    });
    proc.stderr.setEncoding("utf8");
    proc.stderr.on("data", (chunk) => {
      this.stderrTail.push(...chunk.split(/\r?\n/).filter(Boolean));
      this.stderrTail = this.stderrTail.slice(-20);
    });
    const fail = (err) => {
      // An app-server XenoNet stopped, or one another has replaced, is no
      // longer this client's: its end mustn't touch the next one's requests.
      if (this.proc !== proc) return;
      this.#rejectAll(err);
      this.proc = null;
      this.ready = null;
      this.fail(err);
    };
    proc.on("error", fail);
    proc.on("exit", (code) => fail(new Error(`codex app-server exited (${code}): ${this.stderrTail.slice(-3).join(" | ")}`)));

    this.ready = this.#call("initialize", {
      clientInfo: { name: "xenonet", title: "XenoNet", version: VERSION },
      capabilities: { experimentalApi: true },
    }).then((result) => {
      this.#send({ jsonrpc: "2.0", method: "initialized" });
      this.version = codexVersionOf(result?.userAgent);
      answers.check({ initialize: result }, { version: this.version });
      this.info = result;
      this.started = true;
      this.lastOk = this.clock.now();
      this.setState("running");
      return result;
    });
    this.ready.catch(() => {});
    return this.ready;
  }

  // As a block: started is connected.
  async onStart() {
    await this.connect();
  }

  // Stops the app-server: what waits on it fails now, and the next request
  // starts another, without waiting for this one to exit.
  onStop() {
    const proc = this.proc;
    this.proc = null;
    this.ready = null;
    this.#rejectAll(new Error("codex app-server stopped"));
    proc?.kill();
  }

  #rejectAll(err) {
    for (const { reject, timer } of this.pending.values()) {
      this.clock.clearTimeout(timer);
      reject(err);
    }
    this.pending.clear();
  }

  #send(msg) {
    this.proc?.stdin.write(JSON.stringify(msg) + "\n");
  }

  #onMessage(line) {
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      return;
    }
    if (msg.id !== undefined && msg.method) {
      // Server-initiated request (approvals etc.). We never start turns, so decline.
      this.#send({ jsonrpc: "2.0", id: msg.id, error: { code: -32601, message: "XenoNet is read-only" } });
      return;
    }
    if (msg.id !== undefined && this.pending.has(msg.id)) {
      const { resolve, reject, timer } = this.pending.get(msg.id);
      this.clock.clearTimeout(timer);
      this.pending.delete(msg.id);
      if (msg.error) reject(Object.assign(new Error(msg.error.message ?? "app-server error"), { rpc: msg.error }));
      else resolve(msg.result);
    }
    // Notifications are ignored; the poller picks up changes.
  }

  #call(method, params) {
    this.count("requests");
    // Stopped, or gone, since the request was made: there's no one to ask.
    if (!this.proc) return Promise.reject(new Error("codex app-server isn't running"));
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const timer = this.clock.setTimeout(() => {
        this.pending.delete(id);
        this.count("timeouts");
        reject(new Error(`${method} timed out`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.#send({ jsonrpc: "2.0", id, method, params });
    });
  }

  async request(method, params) {
    await this.connect();
    try {
      const result = await this.#call(method, params);
      this.ok();
      return result;
    } catch (err) {
      // An answer that's an error, or none in time, while it runs.
      if (this.proc) this.problem(err);
      throw err;
    }
  }
}

const client = new AppServerClient();
export const codexClient = client;

function toMs(t) {
  if (t == null) return null;
  if (typeof t === "string") return Date.parse(t) || null;
  return t < 1e12 ? t * 1000 : t;
}

// Codex runs sub-agent threads of two sorts. Ones a session spawns record
// their parent in source.subAgent.thread_spawn; the rest (guardian checks,
// reviews, compaction) and `codex exec` runs are background work, which the
// map leaves out unless you ask for it.
export function subAgentOf(t) {
  const sub = t.source && typeof t.source === "object" ? t.source.subAgent : null;
  const spawn = sub && typeof sub === "object" ? sub.thread_spawn ?? null : null;
  const kind = spawn ? "spawned" : sub == null ? null : typeof sub === "string" ? sub : (sub.other ?? Object.keys(sub)[0] ?? "other");
  return { parent: spawn?.parent_thread_id ?? null, nickname: spawn?.agent_nickname ?? null, kind };
}

export function threadToSession(t, archived) {
  const status = typeof t.status === "string" ? t.status : t.status?.type ?? null;
  const agent = subAgentOf(t);
  const parentThread = t.parentThreadId ?? agent.parent;
  return {
    id: `codex:${t.id}`,
    app: "codex",
    rawId: t.id,
    title: t.name || t.preview?.split("\n")[0] || "(untitled thread)",
    preview: t.preview ?? null,
    cwd: t.cwd ?? null,
    createdAt: toMs(t.createdAt),
    updatedAt: toMs(t.recencyAt ?? t.updatedAt),
    archived,
    status,
    model: t.model ?? null,
    // A thread run on another provider (Ollama, Azure, ...) isn't your
    // ChatGPT account's (see identities.js).
    provider: t.modelProvider ?? null,
    effort: t.reasoningEffort ?? null,
    branch: t.gitInfo?.branch ?? null,
    origin: t.gitInfo?.originUrl ?? t.gitInfo?.origin_url ?? null,
    source: typeof t.source === "string" ? t.source : t.source ? Object.keys(t.source)[0] : null,
    originator: t.originator ?? null,
    nativeProjectId: t.projectId ?? null,
    parentId: t.forkedFromId ? `codex:${t.forkedFromId}` : parentThread ? `codex:${parentThread}` : null,
    parentKind: t.forkedFromId ? "fork" : parentThread ? "subagent" : null,
    agentNickname: t.agentNickname ?? agent.nickname,
    agentKind: agent.kind,
    background: t.source === "exec" || t.originator === "codex_exec" || (agent.kind != null && agent.kind !== "spawned"),
  };
}

export async function listCodexThreads() {
  const sessions = [];
  for (const archived of [false, true]) {
    let cursor = null;
    for (let page = 0; page < 20; page++) {
      const res = await client.request("thread/list", {
        archived,
        limit: 100,
        cursor,
        sourceKinds: SOURCE_KINDS,
        useStateDbOnly: true,
      });
      const page = answered("thread/list", res);
      for (const t of page.data ?? []) sessions.push(threadToSession(t, archived));
      cursor = page.nextCursor ?? null;
      if (!cursor) break;
    }
  }
  return sessions;
}

// Renames a thread through the app-server, as the ChatGPT desktop app does:
// Codex writes the name to its session index, its state database and the
// thread's rollout, loaded or not.
export async function renameCodexThread(rawId, name) {
  await client.request("thread/name/set", { threadId: rawId, name });
}

// An app-server's answer to `method`, read leniently: without what's of
// another type, or {} when it isn't an object.
const answered = (method, result) => answers.check({ [method]: result }, { version: client.version })?.[method] ?? {};

// Codex's threads, and Codex desktop's projects, as gatherers (see
// core/blocks/gatherer.js): listed again on every refresh, timed, and
// empty after a listing that failed (the map keeps the sessions it had;
// see state.js). `gather` stands in for the listing, as the demo's does.
export const codexThreadsGatherer = ({ gather = listCodexThreads, clock } = {}) => new Gatherer({ name: "codex-threads", gather, fallback: [], clock });
export const codexProjectsGatherer = ({ gather = async () => listCodexNativeProjects(), clock } = {}) => new Gatherer({ name: "codex-projects", gather, fallback: [], clock });

// Codex desktop "projects" are not in the app-server API yet, so read them from
// the state DB. Undocumented: failures here only remove the extra grouping.
export function listCodexNativeProjects() {
  noteStateDbs();
  const file = path.join(config.codexHome, `state_${STATE_DB}.sqlite`);
  if (!fs.existsSync(file)) return [];
  let db;
  try {
    db = new DatabaseSync(file, { readOnly: true });
    const rows = stateDb.check(
      {
        projects: db.prepare("select id, name from projects").all(),
        project_roots: db.prepare("select project_id, path from project_roots order by position").all(),
      },
      { version: String(STATE_DB) },
    );
    const roots = rows.project_roots ?? [];
    return (rows.projects ?? []).map((p) => ({
      id: p.id,
      name: p.name,
      roots: roots.filter((r) => r.project_id === p.id).map((r) => r.path),
    }));
  } catch {
    if (db) missingColumns(db);
    return [];
  } finally {
    db?.close();
  }
}

// A query that failed in a database that opened: the tables and columns it
// reads that aren't there are drift. A file that isn't a database isn't.
function missingColumns(db) {
  for (const [table, columns] of [["projects", ["id", "name"]], ["project_roots", ["project_id", "path"]]]) {
    let have;
    try {
      have = db.prepare(`pragma table_info(${table})`).all().map((c) => c.name);
    } catch {
      return;
    }
    if (!have.length) stateDb.missing(table, String(STATE_DB));
    else for (const c of columns) if (!have.includes(c)) stateDb.missing(`${table}[].${c}`, String(STATE_DB));
  }
}

// A state database of another number (state_6.sqlite) is a version the
// schema wasn't made for (the reader keeps it as drift once).
function noteStateDbs() {
  let names = [];
  try {
    names = fs.readdirSync(config.codexHome);
  } catch {}
  for (const name of names) {
    const n = /^state_(\d+)\.sqlite$/.exec(name)?.[1];
    if (n && Number(n) !== STATE_DB) stateDb.pick(n);
  }
}

const textOf = (content) =>
  Array.isArray(content) ? content.map((c) => c?.text ?? (c?.type === "image" || c?.type === "localImage" || c?.type === "local_image" ? "[image]" : "")).join("\n") : String(content ?? "");

function normalizeItem(entry) {
  const it = entry.item ?? {};
  const ts = entry.startedAtMs ?? entry.completedAtMs ?? null;
  const base = { id: it.id, ts, kind: it.type };
  switch (it.type) {
    case "userMessage":
      return { ...base, role: "user", text: textOf(it.content) };
    case "agentMessage":
      return { ...base, role: "assistant", text: it.text ?? "" };
    case "reasoning":
      return { ...base, role: "reasoning", text: [...(it.summary ?? []), ...(it.content ?? [])].map((x) => (typeof x === "string" ? x : x.text ?? "")).join("\n") };
    case "plan":
      return { ...base, role: "assistant", title: "plan", text: it.text ?? "" };
    case "commandExecution":
      return { ...base, role: "tool", title: it.command, status: it.status, meta: it.exitCode != null ? `exit ${it.exitCode}` : null, text: excerpt(it.aggregatedOutput) };
    case "mcpToolCall":
      return { ...base, role: "tool", title: `${it.server}.${it.tool}`, status: it.status, text: excerpt(JSON.stringify(it.error ?? it.result?.content ?? it.result ?? it.arguments, null, 2)) };
    case "dynamicToolCall":
      return { ...base, role: "tool", title: it.tool, status: it.status, text: excerpt(JSON.stringify(it.arguments, null, 2)) };
    case "fileChange":
      return { ...base, role: "tool", title: `edit ${it.changes?.length ?? 0} file(s)`, status: it.status, text: (it.changes ?? []).map((c) => `${c.kind?.type ?? c.kind ?? ""} ${c.path}`).join("\n") };
    case "webSearch":
      return { ...base, role: "tool", title: `search: ${it.query ?? ""}` , text: "" };
    case "contextCompaction":
      return { ...base, role: "system", text: "context compacted" };
    default:
      return { ...base, role: "system", title: it.type, text: excerpt(JSON.stringify(it, null, 2), 1500) };
  }
}

// A finished item as a rollout writes it (event_msg's item_completed: the
// app-server's items, in Codex's own names), as the console shows it.
function rolloutItem(it, ts, n) {
  const base = { id: it.id ?? `item-${n}`, ts, kind: it.type };
  switch (it.type) {
    case "UserMessage":
      return { ...base, role: "user", text: textOf(it.content) };
    case "AgentMessage":
      return { ...base, role: "assistant", text: textOf(it.content) };
    case "Reasoning":
      return { ...base, role: "reasoning", text: [...(it.summary_text ?? []), ...(it.raw_content ?? [])].map((x) => (typeof x === "string" ? x : x?.text ?? "")).join("\n") };
    case "CommandExecution":
      return { ...base, role: "tool", title: commandText(it.command), status: it.status, meta: it.exit_code != null ? `exit ${it.exit_code}` : null, text: excerpt(it.aggregated_output) };
    case "McpToolCall":
      return { ...base, role: "tool", title: `${it.server}.${it.tool}`, status: it.status, text: excerpt(JSON.stringify(it.error ?? it.result?.content ?? it.result ?? it.arguments, null, 2)) };
    case "FileChange": {
      const changes = Object.entries(it.changes ?? {});
      return { ...base, role: "tool", title: `edit ${changes.length} file(s)`, status: it.status, text: changes.map(([p, c]) => `${c?.type ?? ""} ${p}`).join("\n") };
    }
    case "WebSearch":
      return { ...base, role: "tool", title: `search: ${it.query || it.action?.query || ""}`, text: "" };
    case "ContextCompaction":
      return { ...base, role: "system", text: "context compacted" };
    default:
      return { ...base, role: "system", title: it.type, text: excerpt(JSON.stringify(it, null, 2), 1500) };
  }
}

// A thread's items from its rollout, oldest first, as its lines are read:
// its finished items, each at the time it started (or finished, or was
// written). A rollout with none, as `codex exec` writes them, has the
// older lines: what you said and the agent said as events, and each call
// and its output, which are kept only until a finished item turns up.
class RolloutEntries {
  items = [];
  older = [];
  get list() {
    return this.items.length ? this.items : this.older;
  }

  add(o) {
    const p = o.payload;
    const at = Number.isFinite(o.ts) ? o.ts : null;
    if (o.type === "event_msg" && p.type === "item_completed" && p.item && typeof p.item === "object") {
      this.items.push(rolloutItem(p.item, p.started_at_ms ?? p.completed_at_ms ?? at, this.items.length));
      this.older = [];
      return;
    }
    if (this.items.length) return;
    const base = { id: p.id ?? p.call_id ?? `item-${this.older.length}`, ts: at, kind: p.type };
    if (o.type === "event_msg" && (p.type === "user_message" || p.type === "agent_message")) this.older.push({ ...base, role: p.type === "user_message" ? "user" : "assistant", text: String(p.message ?? "") });
    else if (o.type === "response_item" && /^(function_call|custom_tool_call|local_shell_call)$/.test(p.type)) {
      const input = p.input ?? p.arguments ?? p.action ?? null;
      this.older.push({ ...base, role: "tool", title: p.name ?? p.type, text: excerpt(typeof input === "string" ? input : JSON.stringify(input, null, 2), 2500) });
    } else if (o.type === "response_item" && /^(function_call|custom_tool_call|local_shell_call)_output$/.test(p.type)) {
      this.older.push({ ...base, role: "tool", title: "result", text: excerpt(typeof p.output === "string" ? p.output : JSON.stringify(p.output ?? null, null, 2)) });
    }
  }
}

// The rollout read last, kept while it's paged through, and read on from
// where it ended as it grows, as the tailer reads: a rollout can be
// hundreds of megabytes, and is written to all the while a thread runs. A
// compressed archive is read whole, again only if it changes. One read at
// a time, so two can't take the same lines.
let lastRollout = null;
let rolloutReads = Promise.resolve();

// A page of a thread from its rollout file, read through the lenient
// reader (lines.js): newest first, `cursor` ("rollout:<n>", the items
// already shown from the end) continuing further back.
export function readRolloutPage(file, opts = {}) {
  const run = rolloutReads.then(() => rolloutPage(file, opts));
  rolloutReads = run.catch(() => {});
  return run;
}

async function rolloutPage(file, { cursor = null, limit = 120 } = {}) {
  const st = await fs.promises.stat(file);
  const zst = file.endsWith(".zst");
  const stamp = `${st.size}:${st.mtimeMs}`;
  let r = lastRollout?.file === file && !(zst && lastRollout.stamp !== stamp) ? lastRollout : null;
  r ??= lastRollout = { file, stamp: null, from: null, entries: new RolloutEntries() };
  if (zst) {
    if (r.stamp !== stamp) for (const o of await readLines(file, "codex")) r.entries.add(o);
  } else if (r.from?.offset !== st.size) {
    const read = await readNewLines(file, "codex", r.from);
    // Written again from the start: what was read of it before is gone.
    if (read.restarted) r.entries = new RolloutEntries();
    for (const o of read.lines) r.entries.add(o);
    r.from = read;
  }
  r.stamp = stamp;
  const all = r.entries.list;
  const shown = Number(/^rollout:(\d+)$/.exec(cursor ?? "")?.[1] ?? 0);
  const end = Math.max(0, all.length - shown);
  const start = Math.max(0, end - limit);
  return { entries: all.slice(start, end), cursor: start > 0 ? `rollout:${all.length - start}` : null, total: all.length };
}

// The versions of Codex whose app-server said it can't list a thread's
// items (this Mac's answers "not supported yet"): they're asked no more.
const noItems = new Set();
const saysUnsupported = (err) => !!err?.rpc && (err.rpc.code === -32601 || /not (yet )?supported|unsupported|not implemented|unknown method|method not found/i.test(err.rpc.message ?? ""));

// Newest page first; `cursor` continues further back in time. When the
// app-server can't list the thread's items, because it doesn't do that
// yet or doesn't answer at all, `rollout()` finds the thread's rollout
// file, which has them too.
export async function readCodexThread(rawId, { cursor = null, limit = 120, rollout = null } = {}) {
  const fromRollout = async (err) => {
    const file = await rollout?.().catch(() => null);
    if (!file) throw err;
    return readRolloutPage(file, { cursor, limit });
  };
  // A page further back in a thread read from its rollout comes from it too.
  if (rollout && /^rollout:/.test(cursor ?? "")) return fromRollout(new Error("XenoNet can't find this thread's rollout any more"));
  if (rollout && noItems.has(client.version)) return fromRollout(new Error(`${client.version ? `Codex ${client.version}` : "This Codex"} can't list a thread's items, and XenoNet can't find this thread's rollout`));
  let res;
  try {
    res = await client.request("thread/items/list", { threadId: rawId, limit, cursor, sortDirection: "desc" });
  } catch (err) {
    if (saysUnsupported(err)) noItems.add(client.version);
    // A page further back by the app-server's own cursor has no place in
    // the rollout.
    if (cursor) throw err;
    return fromRollout(err);
  }
  const page = answered("thread/items/list", res);
  const entries = (page.data ?? []).map(normalizeItem).reverse();
  return { entries, cursor: page.nextCursor ?? null };
}
