// core/engine/connect.js in Rust (crates/xenonet-engine/src/connect.rs):
// what XenoNet adds to Claude Code's and Codex's own settings, shown first
// (preview) and taken out again (remove). Which entries are XenoNet's, the
// hooks and servers it adds and the preview are Rust's; the files, the
// CLIs and core/instances.js's table are the twin's.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { onPath } from "../paths.js";
import { INSTANCES } from "../../../../../core/instances.js";

const instances = () => Object.entries(INSTANCES).map(([id, v]) => ({ id, name: v.name, port: v.port }));

export const serverNames = (port) => native.connectServerNames(port, instances());

export const mergeHooks = (settings, ours, add = null) => native.connectMerge({ ours: (h) => !!ours(h) }, settings, add);

export function createConnect({ port, root, demo = false, exec = process.execPath, runAsNode = false, home = os.homedir(), env = process.env, platform = process.platform, run = runFile }) {
  const core = new native.ConnectCore({
    port,
    root: String(root),
    demo,
    exec: String(exec),
    runAsNode: !!runAsNode,
    home: String(home),
    claudeConfigDir: env.CLAUDE_CONFIG_DIR ?? null,
    codexHome: env.CODEX_HOME ?? null,
    consoleMcp: !!env.XENONET_LAUNCHER_EXEC,
    platform: String(platform),
    instances: instances(),
  });
  // A CLI on PATH (XenoNet's PATH is your login shell's).
  const deps = { fs, run: (file, args) => run(file, args), cli: (name) => onPath(name, { env, platform, canRun: (f) => fs.accessSync(f) }) };
  const go = (machine) => drive(machine, calls(deps));
  return {
    preview: () => go(core.preview()),
    apply: (apps = ["claude", "codex"]) => go(core.apply(apps)),
    remove: () => go(core.remove()),
  };
}

function runFile(file, args) {
  return new Promise((resolve, reject) =>
    execFile(file, args, { timeout: 20000, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) => (err ? reject(new Error(stderr.trim() || err.message)) : resolve(stdout))),
  );
}
