// Self-contained service/tools archives, never Desktop's Electron. Each
// carries the Rust xenonet binary in bin/ (scripts/rust-release.mjs): the
// service Desktop runs (electron/service-runtime.js), and the command. A
// package can't be built without it.
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
async function download(url, file) {
  const res = await fetch(url, { signal: AbortSignal.timeout(120000) });
  if (!res.ok) throw new Error(`Download failed: ${url} (${res.status})`);
  fs.writeFileSync(file, Buffer.from(await res.arrayBuffer()));
}
async function runtime(platform, arch) {
  const base = `https://nodejs.org/dist/v${versions.node}`;
  const cache = path.join(root, ".xenonet", "node", `${versions.node}-${platform}-${arch}`);
  fs.mkdirSync(cache, { recursive: true });
  const sums = await (await fetch(`${base}/SHASUMS256.txt`)).text();
  const asset = platform === "win32" ? `win-${arch}/node.exe` : `node-v${versions.node}-${platform}-${arch}.tar.gz`;
  const expected = sums.split("\n").map((s) => s.trim().split(/\s+/)).find(([, name]) => name === asset)?.[0];
  if (!/^[a-f0-9]{64}$/.test(expected)) throw new Error(`Node has no checksum for ${asset}.`);
  const archive = path.join(cache, path.basename(asset));
  if (!fs.existsSync(archive) || hash(archive) !== expected) await download(`${base}/${asset}`, archive);
  if (hash(archive) !== expected) throw new Error("Node runtime checksum mismatch.");
  if (platform === "win32") return archive;
  await tar.x({ file: archive, cwd: cache, strict: true, chmod: true, filter: (name, entry) => entry.type === "File" && name === `node-v${versions.node}-${platform}-${arch}/bin/node` });
  return path.join(cache, `node-v${versions.node}-${platform}-${arch}`, "bin", "node");
}

export async function buildComponents({ app, platform, arch, output = path.join(root, "release"), bundle }) {
  const { publishedComponents, verifyComponentVersions } = await import("./component-release.mjs");
  const published = await publishedComponents();
  const node = await runtime(platform, arch);
  const nodeLicense = path.join(root, ".xenonet", "node", `LICENSE-${versions.node}.txt`);
  if (!fs.existsSync(nodeLicense)) await download(`https://raw.githubusercontent.com/nodejs/node/v${versions.node}/LICENSE`, nodeLicense);
  const xenonet = buildBinary({ platform, arch });
  const packages = [];
  fs.mkdirSync(output, { recursive: true });
  fs.mkdirSync(bundle, { recursive: true });
  for (const component of ["service", "tools"]) {
    const staging = fs.mkdtempSync(path.join(root, ".xenonet", `build-${component}-`));
    const pkg = JSON.parse(fs.readFileSync(path.join(app, "package.json"), "utf8"));
    // Copy only runtime code; no Electron, tests or build tools. The
    // service also gets the built page, so it can serve the console to a
    // browser on its computer without Desktop.
    if (component === "service" && !fs.existsSync(path.join(app, "dist", "index.html"))) throw new Error("The service package needs the built page: run npm run build first.");
    for (const folder of ["core", "server", "cli", "mcp", "schemas", "node_modules", ...(component === "service" ? ["dist"] : [])]) fs.cpSync(path.join(app, folder), path.join(staging, folder), { recursive: true, dereference: true, filter: (file) => !file.split(path.sep).some((name) => [".bin", ".package-lock.json", "electron-updater"].includes(name)) });
    fs.copyFileSync(node, path.join(staging, platform === "win32" ? "node.exe" : "node"));
    if (platform !== "win32") fs.chmodSync(path.join(staging, "node"), 0o755);
    fs.mkdirSync(path.join(staging, "bin"));
    fs.copyFileSync(xenonet, path.join(staging, "bin", binaryName(platform)));
    if (platform !== "win32") fs.chmodSync(path.join(staging, "bin", binaryName(platform)), 0o755);
    const manifest = { format: 1, component, version: versions[component], platform, arch, node: versions.node, api: SERVICE_API, consoleApi: CONSOLE_API, dataSchema: 1 };
    fs.writeFileSync(path.join(staging, "component.json"), JSON.stringify(manifest, null, 2) + "\n");
    fs.writeFileSync(path.join(staging, "package.json"), JSON.stringify({ name: `xenonet-${component}`, productName: "XenoNet", version: versions[component], type: "module", xenonetComponent: component, dependencies: pkg.dependencies }, null, 2) + "\n");
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(staging, "LICENSE"));
    fs.copyFileSync(nodeLicense, path.join(staging, "LICENSE-Node.txt"));
    if (platform === process.platform && arch === process.arch) {
      const { checkRuntime } = await import("../core/components/install.js");
      checkRuntime({ root: staging, version: manifest.version });
      const imported = spawnSync(path.join(staging, platform === "win32" ? "node.exe" : "node"), ["--input-type=module", "-e", "await import('./core/integrations/index.js'); process.stdout.write('ok')"], { cwd: staging, windowsHide: true, encoding: "utf8", timeout: 15000 });
      if (imported.status !== 0 || imported.stdout !== "ok") throw new Error("The packaged integration adapters could not be imported.");
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
  fs.mkdirSync(path.join(root, ".xenonet"), { recursive: true });
  const app = fs.mkdtempSync(path.join(root, ".xenonet", "headless-"));
  if (!fs.existsSync(path.join(root, "dist", "index.html"))) {
    const build = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build"], { cwd: root, stdio: "inherit", shell: process.platform === "win32" });
    if (build.status !== 0) throw new Error("Could not build the page.");
  }
  for (const name of ["core", "server", "cli", "mcp", "schemas", "dist", "vendor", "package.json", "package-lock.json"]) fs.cpSync(path.join(root, name), path.join(app, name), { recursive: true });
  const install = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["ci", "--omit=dev", "--ignore-scripts"], { cwd: app, stdio: "inherit", shell: process.platform === "win32" });
  if (install.status !== 0) throw new Error("Could not install component dependencies.");
  await buildComponents({ app, platform: process.platform, arch: process.arch, bundle: path.join(app, "bundle") });
}
