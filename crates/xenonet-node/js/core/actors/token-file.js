// core/actors/token-file.js in Rust (crates/xenonet-actors/src/token_file.rs):
// where an actor keeps its token. The doctor's look for tokens left in
// repos is still the JS, until the doctor moves.
import { execFileSync } from "node:child_process";
import { native } from "../../index.js";
import { withRun } from "../../run.js";
export { aclOf, userSid, othersIn } from "../private-file.js";
export { findTokenFiles, createTokenScan, TOKEN_SCAN_MS } from "../../../../../core/actors/token-file.js";

const runSync = (file, args) => execFileSync(file, args, { encoding: "utf8", windowsHide: true, timeout: 10000 });
const windows = (platform) => platform === "win32";
const dir = (d) => (d ? String(d) : undefined);

export const HEAVY_FOLDERS = native.heavyFolders();

export const readTokenFile = (file, { platform = process.platform, run = runSync } = {}) => withRun((r) => native.readTokenFile(String(file), windows(platform), r), run);

export function tokenFromEnv(env = process.env, { platform = process.platform, run = runSync } = {}) {
  return withRun((r) => native.tokenFromEnv(env.XENONET_ACTOR_TOKEN_FILE || undefined, env.XENONET_ACTOR_TOKEN || undefined, windows(platform), r), run);
}

export const gitTreeOf = (d) => native.gitTreeOf(String(d)) ?? null;

export const tokenFileRefusal = (file, { force = false, projectsDir = null } = {}) => native.tokenFileRefusal(String(file), !!force, dir(projectsDir)) ?? null;

export function writeTokenFile(file, token, { platform = process.platform, run = runSync, force = false, projectsDir = null } = {}) {
  return withRun((r) => native.writeTokenFile(String(file), String(token), windows(platform), r, !!force, dir(projectsDir)), run);
}
