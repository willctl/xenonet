import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn, execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { installedConsole } from "../../core/components/console.js";
import { consoleMcp } from "../harness/console-mcp.js";
import { installLauncher, installBundle, activatePackage } from "../../core/components/install.js";
import { connectService } from "../../core/components/runtime.js";
import { nextComponent } from "../harness/component-fixture.js";
import { limit } from "../harness/software.js";

const freePort = () => new Promise((r) => { const s = net.createServer().listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => r(port)); }); });
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };
const until = async (fn) => { for (const end = Date.now() + 60000; Date.now() < end;) { const v = await fn(); if (v) return v; await new Promise((r) => setTimeout(r, 100)); } throw new Error("The expected terminal state did not arrive."); };

// On hosted runners this test has failed 2 of its 19 Windows runs since
// 2026-10-10, each time differently: the service's hello went unanswered for
// 5 s (run 38506902762), and Desktop was reported closed while the test
// expected it up (run 38104211834). Each passed again on the next run with
// the same code, and the test passes on a developer machine, so hosted
// runners skip it until the restart path can wait out a starved runner. It
// stays in the release checklist (docs/guide/releases.md), and
// XENONET_E2E_RESTART=1 runs it on a hosted runner anyway.
const hosted = process.env.CI === "true" && process.env.XENONET_E2E_RESTART !== "1";

test("production Desktop restarts without losing its service, console connection or terminal", { timeout: limit(300000), skip: hosted && "hosted runners have lost the service or Desktop part way through this test (see the note above); XENONET_E2E_RESTART=1 runs it anyway" }, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-components-"));
  const port = await freePort();
  const env = { ...process.env, XENONET_INSTANCE: "test", XENONET_DEMO: "1", XENONET_STANDALONE: "1", XENONET_RUNTIME_DIR: dir, XENONET_RUNTIME_EXEC: process.execPath, XENONET_PORT: String(port), XENONET_DESKTOP_PORT: String(await freePort()), XENONET_SHOW: process.env.XENONET_E2E_SHOW ?? "hidden" };
  delete env.ELECTRON_RUN_AS_NODE;
  if (process.env.XENONET_TEST_COMPONENT_BUNDLE) await installBundle(process.env.XENONET_TEST_COMPONENT_BUNDLE, { env, command: false });
  const electron = process.env.XENONET_TEST_PACKAGED_EXE || createRequire(import.meta.url)("electron");
  const out = fs.openSync(path.join(dir, "desktop.log"), "a");
  const child = spawn(electron, process.env.XENONET_TEST_PACKAGED_EXE ? [] : [process.cwd(), "--prod"], { env, stdio: ["ignore", out, out], windowsHide: true });
  fs.closeSync(out);
  const s = installedConsole({ env });
  let service;
  let mcp;
  try {
    await s.ready({ timeout: 90000 });
    service = await connectService({ env });
    const launcher = installLauncher(env);
    mcp = await consoleMcp({ command: launcher?.runtime, entry: launcher?.entry, env });
    if (launcher) {
      const names = (await mcp.list()).result.tools.map((tool) => tool.name);
      assert.ok(names.includes("terminal"));
      assert.ok(!names.includes("eval") && !names.includes("simulate"));
    }
    assert.equal((await mcp.call("terminal", { action: "list" })).result.isError, undefined);
    const before = await service.request("status");
    const desktop = await s.ping();
    await s.driver("terminal", "open");
    const tabs = await s.driver("terminal", "list");
    const id = tabs.tabs[0].id;
    // PowerShell can discard keystrokes while it initializes PSReadLine.
    // Wait for its prompt just as a person would before entering a command.
    if (process.platform === "win32") await until(async () => /PS [^\r\n]*>/.test((await s.driver("terminal", "read")).text));
    const shell = process.platform === "win32" ? "$xenonetProof = 'still-alive'; Write-Output $xenonetProof\r" : "xenonetProof=still-alive; echo $xenonetProof\n";
    await s.driver("terminal", "write", { id, text: shell });
    await until(async () => (await s.driver("terminal", "read")).text.includes("still-alive"));
    const terminalPid = (await service.request("status")).terminals[0].pid;
    await s.quit();
    assert.equal((await service.request("status")).pid, before.pid);
    assert.equal((await service.request("status")).desktop, false);
    assert.equal((await mcp.call("terminal", { action: "list" })).result.isError, true);
    assert.equal(mcp.proc.exitCode, null);
    await assert.rejects(service.request("console", { op: "ping" }), /closed or restarting/);
    await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
    assert.notEqual((await s.ping()).pid, desktop.pid);
    assert.equal((await mcp.call("terminal", { action: "list" })).result.isError, undefined);
    assert.equal(mcp.proc.exitCode, null);
    assert.equal((await service.request("status")).pid, before.pid);
    assert.equal((await service.request("status")).terminals[0].pid, terminalPid);
    await until(async () => (await s.driver("terminal", "list")).tabs.some((t) => t.id === id));
    await s.driver("terminal", "write", { id, text: process.platform === "win32" ? "Write-Output ('restored-' + $xenonetProof)\r" : "echo restored-$xenonetProof\n" });
    await until(async () => (await s.driver("terminal", "read")).text.includes("restored-still-alive"));
    await assert.rejects(service.request("console", { op: "eval", expression: "1+1" }), /development/);
    // You, as the window reports you: the page's reports reach the service
    // through Desktop, which the service takes for XenoNet's app. On the demo
    // anyone may read your record.
    const { nodes: [atlas] } = await s.driver("find", { query: "atlas-api", kind: "project", limit: 1 });
    const since = Date.now();
    // Hosted runners' software rendering may never settle the camera, and
    // a look is the page's own 3 s timer, so the selection doesn't wait.
    await s.driver("select", atlas.id, { settle: false });
    const looked = await until(async () => (await s.api("/api/actors/you")).recent?.find((e) => e.kind === "status" && e.target === atlas.id && e.at >= since));
    assert.equal(looked.detail, "Looking at atlas-api");
    // Desktop passes on its page's requests alone; other programs go to the
    // service, which sees who they are.
    assert.equal((await fetch(`http://127.0.0.1:${env.XENONET_DESKTOP_PORT}/api/summary`)).status, 403);
    if (process.env.XENONET_TEST_COMPONENT_BUNDLE) {
      const tools = await nextComponent("tools", env);
      const serviceUpdate = await nextComponent("service", env);
      assert.equal((await activatePackage(tools.file, tools.item, { env })).status, "updated");
      assert.equal(mcp.proc.exitCode, null);
      assert.equal((await mcp.call("terminal", { action: "list" })).result.isError, undefined);
      const version = JSON.parse(execFileSync(launcher.runtime, [launcher.entry, "--version", "--json"], { env, encoding: "utf8", windowsHide: true }));
      assert.equal(version.version, tools.item.version);
      assert.ok(fs.existsSync(tools.previous.root));
      assert.equal((await activatePackage(serviceUpdate.file, serviceUpdate.item, { env })).status, "staged");
      assert.equal((await service.request("status")).pid, before.pid);
      await s.driver("terminal", "close", { id });
      service.close(); service = null;
      assert.equal((await activatePackage(serviceUpdate.file, serviceUpdate.item, { env })).status, "updated");
      await s.ready();
      service = await connectService({ env });
      assert.notEqual((await service.request("status")).pid, before.pid);
      assert.equal((await service.request("status")).version, serviceUpdate.item.version);
      assert.equal((await mcp.call("terminal", { action: "list" })).result.isError, undefined);
      assert.equal(mcp.proc.exitCode, null);
    }
    assert.deepEqual((await s.logs({ source: "page", level: "error" })).entries, []);
    await s.screenshot({ name: "components", inline: false, settle: false });
  } catch (err) {
    console.error(await s.driver("terminal", "read", { lines: 100 }).catch((e) => e.message));
    console.error(fs.readFileSync(path.join(dir, "desktop.log"), "utf8").slice(-6000));
    throw err;
  } finally {
    mcp?.close();
    await s.quit().catch(() => {});
    child.kill();
    service ??= await connectService({ env }).catch(() => null);
    const pid = await service?.request("status").then((st) => st.pid).catch(() => null);
    await service?.request("stop", { force: true }).catch(() => {});
    service?.close();
    // Its runtime folder and its demo world go once the service has gone,
    // which could otherwise still be writing to them. A folder Windows
    // still holds stays behind rather than failing the test.
    for (const end = Date.now() + 10000; pid && alive(pid) && Date.now() < end; ) await new Promise((r) => setTimeout(r, 100));
    if (process.env.XENONET_E2E_KEEP !== "1")
      for (const p of [dir, path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`)]) {
        try {
          fs.rmSync(p, { recursive: true, force: true, maxRetries: 10 });
        } catch (err) {
          console.error(`left ${p}: ${err.message}`);
        }
      }
  }
});
