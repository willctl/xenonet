// Which service Desktop starts (electron/service-runtime.js): the JS one
// unless XENONET_SERVICE_RUNTIME or the preference asks for the Rust binary,
// and which binary that is.
import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { serviceRuntime, rustBinary } from "../electron/service-runtime.js";

test("the JS service unless the Rust one is asked for", () => {
  assert.equal(serviceRuntime({ env: {} }), "node");
  assert.equal(serviceRuntime({ env: { XENONET_SERVICE_RUNTIME: "rust" } }), "rust");
  assert.equal(serviceRuntime({ env: { XENONET_SERVICE_RUNTIME: " Rust " } }), "rust");
  assert.equal(serviceRuntime({ env: {}, pref: "rust" }), "rust", "the desktop's preference");
  assert.equal(serviceRuntime({ env: { XENONET_SERVICE_RUNTIME: "node" }, pref: "rust" }), "node", "the environment first");
  assert.equal(serviceRuntime({ env: { XENONET_SERVICE_RUNTIME: "go" } }), "node", "one it doesn't know");
});

test("the binary: named, installed, or built in the checkout, release first", () => {
  const root = "/src/xenonet";
  const have = (...files) => (f) => files.includes(f);
  const release = path.join(root, "target", "release", "xenonet");
  const debug = path.join(root, "target", "debug", "xenonet");
  const installed = path.join("/svc", "bin", "xenonet");
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", exists: have(release, debug) }), release);
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", exists: have(debug) }), debug);
  assert.equal(rustBinary({ env: {}, root, serviceRoot: "/svc", platform: "darwin", exists: have(installed, release) }), installed);
  assert.equal(rustBinary({ env: { XENONET_SERVICE_BIN: "/x/xenonet" }, root, platform: "darwin", exists: have("/x/xenonet", release) }), "/x/xenonet");
  assert.equal(rustBinary({ env: {}, root, platform: "win32", exists: have(path.join(root, "target", "release", "xenonet.exe")) }), path.join(root, "target", "release", "xenonet.exe"));
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", exists: () => false }), null);
  assert.equal(rustBinary({ env: {}, root, platform: "darwin", exists: () => { throw new Error("EACCES"); } }), null);
});
