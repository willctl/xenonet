// Native command and hook execution, in a temporary synthetic workspace.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { commandDir, installCommand, uninstallCommand } from "../core/command.js";
import { createConnect } from "../core/engine/connect.js";
import { pathStyle } from "../core/paths.js";

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

test("Windows hooks run, keep other hooks, and can be reinstalled and removed", { skip: process.platform !== "win32" && "native Windows execution" }, async () => {
  const home = tmp();
  const root = path.join(home, "source O'Brien with spaces");
  const bin = standIn(root, 'let input="";process.stdin.setEncoding("utf8");process.stdin.on("data",s=>input+=s);process.stdin.on("end",()=>console.log(JSON.stringify({port:process.env.XENONET_PORT,args:process.argv.slice(2),input})));');
  const settings = path.join(home, ".claude", "settings.json");
  const hooks = path.join(home, ".codex", "hooks.json");
  const own = { hooks: [{ type: "command", command: "echo owner's hook" }] };
  fs.mkdirSync(path.dirname(settings));
  fs.writeFileSync(settings, JSON.stringify({ hooks: { SessionStart: [own] } }));
  const connect = createConnect({ port: 47842, root, bin, home, env: { PATH: "" }, platform: "win32" });
  await connect.apply();
  await connect.apply();
  const claude = JSON.parse(fs.readFileSync(settings)).hooks.SessionStart;
  const codex = JSON.parse(fs.readFileSync(hooks)).hooks;
  assert.deepEqual(claude[0], own);
  assert.equal(claude.length, 2, "reinstall never duplicates hooks");
  assert.equal(claude[1].hooks[0].shell, "powershell");
  for (const [app, command] of [["claude", claude[1].hooks[0].command], ["codex", codex.SessionStart[0].hooks[0].commandWindows]]) {
    const args = command.split(" ").slice(1);
    // PowerShell can take over thirty seconds to start on a busy CI runner.
    const result = spawnSync("powershell.exe", args, { input: '{"hook_event_name":"SessionStart"}', encoding: "utf8", windowsHide: true, timeout: 120000 });
    assert.equal(result.status, 0, result.error ? `${result.error.message}; stderr: ${result.stderr}` : result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { port: "47842", args: ["hook", app], input: '{"hook_event_name":"SessionStart"}' });
  }
  assert.equal(codex.SessionEnd[0].hooks[0].timeout, 3);
  // Every app that's here is connected; Grok Build isn't here, so
  // connecting every app leaves it out and the preview says so.
  const preview = await connect.preview();
  const here = preview.apps.filter((app) => app.installed !== false);
  assert.deepEqual(here.map((app) => app.app), ["claude", "codex"]);
  assert.ok(here.every((app) => app.changes.find((c) => c.id === "hooks").state === "present"));
  assert.deepEqual(preview.apps.filter((app) => app.installed === false).map((app) => [app.app, app.changes.find((c) => c.id === "hooks").state]), [["grok", "missing"]]);
  assert.equal(fs.existsSync(path.join(home, ".grok")), false);
  await connect.remove();
  assert.deepEqual(JSON.parse(fs.readFileSync(settings)), { hooks: { SessionStart: [own] } });
  assert.deepEqual(JSON.parse(fs.readFileSync(hooks)), {});
});
