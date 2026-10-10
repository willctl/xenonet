// The projects folder XenoNet reads (core/workspace.js): where it comes from,
// choosing another, and checking a folder before XenoNet reads it, all in
// made-up folders of this test's own.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const made = [];
const tmp = (name = "xenonet-workspace-") => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), name));
  made.push(dir);
  return dir;
};
// The store's last change is saved first: a save still waiting would make
// the state folder again once it's gone.
after(() => {
  store.flush();
  made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 }));
});

// Before anything reads the config: a state folder of the test's own, and
// no XENONET_PROJECTS_DIR from whoever runs the tests.
process.env.XENONET_STATE_DIR = tmp();
delete process.env.XENONET_PROJECTS_DIR;
const { config } = await import("../core/config.js");
const store = await import("../core/store.js");
const workspace = await import("../core/workspace.js");
const { pathKey } = await import("../core/paths.js");

// A made-up home folder, with a projects folder of repos and a few others.
const home = tmp("xenonet-home-");
config.homeDir = home;
const at = (...p) => path.join(home, ...p);
function write(rel, text = "") {
  fs.mkdirSync(path.dirname(at(rel)), { recursive: true });
  fs.writeFileSync(at(rel), text);
}
const repo = (rel) => write(`${rel}/.git/HEAD`, "ref: refs/heads/main\n");
for (const r of ["Projects/atlas", "Projects/ledger", "code/studio/app", "code/studio/site", "code/tool"]) repo(r);
write("code/studio/NOTES.md", "notes");
repo("one-repo");
write("one-repo/src/a.js");
fs.mkdirSync(at("empty"));
write("a-file.txt", "not a folder");

test("the folder in use: XENONET_PROJECTS_DIR, then the one you chose, then ~/Projects", () => {
  let r = workspace.root();
  assert.deepEqual([r.path, r.source, r.env, r.chosen, r.default], [at("Projects"), "default", null, null, at("Projects")]);
  store.setWorkspaceRoot(at("code"));
  r = workspace.root();
  assert.deepEqual([r.path, r.source, r.chosen], [at("code"), "chosen", at("code")]);
  process.env.XENONET_PROJECTS_DIR = at("empty");
  try {
    r = workspace.root();
    assert.deepEqual([r.path, r.source, r.env, r.chosen], [at("empty"), "env", at("empty"), at("code")], "the variable wins, and the choice waits");
  } finally {
    delete process.env.XENONET_PROJECTS_DIR;
  }
  // A choice that isn't a full path (state.json edited by hand) is no choice.
  store.setWorkspaceRoot("code");
  assert.equal(workspace.root().source, "default");
  store.setWorkspaceRoot(null);
});

test("config.projectsDir follows the folder in use", () => {
  store.setWorkspaceRoot(at("code"));
  assert.equal(workspace.applyRoot().path, at("code"));
  assert.equal(config.projectsDir, at("code"));
  store.setWorkspaceRoot(null);
  workspace.applyRoot();
  assert.equal(config.projectsDir, at("Projects"));
});

test("what only looks reads the choice from state.json as it is, and never changes the file", () => {
  const dir = tmp("xenonet-peek-");
  const file = path.join(dir, "state.json");
  const look = (text) => (fs.writeFileSync(file, text), workspace.savedRoot(file));
  assert.equal(look(JSON.stringify({ version: 2, workspace: { root: at("code") } })), at("code"));
  // A v1 file from before the versions met, which XenoNet would move up and
  // keep a copy of first, is read as it is, and so is a newer one.
  const v1 = JSON.stringify({ workspace: { root: at("code") }, layout: {} });
  assert.equal(look(v1), at("code"));
  assert.equal(look(JSON.stringify({ version: 9, workspace: { root: at("code") } })), at("code"));
  // None chosen, or no file XenoNet can read: no choice, and nothing moved aside.
  assert.equal(look(JSON.stringify({ version: 2, workspace: { root: null } })), null);
  assert.equal(look("{ not json"), null);
  assert.equal(workspace.savedRoot(path.join(dir, "missing.json")), null);
  fs.writeFileSync(file, v1);
  workspace.savedRoot(file);
  assert.deepEqual([fs.readdirSync(dir), fs.readFileSync(file, "utf8")], [["state.json"], v1], "the file as it was, and no other");

  // root() with peek reads the state folder's state.json, not the store.
  store.flush();
  const stateDir = config.stateDir;
  config.stateDir = dir;
  try {
    assert.deepEqual([workspace.root({ peek: true }).path, workspace.root({ peek: true }).source], [at("code"), "chosen"]);
    assert.equal(workspace.root().source, "default", "the store has no choice");
    process.env.XENONET_PROJECTS_DIR = at("empty");
    assert.equal(workspace.root({ peek: true }).source, "env", "and the variable still wins");
  } finally {
    delete process.env.XENONET_PROJECTS_DIR;
    config.stateDir = stateDir;
  }
});

test("what only looks at XenoNet Dev reads its state folder, not the installed XenoNet's", async () => {
  const { appDir } = await import("../core/dirs.js");
  assert.equal(workspace.instanceDir("state", "XenoNet Dev", { env: {} }), appDir("state", { name: "XenoNet Dev" }));
  assert.equal(workspace.instanceDir("cache", "XenoNet Dev", { env: {} }), appDir("cache", { name: "XenoNet Dev" }));
  assert.notEqual(workspace.instanceDir("state", "XenoNet Dev", { env: {} }), appDir("state"));
  assert.equal(workspace.instanceDir("state", "XenoNet Dev", { env: { XENONET_STATE_DIR: at("elsewhere") } }), at("elsewhere"), "a folder that's set wins");
  // The choice as that folder's state.json has it.
  const dev = tmp("xenonet-dev-state-");
  fs.writeFileSync(path.join(dev, "state.json"), JSON.stringify({ version: 2, workspace: { root: at("code") } }));
  assert.deepEqual([workspace.root({ peek: true, stateDir: dev }).path, workspace.root({ peek: true, stateDir: dev }).source], [at("code"), "chosen"]);
  assert.equal(workspace.root({ peek: true }).source, "default", "XenoNet's own state folder has none");
});

test("a command that finds no XenoNet running stands for the one asked about: XenoNet Dev for --dev or its port", () => {
  assert.equal(workspace.standsFor({ dev: true }), "XenoNet Dev");
  assert.equal(workspace.standsFor({ port: "47842" }), "XenoNet Dev", "as the MCP server XenoNet Dev connects asks");
  assert.equal(workspace.standsFor({ env: { XENONET_PORT: "47842" } }), "XenoNet Dev");
  assert.equal(workspace.standsFor({ port: 47840 }), null, "XenoNet: its own folders, as before");
  assert.equal(workspace.standsFor({ env: {} }), null, "either of them: XenoNet's, as before");
  assert.equal(workspace.standsFor({ demo: true }), null);
});

test("choosing a folder saves it, the default saves as none, and XENONET_PROJECTS_DIR says no", () => {
  assert.equal(workspace.choose(at("code")).source, "chosen");
  assert.equal(store.getWorkspaceRoot(), at("code"));
  assert.equal(config.projectsDir, at("code"));
  assert.equal(workspace.choose(at("Projects") + path.sep).source, "default", "the default folder is no choice");
  assert.equal(store.getWorkspaceRoot(), null);
  workspace.choose(at("code"));
  assert.equal(workspace.choose(null).path, at("Projects"));
  process.env.XENONET_PROJECTS_DIR = at("empty");
  try {
    assert.throws(() => workspace.choose(at("code")), (err) => err.status === 409 && /XENONET_PROJECTS_DIR is set/.test(err.message));
    assert.throws(() => workspace.choose(null), (err) => err.status === 409);
    assert.equal(store.getWorkspaceRoot(), null, "nothing saved");
    assert.match(workspace.locked().message, /\(~[\\/]empty\)/);
  } finally {
    delete process.env.XENONET_PROJECTS_DIR;
  }
  assert.equal(workspace.locked(), null);
});

test("a folder as typed: ~ is home, anything else a full path", () => {
  assert.equal(workspace.resolveFolder("~"), home);
  assert.equal(workspace.resolveFolder("~/code/"), at("code"));
  assert.equal(workspace.resolveFolder(`  ${at("code", "studio", "..")}  `), at("code"));
  for (const no of ["", "   ", "code", "./code", "~nobody/x", null, 42]) assert.equal(workspace.resolveFolder(no), null, String(no));
});

test("what XenoNet can't read as its projects folder, and why", () => {
  const why = (dir) => workspace.validate(dir).error;
  assert.match(why("code"), /full path, or start it with ~/);
  assert.match(why(at("nowhere")), /^~[\\/]nowhere doesn't exist/);
  assert.match(why(at("a-file.txt")), /^~[\\/]a-file\.txt isn't a folder/);
  assert.match(why(path.parse(home).root), /the whole disk/);
  assert.match(why("~"), /your home folder/);
  assert.match(why(path.dirname(home)), /holds your home folder/);
  const ok = workspace.validate("~/code");
  assert.deepEqual([ok.ok, ok.path, ok.error], [true, at("code"), null]);
});

// Links of the test's own, outside the made-up home and in it. Making one
// can need rights Windows doesn't give everyone.
const links = tmp("xenonet-links-");
const link = (target, at) => {
  try {
    fs.symlinkSync(target, at, process.platform === "win32" ? "junction" : "dir");
    return true;
  } catch {
    return false;
  }
};
const linked =
  link(home, path.join(links, "home-link")) &&
  link(path.parse(home).root, path.join(links, "disk-link")) &&
  link(path.dirname(home), path.join(links, "parent-link")) &&
  link(at("code"), path.join(links, "code-link"));

test("a link is judged by where it leads, and chosen as that folder", { skip: !linked && "this user can't make links" }, async () => {
  const why = (name) => workspace.validate(path.join(links, name)).error;
  assert.match(why("home-link"), /home-link is a link to your home folder\. Choose the folder in it/);
  assert.match(why("disk-link"), /disk-link is a link to the whole disk\./);
  assert.match(why("parent-link"), /parent-link is a link to .*, which holds your home folder\./);
  const v = workspace.validate(path.join(links, "code-link"));
  assert.deepEqual([v.ok, v.path, v.link], [true, at("code"), path.join(links, "code-link")]);
  assert.equal(workspace.validate("~/code").link, null, "a folder that's no link says none");

  // The check says so, and counts what the folder holds.
  const c = await workspace.check(path.join(links, "code-link"));
  assert.deepEqual([c.ok, c.path, c.scan.projects], [true, at("code"), 3]);
  assert.match(c.warnings.join(), /code-link is a link, so XenoNet would read the folder it leads to, ~[\\/]code\./);

  // Chosen through the link, the folder is saved and read where sessions
  // record it, and the default through a link of its own is still none.
  workspace.choose(path.join(links, "code-link"));
  assert.deepEqual([store.getWorkspaceRoot(), config.projectsDir], [at("code"), at("code")]);
  assert.equal((await workspace.check(path.join(links, "code-link"))).current, true);
  link(at("Projects"), path.join(links, "projects-link"));
  assert.equal(workspace.choose(path.join(links, "projects-link")).source, "default");
  assert.equal(store.getWorkspaceRoot(), null);
});

test("a check says how many projects and groups XenoNet would find, and what's worth knowing first", async () => {
  const code = await workspace.check("~/code");
  assert.deepEqual([code.ok, code.path, code.error, code.current], [true, at("code"), null, false]);
  assert.deepEqual([code.scan.projects, code.scan.groups], [3, 1]);
  assert.deepEqual(code.warnings, []);
  const current = await workspace.check(at("Projects"));
  assert.deepEqual([current.current, current.scan.projects], [true, 2]);

  assert.match((await workspace.check("~/one-repo")).warnings.join(), /is a repo itself/);
  assert.match((await workspace.check("~/empty")).warnings.join(), /no projects/);
  const capped = await workspace.check("~/code", { limits: { depth: 3, maxDirs: 1 } });
  assert.equal(capped.scan.truncated, true);
  assert.match(capped.warnings.join(), /stopped at its 1-folder limit/);

  const bad = await workspace.check("~/nowhere");
  assert.deepEqual([bad.ok, bad.scan, bad.warnings], [false, null, []]);
  assert.match(bad.error, /doesn't exist/);
  // Rules count: your rule making studio one project leaves one project fewer.
  const ruled = await workspace.check("~/code", { rules: { [pathKey(at("code", "studio"))]: { path: at("code", "studio"), as: "project" } } });
  assert.deepEqual([ruled.scan.projects, ruled.scan.groups], [2, 0]);
});

test("a check that takes too long isn't waited for, and its scan is stopped", async () => {
  // A scan of a slow folder, which goes on until it's told to stop. (A real
  // scan of a small folder against a 0 ms wait is a race either can win.)
  let signal = null;
  const scan = (_dir, opts) => ((signal = opts.signal), new Promise((_resolve, reject) => opts.signal.addEventListener("abort", () => reject(opts.signal.reason))));
  const late = await workspace.check("~/code", { timeMs: 20, scan });
  assert.deepEqual([late.ok, late.scan, signal.aborted], [true, null, true]);
  assert.match(late.warnings.join(), /took more than 0.02 s/);
  // One that finishes in time is left alone.
  const quick = await workspace.check("~/code", { scan: async (dir, opts) => ((signal = opts.signal), (await import("../core/projects.js")).scanProjects(dir, opts)) });
  assert.deepEqual([quick.scan.projects, signal.aborted], [3, false]);
});
