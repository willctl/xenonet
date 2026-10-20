// core/assistant/tunnel-client.js in Rust (crates/xenonet-assistant/src/
// tunnel_client.rs): OpenAI's tunnel-client, as XenoNet finds and drives
// it. The files, the program and its version's output are read here.
import fs from "node:fs";
import { execFile } from "node:child_process";
import { native } from "../../index.js";
import { envOf } from "../../machine.js";
import { settle } from "../../settle.js";

const T = native.asstConstants().tunnel;

export const TUNNEL_ID = /^tunnel_[0-9a-f]{16,64}$/;
export const RUNTIME_KEY = /^sk-[A-Za-z0-9_-]{20,400}$/;
export const RELEASES = T.releases;
export const TUNNELS_PAGE = T.tunnelsPage;

const homeOf = (env, home) => (home === undefined ? native.asstHomeOf(envOf(env), null) : String(home));
// A callback the Rust asks, as { ok } or { error }.
const tried = (fn) => (x) => {
  try {
    return { ok: fn(x) };
  } catch {
    return { error: true };
  }
};

export const clientPlaces = ({ env = process.env, platform = process.platform, home } = {}) => native.asstClientPlaces(envOf(env), String(platform), homeOf(env, home));

export const findClient = ({ env = process.env, platform = process.platform, home, exists = (f) => fs.statSync(f).isFile() } = {}) => native.asstFindClient(envOf(env), String(platform), homeOf(env, home), tried((f) => !!exists(f))) ?? null;

export function clientVersion(file, { run = execFile } = {}) {
  return new Promise((resolve) => {
    try {
      run(file, ["--version"], { timeout: 8000, windowsHide: true }, (err, stdout) => resolve(err ? null : (native.asstVersionIn(String(stdout)) ?? null)));
    } catch {
      resolve(null);
    }
  });
}

export const profileDirs = ({ env = process.env, platform = process.platform, home } = {}) => native.asstProfileDirs(envOf(env), String(platform), homeOf(env, home));

export const readProfile = (text) => native.asstReadProfile(String(text));

export const findProfiles = (dirs, { read = (f) => fs.readFileSync(f, "utf8"), list = (d) => fs.readdirSync(d), stat = (f) => fs.statSync(f) } = {}) => native.asstFindProfiles(dirs.map(String), tried(list), tried(read), tried((f) => stat(f).mtimeMs));

export const commandLine = (argv) => native.asstCommandLine(argv.map(String));

export const adapterArgv = ({ bin, grant, port = 47840 }) => settle(native.asstAdapterArgv({ bin, grant, port }));

export const profileYaml = ({ tunnelId, command, label = null }) => settle(native.asstProfileYaml({ tunnelId, command, label }));

export const logLine = (line) => native.asstLogLine(String(line));

export const healthOf = (h) => native.asstHealthOf(h);
