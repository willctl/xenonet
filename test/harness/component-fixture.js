// Repackage an installed runtime with a new version to exercise real process
// replacement without depending on a future public release.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import * as tar from "tar";
import { currentComponent, digest } from "../../core/components/packages.js";

export async function nextComponent(component, env) {
  const previous = currentComponent(component, env);
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
