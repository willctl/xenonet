// core/components/packages.js in Rust (crates/xenonet-components/src/packages.rs,
// tar.rs): a component's package, checked against its release's hash and
// size before it's unpacked, unpacked only when it holds nothing but files
// and folders inside it, and never replacing a published version.
import { native } from "../../index.js";
import { settle } from "../../settle.js";
import { drive } from "../../task.js";
import { envOf, machine } from "../../machine.js";
import { lockOps } from "../../lifecycle.js";

export const COMPONENTS = native.compConstants().components;

const bytes = (data) => (Buffer.isBuffer(data) ? data : ArrayBuffer.isView(data) ? Buffer.from(data.buffer, data.byteOffset, data.byteLength) : Buffer.from(String(data)));
export const digest = (data) => native.compDigest(bytes(data));

export const newer = (a, b) => settle(native.compNewer(a, b));

export const packageInfo = (value, { platform = process.platform, arch = process.arch } = {}) => settle(native.compPackageInfo(value, String(platform), String(arch)));

export const componentsDir = (env = process.env) => native.compComponentsDir(envOf(env), machine());

export const currentComponent = (component, env = process.env) => settle(native.compCurrent(String(component), envOf(env), machine()));

export const selectComponent = (component, value, env = process.env) => settle(native.compSelect(String(component), value, envOf(env), machine()));

export function stagePackage(file, expected, { env = process.env, platform = process.platform, arch = process.arch } = {}) {
  return drive(native.compStage(String(file), expected, envOf(env), { ...machine(), platform, arch }), async (q) => {
    throw new Error(`stagePackage asked for ${q.op}`);
  });
}

// One updater at a time, shared by Desktop, the CLI and bundle setup.
export async function withInstallLock(env, action) {
  const file = await drive(native.compLock(envOf(env), machine()), lockOps);
  try {
    return await action();
  } finally {
    settle(native.compUnlock(file));
  }
}
