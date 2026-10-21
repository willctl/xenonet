// The experimental native readers as the map uses them (core/native/
// reader.js, core/native/index.js): which are on comes from the app
// registry and XENONET_READERS, and a reader made up here, for an app made
// up here, reaches the map's sessions and sources through the same table,
// with nothing else changed. The state runs on the demo's made-up world, in
// a world of this test's own (the demo keeps one per port).
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
process.env.XENONET_DEMO = "1";
process.env.XENONET_PORT = String(port);
const { XenoNetState } = await import("../core/state.js");
const { config } = await import("../core/config.js");
const { world } = await import("../core/demo.js");
const { APPS, appsWith } = await import("../core/apps.js");
const { NATIVE_READERS, nativeReaders } = await import("../core/native/index.js");
const { NativeReader, editFile, fitsId, pushEdit, runSession, titled } = await import("../core/native/reader.js");

const stops = [];
after(() => {
  for (const stop of stops) stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});

test("the table has a reader for each app the registry gives one, and for no other", () => {
  assert.deepEqual(NATIVE_READERS.map((r) => r.app).sort(), appsWith("native").map((a) => a.id).sort());
  // XENONET_READERS names a reader by its app's id, which is its name as a
  // source of the map too.
  for (const r of NATIVE_READERS) assert.equal(r.source, r.app);
});

test("the readers that are on: the registry's that XENONET_READERS names, in the table's order", () => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-native-"));
  try {
    const settings = { homeDir: home, stateDir: home };
    const on = (names, opts) => nativeReaders({ ...settings, readers: new Set(names) }, opts).map((n) => [n.source, n.reader.name]);
    assert.deepEqual(on(["cursor", "copilot", "gemini-cli", "opencode"]), [
      ["opencode", "opencode-sessions"],
      ["gemini-cli", "gemini-cli-sessions"],
      ["copilot", "copilot-sessions"],
      ["cursor", "cursor-sessions"],
    ]);
    assert.deepEqual(on([]), []);
    assert.deepEqual(on(["vscode", "claude", "nope"]), [], "only apps with a native reader");
    // An app the registry doesn't give `native` has no reader, even with a row.
    const apps = APPS.map((a) => (a.id === "cursor" ? { ...a, has: [] } : a));
    assert.deepEqual(on(["cursor", "opencode"], { apps }), [["opencode", "opencode-sessions"]]);
    assert.deepEqual(nativeReaders({ ...settings }), [], "none without XENONET_READERS");
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
});

// A made-up app, Kite, whose runs are lines in a file of its own:
// { run, cwd, at, edited }.
const KITE = { id: "kite", name: "Kite", numbat: ["kite"], has: ["native"] };

class KiteSessions extends NativeReader {
  constructor({ file, clock } = {}) {
    super({ name: "kite-sessions", clock });
    this.file = file;
    this.runs = new Map();
    this.fileState = "unread";
  }

  read(now = this.clock.now()) {
    let lines;
    try {
      lines = fs.readFileSync(this.file, "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
    } catch (err) {
      this.fileState = err?.code === "ENOENT" ? "missing" : "unreadable";
      this.runs.clear();
      return this.ok(now), this;
    }
    this.fileState = "ok";
    const runs = new Map();
    for (const k of lines) {
      if (!fitsId(k.run)) continue;
      const id = `kite:${k.run}`;
      const run = runs.get(id) ?? { id, rawId: k.run, cwd: k.cwd, createdAt: k.at, updatedAt: k.at, edits: new Map() };
      run.updatedAt = Math.max(run.updatedAt, k.at);
      const file = k.edited ? editFile(run.cwd, k.edited) : null;
      if (file) pushEdit(run.edits, file, k.at);
      runs.set(id, run);
    }
    this.runs = runs;
    this.ok(now);
    return this;
  }

  session(r) {
    return runSession(r, { app: "kite", title: titled("Kite", r.cwd) });
  }

  status() {
    return { name: "kite", file: this.file, state: this.fileState, sessions: this.runs.size };
  }
}

test("a made-up reader for a made-up app is on the map, through the registry and the table alone", async () => {
  const file = path.join(config.stateDir, "kite.ndjson");
  const kite = { app: "kite", source: "kite", what: "Kite's log", create: ({ stateDir }) => new KiteSessions({ file: path.join(stateDir, "kite.ndjson") }) };
  // As XENONET_READERS=kite would, which the demo leaves off.
  config.readers.add("kite");
  const state = new XenoNetState({ native: { apps: [...APPS, KITE], table: [...NATIVE_READERS, kite] } });
  stops.push(() => state.stop());
  assert.deepEqual(state.nativeReaders.map((n) => n.source), ["kite"]);
  assert.ok(state.blocks.health().some((b) => b.name === "kite-sessions"));

  await state.refresh();
  const line = () => state.sources.find((s) => s.name === "kite");
  assert.deepEqual(line(), { name: "kite", ok: false, count: 0, error: "Kite's log is missing" });
  const names = state.sources.map((s) => s.name);
  assert.equal(names.indexOf("kite"), names.indexOf("claude") + 1, "after the session lists, as the native readers are");

  const cwd = path.join(config.projectsDir, "atlas-api");
  const now = Date.now();
  fs.writeFileSync(file, [{ run: "k-0000001", cwd, at: now - 60e3 }, { run: "k-0000001", cwd, at: now - 5e3, edited: "src/orders/refunds.ts" }, { run: "bad id", cwd, at: now }].map((k) => JSON.stringify(k)).join("\n") + "\n");
  await state.refresh();
  assert.deepEqual(line(), { name: "kite", ok: true, count: 1 });
  const node = state.graph.nodes.find((n) => n.id === "kite:k-0000001");
  assert.ok(node, "its run is a session on the map");
  assert.deepEqual([node.kind, node.app, node.rawId, node.title, node.reader, node.cwd], ["session", "kite", "k-0000001", "Kite in atlas-api", "experimental", cwd]);
  const live = state.observed.live("kite:k-0000001");
  assert.equal(live.observed, false);
  assert.equal(live.lastEventAt, now - 5e3);
  assert.deepEqual([...live.edits.keys()], [path.join(cwd, "src/orders/refunds.ts")]);
});
