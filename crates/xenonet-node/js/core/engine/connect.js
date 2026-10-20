// core/engine/connect.js in Rust (crates/xenonet-engine/src/connect.rs):
// what XenoNet adds to Claude Code's and Codex's own settings, shown first
// (preview) and taken out again (remove). Which entries are XenoNet's, the
// hooks and servers it adds and the preview are Rust's; the files, the
// CLIs and core/instances.js's table are the twin's. A registry or config
// formats a test gives are JS code, so connect through them is left to the
// JS.
import { execFile } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { onPath } from "../paths.js";
import { xenonetBin } from "../components/runtime.js";
import { INSTANCES } from "../../../../../core/instances.js";
import { appsWith } from "../../../../../core/apps.js";
import * as js from "../../../../../core/engine/connect.js";

const instances = () => Object.entries(INSTANCES).map(([id, v]) => ({ id, name: v.name, port: v.port }));

export const serverNames = (port) => native.connectServerNames(port, instances());

export const mergeHooks = (settings, ours, add = null) => native.connectMerge({ ours: (h) => !!ours(h) }, settings, add);

// The config formats, as the JS has them; the Rust has its own.
export const FORMATS = js.FORMATS;

export function createConnect(options) {
  if (options?.registry || options?.formats) return js.createConnect(options);
  return twin(options);
}

function twin({ port, root, demo = false, home = os.homedir(), env = process.env, platform = process.platform, bin = xenonetBin({ env, root, platform }), run = runFile, consoleMcp = !!(env.XENONET_LAUNCHER_BIN || env.XENONET_LAUNCHER_EXEC) }) {
  const core = new native.ConnectCore({
    port,
    demo,
    bin: String(bin),
    home: String(home),
    claudeConfigDir: env.CLAUDE_CONFIG_DIR ?? null,
    codexHome: env.CODEX_HOME ?? null,
    consoleMcp,
    platform: String(platform),
    instances: instances(),
  });
  // A CLI on PATH (XenoNet's PATH is your login shell's).
  const deps = { fs, run: (file, args) => run(file, args), cli: (name) => onPath(name, { env, platform, canRun: (f) => fs.accessSync(f) }) };
  const go = (machine) => drive(machine, calls(deps));
  return {
    preview: () => go(core.preview()),
    apply: (apps = appsWith("connect").map((a) => a.id)) => go(core.apply(apps)),
    remove: () => go(core.remove()),
    hookConnected: (event) => go(core.hookConnected(event)),
  };
}

function runFile(file, args) {
  return new Promise((resolve, reject) =>
    execFile(file, args, { timeout: 20000, windowsHide: true, env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined } }, (err, stdout, stderr) => (err ? reject(new Error(stderr.trim() || err.message)) : resolve(stdout))),
  );
}
