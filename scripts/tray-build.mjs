// XenoNet Tray.app, the macOS tray (tray/macos): the Swift package built
// for each Mac CPU, joined into one universal executable with lipo, put in
// a bundle with its Info.plist (LSUIElement, so no Dock icon) and signed ad
// hoc, as the app is. Only on a Mac with Swift (xcrun --find swift).
//
// Its archive is a component of its own (components/versions.json's
// `tray`), so its bytes at a version must never change. The build leaves
// out what would differ from one build to the next or name this machine:
// no debug info (-gnone), the checkout's and the home folder's paths
// remapped, and file times and modes set when the archive is made
// (scripts/component-build.mjs). The linker's UUID is a hash of what it
// links, so it stays too; dropping it (-no_uuid) would leave an executable
// recent macOS refuses to start.
//
//   node scripts/tray-build.mjs [output folder]   builds it, says where
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { checkBinary } from "./rust-release.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkg = path.join(root, "tray", "macos");
export const TRAY_APP = "XenoNet Tray.app";
export const TRAY_EXE = "XenoNet Tray";
const TRIPLES = { arm64: "arm64-apple-macosx13.0", x86_64: "x86_64-apple-macosx13.0" };

// Whether this Mac can build it.
export function canBuildTray() {
  if (process.platform !== "darwin") return false;
  try {
    execFileSync("xcrun", ["--find", "swift"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

// The flags that keep this machine's paths, and any debug info, out of the
// executable.
function flags({ home = os.homedir() } = {}) {
  const maps = [
    [root, "/xenonet"],
    [home, "~"],
  ].flatMap(([from, to]) => ["-Xswiftc", "-debug-prefix-map", "-Xswiftc", `${from}=${to}`, "-Xswiftc", "-file-prefix-map", "-Xswiftc", `${from}=${to}`]);
  return ["-c", "release", "-Xswiftc", "-gnone", ...maps];
}

// Builds the bundle into `output` (made fresh), and answers its path.
export function buildTray({ version, output = path.join(root, ".xenonet", "tray") } = {}) {
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) throw new Error("The tray's version is missing: components/versions.json has it as tray.");
  if (!canBuildTray()) throw new Error("The tray builds only on a Mac with Swift (xcrun --find swift).");
  const env = { ...process.env, SOURCE_DATE_EPOCH: "0", ZERO_AR_DATE: "1" };
  const built = Object.entries(TRIPLES).map(([arch, triple]) => {
    const scratch = path.join(pkg, ".build", arch);
    execFileSync("swift", ["build", "--package-path", pkg, "--scratch-path", scratch, "--triple", triple, "--product", "XenoNetTray", ...flags()], { stdio: "inherit", env });
    const bin = execFileSync("swift", ["build", "--package-path", pkg, "--scratch-path", scratch, "--triple", triple, ...flags(), "--show-bin-path"], { encoding: "utf8", env }).trim();
    return path.join(bin, "XenoNetTray");
  });
  fs.rmSync(output, { recursive: true, force: true });
  const app = path.join(output, TRAY_APP);
  const macos = path.join(app, "Contents", "MacOS");
  fs.mkdirSync(macos, { recursive: true });
  const exe = path.join(macos, TRAY_EXE);
  execFileSync("lipo", ["-create", "-output", exe, ...built]);
  fs.chmodSync(exe, 0o755);
  const plist = fs.readFileSync(path.join(pkg, "Info.plist"), "utf8").replaceAll("@VERSION@", version);
  fs.writeFileSync(path.join(app, "Contents", "Info.plist"), plist);
  fs.writeFileSync(path.join(app, "Contents", "PkgInfo"), "APPL????");
  // Ad hoc, as the app is: a certificate carries its owner's name.
  if (process.env.XENONET_SIGN !== "1") execFileSync("codesign", ["--sign", "-", "--force", "--timestamp=none", app], { stdio: "inherit" });
  checkBinary(exe);
  return app;
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const versions = JSON.parse(fs.readFileSync(path.join(root, "components", "versions.json"), "utf8"));
  console.log(buildTray({ version: versions.tray, ...(process.argv[2] ? { output: path.resolve(process.argv[2]) } : {}) }));
}
