// core/codex.js in Rust (crates/xenonet-sources/src/codex.rs): the Codex
// adapter. Rust reads what the `codex app-server` writes (its lines, the
// requests it makes of XenoNet, which XenoNet declines, its answers and
// errors), makes a thread a session and a thread's items what the console
// shows, from the app-server or from the thread's rollout, pages through
// them, and reads Codex desktop's projects from its state database. The
// app-server's process (through the `spawn` a test passes), the requests'
// promises and time limits, the lenient checks of what it answers
// (vendor.js's readers) and the rollouts' reads stay here.
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { config } from "../../../../core/config.js";
import { VERSION } from "../../../../core/version.js";
import { findCodexBinary } from "./dirs.js";
import { Block } from "./blocks/lifecycle.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader, codexVersionOf } from "./vendor.js";
import { readLines, readNewLines } from "./lines.js";
import { native } from "../index.js";

const answers = reader("openai/codex/app-server");
const stateDb = reader("openai/codex/state-db");
// The state database XenoNet reads, and its format's version.
const STATE_DB = native.codexStateDb();

const SOURCE_KINDS = native.codexSourceKinds();

// The client is a building block: it starts the app-server on the first
// request, and again after it exits. Its health: running once the
// app-server answers, degraded after a request it refused or didn't answer
// in time, failed when it couldn't start or exited, stopped when XenoNet
// stopped it. `spawn` and `bin` stand in for the real ones in tests.
export class AppServerClient extends Block {
  #stderr = new native.CodexStderr();

  constructor({ spawn: spawnFn = spawn, bin = findCodexBinary, timeoutMs = native.codexRequestTimeoutMs(), clock } = {}) {
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
    const stdout = new native.CodexStdout();
    proc.stdout.setEncoding("utf8");
    proc.stdout.on("data", (chunk) => {
      for (const m of stdout.take(String(chunk))) this.#onMessage(m);
    });
    proc.stderr.setEncoding("utf8");
    proc.stderr.on("data", (chunk) => {
      this.stderrTail = this.#stderr.push(String(chunk));
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
    proc.on("exit", (code) => fail(new Error(this.#stderr.exited(code))));

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

  #write(text) {
    this.proc?.stdin.write(text);
  }

  #send(msg) {
    this.#write(JSON.stringify(msg) + "\n");
  }

  // A message Rust read: a request of the app-server's own, which XenoNet
  // declines, or an answer. Notifications are left out; the poller picks
  // up changes.
  #onMessage(m) {
    if (m.decline != null) return this.#write(m.decline);
    if (this.pending.has(m.id)) {
      const { resolve, reject, timer } = this.pending.get(m.id);
      this.clock.clearTimeout(timer);
      this.pending.delete(m.id);
      if (m.error) reject(Object.assign(new Error(m.error.message), { rpc: m.error.rpc }));
      else resolve(m.result);
    }
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

// Codex runs sub-agent threads of two sorts: ones a session spawns, and
// background work (guardian checks, reviews, compaction).
export const subAgentOf = (t) => native.codexSubAgentOf(t);

export const threadToSession = (t, archived) => native.codexThreadToSession(t, !!archived);

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

// Renames a thread through the app-server, as the ChatGPT desktop app does.
export async function renameCodexThread(rawId, name) {
  await client.request("thread/name/set", { threadId: rawId, name });
}

// An app-server's answer to `method`, read leniently: without what's of
// another type, or {} when it isn't an object.
const answered = (method, result) => answers.check({ [method]: result }, { version: client.version })?.[method] ?? {};

// Codex's threads, and Codex desktop's projects, as gatherers: listed
// again on every refresh, timed, and empty after a listing that failed.
// `gather` stands in for the listing, as the demo's does.
export const codexThreadsGatherer = ({ gather = listCodexThreads, clock } = {}) => new Gatherer({ name: "codex-threads", gather, fallback: [], clock });
export const codexProjectsGatherer = ({ gather = async () => listCodexNativeProjects(), clock } = {}) => new Gatherer({ name: "codex-projects", gather, fallback: [], clock });

// Codex desktop "projects" are not in the app-server API yet, so Rust reads
// them from the state DB, read-only. Undocumented: failures here only
// remove the extra grouping. A state database of another number is a
// version the schema wasn't made for, and a column it reads gone is drift.
export function listCodexNativeProjects() {
  for (const n of native.codexOtherStateDbs(String(config.codexHome))) stateDb.pick(n);
  const read = native.codexReadStateDb(path.join(config.codexHome, `state_${STATE_DB}.sqlite`));
  if (!read) return [];
  if (read.missing) {
    for (const m of read.missing) stateDb.missing(m, String(STATE_DB));
    return [];
  }
  try {
    return native.codexNativeProjects(stateDb.check(read.rows, { version: String(STATE_DB) }));
  } catch {
    return [];
  }
}

// The rollout read last, kept while it's paged through, and read on from
// where it ended as it grows, as the tailer reads. A compressed archive is
// read whole, again only if it changes. One read at a time, so two can't
// take the same lines.
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
  r ??= lastRollout = { file, stamp: null, from: null, entries: new native.RolloutEntries() };
  if (zst) {
    if (r.stamp !== stamp) for (const o of await readLines(file, "codex")) r.entries.add(o.raw);
  } else if (r.from?.offset !== st.size) {
    const read = await readNewLines(file, "codex", r.from);
    // Written again from the start: what was read of it before is gone.
    if (read.restarted) r.entries = new native.RolloutEntries();
    for (const o of read.lines) r.entries.add(o.raw);
    r.from = read;
  }
  r.stamp = stamp;
  return r.entries.page(cursor, Number(limit));
}

// The versions of Codex whose app-server said it can't list a thread's
// items: they're asked no more.
const noItems = new Set();

// Newest page first; `cursor` continues further back in time. When the
// app-server can't list the thread's items, `rollout()` finds the
// thread's rollout file, which has them too.
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
    if (native.codexSaysUnsupported(err?.rpc)) noItems.add(client.version);
    // A page further back by the app-server's own cursor has no place in
    // the rollout.
    if (cursor) throw err;
    return fromRollout(err);
  }
  const page = answered("thread/items/list", res);
  return { entries: native.codexNormalizeItems(page.data), cursor: page.nextCursor ?? null };
}
