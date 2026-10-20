// core/delivery/resume.js in Rust (crates/xenonet-engine/src/delivery/resume.rs):
// a headless resume of a Claude Code or Codex session whose process has
// gone, for one turn. What it runs, with what environment, and whether it
// may are Rust's; starting the process and watching it are the twin's.
import { spawn as childSpawn } from "node:child_process";
import fs from "node:fs";
import { native } from "../../index.js";

export const resumeEnv = (env = process.env) => Object.fromEntries(native.resumeEnv(Object.entries(env)));

export const resumeCommand = (agent, { settings = null } = {}) => native.resumeCommand(agent, settings) ?? null;

export function createResume({ spawn = childSpawn, env = process.env, settings = null, timeoutMs = 30 * 60e3, exists = fs.existsSync, platform = process.platform } = {}) {
  const running = new Map(); // agent → its resumed turn's process

  return {
    name: "resume",
    can: (agent) => !!resumeCommand(agent),
    running: (agent) => running.has(agent),
    async deliver({ agent, cwd, text }) {
      const plan = native.resumePlan({ running: (a) => running.has(a), exists: (p) => exists(p) }, agent, typeof settings === "function" ? settings() : settings, cwd, platform === "win32");
      if (plan.why) return { ok: false, why: plan.why };
      const [command, args] = plan.cmd;
      return new Promise((resolve) => {
        let child;
        try {
          child = spawn(command, args, { cwd, env: resumeEnv(env), stdio: ["pipe", "ignore", "ignore"], windowsHide: true, shell: platform === "win32" });
        } catch (err) {
          return resolve({ ok: false, why: err.message });
        }
        running.set(agent, child);
        const timer = setTimeout(() => child.kill(), timeoutMs);
        timer.unref?.();
        const done = () => (clearTimeout(timer), running.delete(agent));
        child.on("exit", done);
        child.on("error", (err) => (done(), resolve({ ok: false, why: err.code === "ENOENT" ? `${command} isn't installed here` : err.message })));
        child.on("spawn", () => {
          child.stdin.on("error", () => {});
          child.stdin.end(text);
          resolve({ ok: true, where: `pid ${child.pid}` });
        });
      });
    },
  };
}
