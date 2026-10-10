// Use Git Bash on Windows rather than the WSL launcher. These tests run
// guest scripts locally and don't need a configured Linux distribution.
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { onPath } from "../../core/paths.js";

const git = onPath("git");
const candidates = process.platform === "win32" ? [process.env.CLAUDE_CODE_GIT_BASH_PATH, git && path.resolve(path.dirname(git), "..", "bin", "bash.exe"), process.env.ProgramFiles && path.join(process.env.ProgramFiles, "Git", "bin", "bash.exe")] : [onPath("bash")];
export const bash = candidates.find((file) => file && fs.existsSync(file) && spawnSync(file, ["--version"], { windowsHide: true, timeout: 5000 }).status === 0);

export function runBash(args, options = {}) {
  const env = { ...process.env, ...options.env };
  if (env.HOME) env.HOME = env.HOME.replace(/\\/g, "/");
  return spawnSync(bash, args, { ...options, env, windowsHide: true });
}
