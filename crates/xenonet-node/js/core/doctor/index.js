// core/doctor/index.js in Rust (crates/xenonet-doctor/): XenoNet's doctor.
// Each check's verdict and words, the hosts' checks, the summary and what
// counts as a new problem are the Rust's; a check is a task that asks
// `ctx`, the machine as the checks see it, which stays here: its files,
// processes and XenoNet's own state, through the modules that know them.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { drive } from "../../task.js";

const { commandDir, commandState, savedPath } = await import(peer("command.js"));
const { onPath } = await import(peer("paths.js"));
const { selfTest } = await import(peer("procs.js"));
const { inspectState } = await import(peer("statefile.js"));
const { readDriftFile, DRIFT_FILE } = await import(peer("drift.js"));
const { findTokenFiles } = await import(peer("actors/token-file.js"));
const { keyChecks } = await import(peer("hosts/doctor.js"));
const { appDir } = await import(peer("dirs.js"));
const { LAUNCHER_SOURCE, launcherVersion } = await import(peer("components/install.js"));
const { logDir, readLogs } = await import(peer("log.js"));

const C = native.doctorConstants();
export const STATUSES = C.statuses;

// The machine as the checks see it (see the JS for what each part is).
export function machine(overrides = {}) {
  const home = overrides.home ?? os.homedir();
  const env = overrides.env ?? process.env;
  const platform = overrides.platform ?? process.platform;
  const cacheDir = overrides.paths?.cacheDir;
  return {
    env,
    platform,
    home,
    node: process.versions.node,
    electron: process.versions.electron ?? null,
    exists: (p) => fs.existsSync(p),
    readText: (p) => {
      try {
        return fs.readFileSync(p, "utf8");
      } catch {
        return null;
      }
    },
    readdir: (p) => fs.promises.readdir(p),
    access: (p, mode) => fs.promises.access(p, mode),
    statfs: (p) => fs.promises.statfs(p),
    which: (cmd) => onPath(cmd, { env }),
    run: (file, args, ms = 5000) => run(file, args, ms),
    command: () => commandState({ dir: commandDir({ home, platform, env }), env, platform }),
    savedPath: () => (platform === "win32" ? savedPath({ run: (file, args) => run(file, args, 5000), env }) : Promise.resolve(null)),
    procs: () => selfTest(),
    errors: () => (overrides.paths?.stateDir ? readLogs(logDir(overrides.paths.stateDir), { since: Date.now() - 24 * 3600e3, level: "error" }) : null),
    async launcher() {
      const dir = path.join(env.XENONET_RUNTIME_DIR || path.join(appDir("state", { env, platform, homeDir: home }), "runtime"), "launcher");
      const node = path.join(dir, platform === "win32" ? "node.exe" : "node");
      const entry = path.join(dir, "cli", "xenonet.js");
      if (!fs.existsSync(node) || !fs.existsSync(entry)) return null;
      const r = await run(node, [entry, "--launcher"], 8000);
      let said = null;
      try {
        said = JSON.parse(r.stdout);
      } catch {}
      return { dir, said, version: launcherVersion(fs.readFileSync(entry, "utf8")), ships: launcherVersion(fs.readFileSync(LAUNCHER_SOURCE, "utf8")) };
    },
    stateFile: (dir) => inspectState(dir),
    drift: () => (cacheDir ? (readDriftFile(path.join(cacheDir, DRIFT_FILE)) ?? { file: path.join(cacheDir, DRIFT_FILE), lookedAt: null, records: [] }) : null),
    base: null,
    health: null,
    live: null,
    connect: null,
    ...overrides,
  };
}

function run(file, args, ms) {
  return new Promise((resolve) =>
    execFile(file, args, { timeout: ms, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) =>
      resolve({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout), stderr: String(stderr), timedOut: !!err?.killed }),
    ),
  );
}

// What the checks read of ctx as data, for the Rust: of the live state
// only its Claude plan reading, and of the health its sources and parts.
function factsOf(ctx) {
  const health = ctx.health ? { sources: ctx.health.sources, blocks: ctx.health.blocks } : null;
  const live = ctx.live ? { limits: { claude: ctx.live.limits?.claude ?? null } } : null;
  return {
    self: ctx.self ?? null,
    base: ctx.base ?? null,
    baseName: ctx.baseName ?? null,
    health,
    live,
    connect: ctx.connect ?? null,
    paths: ctx.paths ?? null,
    home: ctx.home ?? null,
    platform: ctx.platform ?? null,
    electron: ctx.electron ?? null,
    node: ctx.node ?? null,
    codexBin: ctx.codexBin ?? null,
    byHand: ctx.byHand ?? null,
    stateLoad: ctx.stateLoad ?? null,
    projectsDirSet: !!ctx.env?.XENONET_PROJECTS_DIR,
    grokKeySet: !!ctx.env?.XAI_API_KEY,
    actors: ctx.actors === null ? "off" : ctx.actors ? "on" : "none",
  };
}

// What a check asks of ctx: a call (null for one ctx doesn't have, as
// `?.()` reads it), the token files' look, the time and a local time.
const performFor = (ctx) => async (q) => {
  switch (q.op) {
    case "now":
      return Date.now();
    case "call": {
      const [obj, name] = q.fn.startsWith("actors.") ? [ctx.actors, q.fn.slice("actors.".length)] : [ctx, q.fn];
      const fn = obj?.[name];
      if (typeof fn !== "function") return { missing: true };
      const args = q.fn === "access" ? [q.args[0], fs.constants[q.args[1]]] : q.args;
      return { value: (await fn.apply(obj, args)) ?? null };
    }
    case "findTokenFiles":
      return findTokenFiles(q.repos, { run: ctx.run });
    case "localTime":
      return new Date(q.ms).toLocaleString("en-US", { weekday: "short", hour: "numeric", minute: "2-digit" });
    default:
      throw new Error(`no such request: ${q.op}`);
  }
};

const OWN = Symbol("rust check");
const check = (id, facts, ctx) => drive(native.doctorCheck(id, facts), performFor(ctx));

export const CHECKS = C.checks.map(([id, group, title]) => ({ id, group, title, run: (ctx) => check(id, factsOf(ctx), ctx), [OWN]: (facts, ctx) => check(id, facts, ctx) }));

export const hostCheck = (h, now = Date.now()) => native.doctorHostCheck(h, Number(now));

export const hostChecks = (hosts) => (hosts ?? []).map((h) => ({ id: `hosts.${h.id}`, group: "Hosts", title: `${h.id} is connected`, run: async () => hostCheck(h) }));

export async function runDoctor(ctx = machine(), { only = null, timeoutMs = 8000 } = {}) {
  const started = Date.now();
  const hosts = ctx.hosts?.() ?? null;
  const all = [...CHECKS, ...hostChecks(hosts), ...keyChecks(hosts)];
  const checks = only ? all.filter((c) => native.doctorKept(String(c.id), only.map(String))) : all;
  const facts = factsOf(ctx);
  const results = await Promise.all(
    checks.map(async (c) => {
      const t0 = Date.now();
      let r, timer;
      try {
        r = await Promise.race([c[OWN] ? c[OWN](facts, ctx) : c.run(ctx), new Promise((resolve) => (timer = setTimeout(() => resolve({ status: "warn", detail: `didn't finish in ${timeoutMs / 1000} seconds` }), timeoutMs)))]);
      } catch (err) {
        r = { status: "warn", detail: `the check failed: ${err.message}` };
      } finally {
        clearTimeout(timer);
      }
      return { id: c.id, group: c.group, title: c.title, ...r, ms: Date.now() - t0 };
    }),
  );
  return { at: started, ms: Date.now() - started, results, summary: summarize(results) };
}

export const summarize = (results) => native.doctorSummarize(results);

export const newProblems = (prev, next) => native.doctorNewProblems(prev ?? null, next);
