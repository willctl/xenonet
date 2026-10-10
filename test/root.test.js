// The projects folder XenoNet reads, its root, and what happens to the rest
// when it changes: over the demo's made-up world (see demo.js), in a world
// of this test's own, since the demo keeps one per port.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
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
const store = await import("../core/store.js");
const { pathKey } = await import("../core/paths.js");
const { world, clientsReady } = await import("../core/demo.js");
const { scanProjects } = await import("../core/projects.js");
const { RepoWatch, gitAvailable } = await import("../core/git.js");

const state = new XenoNetState();
after(() => {
  state.stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});

test("Codex's picks are kept for the folder Codex looked at, and laid over no other", async () => {
  await state.refresh();
  const { fingerprint } = await state.codexOutline();
  const asked = await state.askCodexAboutProjects(fingerprint);
  assert.deepEqual(asked.codex.picks.map((p) => p.path), ["labs/snippets"]);
  assert.equal(store.getCodexPicks(pathKey(config.projectsDir)).picks.length, 1, "kept under the folder's key");
  assert.deepEqual(Object.values(state.projectRules()).map((r) => [r.by, r.as]), [["codex", "project"]]);
  // Another folder, with a labs/snippets of its own or not, gets none of them.
  assert.deepEqual(state.projectRules({ root: world.home }), {});
  assert.equal((await state.forgetCodexPicks()).codex, null);
  assert.equal(store.getCodexPicks(pathKey(config.projectsDir)), null);
});

test("your rules say whether they're in the projects folder, and any of them can be undone", async () => {
  const labs = path.join(config.projectsDir, "labs");
  // A rule made while another folder was in use: it stays, and waits.
  const elsewhere = path.join(world.home, "Elsewhere", "notes");
  store.setProjectRule(pathKey(elsewhere), { path: elsewhere, as: "group", at: 1 });
  let d = await state.setProjectRule(labs, "project");
  assert.deepEqual(d.rules.map((r) => [path.basename(r.path), r.inRoot]), [["labs", true], ["notes", false]]);
  await assert.rejects(state.setProjectRule(elsewhere, "project"), /Only folders inside Projects can have rules/, "made only inside it");
  d = await state.setProjectRule(elsewhere, null);
  assert.deepEqual(d.rules.map((r) => path.basename(r.path)), ["labs"], "undone from outside");
  d = await state.setProjectRule(path.join(world.home, "never-ruled"), null);
  assert.equal(d.rules.length, 1, "undoing what isn't there does nothing");
  assert.deepEqual((await state.setProjectRule(labs, null)).rules, []);
});

test("the git status loop drops repos that left the projects, one pass at a time", async () => {
  await state.refresh();
  const repos = state.projects.filter((p) => p.git);
  const watch = new RepoWatch();
  await watch.refresh(repos);
  assert.equal(watch.status.size, gitAvailable() ? repos.length : 0);
  await watch.refresh(repos.slice(0, 2));
  assert.deepEqual([...watch.status.keys()].sort(), gitAvailable() ? repos.slice(0, 2).map((p) => p.key).sort() : []);

  // Passes asked for while one runs wait for it, and share the next.
  const seen = [];
  const real = state.repos.refresh.bind(state.repos);
  state.repos.refresh = async (projects) => {
    seen.push("start");
    await real(projects);
    seen.push("end");
  };
  try {
    await Promise.all([state.refreshRepos(), state.refreshRepos(), state.refreshRepos()]);
  } finally {
    delete state.repos.refresh;
  }
  assert.deepEqual(seen, ["start", "end", "start", "end"]);
});

test("the demo has a second projects folder, with a labs/snippets of its own", async () => {
  await clientsReady();
  const found = await scanProjects(world.clientsDir);
  assert.deepEqual(Object.fromEntries(found.map((p) => [p.rel, `${p.role}:${p.detected}`])), { "harbor-api": "project:repo", "harbor-app": "project:repo", kiln: "project:repo", labs: "group:contains", "labs/glaze": "project:repo" });
  assert.ok(found.folders.some((f) => f.rel === "labs/snippets" && f.call === "folder"), "a folder, like ~/Projects' labs/snippets");
  const repos = new RepoWatch();
  await repos.refresh(found);
  assert.equal(repos.status.get(found.find((p) => p.rel === "harbor-api").key)?.dirty, 1, "with work left uncommitted");
});

test("a check says what XenoNet would find in a folder, without reading it for the map", async () => {
  const graph = state.graph;
  const clients = await state.checkRoot("~/Clients");
  assert.deepEqual([clients.ok, clients.path, clients.current, clients.scan.projects, clients.scan.groups], [true, world.clientsDir, false, 4, 1]);
  assert.equal((await state.checkRoot(config.projectsDir)).current, true);
  assert.match((await state.checkRoot("~")).error, /your home folder/);
  assert.equal(state.graph, graph, "the map is as it was");
});

test("a change of projects folder reads it in one pass, and coming back brings the rest back", async () => {
  await state.refresh();
  await state.refreshRepos();
  const home = config.projectsDir;
  const atlas = `project:${state.projects.find((p) => p.name === "atlas-api").key}`;
  const sessions = state.graph.nodes.filter((n) => n.kind === "session").length;
  // In ~/Projects: a position you dragged, a rule of yours, and Codex's picks.
  store.setLayout({ [atlas]: { x: 10, y: 20, z: 30 } });
  await state.setProjectRule(path.join(home, "docs"), "ignore");
  await state.askCodexAboutProjects((await state.codexOutline()).fingerprint);

  const graphs = [];
  const on = (g) => graphs.push(g);
  state.on("graph", on);
  const moved = await state.setRoot("~/Clients/");
  state.off("graph", on);
  assert.deepEqual([moved.path, moved.source, moved.changed, moved.scan.projects], [world.clientsDir, "chosen", true, 4]);
  assert.equal(config.projectsDir, world.clientsDir);
  assert.equal(graphs.length, 1, "one map goes out");
  const g = graphs[0];
  assert.equal(g.nodes.find((n) => n.id === "core").path, world.clientsDir);
  assert.deepEqual(g.nodes.filter((n) => n.kind === "project").map((n) => n.rel).sort(), ["harbor-api", "harbor-app", "kiln", "labs", "labs/glaze"], "~/Projects' picks aren't laid over this labs/snippets");
  assert.equal(g.nodes.find((n) => n.rel === "harbor-api").repo?.dirty, 1, "its repos' status came in the same pass");
  assert.ok([...state.repos.status.keys(), ...state.identities.repos.keys()].every((k) => k.startsWith(`${pathKey(world.clientsDir)}/`)), "and ~/Projects' went");
  assert.equal(g.nodes.filter((n) => n.kind === "session").length, sessions, "sessions carry on");
  assert.ok(g.nodes.some((n) => n.kind === "zone" && n.name === "atlas-api"), "outside it now");
  let d = await state.projectDetection();
  assert.equal(d.codex, null, "Codex hasn't looked at this folder");
  assert.deepEqual(d.rules.map((r) => [path.basename(r.path), r.inRoot]), [["docs", false]]);
  assert.equal(store.getLayout()[atlas].x, 10, "the layout stays as it is");

  const same = await state.setRoot(world.clientsDir);
  assert.equal(same.changed, false);
  await assert.rejects(state.setRoot("~/nowhere"), (err) => err.status === 400 && /doesn't exist/.test(err.message));
  assert.equal(config.projectsDir, world.clientsDir);

  const back = await state.setRoot(null);
  assert.deepEqual([back.path, back.source, back.changed], [home, "default", true]);
  const node = state.graph.nodes.find((n) => n.id === atlas);
  assert.deepEqual([node.fx, node.fy, node.fz, node.pinned], [10, 20, 30, true], "its layout comes back");
  assert.ok(!state.graph.nodes.some((n) => n.rel === "docs"), "your rule applies again");
  assert.ok(state.graph.nodes.some((n) => n.rel === "labs/snippets" && n.rule?.by === "codex"), "and Codex's picks");
  d = await state.projectDetection();
  assert.equal(d.codex.stale, false);
  await state.setProjectRule(path.join(home, "docs"), null);
  await state.forgetCodexPicks();
  store.setLayout({ [atlas]: null });
});
