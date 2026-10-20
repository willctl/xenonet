// Background service: the setting that starts XenoNet's service at login, and
// Desktop's offer of it during setup (src/background.js, electron/background.js).
// XenoNet Dev only says it starts its own service; a --prod Desktop, which
// stands in for an installed one, registers through the xenonet command with
// a made-up launchctl or systemctl in a temporary folder. Nothing here
// registers anything with the machine's own launchd or systemd.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { xenonet } from "../../core/control.js";
import { installedConsole } from "../../core/components/console.js";
import { connectService } from "../../core/components/runtime.js";

const s = xenonet("test");
before(async () => { await s.quit(); await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" }); }, { timeout: 180000 });
after(async () => { await s.quit().catch(() => {}); });

const freePort = () => new Promise((r) => { const v = net.createServer().listen(0, "127.0.0.1", () => { const { port } = v.address(); v.close(() => r(port)); }); });
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };
const until = async (fn, ms = 60000) => { for (const end = Date.now() + ms; Date.now() < end;) { const v = await fn(); if (v) return v; await new Promise((r) => setTimeout(r, 100)); } throw new Error("The expected state did not arrive."); };

test("Background service is reachable from the palette and driver, and XenoNet Dev starts its own service", { timeout: 60000 }, async () => {
  await s.driver("command", "background");
  const st = await s.driver("background");
  assert.equal(st.live, false);
  assert.match(await s.eval('return document.querySelector("[data-background-status]").textContent'), /XenoNet Dev starts and stops its own service/);
  assert.equal(await s.eval('return document.querySelector("[data-background-login]").disabled'), true);
  assert.ok(st.at.login, "the switch says where it is");
  // No offer from XenoNet Dev.
  assert.equal((await s.driver("background", "offer")).shown, false);
  await s.screenshot({ name: "e2e-background", inline: false });
  await s.input({ keys: "Escape" });
  assert.deepEqual((await s.logs({ source: "page", level: "error" })).entries, []);
});

test("the setup offer and the switch ask Desktop, and Windows says it isn't available", { timeout: 60000 }, async () => {
  await s.eval(`
    const { createBackground } = await import('/src/background.js');
    window.bgCalls = [];
    window.bgState = { live: true, supported: true, supervisor: 'launchd', registered: false, running: true, supervised: false, file: '~/Library/LaunchAgents/dev.xenonet.service.plist', offer: true };
    const api = { act: async (action, value) => {
      window.bgCalls.push([action, value ?? null]);
      if (action === 'set') window.bgState = { ...window.bgState, registered: value.on, offer: false, result: { takeover: 'later' } };
      if (action === 'dismiss') window.bgState = { ...window.bgState, offer: false };
      return window.bgState;
    } };
    window.bgPanel = createBackground({ modals: window.__xenonet.modals, desktop: { background: api }, toast: () => {} });
  `);
  // Not now answers it.
  assert.equal(await s.eval("return window.bgPanel.offer()"), true);
  assert.match(await s.eval('return document.querySelector("#modal").textContent'), /Keep XenoNet running in the background\?/);
  await s.screenshot({ name: "e2e-background-offer", inline: false });
  await s.eval('document.querySelector("[data-background-offer=later]").click()');
  await until(() => s.eval('return window.bgCalls.some(([a]) => a === "dismiss")'));
  assert.equal(await s.eval("return window.bgPanel.offer()"), false, "asked once");
  // Closed without an answer counts as Not now too.
  await s.eval("window.bgState = { ...window.bgState, offer: true }; window.bgCalls = []; return await window.bgPanel.offer()");
  await s.input({ keys: "Escape" });
  await until(() => s.eval('return window.bgCalls.some(([a]) => a === "dismiss")'));
  // Start at login turns it on.
  await s.eval("window.bgState = { ...window.bgState, offer: true }; window.bgCalls = []; return await window.bgPanel.offer()");
  await s.eval('document.querySelector("[data-background-offer=on]").click()');
  await until(() => s.eval('return window.bgCalls.some(([a, v]) => a === "set" && v.on === true)'));
  // The switch turns it off again, and says how it stands.
  await s.eval("await window.bgPanel.open()");
  assert.equal(await s.eval('return document.querySelector("[data-background-login]").checked'), true);
  assert.match(await s.eval('return document.querySelector("[data-background-status]").textContent'), /takes over when it stops/);
  await s.eval('const box = document.querySelector("[data-background-login]"); box.checked = false; box.dispatchEvent(new Event("change", { bubbles: true })); return true');
  await until(() => s.eval('return window.bgCalls.some(([a, v]) => a === "set" && v.on === false)'));
  await until(() => s.eval('return /^Off\\./.test(document.querySelector("[data-background-status]").textContent)'));
  // Windows: explained, and the switch can't be turned on.
  await s.eval("window.bgState = { live: true, supported: false, registered: false, offer: false, reason: \"Starting the service at login isn't available on Windows yet. Desktop starts the service when it opens.\" }; return await window.bgPanel.open()");
  assert.match(await s.eval('return document.querySelector("[data-background-status]").textContent'), /isn't available on Windows yet/);
  assert.equal(await s.eval('return document.querySelector("[data-background-login]").disabled'), true);
  await s.screenshot({ name: "e2e-background-windows", inline: false });
  await s.input({ keys: "Escape" });
  assert.deepEqual((await s.logs({ source: "page", level: "error" })).entries, []);
});

// A made-up launchctl or systemctl: each call kept in calls.log beside it,
// and the registration loaded or not, as the real ones keep it.
function fakeSupervisor(dir) {
  fs.mkdirSync(dir, { recursive: true });
  const launchctl = `#!/bin/sh
dir=$(dirname "$0")
echo "$*" >> "$dir/calls.log"
case "$1" in
  bootstrap) touch "$dir/loaded" ;;
  bootout) if [ -f "$dir/loaded" ]; then rm -f "$dir/loaded"; else echo "Boot-out failed: 3: No such process" >&2; exit 3; fi ;;
  print) if [ -f "$dir/loaded" ]; then printf 'gui/1/dev.xenonet.service = {\\n\\tstate = not running\\n}\\n'; else echo "Could not find service" >&2; exit 113; fi ;;
esac
exit 0
`;
  const systemctl = `#!/bin/sh
dir=$(dirname "$0")
echo "$*" >> "$dir/calls.log"
case "$*" in
  "--user show"*) if [ -f "$dir/loaded" ]; then printf 'LoadState=loaded\\nActiveState=inactive\\nMainPID=0\\nUnitFileState=enabled\\n'; else printf 'LoadState=not-found\\nActiveState=inactive\\nMainPID=0\\nUnitFileState=\\n'; fi ;;
  "--user enable --now xenonet.service") touch "$dir/loaded" ;;
  "--user disable --now xenonet.service") rm -f "$dir/loaded" ;;
esac
exit 0
`;
  const bin = path.join(dir, process.platform === "darwin" ? "launchctl" : "systemctl");
  fs.writeFileSync(bin, process.platform === "darwin" ? launchctl : systemctl, { mode: 0o755 });
  return { bin, calls: () => { try { return fs.readFileSync(path.join(dir, "calls.log"), "utf8").trim().split("\n"); } catch { return []; } } };
}

const supported = ["darwin", "linux"].includes(process.platform);

test("a production Desktop offers it once, and registers and unregisters it through the xenonet command", { timeout: 300000, skip: !supported && "starting the service at login isn't available here yet" }, async () => {
  // The shared test instance isn't needed here, and this Desktop is another.
  await s.quit();
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-background-"));
  const port = await freePort();
  const supervisor = fakeSupervisor(path.join(dir, "supervisor"));
  const agents = path.join(dir, "agents");
  const env = { ...process.env, XENONET_INSTANCE: "test", XENONET_DEMO: "1", XENONET_STANDALONE: "1", XENONET_RUNTIME_DIR: dir, XENONET_PORT: String(port), XENONET_DESKTOP_PORT: String(await freePort()), XENONET_SHOW: process.env.XENONET_E2E_SHOW ?? "hidden", XENONET_TEST_PROFILE: path.join(dir, "profile"), XENONET_SUPERVISOR_BIN: supervisor.bin, XENONET_SUPERVISOR_DIR: agents };
  delete env.ELECTRON_RUN_AS_NODE;
  const out = fs.openSync(path.join(dir, "desktop.log"), "a");
  const child = spawn(createRequire(import.meta.url)("electron"), [process.cwd(), "--prod"], { env, stdio: ["ignore", out, out], windowsHide: true });
  fs.closeSync(out);
  const d = installedConsole({ env });
  const file = path.join(agents, process.platform === "darwin" ? "dev.xenonet.service.plist" : "xenonet.service");
  // The panel's controls, once they've stayed put for half a second.
  const settled = async (action) => {
    let last = null;
    let since = 0;
    return until(async () => {
      const st = await d.driver("background", action);
      if (JSON.stringify(st.at) !== JSON.stringify(last?.at)) since = Date.now();
      last = st;
      return Date.now() - since >= 500 ? st : (await new Promise((r) => setTimeout(r, 150)), null);
    });
  };
  let service;
  try {
    await d.ready({ timeout: 90000 });
    // The offer, during setup.
    const offered = await settled("offer");
    assert.equal(offered.shown, true);
    assert.deepEqual([offered.live, offered.supported, offered.registered], [true, true, false]);
    assert.ok(offered.at.accept, JSON.stringify(offered));
    await d.input({ click: offered.at.accept });
    const written = await until(() => fs.existsSync(file) && fs.readFileSync(file, "utf8"));
    if (process.platform === "darwin") {
      assert.match(written, /<string>service<\/string>\n\t\t<string>run<\/string>/);
      assert.match(written, new RegExp(`<key>XENONET_PORT</key>\\n\\t\\t<string>${port}</string>`));
      assert.ok(written.includes(`<string>${path.join(dir, `service-${port}.log`)}</string>`), written);
      await until(() => supervisor.calls().some((c) => c.startsWith("bootstrap ")));
      assert.deepEqual(supervisor.calls().filter((c) => !c.startsWith("print")), [`enable gui/${process.getuid()}/dev.xenonet.service`, `bootstrap gui/${process.getuid()} ${file}`]);
    } else {
      assert.match(written, / service run\n/);
      assert.ok(written.includes(`Environment="XENONET_PORT=${port}"`), written);
      await until(() => supervisor.calls().includes("--user enable --now xenonet.service"));
    }
    // Answered: not offered again, and the panel says it's on. The service
    // Desktop started keeps running, since the made-up supervisor has none
    // waiting to take over.
    assert.equal((await d.driver("background", "offer")).shown, false);
    const on = await settled("status");
    assert.equal(on.registered, true);
    assert.equal(on.running, true);
    assert.equal(on.supervised, false);
    service = await connectService({ env });
    const pid = (await service.request("status")).pid;
    await d.screenshot({ name: "e2e-background-on", inline: false, settle: false });
    // The switch turns it off.
    await d.input({ click: on.at.login });
    await until(() => !fs.existsSync(file));
    if (process.platform === "darwin") await until(() => supervisor.calls().includes(`bootout gui/${process.getuid()}/dev.xenonet.service`));
    else await until(() => supervisor.calls().includes("--user disable --now xenonet.service"));
    const off = await until(async () => { const st = await d.driver("background", "status"); return st.registered === false && !st.message?.endsWith("…") ? st : null; });
    assert.equal(off.offer, false);
    assert.equal((await service.request("status")).pid, pid, "the service Desktop started kept running");
    assert.deepEqual((await d.logs({ source: "page", level: "error" })).entries, []);
  } catch (err) {
    console.error(JSON.stringify(await d.driver("background", "status").catch((e) => e.message)));
    console.error(fs.readFileSync(path.join(dir, "desktop.log"), "utf8").slice(-6000));
    console.error(supervisor.calls().join("\n"));
    throw err;
  } finally {
    await d.quit().catch(() => {});
    child.kill();
    service ??= await connectService({ env }).catch(() => null);
    const pid = await service?.request("status").then((st) => st.pid).catch(() => null);
    await service?.request("stop", { force: true }).catch(() => {});
    service?.close();
    for (const end = Date.now() + 10000; pid && alive(pid) && Date.now() < end; ) await new Promise((r) => setTimeout(r, 100));
    if (process.env.XENONET_E2E_KEEP !== "1")
      for (const p of [dir, path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`)]) fs.rmSync(p, { recursive: true, force: true, maxRetries: 10 });
  }
});
