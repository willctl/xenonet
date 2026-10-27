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
import { withPromptFile, HOLD_MS } from "../../../../../core/delivery/resume.js";

export { withPromptFile, HOLD_MS };

export const resumeEnv = (env = process.env) => Object.fromEntries(native.resumeEnv(Object.entries(env)));

export const resumeCommand = (agent, { settings = null } = {}) => native.resumeCommand(agent, settings) ?? null;

export const forkCommand = (agent, { settings = null } = {}) => native.resumeFork(agent, settings) ?? null;

export const withPermissionTool = (app, args, server = "xenonet") => native.resumeWithPermissionTool(app, args, server);

export function turnEnv(app, env, holdMs = HOLD_MS) {
  const out = resumeEnv(env);
  if (app === "claude") out.MCP_TOOL_TIMEOUT = String(Math.round(holdMs + 60e3));
  return out;
}

export function createResume({ runs = null, env = process.env, settings = null, exists = fs.existsSync, server = "xenonet", holdMs = HOLD_MS, stateDir = null, ...options } = {}) {
  const runner = runs ?? createRuns(options);
  const listen = () => (typeof settings === "function" ? settings() : settings);
  const promptDir = stateDir ?? options.dir ?? null;
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
      if (fork) return runner.run({ key: `fork:${agent}`, app, args: withPermissionTool(app, args, server), cwd, env: turnEnv(app, env, holdMs), text });
      const [withFile, input] = withPromptFile(args, text, promptDir ? path.join(promptDir, "runs") : undefined);
      return runner.run({ key: agent, app, args: withPermissionTool(app, withFile, server), cwd, env: turnEnv(app, env, holdMs), text: input });
    },
  };
}
