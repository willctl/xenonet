// Repackage an installed runtime with a new version to exercise real process
// replacement without depending on a future public release.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import * as tar from "tar";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { digest } from "./components.js";

// The installed `component`, as the xenonet binary `bin` says: { version,
// root, sha256 }, or null.
export function currentComponent(bin, component, env) {
  return JSON.parse(execFileSync(bin, ["components", "current", component], { env, encoding: "utf8", windowsHide: true }))?.component ?? null;
}

export async function nextComponent(bin, component, env) {
  const previous = currentComponent(bin, component, env);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-component-release-"));
  const app = path.join(dir, "app");
  fs.cpSync(previous.root, app, { recursive: true });
  const parts = previous.version.split(".").map(Number);
  parts[2]++;
  const version = parts.join(".");
  let manifest;
  for (const name of ["package.json", "component.json"]) {
    const file = path.join(app, name);
    const value = JSON.parse(fs.readFileSync(file, "utf8"));
    value.version = version;
    fs.writeFileSync(file, JSON.stringify(value));
    if (name === "component.json") manifest = value;
  }
  const file = path.join(dir, `${component}.tar.gz`);
  await tar.c({ file, cwd: app, gzip: { level: 1 } }, fs.readdirSync(app).sort());
  const bytes = fs.readFileSync(file);
  return { file, item: { ...manifest, sha256: digest(bytes), size: bytes.length }, previous };
}

// A bundle as an installer carries one, from the xenonet binary `bin`: the
// Service (with the built page) and the Tools, each with the schemas, the
// components.json that lists them, and `bin` beside them to install them.
export async function testBundle(bin, dir) {
  const { SERVICE_API, CONSOLE_API } = await import("../../electron/lib/components/protocol.js");
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
  const versions = JSON.parse(fs.readFileSync(path.join(root, "components", "versions.json"), "utf8"));
  const out = path.join(dir, "bundle");
  fs.mkdirSync(out, { recursive: true });
  const packages = [];
  for (const component of ["service", "tools"]) {
    const app = fs.mkdtempSync(path.join(dir, `${component}-`));
    for (const folder of ["schemas", ...(component === "service" ? ["dist"] : [])]) fs.cpSync(path.join(root, folder), path.join(app, folder), { recursive: true });
    fs.mkdirSync(path.join(app, "bin"));
    fs.copyFileSync(bin, path.join(app, "bin", path.basename(bin)));
    const manifest = { format: 1, component, version: versions[component], platform: process.platform, arch: process.arch, api: SERVICE_API, consoleApi: CONSOLE_API, dataSchema: 1 };
    fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(manifest));
    fs.writeFileSync(path.join(app, "package.json"), JSON.stringify({ name: `xenonet-${component}`, version: manifest.version, xenonetComponent: component }));
    const file = `XenoNet-${component}-${manifest.version}-${process.platform}-${process.arch}.tar.gz`;
    await tar.c({ file: path.join(out, file), cwd: app, gzip: { level: 1 } }, fs.readdirSync(app).sort());
    const bytes = fs.readFileSync(path.join(out, file));
    packages.push({ ...manifest, file, sha256: digest(bytes), size: bytes.length });
  }
  fs.writeFileSync(path.join(out, "components.json"), JSON.stringify({ format: 1, packages }));
  fs.copyFileSync(bin, path.join(out, path.basename(bin)));
  return out;
}
