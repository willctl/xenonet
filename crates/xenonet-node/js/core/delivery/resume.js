// core/delivery/resume.js in Rust (crates/xenonet-engine/src/delivery/resume.rs):
// a headless resume of a Claude Code or Codex session whose process has
// gone, for one turn. What it runs, with what environment, and whether it
// may are Rust's; the run itself is the shared runner's (runs.js).
import fs from "node:fs";
import { native } from "../../index.js";
import { createRuns } from "./runs.js";

export const resumeEnv = (env = process.env) => Object.fromEntries(native.resumeEnv(Object.entries(env)));

export const resumeCommand = (agent, { settings = null } = {}) => native.resumeCommand(agent, settings) ?? null;

export function createResume({ runs = null, env = process.env, settings = null, exists = fs.existsSync, ...options } = {}) {
  const runner = runs ?? createRuns(options);
  return {
    name: "resume",
    can: (agent) => !!resumeCommand(agent),
    running: (agent) => runner.running(agent),
    async deliver({ agent, cwd, text }) {
      const plan = native.resumePlan({ running: (a) => runner.running(a), exists: (p) => exists(p) }, agent, typeof settings === "function" ? settings() : settings, cwd);
      if (plan.why) return { ok: false, why: plan.why };
      const [app, args] = plan.cmd;
      return runner.run({ key: agent, app, args, cwd, env: resumeEnv(env), text });
    },
  };
}
