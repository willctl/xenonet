// Headless resume: a Claude Code, Codex or Grok Build session whose process has gone
// (its terminal closed, its headless run finished) is started again for one
// turn, with the message as its prompt:
//
//   claude -p --resume <id> [--settings <listen file>]
//   codex exec resume <id> -
//   grok -r <id> --output-format json --prompt-file <file>
//
// Each continues the same session, so the turn lands in its transcript and
// the session's next interactive resume shows it. The router uses this only
// for a session you allowed (or any, under wake: network), since a turn
// spends its plan, and never while the session's own process runs, so two
// processes never write one session: a Claude Code session that one holds
// gets a copy instead (forkCommand).
// The message goes in on stdin, never as an argument, where a leading "-"
// would read as a flag. The run itself is the shared runner's (runs.js):
// it finds the program, logs the turn and caps how many go at once.
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { deliveryApps } from "./apps.js";
import { createRuns, TURN_MS } from "./runs.js";

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

// The command for a copy of a Claude Code session that a process holds:
// its whole context under a new id, which the stream's first event names.
//
//   claude -p --resume <id> --fork-session --output-format stream-json --verbose [--settings <listen file>]
export function forkCommand(agent, { settings = null } = {}) {
  const id = String(agent).startsWith("claude:") ? String(agent).slice("claude:".length) : "";
  if (!UUID.test(id)) return null;
  return ["claude", ["-p", "--resume", id, "--fork-session", "--output-format", "stream-json", "--verbose", ...(settings ? ["--settings", settings] : [])]];
}

// A command that ends with --prompt-file (Grok Build's) reads the prompt
// from a file only you can read, written here and taken away when the turn
// has had its time: [args, text for standard input].
export function withPromptFile(args, text, dir = os.tmpdir()) {
  if (args.at(-1) !== "--prompt-file") return [args, text];
  const file = path.join(dir, `xenonet-prompt-${crypto.randomUUID()}.txt`);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, text, { mode: 0o600 });
  setTimeout(() => fs.rmSync(file, { force: true }), TURN_MS).unref?.();
  return [[...args, file], ""];
}

// `runs` is the shared runner (runs.js); without one, a runner of its own
// with the rest of the options (spawn, find, dir, timeoutMs, platform).
// deliver({ fork }) runs a copy of a Claude Code session (forkCommand),
// while the session itself runs too: that's what it's for.
export function createResume({ runs = null, env = process.env, settings = null, exists = fs.existsSync, ...options } = {}) {
  const runner = runs ?? createRuns(options);
  const listen = () => (typeof settings === "function" ? settings() : settings);
  return {
    name: "resume",
    fork: true,
    can: (agent) => !!resumeCommand(agent),
    running: (agent) => runner.running(agent),
    async deliver({ agent, cwd, text, fork = false }) {
      if (fork) {
        const cmd = forkCommand(agent, { settings: listen() });
        if (!cmd) return { ok: false, why: "only Claude Code sessions can be forked" };
        if (!cwd || !exists(cwd)) return { ok: false, why: "its folder isn't there" };
        return runner.run({ key: `fork:${agent}`, app: cmd[0], args: cmd[1], cwd, env: resumeEnv(env), text });
      }
      const cmd = resumeCommand(agent, { settings: listen() });
      if (!cmd) return { ok: false, why: "only Claude Code, Codex and Grok Build sessions can be resumed" };
      if (runner.running(agent)) return { ok: false, why: "a resumed turn is under way" };
      if (!cwd || !exists(cwd)) return { ok: false, why: "its folder isn't there" };
      const [args, input] = withPromptFile(cmd[1], text, options.dir ? path.join(options.dir, "runs") : undefined);
      return runner.run({ key: agent, app: cmd[0], args, cwd, env: resumeEnv(env), text: input });
    },
  };
}
