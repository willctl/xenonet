// core/delivery/resume.js in Rust (crates/xenonet-engine/src/delivery/resume.rs):
// a headless resume of a Claude Code, Codex or Grok Build session whose
// process has gone, for one turn, or a copy of a Claude Code session one
// holds. What it runs, with what environment, and whether it may are
// Rust's; the run itself is the shared runner's (runs.js), and Grok Build's
// prompt file the JS's.
import fs from "node:fs";
import path from "node:path";
import { native } from "../../index.js";
import { createRuns } from "./runs.js";
import { withPromptFile } from "../../../../../core/delivery/resume.js";

export { withPromptFile };

export const resumeEnv = (env = process.env) => Object.fromEntries(native.resumeEnv(Object.entries(env)));

export const resumeCommand = (agent, { settings = null } = {}) => native.resumeCommand(agent, settings) ?? null;

export const forkCommand = (agent, { settings = null } = {}) => native.resumeFork(agent, settings) ?? null;

export function createResume({ runs = null, env = process.env, settings = null, exists = fs.existsSync, ...options } = {}) {
  const runner = runs ?? createRuns(options);
  const listen = () => (typeof settings === "function" ? settings() : settings);
  return {
    name: "resume",
    fork: true,
    can: (agent) => !!resumeCommand(agent),
    running: (agent) => runner.running(agent),
    async deliver({ agent, cwd, text, fork = false }) {
      const deps = { running: (a) => runner.running(a), exists: (p) => exists(p) };
      const plan = fork ? native.resumePlanFork(deps, agent, listen(), cwd) : native.resumePlan(deps, agent, listen(), cwd);
      if (plan.why) return { ok: false, why: plan.why };
      const [app, args] = plan.cmd;
      if (fork) return runner.run({ key: `fork:${agent}`, app, args, cwd, env: resumeEnv(env), text });
      const [withFile, input] = withPromptFile(args, text, options.dir ? path.join(options.dir, "runs") : undefined);
      return runner.run({ key: agent, app, args: withFile, cwd, env: resumeEnv(env), text: input });
    },
  };
}
