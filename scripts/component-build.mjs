// Self-contained Service and Tools archives, never Desktop's Electron. Each
// is the Rust xenonet binary in bin/ (scripts/rust-release.mjs), the service
// Desktop runs (electron/service-runtime.js) and the command, with no Node
// runtime and none of XenoNet's JS beside it: only the schemas the binary
// reads the apps' data against, and for the service the built page it
// serves. A package can't be built without the binary. On a Mac, the tray
// is a third, XenoNet Tray.app, one package for both CPUs
// (buildTrayPackage).
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
  fs.writeFileSync(path.join(output, `components-${platform}-${arch}.json`), JSON.stringify({ format: 1, packages }, null, 2));
  // The Mac's tray, one universal package for both CPUs, in the bundle
  // beside them and in a manifest of its own.
  if (platform === "darwin") {
    const tray = await buildTrayPackage({ output, published, verifyComponentVersions });
    if (tray) {
      fs.copyFileSync(path.join(output, tray.file), path.join(bundle, tray.file));
      packages.push(tray);
    }
  }
  const manifest = { format: 1, packages };
  fs.writeFileSync(path.join(bundle, "components.json"), JSON.stringify(manifest, null, 2));
  return manifest;
}

// Every file and folder under `dir`, sorted, so an archive lists them in
// the same order wherever it's made.
function walk(dir, rel = "") {
  return fs
    .readdirSync(path.join(dir, rel))
    .sort()
    .flatMap((name) => {
      const p = rel ? `${rel}/${name}` : name;
      return fs.statSync(path.join(dir, p)).isDirectory() ? [p, ...walk(dir, p)] : [p];
    });
}

// XenoNet Tray.app's archive (scripts/tray-build.mjs): the bundle, its
// component.json and the license, with fixed modes and times, so a build
// of the same source and toolchain has the same bytes. None where this Mac
// can't build Swift; the release check then finds it missing.
export async function buildTrayPackage({ output = path.join(root, "release"), published = [], verifyComponentVersions = () => {} } = {}) {
  const { buildTray, canBuildTray, TRAY_APP } = await import("./tray-build.mjs");
  if (!canBuildTray()) {
    console.warn("  • skipping the tray: it builds only on a Mac with Swift (xcrun --find swift)");
    return null;
  }
  const version = versions.tray;
  fs.mkdirSync(path.join(root, ".xenonet"), { recursive: true });
  const staging = fs.mkdtempSync(path.join(root, ".xenonet", "build-tray-"));
  try {
    buildTray({ version, output: staging });
    const manifest = { format: 1, component: "tray", version, platform: "darwin", arch: "universal", api: SERVICE_API, consoleApi: CONSOLE_API, dataSchema: 1, bundle: TRAY_APP };
    fs.writeFileSync(path.join(staging, "component.json"), JSON.stringify(manifest, null, 2) + "\n");
    fs.copyFileSync(path.join(root, "LICENSE"), path.join(staging, "LICENSE"));
    const entries = walk(staging);
    for (const p of entries) {
      const full = path.join(staging, p);
      const stat = fs.statSync(full);
      fs.chmodSync(full, stat.isDirectory() || stat.mode & 0o111 ? 0o755 : 0o644);
    }
    fs.mkdirSync(output, { recursive: true });
    const file = `XenoNet-tray-${version}-darwin-universal.tar.gz`;
    const full = path.join(output, file);
    await tar.c({ file: full, cwd: staging, gzip: { level: 9 }, portable: true, noDirRecurse: true, mtime: new Date(0), noMtime: false }, entries);
    const item = { ...manifest, file, sha256: hash(full), size: fs.statSync(full).size };
    verifyComponentVersions([item], published);
    fs.writeFileSync(path.join(output, "components-darwin-universal.json"), JSON.stringify({ format: 1, packages: [item] }, null, 2));
    return item;
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  if (!fs.existsSync(path.join(root, "dist", "index.html"))) {
    const build = spawnSync(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build"], { cwd: root, stdio: "inherit", shell: process.platform === "win32" });
    if (build.status !== 0) throw new Error("Could not build the page.");
  }
  fs.mkdirSync(path.join(root, ".xenonet"), { recursive: true });
  await buildComponents({ platform: process.platform, arch: process.arch, bundle: fs.mkdtempSync(path.join(root, ".xenonet", "headless-")) });
}
