// Made-up components for the tests of installing and updating them
// (service-runtime.test.js): packages whose xenonet binary is a shell
// script that answers the startup check, and bundles of them, for the real
// installer (xenonet components) to install.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import * as tar from "tar";

// A package's SHA-256, as a release manifest lists it.
export const digest = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

export const EXE = process.platform === "win32" ? "xenonet.exe" : "xenonet";
export const BIN = path.join("bin", EXE);
export const posixOnly = { skip: process.platform === "win32" ? "a shell script stands in for the binary" : false };

// A xenonet binary that passes the startup check as `version`, or fails it:
// `fail` "runtime" doesn't run, "command" reports another version.
export const fakeBin = (version, fail = null) => `#!/bin/sh
${fail === "runtime" ? "echo 'dyld: Library not loaded' >&2; exit 1" : `printf '{"version":"${fail === "command" ? "0.0.1" : version}"}'`}
`;

// A component package in `out`: { file, item } as a release manifest
// lists it.
export async function pack(out, component, version, { fail = null, platform = process.platform, arch = process.arch, bin = null } = {}) {
  const app = fs.mkdtempSync(path.join(out, `${component}-${version}-`));
  const meta = { format: 1, component, version, platform, arch, dataSchema: 1, api: { min: 1, max: 1 } };
  fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
  // The xenonet binary: one that answers the startup check, or `bin`, as
  // made-up text.
  fs.mkdirSync(path.join(app, "bin"));
  fs.writeFileSync(path.join(app, BIN), bin ?? fakeBin(version, fail), { mode: 0o755 });
  const name = `XenoNet-${component}-${version}-${platform}-${arch}.tar.gz`;
  const file = path.join(out, name);
  await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app));
  const bytes = fs.readFileSync(file);
  return { file, item: { ...meta, file: name, size: bytes.length, sha256: digest(bytes) } };
}

// A bundle folder as the installer ships it: the packages and their
// components.json.
export async function bundle(dir, name, packages) {
  const out = path.join(dir, name);
  fs.mkdirSync(out);
  const items = [];
  for (const [component, version, options] of packages) items.push((await pack(out, component, version, options)).item);
  fs.writeFileSync(path.join(out, "components.json"), JSON.stringify({ format: 1, packages: items }));
  return out;
}
