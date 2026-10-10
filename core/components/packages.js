import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import * as tar from "tar";
import { runtimeDir } from "./runtime.js";
import { negotiate, SERVICE_API } from "./protocol.js";

export const COMPONENTS = ["service", "tools"];
export const digest = (data) => crypto.createHash("sha256").update(data).digest("hex");
export function newer(a, b) {
  if (![a, b].every((v) => /^\d+\.\d+\.\d+$/.test(v))) throw new Error("Invalid component version.");
  const x = a.split(".").map(Number), y = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}
export function packageInfo(value, { platform = process.platform, arch = process.arch } = {}) {
  if (value?.format !== 1 || !COMPONENTS.includes(value.component) || !/^\d+\.\d+\.\d+$/.test(value.version) || value.platform !== platform || value.arch !== arch || value.dataSchema !== 1) throw new Error("This component package is not compatible with this XenoNet installation.");
  negotiate(SERVICE_API, value.api);
  return value;
}
export const componentsDir = (env = process.env) => path.join(runtimeDir(env), "components");
export function currentComponent(component, env = process.env) {
  if (!COMPONENTS.includes(component)) throw new Error("Choose service or tools.");
  try {
    const dir = path.join(componentsDir(env), component);
    const current = JSON.parse(fs.readFileSync(path.join(dir, "current.json"), "utf8"));
    if (!/^[a-f0-9]{64}$/.test(current.sha256) || !/^\d+\.\d+\.\d+$/.test(current.version)) throw new Error("Invalid installed component pointer.");
    return { ...current, root: path.join(dir, `${current.version}-${current.sha256.slice(0, 16)}`) };
  } catch (err) { if (err.code === "ENOENT") return null; throw err; }
}
export function selectComponent(component, value, env = process.env) {
  const dir = path.join(componentsDir(env), component);
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "current.json");
  const tmp = `${file}.${crypto.randomUUID()}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify({ version: value.version, sha256: value.sha256 }), { mode: 0o600 });
  fs.renameSync(tmp, file);
}

export async function stagePackage(file, expected, { env = process.env, platform = process.platform, arch = process.arch } = {}) {
  if (!COMPONENTS.includes(expected.component) || !/^\d+\.\d+\.\d+$/.test(expected.version) || !/^[a-f0-9]{64}$/.test(expected.sha256)) throw new Error("Invalid component release manifest.");
  const bytes = fs.readFileSync(file);
  if (bytes.length > 512 << 20 || bytes.length !== expected.size || digest(bytes) !== expected.sha256) throw new Error("Component download failed its hash or size check.");
  const parent = path.join(componentsDir(env), expected.component);
  fs.mkdirSync(parent, { recursive: true, mode: 0o700 });
  const dest = path.join(parent, `${expected.version}-${expected.sha256.slice(0, 16)}`);
  const existing = currentComponent(expected.component, env);
  if (existing?.version === expected.version && existing.sha256 !== expected.sha256) throw new Error("A published component version cannot be replaced. Release a new version.");
  if (fs.existsSync(dest)) return { ...packageInfo(JSON.parse(fs.readFileSync(path.join(dest, "component.json"), "utf8")), { platform, arch }), root: dest, sha256: expected.sha256 };
  const staging = fs.mkdtempSync(path.join(parent, ".stage-"));
  let size = 0;
  let count = 0;
  let invalid = null;
  const filter = (name, entry) => {
    if (++count > 100000 || !["File", "Directory"].includes(entry.type) || name.includes("\\") || name.includes(":") || name.startsWith("/") || name.split("/").includes("..") || (size += entry.size) > 2 ** 30) { invalid = "Unsafe or oversized component archive."; return false; }
    return true;
  };
  await tar.x({ file, cwd: staging, strict: true, preservePaths: false, filter, chmod: true });
  if (invalid) throw new Error(invalid);
  const manifest = packageInfo(JSON.parse(fs.readFileSync(path.join(staging, "component.json"), "utf8")), { platform, arch });
  if (manifest.component !== expected.component || manifest.version !== expected.version) throw new Error("Component archive differs from its release manifest.");
  for (const entry of [platform === "win32" ? "node.exe" : "node", manifest.component === "service" ? "server/index.js" : "cli/xenonet.js"]) if (!fs.statSync(path.join(staging, entry)).isFile()) throw new Error(`Component is missing ${entry}.`);
  // Windows scanners may briefly hold newly extracted executables open.
  // Retry only the final directory rename; never re-extract or change files
  // that an existing process could be using.
  for (const until = Date.now() + 30000;;) {
    try { fs.renameSync(staging, dest); break; }
    catch (err) {
      if (fs.existsSync(dest)) break;
      if (process.platform !== "win32" || !["EPERM", "EACCES", "EBUSY"].includes(err.code) || Date.now() >= until) throw err;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  return { ...manifest, root: dest, sha256: expected.sha256 };
}

// One updater at a time, shared by Desktop, the CLI and bundle setup.
export async function withInstallLock(env, action) {
  const dir = runtimeDir(env);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const file = path.join(dir, "install.lock");
  let fd;
  try { fd = fs.openSync(file, "wx", 0o600); }
  catch (err) {
    if (err.code !== "EEXIST") throw err;
    const pid = Number(fs.readFileSync(file, "utf8"));
    if (pid > 0) { try { process.kill(pid, 0); throw new Error("Another XenoNet update is in progress."); } catch (e) { if (e.code !== "ESRCH") throw e; } }
    else throw new Error("Another XenoNet update is starting.");
    fs.unlinkSync(file);
    fd = fs.openSync(file, "wx", 0o600);
  }
  fs.writeFileSync(fd, String(process.pid));
  try { return await action(); } finally { fs.closeSync(fd); fs.unlinkSync(file); }
}
