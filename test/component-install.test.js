// Installing XenoNet's components (core/components/install.js) beyond the
// service switch component-activation.test.js covers: a bundle installs
// what's missing, replaces a Service or Tools from before 0.4.0 and never
// upgrades what else is there, a runtime that fails its startup check is
// never selected, a Tools update leaves the service alone, recovery stops
// only an unexpected service and never one with open terminals, and a
// launcher that can't be replaced now is left for next time. Packages are
// made up (test/harness/components.js); their xenonet binary is a shell
// script that answers the startup check, so the bundle tests run where a
// script can stand in for a program.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { activatePackage, checkRuntime, hasBinary, installBundle, installedRuntime, installLauncher, recoverServiceUpdate, writeLauncher, launcherVersion, LAUNCHER_SOURCE } from "../core/components/install.js";
import { currentComponent, selectComponent, stagePackage } from "../core/components/packages.js";
import { EXE, BIN, NODE, posixOnly, fakeBin, pack, bundle, fakeService as service, earlierInstall } from "./harness/components.js";

function workspace(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-install-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return { dir, env: { XENONET_RUNTIME_DIR: path.join(dir, "runtime") } };
}

test("a bundle installs the components that are missing, with the launcher, and says how to start the service", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  // Another machine's package, listed first, is never opened: its file
  // isn't even there.
  const first = await bundle(dir, "bundle-1", [["service", "1.0.0"], ["tools", "1.0.0"]]);
  const manifest = JSON.parse(fs.readFileSync(path.join(first, "components.json"), "utf8"));
  manifest.packages.unshift({ ...manifest.packages[0], platform: process.platform === "linux" ? "darwin" : "linux", file: "XenoNet-service-1.0.0-elsewhere.tar.gz" });
  fs.writeFileSync(path.join(first, "components.json"), JSON.stringify(manifest));

  const runtime = await installBundle(first, { env, command: false });
  const service = currentComponent("service", env);
  assert.deepEqual([service.version, currentComponent("tools", env).version], ["1.0.0", "1.0.0"]);
  assert.equal(runtime.root, service.root);
  assert.deepEqual([runtime.exec, runtime.args], [path.join(service.root, BIN), ["serve"]], "the service's own binary, serving");
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  assert.deepEqual([runtime.env.XENONET_RUNTIME_DIR, runtime.env.XENONET_LAUNCHER_BIN], [env.XENONET_RUNTIME_DIR, path.join(launcher, EXE)]);
  assert.deepEqual([runtime.env.XENONET_LAUNCHER_ROOT, runtime.env.XENONET_LAUNCHER_EXEC], [undefined, undefined], "no node launcher to name");
  assert.equal(fs.readFileSync(path.join(launcher, EXE), "utf8"), fakeBin("1.0.0"), "the launcher is a copy of the Tools' binary");
  assert.ok(fs.statSync(path.join(launcher, EXE)).mode & 0o100, "and it can run");
  assert.deepEqual(fs.readdirSync(launcher).sort(), [EXE, `${EXE}.from`], "and nothing else: no node, no launcher script");
});

test("an earlier XenoNet's node launcher gets this XenoNet's script, which runs the binary, and nothing gets a node", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  const entry = path.join(launcher, "cli", "xenonet.js");
  fs.mkdirSync(path.dirname(entry), { recursive: true });
  fs.writeFileSync(path.join(launcher, NODE), "an earlier XenoNet's node", { mode: 0o755 });
  fs.writeFileSync(entry, "// xenonet-launcher 2 (protocol 1)\nearlier();\n");
  await installBundle(await bundle(dir, "bundle-1", [["service", "1.0.0"], ["tools", "1.0.0"]]), { env, command: false });
  assert.equal(fs.readFileSync(entry, "utf8"), fs.readFileSync(LAUNCHER_SOURCE, "utf8"));
  assert.ok(launcherVersion(fs.readFileSync(entry, "utf8")) >= 3, "one that runs the Tools' binary");
  assert.equal(fs.readFileSync(path.join(launcher, NODE), "utf8"), "an earlier XenoNet's node", "its node left as it was");
  assert.equal(installedRuntime(env).env.XENONET_LAUNCHER_EXEC, undefined, "and the service isn't told of it");

  // Without a node there's no node launcher to keep current.
  const other = { XENONET_RUNTIME_DIR: path.join(dir, "other") };
  fs.mkdirSync(path.join(other.XENONET_RUNTIME_DIR, "launcher", "cli"), { recursive: true });
  fs.writeFileSync(path.join(other.XENONET_RUNTIME_DIR, "launcher", "cli", "xenonet.js"), "// xenonet-launcher 2 (protocol 1)\n");
  await installBundle(await bundle(dir, "bundle-2", [["service", "1.0.0"], ["tools", "1.0.0"]]), { env: other, command: false });
  assert.equal(fs.readFileSync(path.join(other.XENONET_RUNTIME_DIR, "launcher", "cli", "xenonet.js"), "utf8"), "// xenonet-launcher 2 (protocol 1)\n");
  assert.equal(fs.existsSync(path.join(other.XENONET_RUNTIME_DIR, "launcher", NODE)), false);
});

test("a bundle never upgrades or restarts a service or Tools already installed", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  await installBundle(await bundle(dir, "bundle-1", [["service", "1.0.0"], ["tools", "1.0.0"]]), { env, command: false });
  const before = currentComponent("service", env);
  const runtime = await installBundle(await bundle(dir, "bundle-2", [["service", "2.0.0"], ["tools", "2.0.0"]]), { env, command: false });
  assert.deepEqual([currentComponent("service", env).version, currentComponent("tools", env).version], ["1.0.0", "1.0.0"]);
  assert.equal(runtime.root, before.root);
  assert.deepEqual(fs.readdirSync(path.join(env.XENONET_RUNTIME_DIR, "components", "service")).filter((f) => f.startsWith("2.0.0")), [], "the newer one isn't even unpacked");
});

test("a component whose runtime fails its startup check is never selected", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  await assert.rejects(installBundle(await bundle(dir, "broken-runtime", [["service", "1.0.0", { fail: "runtime" }]]), { env, command: false }), /runtime did not pass its startup check/);
  assert.equal(currentComponent("service", env), null);
  await assert.rejects(installBundle(await bundle(dir, "wrong-version", [["service", "1.0.0", { fail: "command" }]]), { env, command: false }), /command did not pass its startup check/);
  assert.equal(currentComponent("service", env), null);
  // Nothing selected, so there's no service to start.
  await assert.rejects(installBundle(await bundle(dir, "tools-only", [["tools", "1.0.0"]]), { env, command: false }), /XenoNet Service is not installed/);
  assert.equal(currentComponent("tools", env).version, "1.0.0");
});

test("the startup check runs the staged binary itself, without ELECTRON_RUN_AS_NODE, and wants the version it was promised", posixOnly, async (t) => {
  const { dir } = workspace(t);
  const root = path.join(dir, "staged");
  fs.mkdirSync(path.join(root, "bin"), { recursive: true });
  // A binary that fails unless it's run without the variable Electron's
  // runtime would pass on, and only as `--version --json`.
  fs.writeFileSync(path.join(root, BIN), `#!/bin/sh\n[ -n "$ELECTRON_RUN_AS_NODE" ] && exit 3\n[ "$*" = "--version --json" ] || exit 4\n${fakeBin("1.4.2").split("\n").slice(1).join("\n")}`, { mode: 0o755 });
  const saved = process.env.ELECTRON_RUN_AS_NODE;
  process.env.ELECTRON_RUN_AS_NODE = "1";
  try {
    assert.equal(checkRuntime({ root, version: "1.4.2" }), undefined);
    assert.throws(() => checkRuntime({ root, version: "1.4.3" }), /command did not pass its startup check/);
  } finally {
    if (saved === undefined) delete process.env.ELECTRON_RUN_AS_NODE;
    else process.env.ELECTRON_RUN_AS_NODE = saved;
  }
  fs.rmSync(path.join(root, BIN));
  assert.throws(() => checkRuntime({ root, version: "1.4.2" }), /runtime did not pass its startup check/, "no binary at all");
});

test("a Tools update selects the new Tools and its launcher, and leaves the running service alone", async (t) => {
  const { dir, env } = workspace(t);
  const svc = await pack(dir, "service", "1.0.0");
  selectComponent("service", await stagePackage(svc.file, svc.item, { env }), env);
  const old = await pack(dir, "tools", "1.0.0");
  selectComponent("tools", await stagePackage(old.file, old.item, { env }), env);
  const running = service(env, { version: "1.0.0", terminals: [{ id: 1, exited: false }] });

  const next = await pack(dir, "tools", "1.1.0");
  const r = await activatePackage(next.file, next.item, { env, operations: running.operations });
  assert.deepEqual([r.status, r.version], ["updated", "1.1.0"]);
  assert.match(r.detail, /Connected MCP processes keep running/);
  assert.equal(currentComponent("tools", env).version, "1.1.0");
  assert.deepEqual([currentComponent("service", env).version, running.state.version, running.stops, running.starts], ["1.0.0", "1.0.0", 0, []], "the service, and its open terminal, untouched");
  assert.equal(fs.readFileSync(path.join(env.XENONET_RUNTIME_DIR, "launcher", EXE), "utf8"), fakeBin("1.1.0"), "the launcher, the new Tools' binary");

  // The same version again, or an older one: nothing changes.
  for (const again of [next, old]) {
    const same = await activatePackage(again.file, again.item, { env, operations: running.operations });
    assert.deepEqual([same.status, same.version], ["current", "1.1.0"]);
  }
  assert.equal(currentComponent("tools", env).version, "1.1.0");
});

// A crashed update's record, with 1.0.0 before and 1.1.0 after, and 1.1.0
// selected as it was when the updater stopped.
async function interrupted(t) {
  const { dir, env } = workspace(t);
  const previous = await pack(dir, "service", "1.0.0");
  const next = await pack(dir, "service", "1.1.0");
  const old = await stagePackage(previous.file, previous.item, { env });
  const staged = await stagePackage(next.file, next.item, { env });
  selectComponent("service", staged, env);
  const journal = path.join(env.XENONET_RUNTIME_DIR, "service-update.json");
  fs.writeFileSync(journal, JSON.stringify({ previous: { version: old.version, sha256: old.sha256 }, next: { version: staged.version, sha256: staged.sha256 } }));
  return { env, journal };
}

test("recovering a crashed update never stops a service that has open terminals", async (t) => {
  const { env, journal } = await interrupted(t);
  const running = service(env, { version: "0.9.0", terminals: [{ id: 2, exited: true }, { id: 3, exited: false }] });
  await assert.rejects(recoverServiceUpdate({ env, operations: running.operations }), /Close running terminals/);
  assert.deepEqual([running.stops, running.state.version], [0, "0.9.0"], "still running, terminals and all");
  assert.equal(currentComponent("service", env).version, "1.1.0", "the selection as it was");
  assert.ok(fs.existsSync(journal), "kept, to recover once the terminals are closed");

  running.state.terminals = running.state.terminals.map((x) => ({ ...x, exited: true }));
  assert.equal(await recoverServiceUpdate({ env, operations: running.operations }), "restored");
  assert.deepEqual([running.stops, running.starts, running.state.version], [1, ["1.0.0"], "1.0.0"], "the unexpected one stopped, the previous started");
  assert.equal(currentComponent("service", env).version, "1.0.0");
  assert.equal(fs.existsSync(journal), false);
  assert.equal(await recoverServiceUpdate({ env, operations: running.operations }), null, "nothing left to recover");
});

test("a recovery record that isn't one XenoNet wrote changes nothing", async (t) => {
  const { env, journal } = await interrupted(t);
  const running = service(env, { version: "1.0.0", terminals: [] });
  const record = JSON.parse(fs.readFileSync(journal, "utf8"));
  for (const bad of [{ ...record, next: { ...record.next, version: "../../x" } }, { ...record, previous: { version: "1.0.0", sha256: "not-a-hash" } }, { previous: null, next: null }]) {
    fs.writeFileSync(journal, JSON.stringify(bad));
    await assert.rejects(recoverServiceUpdate({ env, operations: running.operations }), /Invalid service update recovery record/);
  }
  assert.deepEqual([currentComponent("service", env).version, running.stops, running.starts], ["1.1.0", 0, []]);
});

test("a launcher that can't be replaced now is left as it is, with nothing half-written, and replaced next time", (t) => {
  const { dir } = workspace(t);
  const entry = path.join(dir, "launcher", "cli", "xenonet.js");
  // Something in the way that a rename can't replace.
  fs.mkdirSync(path.join(entry, "busy"), { recursive: true });
  assert.equal(writeLauncher(entry, { source: "// xenonet-launcher 7\n" }), false);
  assert.deepEqual(fs.readdirSync(path.dirname(entry)), ["xenonet.js"], "no temporary file left");
  assert.ok(fs.statSync(entry).isDirectory());
  fs.rmSync(entry, { recursive: true });
  assert.equal(writeLauncher(entry, { source: "// xenonet-launcher 7\n" }), true);
  assert.equal(fs.readFileSync(entry, "utf8"), "// xenonet-launcher 7\n");
});

test("the launcher's xenonet is a copy of the Tools' binary, made again only when the Tools change, and the service is told of it", async (t) => {
  const { dir, env } = workspace(t);
  const svc = await pack(dir, "service", "1.0.0");
  selectComponent("service", await stagePackage(svc.file, svc.item, { env }), env);
  const first = await pack(dir, "tools", "1.0.0", { bin: "binary 1.0.0" });
  selectComponent("tools", await stagePackage(first.file, first.item, { env }), env);
  const bin = path.join(env.XENONET_RUNTIME_DIR, "launcher", EXE);
  assert.equal(installLauncher(env).bin, bin);
  assert.equal(fs.readFileSync(bin, "utf8"), "binary 1.0.0");
  if (process.platform !== "win32") assert.ok(fs.statSync(bin).mode & 0o100, "and it can run");
  assert.equal(installedRuntime(env).env.XENONET_LAUNCHER_BIN, bin);
  // The same Tools again: the copy is left as it is.
  fs.writeFileSync(bin, "the copy, as it was");
  installLauncher(env);
  assert.equal(fs.readFileSync(bin, "utf8"), "the copy, as it was");
  // Newer Tools: copied again, with no temporary file left.
  const next = await pack(dir, "tools", "1.1.0", { bin: "binary 1.1.0" });
  await activatePackage(next.file, next.item, { env, operations: service(env, { version: "1.0.0" }).operations });
  assert.equal(fs.readFileSync(bin, "utf8"), "binary 1.1.0");
  assert.ok(!fs.readdirSync(path.dirname(bin)).some((f) => f.endsWith(".tmp")));
});

// Moving from 0.3.0, whose packages were a node and XenoNet's JS
// (earlierInstall lays one out as it left it).

test("a bundle replaces a Service and Tools from before 0.4.0, which have no binary, starts the new service and keeps the old files", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  const earlier = earlierInstall(env);
  assert.deepEqual([hasBinary(earlier.service), hasBinary(earlier.tools)], [false, false]);
  const svc = service(env, null);
  const runtime = await installBundle(await bundle(dir, "bundle-1", [["service", "0.4.0"], ["tools", "0.4.0"]]), { env, command: false, operations: svc.operations });
  const [now, tools] = [currentComponent("service", env), currentComponent("tools", env)];
  assert.deepEqual([now.version, tools.version, hasBinary(now), hasBinary(tools)], ["0.4.0", "0.4.0", true, true]);
  assert.deepEqual([svc.starts, svc.state.version], [["0.4.0"], "0.4.0"], "started, and checked it answers as 0.4.0");
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  assert.equal(svc.runtimes[0].env.XENONET_LAUNCHER_BIN, path.join(launcher, EXE), "after the Tools, so it names their launcher");
  assert.deepEqual([runtime.exec, runtime.args], [path.join(now.root, BIN), ["serve"]]);
  assert.equal(fs.readFileSync(path.join(launcher, EXE), "utf8"), fakeBin("0.4.0"));
  assert.ok(launcherVersion(fs.readFileSync(path.join(launcher, "cli", "xenonet.js"), "utf8")) >= 3, "the node launcher 0.3.0's entries run now runs the binary");
  assert.equal(fs.readFileSync(path.join(launcher, NODE), "utf8"), "0.3.0's node");
  for (const c of ["service", "tools"]) assert.equal(fs.readFileSync(path.join(earlier[c].root, NODE), "utf8"), "0.3.0's node", `0.3.0's ${c} kept, for what still runs it`);
  assert.equal(fs.existsSync(path.join(env.XENONET_RUNTIME_DIR, "service-update.json")), false);
});

test("a 0.3.0 service still running is stopped for the bundle's, unless a terminal is open in it, when it runs on until the next try", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  const earlier = earlierInstall(env);
  const svc = service(env, { version: "0.3.0", terminals: [{ id: 1, exited: false }] });
  const b = await bundle(dir, "bundle-1", [["service", "0.4.0"], ["tools", "0.4.0"]]);
  const runtime = await installBundle(b, { env, command: false, operations: svc.operations });
  assert.deepEqual([currentComponent("service", env).version, currentComponent("tools", env).version], ["0.3.0", "0.4.0"], "the Tools switch; the service waits");
  assert.deepEqual([svc.stops, svc.starts, svc.state.version], [0, [], "0.3.0"], "its terminal untouched");
  // Meanwhile the 0.3.0 service runs as 0.3.0 ran it, told of the node
  // launcher, so what it writes for hooks and MCP servers stays put.
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  assert.deepEqual([runtime.exec, runtime.args], [path.join(earlier.service.root, NODE), [path.join(earlier.service.root, "server", "index.js")]]);
  assert.deepEqual([runtime.env.XENONET_LAUNCHER_EXEC, runtime.env.XENONET_LAUNCHER_ROOT, runtime.env.XENONET_LAUNCHER_BIN], [path.join(launcher, NODE), launcher, path.join(launcher, EXE)]);

  svc.state.terminals[0].exited = true;
  await installBundle(b, { env, command: false, operations: svc.operations });
  assert.deepEqual([svc.stops, svc.starts, svc.state.version, currentComponent("service", env).version], [1, ["0.4.0"], "0.4.0", "0.4.0"]);
});

test("a bundle's service that doesn't start brings back the one from before 0.4.0, run with its node", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  const earlier = earlierInstall(env);
  const svc = service(env, { version: "0.3.0", terminals: [] }, { failing: "0.4.0" });
  await assert.rejects(installBundle(await bundle(dir, "bundle-1", [["service", "0.4.0"], ["tools", "0.4.0"]]), { env, command: false, operations: svc.operations }), /previous version was restored\. XenoNet Service exited/);
  assert.equal(currentComponent("service", env).version, "0.3.0");
  assert.deepEqual([svc.stops, svc.starts, svc.state.version], [1, ["0.4.0", "0.3.0"], "0.3.0"]);
  assert.deepEqual([svc.runtimes[1].exec, svc.runtimes[1].args], [path.join(earlier.service.root, NODE), [path.join(earlier.service.root, "server", "index.js")]]);
  assert.equal(fs.existsSync(path.join(env.XENONET_RUNTIME_DIR, "service-update.json")), false);
});

test("a bundle replaces only the component from before 0.4.0, never one with its binary", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env, { components: ["tools"] });
  const svc = await pack(dir, "service", "0.4.0");
  selectComponent("service", await stagePackage(svc.file, svc.item, { env }), env);
  const running = service(env, { version: "0.4.0", terminals: [] });
  await installBundle(await bundle(dir, "bundle-1", [["service", "0.5.0"], ["tools", "0.5.0"]]), { env, command: false, operations: running.operations });
  assert.deepEqual([currentComponent("service", env).version, currentComponent("tools", env).version], ["0.4.0", "0.5.0"]);
  assert.deepEqual([running.stops, running.starts], [0, []]);
});

test("an update from the release replaces a Service and Tools from before 0.4.0, as it does any older one", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const svc = service(env, { version: "0.3.0", terminals: [] });
  const tools = await pack(dir, "tools", "0.4.0");
  assert.equal((await activatePackage(tools.file, tools.item, { env, operations: svc.operations })).status, "updated");
  const next = await pack(dir, "service", "0.4.0");
  assert.equal((await activatePackage(next.file, next.item, { env, operations: svc.operations })).status, "updated");
  assert.deepEqual([svc.stops, svc.starts, svc.state.version], [1, ["0.4.0"], "0.4.0"]);
  assert.equal(svc.runtimes[0].env.XENONET_LAUNCHER_BIN, path.join(env.XENONET_RUNTIME_DIR, "launcher", EXE));
});
