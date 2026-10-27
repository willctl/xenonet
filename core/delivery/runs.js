// Headless runs: one turn of Claude Code or Codex that XenoNet starts
// itself, for a session it resumes (resume.js) and, later, for one it
// starts. The program is found by findCli (dirs.js): on PATH, with PATHEXT
// on Windows; Claude Code also where Claude desktop keeps it, then at
// XENONET_CLAUDE_BIN. A Windows batch file (an npm install's claude.cmd)
// runs through cmd.exe, by its full path; a turn stopped there is stopped
// with everything it started.
//
// At most CAP runs go at once on a machine, one per session, and a turn
// runs TURN_MS at most. Each run's output goes to runs/<id>.log in
// XenoNet's state folder (LOG_MAX at most, kept a week), and its record
// (when it started, its exit, how long it took) to delivery-runs.json
// beside it, so a restart keeps counting the runs still under way.
import { spawn as childSpawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { findCli } from "../dirs.js";

export const CAP = 4;
export const TURN_MS = 30 * 60e3;
export const LOG_MAX = 1 << 20;
export const KEEP_MS = 7 * 24 * 3600e3;
const KEEP_RUNS = 200;

// cmd.exe by its full path, never by its name, which Windows looks for in
// the folder a run starts in (the session's project) first: %ComSpec%,
// else %SystemRoot%\System32\cmd.exe.
export const cmdExe = (comspec, systemRoot) => comspec || `${systemRoot || "C:\\Windows"}\\System32\\cmd.exe`;

// How to start `program` with `args` here: { program, args, verbatim }, or
// { why } it can't be done safely. Elsewhere each argument goes as it is,
// and the platform quotes it once. On Windows a batch file (an npm
// install's claude.cmd) runs only through cmd.exe (by its full path, from
// `env`), which takes one command line as written: each part is quoted
// once, and a part cmd.exe would still read inside quotes is refused: a
// quote, a % or a ! (which it expands), a line break, or a \ at its end
// (which would escape the closing quote).
export function launch(program, args, { platform = process.platform, env = process.env } = {}) {
  if (platform !== "win32" || !/\.(cmd|bat)$/i.test(program)) return { program, args: [...args], verbatim: false };
  const parts = [program, ...args].map(String);
  const bad = parts.find((p) => /["%!\r\n]/.test(p) || p.endsWith("\\"));
  if (bad !== undefined) return { why: `XenoNet can't pass ${bad} to a batch file safely` };
  const cmd = cmdExe(env.ComSpec ?? env.COMSPEC, env.SystemRoot ?? env.SYSTEMROOT);
  return { program: cmd, args: ["/d", "/s", "/c", `"${parts.map((p) => `"${p}"`).join(" ")}"`], verbatim: true };
}

// Whether a run counts as under way: not ended, within its turn, and its
// process alive (one a XenoNet before a restart started is watched by no
// one, but still runs).
const live = (r, now, alive) => r.end == null && now - r.at < TURN_MS && r.pid != null && alive(r.pid);

// Whether a run for `key` may start now: null, or why not. `also` are the
// keys of runs a runner has under way, whether or not they have a record
// yet (one just admitted, still starting), each counted once.
export function admit(runs, key, { now = Date.now(), cap = CAP, alive = isAlive, also = [] } = {}) {
  const under = runs.filter((r) => !also.includes(r.key) && live(r, now, alive));
  if (also.includes(key) || under.some((r) => r.key === key)) return "a run is under way for it";
  if (under.length + also.length >= cap) return `XenoNet runs at most ${cap} turns at once on this machine`;
  return null;
}

// The records worth keeping: the last week's, at most 200, the newest last.
export const kept = (runs, now = Date.now()) => runs.filter((r) => now - r.at < KEEP_MS).slice(-KEEP_RUNS);

// The logs to delete, of the files in the runs folder ({ name, mtime }):
// those last written a week ago or more.
export const staleLogs = (files, now = Date.now()) => files.filter((f) => f.name.endsWith(".log") && now - f.mtime >= KEEP_MS).map((f) => f.name);

// Whether a process is running, as kill(pid, 0) says.
export function isAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
}

// A file only you can read, written whole: a temporary file made with mode
// 0600, then a rename.
function writePrivate(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp`;
  fs.rmSync(tmp, { force: true });
  fs.writeFileSync(tmp, text, { mode: 0o600, flag: "wx" });
  fs.renameSync(tmp, file);
}

// The decisions above, which the Rust core's twin gives its own of.
const RULES = { launch, admit, kept, staleLogs };

// The runner. `dir` is XenoNet's state folder, where the logs and records
// go: none in memory, as in tests. `find(app)` names the program for an
// app's CLI, or null.
export function createRuns({ dir = null, spawn = childSpawn, now = () => Date.now(), cap = CAP, timeoutMs = TURN_MS, alive = isAlive, platform = process.platform, find = (app) => findCli(app, { platform }), rules = RULES } = {}) {
  const { launch, admit, kept, staleLogs } = rules;
  const file = dir ? path.join(dir, "delivery-runs.json") : null;
  const logs = dir ? path.join(dir, "runs") : null;
  // key → its run's process, or null while it starts: a run counts from
  // the moment it's admitted, so two started in one tick can't both pass.
  const under = new Map();
  let runs = [];
  if (file) {
    try {
      runs = JSON.parse(fs.readFileSync(file, "utf8")).runs ?? [];
    } catch {}
  }
  const save = () => {
    runs = kept(runs, now());
    if (!file) return;
    try {
      writePrivate(file, JSON.stringify({ runs }));
    } catch {}
  };
  // Logs older than a week go when a run starts.
  const sweep = () => {
    try {
      const files = fs.readdirSync(logs).map((name) => ({ name, mtime: fs.statSync(path.join(logs, name)).mtimeMs }));
      for (const name of staleLogs(files, now())) fs.rmSync(path.join(logs, name), { force: true });
    } catch {}
  };
  // What a run writes, up to LOG_MAX, then a line that says the rest isn't
  // kept.
  const logTo = (where) => {
    let fd = null;
    let written = 0;
    try {
      fs.mkdirSync(logs, { recursive: true });
      fd = fs.openSync(where, "a", 0o600);
    } catch {
      return null;
    }
    return {
      write(chunk) {
        if (written >= LOG_MAX) return;
        const part = chunk.subarray(0, LOG_MAX - written);
        written += part.length;
        try {
          fs.writeSync(fd, part);
          if (written >= LOG_MAX) fs.writeSync(fd, "\n[XenoNet keeps the first 1 MB of a run's output]\n");
        } catch {}
      },
      close() {
        try {
          fs.closeSync(fd);
        } catch {}
      },
    };
  };

  // Stops a run and everything it started, at once, as the binary does: on
  // Windows the whole tree (taskkill /T /F), since cmd.exe running a .cmd
  // would leave its node running if it were killed alone; elsewhere
  // SIGKILL.
  const stop = (child) => {
    if (platform !== "win32") return child.kill("SIGKILL");
    try {
      const taskkill = spawn(`${process.env.SystemRoot || "C:\\Windows"}\\System32\\taskkill.exe`, ["/T", "/F", "/PID", String(child.pid)], { stdio: "ignore", windowsHide: true });
      taskkill.on?.("error", () => child.kill());
    } catch {
      child.kill();
    }
  };

  return {
    // Under way for `key`: one this runner admitted, or one a record says
    // still runs.
    running: (key) => under.has(key) || admit(runs, key, { now: now(), cap: Infinity, alive }) !== null,
    list: () => runs.map((r) => ({ ...r })),
    // One turn: { ok: true, where }, or { ok: false, why }.
    async run({ key, app, args, cwd, env = process.env, text = "" }) {
      const no = admit(runs, key, { now: now(), cap, alive, also: [...under.keys()] });
      if (no) return { ok: false, why: no };
      under.set(key, null);
      const fail = (why) => (under.delete(key), { ok: false, why });
      const program = find(app);
      if (!program) return fail(`${app} isn't installed here`);
      const how = launch(program, args, { platform });
      if (how.why) return fail(how.why);
      const id = `run_${crypto.randomUUID()}`;
      if (logs) sweep();
      const logFile = logs ? path.join(logs, `${id}.log`) : null;
      const log = logFile ? logTo(logFile) : null;
      // No run, so no log of one.
      const unlog = () => {
        log?.close();
        if (logFile) fs.rmSync(logFile, { force: true });
      };
      const rec = { id, key, app, pid: null, at: now(), end: null, exit: null, ms: null };
      return new Promise((resolve) => {
        let child;
        try {
          child = spawn(how.program, how.args, { cwd, env, stdio: ["pipe", log ? "pipe" : "ignore", log ? "pipe" : "ignore"], windowsHide: true, windowsVerbatimArguments: how.verbatim });
        } catch (err) {
          unlog();
          return resolve(fail(err.message));
        }
        under.set(key, child);
        child.stdout?.on("data", (d) => log?.write(d));
        child.stderr?.on("data", (d) => log?.write(d));
        let timedOut = false;
        const timer = setTimeout(() => ((timedOut = true), stop(child)), timeoutMs);
        timer.unref?.();
        let started = false;
        child.on("exit", (code) => {
          clearTimeout(timer);
          under.delete(key);
          log?.close();
          if (!started) return;
          // By its id: what save() keeps may be a copy of the record.
          const mine = runs.find((r) => r.id === id);
          const end = now();
          if (mine) Object.assign(mine, { end, exit: code ?? null, ms: end - mine.at }, timedOut ? { timedOut: true } : {});
          save();
        });
        child.on("error", (err) => {
          clearTimeout(timer);
          under.delete(key);
          if (started) return log?.close();
          unlog();
          resolve({ ok: false, why: err.code === "ENOENT" ? `${app} isn't installed here` : err.message });
        });
        child.on("spawn", () => {
          started = true;
          rec.pid = child.pid ?? null;
          runs.push(rec);
          save();
          child.stdin.on("error", () => {});
          child.stdin.end(text);
          resolve({ ok: true, where: `pid ${child.pid}` });
        });
      });
    },
  };
}
