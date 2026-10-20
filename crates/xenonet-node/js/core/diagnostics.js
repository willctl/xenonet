// core/diagnostics.js in Rust (crates/xenonet-app/src/diagnostics.rs):
// what XenoNet can tell someone helping you, safe to share, scrubbed as
// it's made and refused if anything private is left; and the folder and
// command for asking an agent on this computer about it.
import os from "node:os";
import { native } from "../index.js";
import { settle } from "../settle.js";

export const SCHEMA = native.appDiagnosticsSchema();

export function makeBundle({ xenonet = {}, doctor = null, errors = [], connect = null, launcher = null, env = process.env, home = os.homedir(), now = Date.now(), platform = process.platform, arch = process.arch } = {}) {
  return settle(native.appMakeBundle({ xenonet, doctor, errors, connect, launcher, env: { ...env }, home: String(home), now: Number(now), platform: String(platform), arch: String(arch), release: os.release(), node: process.versions.node, electron: process.versions.electron ?? null }));
}

export const bundleMarkdown = (b) => native.appBundleMarkdown(b);

export const agentBrief = ({ sources = [] } = {}) => native.appAgentBrief(sources.map(String));

export const writeLook = (stateDir, bundle, { sources = [], now = Date.now() } = {}) => settle(native.appWriteLook(String(stateDir), bundle, sources.map(String), Number(now)));

export const askCommand = ({ app = "codex", model = null, dir, sources = [] }) => native.appAskCommand(String(app), model ? String(model) : null, dir, sources.map(String));
