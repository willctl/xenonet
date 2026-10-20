import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import * as tar from "tar";
import { digest, stagePackage, selectComponent, currentComponent, withInstallLock } from "../core/components/packages.js";

const BIN = path.join("bin", process.platform === "win32" ? "xenonet.exe" : "xenonet");

async function fixture(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-package-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const app = path.join(dir, "app");
  fs.mkdirSync(path.join(app, "bin"), { recursive: true });
  const meta = { format: 1, component: "tools", version: "1.2.3", platform: process.platform, arch: process.arch, dataSchema: 1, api: { min: 1, max: 1 } };
  fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
  fs.writeFileSync(path.join(app, BIN), "the xenonet binary");
  const file = path.join(dir, "component.tar.gz");
  const pack = async () => { await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app)); const bytes = fs.readFileSync(file); return { ...meta, size: bytes.length, sha256: digest(bytes) }; };
  return { app, file, pack, meta, env: { XENONET_RUNTIME_DIR: path.join(dir, "installed") } };
}

test("staging validates a download before switching the selected component", async (t) => {
  const f = await fixture(t);
  const item = await f.pack();
  const staged = await stagePackage(f.file, item, f);
  assert.equal(currentComponent("tools", f.env), null);
  selectComponent("tools", staged, f.env);
  assert.equal(currentComponent("tools", f.env).version, "1.2.3");
  fs.appendFileSync(f.file, "bad download");
  await assert.rejects(stagePackage(f.file, item, f), /hash or size/);
  assert.equal(currentComponent("tools", f.env).sha256, item.sha256);
});

test("a package without the xenonet binary is refused, and one with only the binary is enough", async (t) => {
  const f = await fixture(t);
  fs.rmSync(path.join(f.app, "bin"), { recursive: true });
  fs.mkdirSync(path.join(f.app, "cli"));
  fs.writeFileSync(path.join(f.app, "cli", "xenonet.js"), "// an earlier XenoNet's command");
  fs.writeFileSync(path.join(f.app, process.platform === "win32" ? "node.exe" : "node"), "an earlier XenoNet's runtime");
  await assert.rejects(stagePackage(f.file, await f.pack(), f), /Component is missing bin\/xenonet/);
  assert.equal(currentComponent("tools", f.env), null);
  fs.rmSync(f.app, { recursive: true });
  fs.mkdirSync(path.join(f.app, "bin"), { recursive: true });
  fs.writeFileSync(path.join(f.app, "component.json"), JSON.stringify(f.meta));
  fs.writeFileSync(path.join(f.app, BIN), "the xenonet binary");
  const staged = await stagePackage(f.file, await f.pack(), f);
  assert.deepEqual(fs.readdirSync(staged.root).sort(), ["bin", "component.json"]);
});

test("an incompatible package and a changed published version cannot replace installed tools", async (t) => {
  const f = await fixture(t);
  let item = await f.pack();
  selectComponent("tools", await stagePackage(f.file, item, f), f.env);
  fs.writeFileSync(path.join(f.app, BIN), "a changed binary");
  await assert.rejects(stagePackage(f.file, await f.pack(), f), /cannot be replaced/);
  f.meta.version = "1.2.4"; f.meta.api = { min: 2, max: 2 };
  fs.writeFileSync(path.join(f.app, "component.json"), JSON.stringify(f.meta));
  item = await f.pack();
  await assert.rejects(stagePackage(f.file, item, f), /incompatible/);
  assert.equal(currentComponent("tools", f.env).version, "1.2.3");
});

test("archives cannot install links and concurrent updates cannot overwrite each other", async (t) => {
  const f = await fixture(t);
  const source = path.join(f.app, BIN);
  fs.linkSync(source, path.join(f.app, "another-xenonet"));
  await assert.rejects(stagePackage(f.file, await f.pack(), f), /Unsafe/);
  await withInstallLock(f.env, async () => {
    await assert.rejects(withInstallLock(f.env, () => {}), /in progress/);
  });
  await withInstallLock(f.env, () => {});
});
