// npm run dev:install (macOS): puts XenoNet Dev.app in ~/Applications and opens
// it. It runs this checkout the way `npm run app` does, hot-reloading as you
// save (see watchSource in electron/main.js), but from the Dock, Spotlight
// and the menu bar, with its own name and icon in notifications.
//
// It's the Electron that `npm install` fetched, renamed, holding only a
// pointer to this folder, so code changes never need a reinstall. Run this
// again after Electron updates or the checkout moves. It's ad-hoc signed,
// like release builds, so no certificate (which carries a personal name) is
// ever used.
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const NAME = "XenoNet Dev";
const ID = "dev.xenonet.app.dev";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const electron = path.join(root, "node_modules", "electron", "dist", "Electron.app");
const icon = path.join(root, "build", "icon.icns");
const dest = path.join(os.homedir(), "Applications", `${NAME}.app`);
const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));

const fail = (msg) => (console.error(msg), process.exit(1));
const run = (cmd, args) => execFileSync(cmd, args, { stdio: ["ignore", "pipe", "inherit"], encoding: "utf8" });

if (process.platform !== "darwin") fail("dev:install makes a macOS app. Elsewhere, use npm run app.");
if (!fs.existsSync(electron)) fail("Electron isn't installed here yet. Run npm install first.");
if (!fs.existsSync(icon)) {
  console.log("drawing the icons (npm run icon)");
  if (spawnSync("npx", ["electron", ".", "--make-icons", "build"], { cwd: root, stdio: "inherit" }).status !== 0) fail("couldn't draw the icons");
}

// A running XenoNet Dev quits first (SIGTERM is a clean quit): its bundle is
// about to be replaced.
const running = () => spawnSync("pgrep", ["-f", `${NAME}.app/Contents/MacOS/`], { encoding: "utf8" }).stdout.trim().split("\n").filter(Boolean).map(Number);
const pids = running();
if (pids.length) {
  console.log(`quitting the running ${NAME}`);
  for (const pid of pids) process.kill(pid, "SIGTERM");
  const until = Date.now() + 10000;
  while (running().length && Date.now() < until) spawnSync("sleep", ["0.2"]);
  if (running().length) fail(`${NAME} didn't quit. Quit it from its menu bar icon and run this again.`);
}

fs.rmSync(dest, { recursive: true, force: true });
fs.mkdirSync(path.dirname(dest), { recursive: true });
run("ditto", [electron, dest]);

const contents = path.join(dest, "Contents");
const plist = path.join(contents, "Info.plist");
const set = (key, value) => run("plutil", ["-replace", key, "-string", value, plist]);
set("CFBundleName", NAME);
set("CFBundleDisplayName", NAME);
set("CFBundleIdentifier", ID);
set("CFBundleIconFile", "icon.icns");
set("CFBundleShortVersionString", pkg.version);
for (const [key, value] of Object.entries(pkg.build?.mac?.extendInfo ?? {})) set(key, value);
fs.copyFileSync(icon, path.join(contents, "Resources", "icon.icns"));

// Electron runs Resources/app ahead of its own default app.
const app = path.join(contents, "Resources", "app");
fs.mkdirSync(app);
fs.writeFileSync(path.join(app, "package.json"), JSON.stringify({ name: "xenonet-dev", productName: NAME, version: pkg.version, private: true, type: "module", main: "main.js" }, null, 2) + "\n");
fs.writeFileSync(path.join(app, "main.js"), `// ${NAME}: runs the XenoNet checkout below. Made by npm run dev:install.\nimport ${JSON.stringify(pathToFileURL(path.join(root, "electron", "main.js")).href)};\n`);

run("codesign", ["--force", "--sign", "-", "--identifier", ID, dest]);
const info = execFileSync("sh", ["-c", 'codesign -dvv "$1" 2>&1 || true', "sh", dest], { encoding: "utf8" });
if (/^Authority=/m.test(info) || !/Signature=adhoc/.test(info)) fail(`${NAME}.app came out signed with a certificate; delete it.`);

// Spotlight, Launchpad and the Dock pick up the new name and icon now.
spawnSync("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister", ["-f", dest]);

console.log(`installed ${dest.replace(os.homedir(), "~")}, running ${root.replace(os.homedir(), "~")}`);
if (!process.argv.includes("--no-open")) spawnSync("open", [dest]);
