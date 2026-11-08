// Native command and hook execution, in a temporary synthetic workspace.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { commandDir, installCommand, uninstallCommand } from "../electron/lib/command.js";
import { pathStyle } from "../electron/lib/paths.js";

const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-native-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

test("path syntax stays with the machine that supplied it", () => {
  assert.equal(pathStyle("/Users/nova/Projects").resolve("/Users/nova/Projects", "../scratch"), "/Users/nova/scratch");
  assert.equal(pathStyle("C:/Users/nova/Projects").resolve("C:/Users/nova/Projects", "../scratch"), "C:\\Users\\nova\\scratch");
  assert.equal(pathStyle("\\\\server\\share").join("\\\\server\\share", "repo"), "\\\\server\\share\\repo");
});

// A stand-in for the xenonet binary: a batch file in `dir` that runs a probe
// beside it, which says what it was given. It finds the probe through %~dp0,
// so a % or ! in the folder's name is never read again.
function standIn(dir, probe) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "probe.js"), probe);
  const bin = path.join(dir, "xenonet.cmd");
  fs.writeFileSync(bin, `@"${process.execPath}" "%~dp0probe.js" %*\r\n`);
  return bin;
}

test("a Windows command wrapper preserves paths, arguments and exit status", { skip: process.platform !== "win32" && "native Windows execution" }, () => {
  const home = tmp();
  const dir = path.join(home, "bin with spaces");
  const bin = standIn(path.join(home, "source %literal%!"), "console.log(JSON.stringify({ args: process.argv.slice(2) })); process.exit(7);");
  assert.equal(commandDir({ home, platform: "win32", env: {} }), path.join(home, "AppData", "Local", "XenoNet", "bin"));
  const st = installCommand({ dir, bin, platform: "win32" });
  assert.deepEqual([st.installed, st.ours, st.broken, st.runs], [true, true, false, [bin]]);
  const result = spawnSync(process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", `""${st.file}" "two words" plain"`], { env: { ...process.env, literal: "wrong" }, encoding: "utf8", windowsHide: true, windowsVerbatimArguments: true });
  assert.equal(result.status, 7, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { args: ["two words", "plain"] });
  assert.equal(uninstallCommand({ dir, platform: "win32" }).installed, false);
  assert.throws(() => installCommand({ dir, bin: "bad\npath", platform: "win32" }), /line break/);
});

