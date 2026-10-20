// Self-contained Service and Tools archives, never Desktop's Electron. Each
// is the Rust xenonet binary in bin/ (scripts/rust-release.mjs), the service
// Desktop runs (electron/service-runtime.js) and the command, with no Node
// runtime and none of XenoNet's JS beside it: only the schemas the binary
// reads the apps' data against, and for the service the built page it
// serves. A package can't be built without the binary.
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import * as tar from "tar";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";
import { SERVICE_API, CONSOLE_API } from "../core/components/protocol.js";
import { buildBinary, binaryName } from "./rust-release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const versions = JSON.parse(fs.readFileSync(path.join(root, "components/versions.json"), "utf8"));
const hash = (file) => crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");

export async function buildComponents({ app = root, platform, arch, output = path.join(root, "release"), bundle }) {
  const { publishedComponents, verifyComponentVersions } = await import("./component-release.mjs");
  const published = await publishedComponents();
  const xenonet = buildBinary({ platform, arch });
  const packages = [];
  fs.mkdirSync(output, { recursive: true });
  fs.mkdirSync(bundle, { recursive: true });
  fs.mkdirSync(path.join(root, ".xenonet"), { recursive: true });
  for (const component of ["service", "tools"]) {
    const staging = fs.mkdtempSync(path.join(root, ".xenonet", `build-${component}-`));
    // The service also gets the built page, so it can serve the console to a
    // browser on its computer without Desktop.
    if (component === "service" && !fs.existsSync(path.join(app, "dist", "index.html"))) throw new Error("The service package needs the built page: run npm run build first.");
    for (const folder of ["schemas", ...(component === "service" ? ["dist"] : [])]) fs.cpSync(path.join(app, folder), path.join(staging, folder), { recursive: true, dereference: true });
    fs.mkdirSync(path.join(staging, "bin"));
    fs.copyFileSync(xenonet, path.join(staging, "bin", binaryName(platform)));
    if (platform !== "win32") fs.chmodSync(path.join(staging, "bin", binaryName(platform)), 0o755);
    const manifest = { format: 1, component, version: versions[component], platform, arch, api: SERVICE_API, consoleApi: CONSOLE_API, dataSchema: 1 };
    fs.writeFileSync(path.join(staging, "component.json"), JSON.stringify(manifest, null, 2) + "\n");
    // The binary finds its package by this file above bin/: the version it
    // reports is the component's, and xenonetComponent says which it is.
    fs.writeFileSync(path.join(staging, "package.json"), JSON.stringify({ name: `xenonet-${component}`, productName: "XenoNet", version: versions[component], xenonetComponent: component }, null, 2) + "\n");
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(staging, "LICENSE"));
    if (platform === process.platform && arch === process.arch) {
      const { checkRuntime } = await import("../core/components/install.js");
      checkRuntime({ root: staging, version: manifest.version });
    }
    const file = `XenoNet-${component}-${manifest.version}-${platform}-${arch}.tar.gz`;
    const full = path.join(output, file);
    await tar.c({ file: full, cwd: staging, gzip: { level: 9 }, portable: true, mtime: new Date(0), noMtime: false }, fs.readdirSync(staging).sort());
    const item = { ...manifest, file, sha256: hash(full), size: fs.statSync(full).size };
    verifyComponentVersions([item], published);
    packages.push(item);
    fs.copyFileSync(full, path.join(bundle, file));
  }
  const manifest = { format: 1, packages };
  fs.writeFileSync(path.join(bundle, "components.json"), JSON.stringify(manifest, null, 2));
  fs.writeFileSync(path.join(output, `components-${platform}-${arch}.json`), JSON.stringify(manifest, null, 2));
  return manifest;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  if (!fs.existsSync(path.join(root, "dist", "index.html"))) {
    const build = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build"], { cwd: root, stdio: "inherit", shell: process.platform === "win32" });
    if (build.status !== 0) throw new Error("Could not build the page.");
  }
  fs.mkdirSync(path.join(root, ".xenonet"), { recursive: true });
  await buildComponents({ platform: process.platform, arch: process.arch, bundle: fs.mkdtempSync(path.join(root, ".xenonet", "headless-")) });
}
