// The service Desktop runs (electron/service-runtime.js): the Rust binary,
// which one that is, and what Desktop says when there's none; and, in a
// packaged XenoNet, the Service and Tools it gets ready first, from its
// bundle or the latest release, including over an install 0.3.0 left,
// whose packages were a node and XenoNet's JS. Installs and packages are
// made up (test/harness/components.js) and the service is a fake that
// answers status and stop.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { rustBinary, missingBinary, serviceCommand, readyComponents, earlierNotice } from "../electron/service-runtime.js";
import { installedRuntime } from "../core/components/install.js";
import { currentComponent } from "../core/components/packages.js";
import { NODE, posixOnly, pack, bundle, fakeService, earlierInstall } from "./harness/components.js";

// Each file that exists, with when it was written.
const have = (files) => (f) => {
  if (!(f in files)) throw Object.assign(new Error("ENOENT"), { code: "ENOENT" });
  return { isFile: () => true, mtimeMs: files[f] };
};

test("the binary: named, installed, or the checkout's newer build", () => {
  const root = "/src/xenonet";
  const release = path.join(root, "target", "release", "xenonet");
  const debug = path.join(root, "target", "debug", "xenonet");
  const installed = path.join("/svc", "bin", "xenonet");
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", stat: have({ [release]: 2, [debug]: 1 }) }), release);
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", stat: have({ [release]: 1, [debug]: 2 }) }), debug, "a stale release build doesn't win");
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", stat: have({ [debug]: 1 }) }), debug);
  assert.equal(rustBinary({ env: {}, root, serviceRoot: "/svc", platform: "darwin", stat: have({ [installed]: 1, [release]: 9 }) }), installed);
  assert.equal(rustBinary({ env: { XENONET_SERVICE_BIN: "/x/xenonet" }, root, platform: "darwin", stat: have({ "/x/xenonet": 1, [release]: 9 }) }), "/x/xenonet");
  assert.equal(rustBinary({ env: {}, root, platform: "win32", stat: have({ [path.join(root, "target", "release", "xenonet.exe")]: 1 }) }), path.join(root, "target", "release", "xenonet.exe"));
  assert.equal(rustBinary({ env: {}, root: null, serviceRoot: "/svc", platform: "darwin", stat: have({}) }), null, "a packaged XenoNet doesn't look for a checkout's build");
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", stat: have({}) }), null);
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", stat: () => ({ isFile: () => false, mtimeMs: 1 }) }), null, "a folder isn't a binary");
});

test("no binary is an error that says how to get one", () => {
  assert.match(missingBinary({ fromSource: true }), /npm run rust:bin/);
  assert.match(missingBinary({ fromSource: false }), /Reinstall XenoNet/);
});

test("how the service starts: the installed binary serving, a checkout's with --dev, or a 0.3.0 Service's node", () => {
  const bin = path.join("/svc", "bin", "xenonet");
  const current = { root: "/svc", exec: bin, args: ["serve"], env: {} };
  assert.deepEqual(serviceCommand({ env: {}, installed: current, platform: "darwin", stat: have({ [bin]: 1 }) }), { root: "/svc", exec: bin, args: ["serve"] });
  const debug = path.join("/src", "target", "debug", "xenonet");
  assert.deepEqual(serviceCommand({ env: {}, root: "/src", dev: true, platform: "darwin", stat: have({ [debug]: 1 }) }), { root: "/src", exec: debug, args: ["serve", "--dev"] });
  // One from before 0.4.0, still selected while its update waits: its node
  // and server, as installedRuntime says, and nothing when they're gone.
  const earlier = { root: "/old", exec: path.join("/old", "node"), args: [path.join("/old", "server", "index.js")], env: {} };
  assert.deepEqual(serviceCommand({ env: {}, installed: earlier, platform: "darwin", stat: have({ [earlier.exec]: 1 }) }), { root: "/old", exec: earlier.exec, args: earlier.args });
  assert.equal(serviceCommand({ env: {}, installed: earlier, platform: "darwin", stat: have({}) }), null);
  assert.equal(serviceCommand({ env: { XENONET_SERVICE_BIN: "/x/xenonet" }, installed: earlier, platform: "darwin", stat: have({ "/x/xenonet": 1 }) }).exec, "/x/xenonet", "a binary named still wins");
});

function workspace(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-desktop-start-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return { dir, env: { XENONET_RUNTIME_DIR: path.join(dir, "runtime") } };
}

// The latest release, as componentUpdates and downloadComponent fetch it:
// its components.json and the packages it lists, from `packages`. `asked`
// counts the requests; `offline` fails them all.
async function release(dir, packages, { offline = false } = {}) {
  const out = path.join(dir, "release");
  fs.mkdirSync(out);
  const items = [];
  for (const [component, version] of packages) items.push((await pack(out, component, version)).item);
  const r = { asked: 0 };
  r.fetcher = async (url) => {
    r.asked++;
    if (offline) throw new TypeError("fetch failed");
    const name = url.split("/").pop();
    if (name === "components.json") return { ok: true, status: 200, json: async () => ({ format: 1, packages: items }) };
    return { ok: true, status: 200, body: Readable.from([fs.readFileSync(path.join(out, name))]) };
  };
  return r;
}

const selected = (env) => ["service", "tools"].map((c) => currentComponent(c, env)?.version ?? null);

test("Desktop with a bundle over 0.3.0: the bundle's Service and Tools replace it, the old service stops, and the new one starts", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const svc = fakeService(env, { version: "0.3.0", terminals: [] });
  const latest = await release(dir, [["service", "0.4.0"], ["tools", "0.4.0"]]);
  const ready = await readyComponents({ env, bundle: await bundle(dir, "bundle", [["service", "0.4.0"], ["tools", "0.4.0"]]), test: true, operations: svc.operations, fetcher: latest.fetcher });
  assert.deepEqual(ready, { earlier: [], waiting: false, problems: [] });
  assert.equal(earlierNotice(ready), null);
  assert.deepEqual(selected(env), ["0.4.0", "0.4.0"]);
  assert.deepEqual([svc.stops, svc.starts, svc.state.version], [1, ["0.4.0"], "0.4.0"]);
  assert.equal(latest.asked, 0, "nothing downloaded: the bundle brought both");
  const installed = installedRuntime(env);
  assert.deepEqual(serviceCommand({ env: {}, installed }), { root: installed.root, exec: installed.exec, args: ["serve"] }, "and Desktop has a binary to run, not 'binary is missing'");
});

test("Desktop with a bundle over a 0.3.0 service with a terminal open: it keeps that service until the terminal closes, and says so", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  const earlier = earlierInstall(env);
  const svc = fakeService(env, { version: "0.3.0", terminals: [{ id: 1, exited: false }] });
  const latest = await release(dir, [["service", "0.4.0"], ["tools", "0.4.0"]]);
  const b = await bundle(dir, "bundle", [["service", "0.4.0"], ["tools", "0.4.0"]]);
  const ready = await readyComponents({ env, bundle: b, test: true, operations: svc.operations, fetcher: latest.fetcher });
  assert.deepEqual(ready, { earlier: ["service"], waiting: true, problems: [] });
  assert.match(earlierNotice(ready).body, /Close them, then quit and reopen XenoNet/);
  assert.deepEqual(selected(env), ["0.3.0", "0.4.0"]);
  assert.deepEqual([svc.stops, latest.asked], [0, 0]);
  const command = serviceCommand({ env: {}, installed: installedRuntime(env) });
  assert.deepEqual(command, { root: earlier.service.root, exec: path.join(earlier.service.root, NODE), args: [path.join(earlier.service.root, "server", "index.js")] }, "the service it had, if it has to start it again");

  // Reopened once the terminal is closed.
  svc.state.terminals[0].exited = true;
  assert.deepEqual(await readyComponents({ env, bundle: b, test: true, operations: svc.operations }), { earlier: [], waiting: false, problems: [] });
  assert.deepEqual([selected(env), svc.stops, svc.starts], [["0.4.0", "0.4.0"], 1, ["0.4.0"]]);
});

test("Desktop with a bundle whose service doesn't start: the 0.3.0 one is back and running, Desktop starts with it and says why", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const svc = fakeService(env, { version: "0.3.0", terminals: [] }, { failing: "0.4.0" });
  const latest = await release(dir, [["service", "0.4.0"], ["tools", "0.4.0"]]);
  const ready = await readyComponents({ env, bundle: await bundle(dir, "bundle", [["service", "0.4.0"], ["tools", "0.4.0"]]), test: true, operations: svc.operations, fetcher: latest.fetcher });
  assert.deepEqual([ready.earlier, ready.waiting], [["service"], false]);
  assert.match(ready.problems[0], /previous version was restored/);
  assert.match(earlierNotice(ready).body, /The Service you had keeps running/);
  assert.deepEqual([svc.state.version, latest.asked], ["0.3.0", 0]);
});

test("Desktop-only update over 0.3.0: the Service and Tools come from the latest release, Tools first, and the old service stops for the new one", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const svc = fakeService(env, { version: "0.3.0", terminals: [] });
  const latest = await release(dir, [["service", "0.4.0"], ["tools", "0.4.0"]]);
  assert.deepEqual(await readyComponents({ env, operations: svc.operations, fetcher: latest.fetcher }), { earlier: [], waiting: false, problems: [] });
  assert.deepEqual(selected(env), ["0.4.0", "0.4.0"]);
  assert.deepEqual([svc.stops, svc.starts], [1, ["0.4.0"]]);
  assert.ok(svc.runtimes[0].env.XENONET_LAUNCHER_BIN, "the Tools were in place when the service started");

  // A second start has nothing to fetch.
  const asked = latest.asked;
  await readyComponents({ env, operations: svc.operations, fetcher: latest.fetcher });
  assert.equal(latest.asked, asked);
});

test("Desktop-only update over 0.3.0 with a terminal open: the Tools update, the service waits", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const svc = fakeService(env, { version: "0.3.0", terminals: [{ id: 1, exited: false }] });
  const latest = await release(dir, [["service", "0.4.0"], ["tools", "0.4.0"]]);
  assert.deepEqual(await readyComponents({ env, operations: svc.operations, fetcher: latest.fetcher }), { earlier: ["service"], waiting: true, problems: [] });
  assert.deepEqual([selected(env), svc.stops], [["0.3.0", "0.4.0"], 0]);
});

test("Desktop-only update over 0.3.0 while offline: Desktop starts with what it had and says so; with nothing installed it stops", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  earlierInstall(env);
  const offline = await release(dir, [], { offline: true });
  const ready = await readyComponents({ env, operations: fakeService(env, null).operations, fetcher: offline.fetcher });
  assert.deepEqual([ready.earlier, ready.waiting], [["service", "tools"], false]);
  assert.match(earlierNotice(ready).body, /The Service and Tools you had keep running\. Quit and reopen XenoNet to try again\./);
  assert.deepEqual(selected(env), ["0.3.0", "0.3.0"]);

  const fresh = { XENONET_RUNTIME_DIR: path.join(dir, "fresh") };
  await assert.rejects(readyComponents({ env: fresh, operations: fakeService(fresh, null).operations, fetcher: offline.fetcher }), /fetch failed/);
});

test("a packaged test instance never downloads, and a bundle that fails a first install still stops Desktop, with its reason", posixOnly, async (t) => {
  const { dir, env } = workspace(t);
  await assert.rejects(readyComponents({ env, bundle: await bundle(dir, "tools-only", [["tools", "0.4.0"]]), test: true }), /test bundle is missing its components/);
  const other = { XENONET_RUNTIME_DIR: path.join(dir, "other") };
  const offline = await release(dir, [], { offline: true });
  await assert.rejects(readyComponents({ env: other, bundle: await bundle(dir, "broken", [["service", "0.4.0", { fail: "runtime" }], ["tools", "0.4.0"]]), fetcher: offline.fetcher }), /runtime did not pass its startup check/);
});
