// A made-up home for a provider's CLI to run in, so nothing it does reads
// or writes the real ~/.claude, ~/.codex or the desktop apps' folders on the
// machine the check runs on: every place the CLIs look for a home, a config
// folder or an app data folder points inside a temporary folder.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// The variables a child keeps from this process: enough to find programs
// and run them, and nothing that says who's signed in where.
const KEEP = ["PATH", "Path", "PATHEXT", "SystemRoot", "SYSTEMROOT", "ComSpec", "COMSPEC", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG", "LC_ALL", "TERM", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE"];

export function makeSandbox(parent = os.tmpdir(), name = "xenonet-provider-drift-") {
  fs.mkdirSync(parent, { recursive: true });
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(parent, name)));
  const home = path.join(root, "home");
  const dirs = {
    root,
    home,
    claudeConfig: path.join(home, ".claude"),
    codexHome: path.join(home, ".codex"),
    project: path.join(root, "project"),
    appData: path.join(home, "AppData", "Roaming"),
    localAppData: path.join(home, "AppData", "Local"),
    xdgConfig: path.join(home, ".config"),
    xdgData: path.join(home, ".local", "share"),
    xdgState: path.join(home, ".local", "state"),
    xdgCache: path.join(home, ".cache"),
  };
  for (const d of Object.values(dirs)) fs.mkdirSync(d, { recursive: true });
  const env = {
    ...Object.fromEntries(KEEP.filter((k) => process.env[k] != null).map((k) => [k, process.env[k]])),
    HOME: home,
    USERPROFILE: home,
    APPDATA: dirs.appData,
    LOCALAPPDATA: dirs.localAppData,
    XDG_CONFIG_HOME: dirs.xdgConfig,
    XDG_DATA_HOME: dirs.xdgData,
    XDG_STATE_HOME: dirs.xdgState,
    XDG_CACHE_HOME: dirs.xdgCache,
    CLAUDE_CONFIG_DIR: dirs.claudeConfig,
    CODEX_HOME: dirs.codexHome,
    // No telemetry, error reports, update checks or other traffic of the
    // CLIs' own: the check talks to the stand-in API and nothing else.
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    DISABLE_TELEMETRY: "1",
    DISABLE_ERROR_REPORTING: "1",
    DISABLE_AUTOUPDATER: "1",
    CODEX_DISABLE_UPDATE_CHECK: "1",
    NO_COLOR: "1",
    CI: process.env.CI ?? "1",
  };
  return { ...dirs, env, remove: () => fs.rmSync(root, { recursive: true, force: true }) };
}

// Runs a program to its end: { code, stdout, stderr, timedOut }. `input`
// goes to its stdin, which is then closed.
export function run(cmd, args, { env, cwd, input = null, timeoutMs = 120_000 } = {}) {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, { env, cwd, stdio: ["pipe", "pipe", "pipe"], windowsHide: true, shell: false });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    child.stdout.setEncoding("utf8").on("data", (c) => (stdout += c));
    child.stderr.setEncoding("utf8").on("data", (c) => (stderr += c));
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, timeoutMs);
    child.on("error", (err) => {
      clearTimeout(timer);
      resolve({ code: null, stdout, stderr: `${stderr}${err.message}`, timedOut });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve({ code, stdout, stderr, timedOut });
    });
    if (input != null) child.stdin.end(input);
    else child.stdin.end();
  });
}

// The files under `dir` whose names match `re`, at any depth.
export function filesUnder(dir, re) {
  const out = [];
  const walk = (d) => {
    let entries;
    try {
      entries = fs.readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) walk(full);
      else if (re.test(e.name)) out.push(full);
    }
  };
  walk(dir);
  return out.sort();
}
