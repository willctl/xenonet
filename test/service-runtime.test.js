// The service Desktop runs (electron/service-runtime.js): the Rust binary,
// which one that is, and what Desktop says when there's none; and, in a
// packaged XenoNet, the Service and Tools the xenonet binary gets ready
// first (xenonet components). The packages are made up
// (test/harness/components.js), with a shell script that answers the
// startup check as their binary, and the installer is this checkout's
// build, in a runtime folder of the test's own.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { rustBinary, missingBinary, serviceCommand, readyComponents, installedRuntime, earlierNotice } from "../electron/service-runtime.js";
import { componentsBin, components } from "../electron/lib/components/run.js";
import { posixOnly, pack, bundle } from "./harness/components.js";
import { currentComponent } from "./harness/component-fixture.js";
import { builtBinary } from "../scripts/xenonet.mjs";

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

// The installer: this checkout's build, which `npm run rust:bin` makes.
const xenonet = builtBinary();
const real = posixOnly.skip ? posixOnly : { skip: !xenonet && "no xenonet binary: npm run rust:bin" };

function workspace(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-desktop-start-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  // Nothing of the machine's own: its runtime folder, port and PATH.
  const env = { ...process.env, XENONET_RUNTIME_DIR: path.join(dir, "runtime"), XENONET_PORT: "47979", PATH: "/usr/bin:/bin" };
  delete env.XENONET_LAUNCHER_BIN;
  delete env.XENONET_LAUNCHER_ROOT;
  return { dir, env };
}

// A bundle as the installer ships it, with the binary that installs it.
async function shipped(dir, packages) {
  const out = await bundle(dir, "bundle", packages);
  fs.copyFileSync(xenonet, path.join(out, path.basename(xenonet)));
  return out;
}

test("the installer is the bundle's own binary, else the launcher's", () => {
  const launcher = path.join("/rt", "launcher", "xenonet");
  const env = { XENONET_RUNTIME_DIR: "/rt" };
  assert.equal(componentsBin({ env, bundle: "/b", platform: "darwin", exists: (f) => [path.join("/b", "xenonet"), launcher].includes(f) }), path.join("/b", "xenonet"));
  assert.equal(componentsBin({ env, bundle: null, platform: "darwin", exists: (f) => f === launcher }), launcher, "a Desktop-only update has no bundle");
  assert.equal(componentsBin({ env, bundle: "/b", platform: "darwin", exists: () => false }), null);
});

test("with no installer, Desktop says to install the bundle", async () => {
  await assert.rejects(components(null, ["ready"]), /Install the XenoNet bundle/);
});

test("Desktop's first start: the bundle's Service and Tools are installed, and the service runs from the installed Service", real, async (t) => {
  const { dir, env } = workspace(t);
  const b = await shipped(dir, [["service", "9.9.1"], ["tools", "9.9.1"]]);
  assert.deepEqual(await readyComponents({ env, bundle: b, test: true }), { earlier: [], waiting: false, problems: [] });
  for (const c of ["service", "tools"]) assert.equal(currentComponent(xenonet, c, env).version, "9.9.1", c);
  const runtime = await installedRuntime({ env, bundle: b });
  // From the Service's stable path, which an update points at the new one.
  assert.equal(runtime.exec, path.join(env.XENONET_RUNTIME_DIR, "components", "service", "active", "bin", "xenonet"));
  assert.ok(fs.existsSync(runtime.exec));
  assert.deepEqual(runtime.args, ["serve"]);
  // Started again, nothing changes: a bundle never reinstalls what's there.
  assert.deepEqual(await readyComponents({ env, bundle: b, test: true }), { earlier: [], waiting: false, problems: [] });
});

test("a newer Tools from a release switches at once", real, async (t) => {
  const { dir, env } = workspace(t);
  const b = await shipped(dir, [["service", "9.9.1"], ["tools", "9.9.1"]]);
  await readyComponents({ env, bundle: b, test: true });
  const out = path.join(dir, "release");
  fs.mkdirSync(out);
  const next = await pack(out, "tools", "9.9.2");
  const result = await components(xenonet, ["activate", next.file, JSON.stringify(next.item)], { env });
  assert.equal(result.status, "updated");
  assert.equal(currentComponent(xenonet, "tools", env).version, "9.9.2");
});

test("a test instance never downloads: a bundle missing a component stops Desktop, with its reason", real, async (t) => {
  const { dir, env } = workspace(t);
  const b = await shipped(dir, [["tools", "9.9.1"]]);
  await assert.rejects(readyComponents({ env, bundle: b, test: true }), /The test bundle is missing its components/);
});

test("a bundle whose binary doesn't start fails Desktop's first start, and says why", real, async (t) => {
  const { dir, env } = workspace(t);
  const b = await shipped(dir, [["service", "9.9.1", { fail: "runtime" }], ["tools", "9.9.1"]]);
  await assert.rejects(readyComponents({ env, bundle: b, test: true }));
  assert.equal(currentComponent(xenonet, "service", env), null);
});

test("what Desktop says while a Service or Tools from before 0.4.0 still runs", () => {
  assert.equal(earlierNotice({ earlier: [], waiting: false, problems: [] }), null);
  assert.match(earlierNotice({ earlier: ["service"], waiting: true, problems: [] }).title, /Close your terminals/);
  const failed = earlierNotice({ earlier: ["service", "tools"], waiting: false, problems: ["The download failed"] });
  assert.match(failed.body, /^The download failed\. The Service and Tools you had keep running/);
});
