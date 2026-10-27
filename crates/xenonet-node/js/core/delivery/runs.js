// core/delivery/runs.js in Rust (crates/xenonet-engine/src/delivery/runs.rs):
// headless runs of Claude Code and Codex. How a program is started on each
// platform, whether a run may start now, and which records and logs are
// kept are Rust's; starting the process, its log and watching it are the
// JS runner's, given Rust's decisions.
import { native } from "../../index.js";
import { createRuns as jsRuns, isAlive } from "../../../../../core/delivery/runs.js";

const C = native.runsConstants();
export const CAP = C.cap;
export const TURN_MS = C.turnMs;
export const LOG_MAX = C.logMax;
export const KEEP_MS = C.keepMs;

export function launch(program, args, { platform = process.platform } = {}) {
  return native.runsLaunch(String(program), [...args].map(String), platform === "win32");
}

export function admit(runs, key, { now = Date.now(), cap = CAP, alive = isAlive } = {}) {
  return native.runsAdmit(runs, String(key), Number(now), Math.min(cap, 2 ** 32 - 1), (pid) => !!alive(pid)) ?? null;
}

export const kept = (runs, now = Date.now()) => native.runsKept(runs, Number(now));

export const staleLogs = (files, now = Date.now()) => native.runsStaleLogs(files, Number(now));

export { isAlive };

export const createRuns = (options = {}) => jsRuns({ ...options, rules: { launch, admit, kept, staleLogs } });
