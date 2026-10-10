// Installing XenoNet's components (core/components/install.js) beyond the
// service switch component-activation.test.js covers: a bundle installs
// what's missing and never upgrades what's there, a runtime that fails its
// startup check is never selected, a Tools update leaves the service
// alone, recovery stops only an unexpected service and never one with open
// terminals, and a launcher that can't be replaced now is left for next
// time. Packages are made up; their "node" is a shell script that answers
// the startup check, so the bundle tests run where a script can stand in
// for a program.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import * as tar from "tar";
import { activatePackage, checkRuntime, installBundle, recoverServiceUpdate, writeLauncher } from "../core/components/install.js";
import { currentComponent, digest, selectComponent, stagePackage } from "../core/components/packages.js";

const NODE = process.platform === "win32" ? "node.exe" : "node";
const posixOnly = { skip: process.platform === "win32" ? "a shell script stands in for node" : false };

// A "node" that passes the startup check as `version`, or fails it: `fail`
// "runtime" exits 1 for the module check, "command" reports another
// version.
const fakeNode = (version, fail = null) => `#!/bin/sh
if [ "$1" = "--input-type=module" ]; then
  ${fail === "runtime" ? "echo 'node:sqlite missing' >&2; exit 1" : "printf ok"}
else
  printf '{"version":"${fail === "command" ? "0.0.1" : version}"}'
fi
`;

function workspace(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-install-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return { dir, env: { XENONET_RUNTIME_DIR: path.join(dir, "runtime") } };
}

// A component package in `out`: { file, item } as a release manifest
// lists it.
async function pack(out, component, version, { fail = null, platform = process.platform, arch = process.arch } = {}) {
  const app = fs.mkdtempSync(path.join(out, `${component}-${version}-`));
  const meta = { format: 1, component, version, platform, arch, dataSchema: 1, api: { min: 1, max: 1 } };
  fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
  fs.writeFileSync(path.join(app, NODE), fakeNode(version, fail), { mode: 0o755 });
  fs.mkdirSync(path.join(app, "server"));
  fs.mkdirSync(path.join(app, "cli"));
  fs.writeFileSync(path.join(app, "server", "index.js"), "// a made-up service");
  fs.writeFileSync(path.join(app, "cli", "xenonet.js"), "// made-up tools");
  const name = `XenoNet-${component}-${version}-${platform}-${arch}.tar.gz`;
  const file = path.join(out, name);
  await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app));
  const bytes = fs.readFileSync(file);
  return { file, item: { ...meta, file: name, size: bytes.length, sha256: digest(bytes) } };
}

// A bundle folder as the installer ships it: the packages and their
// components.json.
async function bundle(dir, name, packages) {
  const out = path.join(dir, name);
  fs.mkdirSync(out);
  const items = [];
  for (const [component, version, options] of packages) items.push((await pack(out, component, version, options)).item);
  fs.writeFileSync(path.join(out, "components.json"), JSON.stringify({ format: 1, packages: items }));
  return out;
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
  assert.equal(runtime.exec, path.join(service.root, NODE));
  const launcher = path.join(env.XENONET_RUNTIME_DIR, "launcher");
  assert.deepEqual([runtime.env.XENONET_RUNTIME_DIR, runtime.env.XENONET_LAUNCHER_ROOT, runtime.env.XENONET_LAUNCHER_EXEC], [env.XENONET_RUNTIME_DIR, launcher, path.join(launcher, NODE)]);
  assert.equal(fs.readFileSync(path.join(launcher, NODE), "utf8"), fakeNode("1.0.0"), "the launcher's node is a copy of the Tools' own");
  assert.match(fs.readFileSync(path.join(launcher, "cli", "xenonet.js"), "utf8"), /^\/\/ xenonet-launcher \d+/);
  assert.ok(fs.statSync(path.join(launcher, NODE)).mode & 0o100, "and it can run");
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

test("the startup check runs the staged node itself, without ELECTRON_RUN_AS_NODE, and wants the version it was promised", posixOnly, async (t) => {
  const { dir } = workspace(t);
  const root = path.join(dir, "staged");
  fs.mkdirSync(root);
  // A node that fails unless it's run without the variable Electron's
  // runtime would pass on.
  fs.writeFileSync(path.join(root, NODE), `#!/bin/sh\n[ -n "$ELECTRON_RUN_AS_NODE" ] && exit 3\n${fakeNode("1.4.2").split("\n").slice(1).join("\n")}`, { mode: 0o755 });
  const saved = process.env.ELECTRON_RUN_AS_NODE;
  process.env.ELECTRON_RUN_AS_NODE = "1";
  try {
    assert.equal(checkRuntime({ root, version: "1.4.2" }), undefined);
    assert.throws(() => checkRuntime({ root, version: "1.4.3" }), /command did not pass its startup check/);
  } finally {
    if (saved === undefined) delete process.env.ELECTRON_RUN_AS_NODE;
    else process.env.ELECTRON_RUN_AS_NODE = saved;
  }
  fs.rmSync(path.join(root, NODE));
  assert.throws(() => checkRuntime({ root, version: "1.4.2" }), /runtime did not pass its startup check/, "no node at all");
});

// A service that answers status and stop, for activatePackage and
// recoverServiceUpdate: `state` is what status says, null when it's down.
function service(env, state) {
  const s = { state, stops: 0, starts: [] };
  s.operations = {
    check() {},
    now: () => 0,
    pause: async () => {},
    stopTimeout: 0,
    async connect() {
      if (!s.state) throw new Error("not running");
      return {
        close() {},
        async request(method) {
          if (method === "status") return s.state;
          s.stops++;
          s.state = null;
          return { stopping: true };
        },
      };
    },
    async start() {
      const version = currentComponent("service", env).version;
      s.starts.push(version);
      s.state = { version, terminals: [] };
    },
  };
  return s;
}

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
  assert.ok(fs.existsSync(path.join(env.XENONET_RUNTIME_DIR, "launcher", "cli", "xenonet.js")));

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
