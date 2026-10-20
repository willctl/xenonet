// Headless resume: a Claude Code or Codex session whose process has gone
// (its terminal closed, its headless run finished) is started again for one
// turn, with the message as its prompt:
//
//   claude -p --resume <id> [--settings <listen file>]
//   codex exec resume <id> -
//
// Both continue the same session, so the turn lands in its transcript and
// the session's next interactive resume shows it. The router uses this only
// for a session you allowed, since a turn spends its plan, and never while
// the session's own process runs, so two processes never write one session.
// The message goes in on stdin, never as an argument, where a leading "-"
// would read as a flag.
import { spawn as childSpawn } from "node:child_process";
import fs from "node:fs";
import { deliveryApps } from "./apps.js";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// The apps whose sessions can be resumed, each with its command (apps.js).
const RESUMED = deliveryApps().filter((a) => a.resume);

// The environment a resumed turn runs with: XenoNet's own, without what
// marks a process as part of another session (Claude Code's and Codex's
// session variables, XenoNet's channel switch and actor tokens), which a
// XenoNet started from inside a session would otherwise pass on. Where
// sessions are kept (CLAUDE_CONFIG_DIR, CODEX_HOME: each app's roots.env)
// stays.
export function resumeEnv(env = process.env) {
  const keep = new Set(RESUMED.map((a) => a.roots?.env).filter(Boolean));
  const marks = [...RESUMED.map((a) => a.marks).filter(Boolean), "XENONET_"];
  return Object.fromEntries(Object.entries(env).filter(([k]) => keep.has(k) || !marks.some((m) => k.startsWith(m))));
}

// The command for one resumed turn: [command, args], or null for an app or
// id it can't resume.
export function resumeCommand(agent, { settings = null } = {}) {
  const [app, id] = String(agent).split(/:(.*)/s);
  if (!UUID.test(id ?? "")) return null;
  return RESUMED.find((a) => a.id === app)?.resume(id, { settings }) ?? null;
}

export function createResume({ spawn = childSpawn, env = process.env, settings = null, timeoutMs = 30 * 60e3, exists = fs.existsSync, platform = process.platform } = {}) {
  const running = new Map(); // agent → its resumed turn's process

  return {
    name: "resume",
    can: (agent) => !!resumeCommand(agent),
    running: (agent) => running.has(agent),
    async deliver({ agent, cwd, text }) {
      const cmd = resumeCommand(agent, { settings: typeof settings === "function" ? settings() : settings });
      if (!cmd) return { ok: false, why: "only Claude Code and Codex sessions can be resumed" };
      if (running.has(agent)) return { ok: false, why: "a resumed turn is under way" };
      if (!cwd || !exists(cwd)) return { ok: false, why: "its folder isn't there" };
      return new Promise((resolve) => {
        let child;
        // Windows finds claude.cmd only through a shell, which joins the
        // arguments as they are: a path with a space is quoted.
        const args = platform === "win32" ? cmd[1].map((a) => (/\s/.test(a) ? `"${a}"` : a)) : cmd[1];
        try {
          child = spawn(cmd[0], args, { cwd, env: resumeEnv(env), stdio: ["pipe", "ignore", "ignore"], windowsHide: true, shell: platform === "win32" });
        } catch (err) {
          return resolve({ ok: false, why: err.message });
        }
        running.set(agent, child);
        const timer = setTimeout(() => child.kill(), timeoutMs);
        timer.unref?.();
        const done = () => (clearTimeout(timer), running.delete(agent));
        child.on("exit", done);
        child.on("error", (err) => (done(), resolve({ ok: false, why: err.code === "ENOENT" ? `${cmd[0]} isn't installed here` : err.message })));
        child.on("spawn", () => {
          child.stdin.on("error", () => {});
          child.stdin.end(text);
          resolve({ ok: true, where: `pid ${child.pid}` });
        });
      });
    },
  };
}
