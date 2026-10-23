import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import * as tar from "tar";
import { digest, stagePackage, selectComponent, currentComponent, withInstallLock, fitsHere, TRAY, TRAY_APP, TRAY_EXE } from "../core/components/packages.js";

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

test("the Mac's tray is one package for both CPUs, checked for its bundle's executable, and never another system's", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-package-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const app = path.join(dir, "app");
  const meta = { format: 1, component: TRAY, version: "0.4.1", platform: "darwin", arch: "universal", dataSchema: 1, api: { min: 1, max: 1 } };
  fs.mkdirSync(path.join(app, path.dirname(TRAY_EXE)), { recursive: true });
  fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
  const file = path.join(dir, "tray.tar.gz");
  const pack = async () => { await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app)); const bytes = fs.readFileSync(file); return { ...meta, size: bytes.length, sha256: digest(bytes) }; };
  const env = { XENONET_RUNTIME_DIR: path.join(dir, "installed") };
  for (const arch of ["arm64", "x64"]) assert.ok(fitsHere(meta, { platform: "darwin", arch }), arch);
  assert.ok(!fitsHere(meta, { platform: "linux", arch: "x64" }) && !fitsHere(meta, { platform: "win32", arch: "x64" }));
  assert.ok(!fitsHere({ ...meta, arch: "arm64" }, { platform: "darwin", arch: "arm64" }), "the tray is always both CPUs'");
  assert.ok(!fitsHere({ ...meta, component: "service" }, { platform: "darwin", arch: "arm64" }), "and nothing else is universal");
  assert.ok(!fitsHere(null));
  await assert.rejects(stagePackage(file, await pack(), { env, platform: "darwin", arch: "arm64" }), /Component is missing XenoNet Tray\.app\/Contents\/MacOS\/XenoNet Tray\./);
  fs.writeFileSync(path.join(app, TRAY_EXE), "the tray");
  const item = await pack();
  await assert.rejects(stagePackage(file, item, { env, platform: "linux", arch: "x64" }), /not compatible/);
  const staged = await stagePackage(file, item, { env, platform: "darwin", arch: "arm64" });
  selectComponent(TRAY, staged, env);
  assert.equal(currentComponent(TRAY, env).version, "0.4.1");
  assert.equal(fs.readFileSync(path.join(currentComponent(TRAY, env).root, TRAY_EXE), "utf8"), "the tray");
  assert.equal(TRAY_APP, "XenoNet Tray.app");
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
