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

test("a Windows command wrapper preserves paths, arguments and exit status", { skip: process.platform !== "win32" && "native Windows execution" }, () => {
  const home = tmp();
  const dir = path.join(home, "bin with spaces");
  const source = path.join(home, "source %literal%!", "probe.js");
  fs.mkdirSync(path.dirname(source));
  fs.writeFileSync(source, 'console.log(JSON.stringify({ args: process.argv.slice(2), electron: process.env.ELECTRON_RUN_AS_NODE })); process.exit(7);');
  assert.equal(commandDir({ home, platform: "win32", env: {} }), path.join(home, "AppData", "Local", "XenoNet", "bin"));
  const st = installCommand({ dir, runtime: process.execPath, entry: source, runAsNode: true });
  assert.deepEqual([st.installed, st.ours, st.broken, st.runs], [true, true, false, [process.execPath, source]]);
  const result = spawnSync(process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", `""${st.file}" "two words" plain"`], { env: { ...process.env, literal: "wrong" }, encoding: "utf8", windowsHide: true, windowsVerbatimArguments: true });
  assert.equal(result.status, 7, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { args: ["two words", "plain"], electron: "1" });
  assert.equal(uninstallCommand({ dir }).installed, false);
  assert.throws(() => installCommand({ dir, runtime: "bad\npath", entry: source }), /line break/);
});

test("Windows hooks run, keep other hooks, and can be reinstalled and removed", { skip: process.platform !== "win32" && "native Windows execution" }, async () => {
  const home = tmp();
  const root = path.join(home, "source O'Brien with spaces");
  const entry = path.join(root, "cli", "xenonet.js");
  fs.mkdirSync(path.dirname(entry), { recursive: true });
  fs.writeFileSync(entry, 'let input="";process.stdin.setEncoding("utf8");process.stdin.on("data",s=>input+=s);process.stdin.on("end",()=>console.log(JSON.stringify({port:process.env.XENONET_PORT,electron:process.env.ELECTRON_RUN_AS_NODE,args:process.argv.slice(2),input})));');
  const settings = path.join(home, ".claude", "settings.json");
  const hooks = path.join(home, ".codex", "hooks.json");
  const own = { hooks: [{ type: "command", command: "echo owner's hook" }] };
  fs.mkdirSync(path.dirname(settings));
  fs.writeFileSync(settings, JSON.stringify({ hooks: { SessionStart: [own] } }));
  const connect = createConnect({ port: 47842, root, exec: process.execPath, runAsNode: true, home, env: { PATH: "" }, platform: "win32" });
  await connect.apply();
  await connect.apply();
  const claude = JSON.parse(fs.readFileSync(settings)).hooks.SessionStart;
  const codex = JSON.parse(fs.readFileSync(hooks)).hooks;
  assert.deepEqual(claude[0], own);
  assert.equal(claude.length, 2, "reinstall never duplicates hooks");
  assert.equal(claude[1].hooks[0].shell, "powershell");
  for (const [app, command] of [["claude", claude[1].hooks[0].command], ["codex", codex.SessionStart[0].hooks[0].commandWindows]]) {
    const args = command.split(" ").slice(1);
    // PowerShell can take over ten seconds to start on a busy CI runner.
    const result = spawnSync("powershell.exe", args, { input: '{"hook_event_name":"SessionStart"}', encoding: "utf8", windowsHide: true, timeout: 30000 });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), { port: "47842", electron: "1", args: ["hook", app], input: '{"hook_event_name":"SessionStart"}' });
  }
  assert.equal(codex.SessionEnd[0].hooks[0].timeout, 3);
  const preview = await connect.preview();
  assert.ok(preview.apps.every((app) => app.changes.find((c) => c.id === "hooks").state === "present"));
  await connect.remove();
  assert.deepEqual(JSON.parse(fs.readFileSync(settings)), { hooks: { SessionStart: [own] } });
  assert.deepEqual(JSON.parse(fs.readFileSync(hooks)), {});
});
