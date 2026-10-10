import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// Before anything reads the config: a state folder of the test's own.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-store-"));
const dir0 = dir;
process.env.XENONET_STATE_DIR = dir;
after(() => fs.rmSync(dir, { recursive: true, force: true }));
const { config } = await import("../core/config.js");
const store = await import("../core/store.js");
const { pathKey } = await import("../core/paths.js");
const file = () => path.join(config.stateDir, "state.json");
const saved = () => JSON.parse(fs.readFileSync(file(), "utf8"));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

test("an unreadable state file is moved aside, and XenoNet starts afresh with every part in place", () => {
  fs.writeFileSync(file(), "{ not json");
  const s = store.load();
  assert.equal(s.version, 2);
  assert.deepEqual(s.workspace, { root: null });
  assert.deepEqual(s.layout, {});
  assert.deepEqual(s.projects, { rules: {}, codexByRoot: {} });
  assert.deepEqual(s.workspace, { root: null });
  assert.deepEqual(Object.keys(s.identities).sort(), ["hidden", "known", "names", "pins", "signins"]);
  assert.equal(store.load(), s, "loaded once");
  const { broken, readOnly } = store.loadReport();
  assert.equal(fs.readFileSync(broken, "utf8"), "{ not json", "kept, not written over");
  assert.match(path.basename(broken), /^state\.broken-.+\.json$/);
  assert.equal(readOnly, false);
});

test("what you change is saved a moment later, in one write", async () => {
  store.setLayout({ a: { x: 1.234, y: 2, z: 3 }, b: { x: NaN, y: 0, z: 0 } });
  store.setLayout({ c: { x: 0, y: 0, z: 0 } });
  store.setLayout({ c: null });
  assert.deepEqual(store.getLayout(), { a: { x: 1.23, y: 2, z: 3 } });
  store.ack("claude:s1", 42);
  assert.equal(store.mark("tour"), undefined);
  store.mark("tour", "2026-10-03");
  assert.equal(store.mark("tour"), "2026-10-03");
  store.setTheme("noir");
  assert.equal(store.getTheme(), "noir");
  store.setWorkspaceRoot("/home/me/code");
  assert.equal(store.getWorkspaceRoot(), "/home/me/code");
  await wait(300);
  const s = saved();
  assert.deepEqual([s.layout, s.acks, s.marks.tour, s.theme, s.workspace.root], [{ a: { x: 1.23, y: 2, z: 3 } }, { "claude:s1": 42 }, "2026-10-03", "noir", "/home/me/code"]);
  store.setWorkspaceRoot(null);
  assert.ok(!fs.existsSync(`${file()}.tmp`));
  store.clearLayout();
  assert.deepEqual(store.getLayout(), {});
});

test("project rules, Codex's picks and identities are kept and taken back", () => {
  store.setProjectRule("/p/a", { path: "/p/a", as: "group", at: 1 });
  store.setProjectRule("/p/b", { path: "/p/b", as: "project", at: 2 });
  assert.deepEqual(Object.keys(store.setProjectRule("/p/b", null)), ["/p/a"]);
  store.setProjectRule("/q/c", { path: "/q/c", as: "ignore", at: 3 });
  store.setProjectRule("/pq", { path: "/pq", as: "project", at: 4 });
  store.clearProjectRules("/p");
  assert.deepEqual(Object.keys(store.getProjectRules()).sort(), ["/pq", "/q/c"], "one folder's, and no other's");
  store.clearProjectRules();
  assert.deepEqual(store.getProjectRules(), {});
  store.setCodexPicks("/p", { ranAt: 1, picks: [] });
  store.setCodexPicks("/q", { ranAt: 2, picks: [{ path: "x", as: "project", reason: "" }] });
  assert.deepEqual(store.getCodexPicks("/p"), { ranAt: 1, picks: [] });
  store.setCodexPicks("/p", null);
  assert.deepEqual([store.getCodexPicks("/p"), store.getCodexPicks("/q").ranAt], [null, 2], "each folder keeps its own");
  assert.throws(() => store.getCodexPicks(), /which projects folder/);
  store.setCodexPicks("/q", null);

  store.setIdentityName("github:github.com/nova", "Nova");
  store.setIdentityHidden("git:abc", true);
  store.setIdentityPin("claude:s1", "github:github.com/nova");
  const ids = store.getIdentities();
  assert.equal(ids.names["github:github.com/nova"], "Nova");
  assert.equal(ids.hidden["git:abc"], true);
  assert.equal(ids.pins["claude:s1"].identity, "github:github.com/nova");
  store.setIdentityName("github:github.com/nova", null);
  store.setIdentityHidden("git:abc", false);
  store.setIdentityPin("claude:s1", null);
  assert.deepEqual([ids.names, ids.hidden, ids.pins], [{}, {}, {}]);
  store.saveIdentities();
  store.flush();
  assert.deepEqual(saved().identities.pins, {});
});

test("a save that fails is reported, kept and tried again, and never takes XenoNet down", async () => {
  const blocker = path.join(dir, "a-file");
  fs.writeFileSync(blocker, "");
  const errors = [];
  const error = console.error;
  console.error = (...args) => errors.push(args.join(" "));
  try {
    config.stateDir = path.join(blocker, "state"); // can't be made: a file is in the way
    store.ack("claude:s2", 7);
    await wait(300);
    store.flush();
    assert.ok(errors.length >= 1 && errors.every((e) => /couldn't save its state/.test(e)), errors.join("\n"));
    config.stateDir = dir;
    store.flush();
    assert.equal(saved().acks["claude:s2"], 7);
  } finally {
    console.error = error;
    config.stateDir = dir;
  }
});

// A store of its own over `dir`: a fresh copy of the module, which reads
// the file again.
async function storeIn(dir) {
  config.stateDir = dir;
  return import(`../core/store.js?${dir}`);
}

test("a v1 file is kept as state.v1.json and saved as v2, with Codex's picks under the projects folder", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-store-v1-"));
  const picks = { ranAt: 1, ms: 2, fingerprint: "f", sent: 3, picks: [{ path: "labs", as: "group", reason: "r" }] };
  const v1 = JSON.stringify({ layout: { core: { x: 1, y: 2, z: 3 } }, projects: { rules: {}, codex: picks }, theme: "gridline", mine: { kept: true } });
  try {
    fs.writeFileSync(path.join(dir, "state.json"), v1);
    const s = await storeIn(dir);
    assert.deepEqual(s.getCodexPicks(pathKey(config.projectsDir)), picks, "for the projects folder it was made for");
    assert.equal(s.getTheme(), "gridline");
    assert.deepEqual(s.loadReport().migrated, { from: 1, to: 2 });
    await wait(300);
    const saved = JSON.parse(fs.readFileSync(path.join(dir, "state.json"), "utf8"));
    assert.equal(saved.version, 2, "saved as v2 without waiting for a change");
    assert.deepEqual(saved.projects.codexByRoot, { [pathKey(config.projectsDir)]: picks });
    assert.deepEqual(saved.mine, { kept: true }, "a part XenoNet doesn't know stays");
    assert.equal(fs.readFileSync(path.join(dir, "state.v1.json"), "utf8"), v1);
    s.setCodexPicks(pathKey(config.projectsDir), null);
    assert.equal(s.getCodexPicks(pathKey(config.projectsDir)), null);
    // Saved now, into this folder: a save left waiting would land in the
    // test's main folder after it has been taken away.
    s.flush();
  } finally {
    config.stateDir = dir0;
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a file from a newer XenoNet is read but never written back", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-store-v3-"));
  const v3 = JSON.stringify({ version: 3, layout: { core: { x: 1, y: 2, z: 3 } }, shapeOfTheFuture: [1] });
  try {
    fs.writeFileSync(path.join(dir, "state.json"), v3);
    const s = await storeIn(dir);
    assert.deepEqual(s.getLayout(), { core: { x: 1, y: 2, z: 3 } }, "read as it is");
    s.setLayout({ other: { x: 0, y: 0, z: 0 } });
    s.flush();
    assert.equal(fs.readFileSync(path.join(dir, "state.json"), "utf8"), v3, "untouched");
    assert.equal(s.loadReport().readOnly, true);
  } finally {
    config.stateDir = dir0;
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a part of the wrong type starts afresh and the rest is kept, so XenoNet starts; a v1 file with one is copied once", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-store-typed-"));
  const errors = [];
  const error = console.error;
  console.error = (...args) => errors.push(args.join(" "));
  try {
    // A hand edit gone wrong: parts that should be objects aren't.
    fs.writeFileSync(path.join(dir, "state.json"), JSON.stringify({ version: 2, workspace: "code", layout: { core: { x: 1, y: 2, z: 3 } }, acks: [1], projects: { rules: true, codexByRoot: {} }, identities: "x", theme: "noir" }));
    const s = await storeIn(dir);
    assert.deepEqual([s.getLayout(), s.getTheme()], [{ core: { x: 1, y: 2, z: 3 } }, "noir"], "the rest is kept");
    assert.deepEqual([s.getAcks(), s.getProjectRules(), s.getWorkspaceRoot()], [{}, {}, null]);
    assert.deepEqual(Object.keys(s.getIdentities()).sort(), ["hidden", "known", "names", "pins", "signins"]);
    s.setWorkspaceRoot("/home/me/code");
    s.setIdentityName("github:github.com/nova", "Nova");
    assert.deepEqual([s.loadReport().readOnly, s.loadReport().repaired], [false, ["workspace", "acks", "projects.rules", "identities"]]);
    assert.match(errors.join("\n"), /state\.json: workspace, acks, projects\.rules, identities weren't objects/);
    s.flush();

    // A v1 file with one: moved up and copied once, however often XenoNet
    // starts.
    const v1dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-store-typed-v1-"));
    fs.writeFileSync(path.join(v1dir, "state.json"), JSON.stringify({ identities: 5, layout: {} }));
    for (let start = 0; start < 2; start++) {
      config.stateDir = v1dir;
      const again = await import(`../core/store.js?${v1dir}&${start}`);
      assert.deepEqual(again.getIdentities().names, {});
      again.flush();
    }
    assert.deepEqual(fs.readdirSync(v1dir).sort(), ["state.json", "state.v1.json"]);
    fs.rmSync(v1dir, { recursive: true, force: true });
  } finally {
    console.error = error;
    config.stateDir = dir0;
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
