import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import * as tar from "tar";
import { activatePackage, recoverServiceUpdate } from "../core/components/install.js";
import { digest, stagePackage, selectComponent, currentComponent } from "../core/components/packages.js";

async function fixture(t, { terminals = [], stopWorks = true, failNew = false, wrongVersion = false } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-activation-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const env = { XENONET_RUNTIME_DIR: path.join(dir, "runtime") };
  const app = path.join(dir, "app");
  fs.mkdirSync(path.join(app, "server"), { recursive: true });
  fs.writeFileSync(path.join(app, "server/index.js"), "// test service");
  fs.writeFileSync(path.join(app, process.platform === "win32" ? "node.exe" : "node"), "test runtime");
  async function pack(version) {
    const meta = { format: 1, component: "service", version, platform: process.platform, arch: process.arch, dataSchema: 1, api: { min: 1, max: 1 } };
    fs.writeFileSync(path.join(app, "component.json"), JSON.stringify(meta));
    const file = path.join(dir, `${version}.tar.gz`);
    await tar.c({ file, cwd: app, gzip: true }, fs.readdirSync(app));
    const bytes = fs.readFileSync(file);
    return { file, item: { ...meta, size: bytes.length, sha256: digest(bytes) } };
  }
  const previous = await pack("1.0.0");
  const old = await stagePackage(previous.file, previous.item, { env });
  selectComponent("service", old, env);
  const next = await pack("1.1.0");
  let state = { version: "1.0.0", terminals };
  let stops = 0, time = 0;
  const starts = [];
  const operations = {
    check() {},
    now: () => time,
    pause: async (ms) => { time += ms; },
    stopTimeout: 200,
    async connect() {
      if (!state) throw new Error("not running");
      return { close() {}, async request(method) {
        if (method === "status") return state;
        assert.equal(method, "stop");
        stops++;
        if (stopWorks) state = null;
        return { stopping: true };
      } };
    },
    async start() {
      const selected = currentComponent("service", env).version;
      starts.push(selected);
      if (selected === "1.1.0" && failNew) throw new Error("native module unavailable");
      state ??= { version: wrongVersion && selected === "1.1.0" ? "9.9.9" : selected, terminals: [] };
    },
  };
  return { ...next, env, operations, old, starts, journal: path.join(env.XENONET_RUNTIME_DIR, "service-update.json"), get stops() { return stops; }, get state() { return state; }, set state(value) { state = value; } };
}

test("service update leaves the selected version and running terminals alone", async (t) => {
  const f = await fixture(t, { terminals: [{ id: 1, exited: false }] });
  const result = await activatePackage(f.file, f.item, f);
  assert.equal(result.status, "staged");
  assert.equal(currentComponent("service", f.env).version, "1.0.0");
  assert.equal(f.stops, 0);
  assert.deepEqual(f.starts, []);
  assert.equal(fs.existsSync(f.journal), false);
});

test("service startup failure restores and starts the previous version", async (t) => {
  const f = await fixture(t, { failNew: true });
  await assert.rejects(activatePackage(f.file, f.item, f), /previous version was restored.*native module unavailable/);
  assert.equal(currentComponent("service", f.env).version, "1.0.0");
  assert.deepEqual(f.starts, ["1.1.0", "1.0.0"]);
  assert.equal(f.state.version, "1.0.0");
  assert.equal(fs.existsSync(f.journal), false);
});

test("a service that refuses to stop cannot masquerade as a successful update", async (t) => {
  const f = await fixture(t, { stopWorks: false });
  await assert.rejects(activatePackage(f.file, f.item, f), /did not stop/);
  assert.equal(currentComponent("service", f.env).version, "1.0.0");
  assert.equal(f.state.version, "1.0.0");
  assert.ok(!f.starts.includes("1.1.0"));
});

test("an unexpected running version fails the health check and rolls back", async (t) => {
  const f = await fixture(t, { wrongVersion: true });
  await assert.rejects(activatePackage(f.file, f.item, f), /Expected XenoNet Service 1.1.0, but 9.9.9/);
  assert.equal(currentComponent("service", f.env).version, "1.0.0");
  assert.equal(f.state.version, "1.0.0");
});

for (const started of [false, true]) test(`a crashed updater is recovered when the new service ${started ? "has" : "has not"} started`, async (t) => {
  const f = await fixture(t);
  const next = await stagePackage(f.file, f.item, f);
  selectComponent("service", next, f.env);
  fs.writeFileSync(f.journal, JSON.stringify({ previous: f.old, next }));
  f.state = started ? { version: next.version, terminals: [{ id: 4, exited: false }] } : null;
  assert.equal(await recoverServiceUpdate(f), started ? "completed" : "restored");
  assert.equal(currentComponent("service", f.env).version, started ? "1.1.0" : "1.0.0");
  assert.equal(f.stops, 0);
  assert.equal(fs.existsSync(f.journal), false);
});

test("a verified service update selects the new version and clears recovery state", async (t) => {
  const f = await fixture(t);
  assert.equal((await activatePackage(f.file, f.item, f)).status, "updated");
  assert.equal(currentComponent("service", f.env).version, "1.1.0");
  assert.equal(f.state.version, "1.1.0");
  assert.equal(fs.existsSync(f.journal), false);
});

test("an unavailable rollback remains recoverable on the next launch", async (t) => {
  const f = await fixture(t, { failNew: true });
  const start = f.operations.start;
  f.operations.start = async () => { throw new Error("runtime temporarily unavailable"); };
  await assert.rejects(activatePackage(f.file, f.item, f), /Recovery is pending/);
  assert.equal(fs.existsSync(f.journal), true);
  assert.equal(currentComponent("service", f.env).version, "1.0.0");
  f.operations.start = start;
  assert.equal(await recoverServiceUpdate(f), "restored");
  assert.equal(f.state.version, "1.0.0");
  assert.equal(fs.existsSync(f.journal), false);
});

test("a failed first installation does not leave a broken component selected", async (t) => {
  const f = await fixture(t, { failNew: true });
  fs.unlinkSync(path.join(f.env.XENONET_RUNTIME_DIR, "components/service/current.json"));
  f.state = null;
  await assert.rejects(activatePackage(f.file, f.item, f), /native module unavailable/);
  assert.equal(currentComponent("service", f.env), null);
  assert.equal(fs.existsSync(f.journal), false);
});
