// The service Desktop runs (electron/service-runtime.js): always the Rust
// binary, which one that is, and what Desktop says when there's none.
import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { rustBinary, missingBinary } from "../electron/service-runtime.js";

test("the binary: named, installed, or the checkout's newer build", () => {
  const root = "/src/xenonet";
  // Each file that exists, with when it was written.
  const have = (files) => (f) => {
    if (!(f in files)) throw Object.assign(new Error("ENOENT"), { code: "ENOENT" });
    return { isFile: () => true, mtimeMs: files[f] };
  };
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
