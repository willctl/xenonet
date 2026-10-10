// The Windows installer's Path handling (build/installer.nsh XenoNetPathAdd and
// XenoNetPathRemove), compiled with electron-builder's NSIS and run against a
// throwaway registry key, never the real Environment key.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ENTRY = "%LOCALAPPDATA%\\XenoNet\\bin";
const KEY = `Software\\XenoNet-installer-path-test-${process.pid}`;

// MAKENSIS, or the NSIS electron-builder downloaded for npm run dist:win.
function findMakensis() {
  if (process.env.MAKENSIS) return process.env.MAKENSIS;
  const cache = path.join(process.env.LOCALAPPDATA ?? "", "electron-builder", "Cache");
  for (const dir of fs.existsSync(cache) ? fs.readdirSync(cache).filter((d) => d.startsWith("nsis-") && !d.includes("resources")) : []) {
    for (const sub of fs.readdirSync(path.join(cache, dir))) {
      const exe = path.join(cache, dir, sub, "Bin", "makensis.exe");
      if (fs.existsSync(exe)) return exe;
    }
  }
  return null;
}

const makensis = process.platform === "win32" ? findMakensis() : null;
const skip = process.platform !== "win32" ? "native Windows execution" : !makensis && "needs NSIS: run npm run dist:win once, or set MAKENSIS";

const made = [];
after(() => {
  made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 }));
  if (process.platform === "win32") spawnSync("reg.exe", ["delete", `HKCU\\${KEY}`, "/f"], { windowsHide: true });
});

let exe;
function harness() {
  if (exe) return exe;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-nsis-"));
  made.push(dir);
  exe = path.join(dir, "path-test.exe");
  fs.writeFileSync(path.join(dir, "path-test.nsi"), `Unicode true
!define BUILD_UNINSTALLER
!define APP_ID "dev.xenonet.app.test"
!addincludedir "${path.join(root, "build")}"
!include "installer.nsh"
Name "XenoNet Path test"
OutFile "${exe}"
RequestExecutionLevel user
SilentInstall silent
Section
  \${GetParameters} $0
  ClearErrors
  \${GetOptions} $0 "/remove" $1
  \${If} \${Errors}
    !insertmacro XenoNetPathAdd "${KEY}"
  \${Else}
    !insertmacro XenoNetPathRemove "${KEY}"
  \${EndIf}
SectionEnd
`);
  execFileSync(makensis, ["/V1", "/WX", path.join(dir, "path-test.nsi")], { windowsHide: true });
  return exe;
}

const reg = (...args) => spawnSync("reg.exe", args, { windowsHide: true, encoding: "utf8" });
function setPath(value) {
  if (value === null) reg("delete", `HKCU\\${KEY}`, "/v", "Path", "/f");
  else assert.equal(reg("add", `HKCU\\${KEY}`, "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", value, "/f").status, 0);
}
function readPath() {
  const out = reg("query", `HKCU\\${KEY}`, "/v", "Path");
  if (out.status !== 0) return null;
  const [, type, value] = out.stdout.match(/^\s*Path\s+(REG_\w+)\s+(.*?)\r?$/m);
  return { type, value };
}
const run = (before, ...args) => {
  setPath(before);
  execFileSync(harness(), args, { windowsHide: true });
  return readPath();
};

test("Setup adds the xenonet command's folder to the user's Path once, unexpanded", { skip }, () => {
  assert.deepEqual(run(null), { type: "REG_EXPAND_SZ", value: ENTRY }, "no Path yet");
  assert.deepEqual(run("C:\\Tools;%USERPROFILE%\\bin;;"), { type: "REG_EXPAND_SZ", value: `C:\\Tools;%USERPROFILE%\\bin;${ENTRY}` }, "appended; other entries keep their variables; no empty entry");
  assert.equal(run(`C:\\Tools;${ENTRY.toLowerCase()};D:\\x`).value, `C:\\Tools;${ENTRY.toLowerCase()};D:\\x`, "already there, in any case");
  const expanded = `C:\\Tools;${process.env.LOCALAPPDATA}\\XenoNet\\bin`;
  assert.equal(run(expanded).value, expanded, "already there, expanded");
});

test("a full uninstall takes out only Setup's entry", { skip }, () => {
  assert.equal(run(`C:\\Tools;${ENTRY};%USERPROFILE%\\bin`, "/remove").value, "C:\\Tools;%USERPROFILE%\\bin");
  assert.equal(run(ENTRY, "/remove"), null, "an emptied Path is removed");
  assert.equal(run("C:\\Tools", "/remove").value, "C:\\Tools", "nothing to take out");
});

test("a Path near NSIS's string limit is left alone", { skip }, () => {
  const long = `C:\\${"x".repeat(8100)}`;
  assert.equal(run(long).value, long);
});
