// XenoNet's state over the demo's made-up world (see demo.js), in a world of
// this test's own: the demo keeps one per port.
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
const { world } = await import("../core/demo.js");
const { pathKey } = await import("../core/paths.js");

const SESSION = (i, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const state = new XenoNetState();
const stops = [() => state.stop()];
after(() => {
  for (const stop of stops) stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});

test("a refresh builds the map from every source and says how each went", async () => {
  const graphs = [];
  state.on("graph", (g) => graphs.push(g));
  const [a, b] = await Promise.all([state.refresh(), state.refresh()]);
  assert.equal(a, b, "refreshes at once are one");
  assert.equal(graphs.length, 1);
  const sources = Object.fromEntries(state.sources.map((s) => [s.name, s]));
  for (const name of ["projects", "codex", "codex-projects", "claude", "live", "wiki", "search", "identities"]) assert.ok(sources[name], name);
  assert.ok(sources.claude.ok && sources.claude.count > 0);
  assert.match(sources.projects.note, /^\d+ projects in \d+ groups?, \d+ folders read$/);
  // The scan's walk is a block of its own, with its budget and what it read.
  const walk = state.blocks.health().find((b) => b.name === "projects");
  assert.deepEqual([walk.kind, walk.state, walk.budget.depth, walk.budget.folders, walk.budget.last.read, walk.counters.scans], ["scanner", "running", config.projectLimits.depth, config.projectLimits.maxDirs, state.scan.dirsRead, 1]);
  assert.match(sources.identities.note, /\d+ claude/);
  assert.equal(a.hideIdentities, false);
  assert.ok(a.nodes.find((n) => n.id === SESSION(1)).actions.some((x) => x.id === "preview"));

  await state.refresh();
  assert.equal(graphs.length, 1, "nothing changed, so no new map goes out");
  const pulses = [];
  state.on("pulse", (p) => pulses.push(p));
  const file = state.index.get(SESSION(1)) && path.join(world.claudeRoot, fs.readdirSync(world.claudeRoot).find((d) => fs.existsSync(path.join(world.claudeRoot, d, `${SESSION(1).split(":")[1]}.jsonl`))), `${SESSION(1).split(":")[1]}.jsonl`);
  const later = new Date(Date.now() + 60e3);
  fs.utimesSync(file, later, later);
  await state.refresh();
  assert.deepEqual(pulses, [{ nodeId: SESSION(1), reason: "updated" }]);
});

test("a source that fails keeps the sessions it had on the map, and says what went wrong", async () => {
  await state.refresh();
  const claudeIds = () => state.graph.nodes.filter((n) => n.kind === "session" && n.app === "claude").map((n) => n.id).sort();
  const before = claudeIds();
  // The demo lists its Claude sessions from their transcripts: one gone
  // makes the listing fail.
  const dir = fs.readdirSync(world.claudeRoot).find((d) => fs.existsSync(path.join(world.claudeRoot, d, `${SESSION(1).split(":")[1]}.jsonl`)));
  const file = path.join(world.claudeRoot, dir, `${SESSION(1).split(":")[1]}.jsonl`);
  const aside = `${file}.aside`;
  fs.renameSync(file, aside);
  try {
    await state.refresh();
    const claude = state.sources.find((s) => s.name === "claude");
    assert.equal(claude.ok, false);
    assert.match(claude.error, /ENOENT/);
    assert.equal(typeof claude.ms, "number");
    assert.equal(claude.count, undefined);
    assert.deepEqual(claudeIds(), before, "the sessions it had stay");
    assert.ok(state.sources.find((s) => s.name === "codex").ok, "the other sources go on");
  } finally {
    fs.renameSync(aside, file);
  }
  await state.refresh();
  assert.deepEqual(state.sources.find((s) => s.name === "claude").ok, true);
  assert.deepEqual(claudeIds(), before);
});

test("a refresh after a change waits for one already under way, then runs again", async () => {
  const running = state.refresh();
  assert.equal(state.refresh(), running);
  const again = state.refreshAfterChange();
  const [first, second] = await Promise.all([running, again]);
  assert.notEqual(first, second, "a second refresh, which sees the change");
});

test("your rules for the projects folder", async () => {
  const labs = path.join(config.projectsDir, "labs");
  await assert.rejects(state.setProjectRule(labs, "planet"), /unknown rule/);
  await assert.rejects(state.setProjectRule(path.dirname(config.projectsDir), "project"), /Only folders inside Projects can have rules/);
  await assert.rejects(state.setProjectRule(config.projectsDir, "group"), /Only folders inside/);
  await assert.rejects(state.setProjectRule(path.join(config.projectsDir, "gone"), "project"), /doesn't exist/);
  let detection = await state.setProjectRule(labs, "project");
  assert.deepEqual(detection.rules.map((r) => [r.as, r.path]), [["project", labs]]);
  assert.equal(state.projectRules()[Object.keys(state.projectRules())[0]].by, "you");
  assert.equal(state.projects.find((p) => p.path === labs).rule.as, "project");
  detection = await state.setProjectRule(labs, null);
  assert.deepEqual(detection.rules, []);
  await state.setProjectRule(labs, "ignore");
  // A rule made under another projects folder stays when this one's are
  // cleared, and goes when every folder's are.
  const elsewhere = path.join(path.dirname(config.projectsDir), "Clients", "acme");
  store.setProjectRule(pathKey(elsewhere), { path: elsewhere, as: "project", at: 1 });
  detection = await state.clearProjectRules();
  assert.deepEqual(detection.rules.map((r) => [r.path, r.inRoot]), [[elsewhere, false]]);
  assert.deepEqual((await state.clearProjectRules({ all: true })).rules, []);
  assert.ok((await state.rescanProjects()).scan.ok);
});

test("the Codex pass: shown first, sent only as shown, and forgotten on request", async () => {
  const { outline, fingerprint } = await state.codexOutline();
  assert.ok(outline.some((f) => f.path === "labs/snippets"));
  await assert.rejects(state.askCodexAboutProjects("not-what-you-saw"), (err) => err.status === 409);
  const detection = await state.askCodexAboutProjects(fingerprint);
  assert.deepEqual([detection.codex.picks.map((p) => p.path), detection.codex.stale, detection.codex.sent], [["labs/snippets"], false, outline.length]);
  const rules = Object.values(state.projectRules());
  assert.deepEqual(rules.map((r) => [r.by, r.as]), [["codex", "project"]]);
  assert.deepEqual(state.projectRules({ codex: false }), {});
  assert.equal((await state.forgetCodexPicks()).codex, null);
});

test("identities: names, hiding, pins and detecting again", async () => {
  const before = state.identityDetection();
  assert.ok(before.enabled && before.identities.length > 0);
  const id = before.identities[0].id;
  await assert.rejects(state.nameIdentity("github:nowhere/nobody", "X"), /doesn't know that identity/);
  let d = await state.nameIdentity(id, `  Work\u0000\u001f stuff ${"x".repeat(50)}`);
  assert.equal(d.identities.find((i) => i.id === id).name, `Work stuff ${"x".repeat(29)}`);
  d = await state.nameIdentity(id, 42);
  assert.equal(d.identities.find((i) => i.id === id).name, null, "not a name: back to none");
  assert.equal((await state.hideIdentity(id, 1)).identities.find((i) => i.id === id).hidden, true);
  await state.hideIdentity(id, false);

  await assert.rejects(state.pinIdentity("core", id), /Only a session or a project/);
  await assert.rejects(state.pinIdentity(SESSION(1), "github:nowhere/nobody"), /doesn't know/);
  d = await state.pinIdentity(SESSION(1), id);
  assert.deepEqual(d.pins.map((p) => [p.node, p.kind, p.identity]), [[SESSION(1), "session", id]]);
  assert.equal(state.index.get(SESSION(1)).ranAs.confidence, "yours");
  // A pin on something no longer on the map can still be taken back.
  store.setIdentityPin("claude:gone-session", id);
  d = await state.pinIdentity("claude:gone-session", null);
  assert.deepEqual(d.pins.map((p) => p.node), [SESSION(1)]);
  assert.deepEqual((await state.pinIdentity(SESSION(1), null)).pins, []);
  assert.ok((await state.rescanIdentities()).identities.length > 0);
});

test("a change of sign-in shows in the activity feed", async () => {
  const seen = [];
  const on = (a) => a.kind === "identity" && seen.push(a);
  state.on("activity", on);
  const file = config.claudeConfigFile;
  const original = fs.readFileSync(file, "utf8");
  const other = JSON.parse(original);
  other.oauthAccount = { ...other.oauthAccount, accountUuid: "0190d3e0-0000-7000-a000-0000000000ff", emailAddress: "other@example.com" };
  fs.writeFileSync(file, JSON.stringify(other));
  const later = new Date(Date.now() + 5000);
  fs.utimesSync(file, later, later);
  await state.refresh();
  state.off("activity", on);
  fs.writeFileSync(file, original);
  assert.equal(seen.length, 1);
  assert.deepEqual([seen[0].app, seen[0].identity.split(":")[1], seen[0].before.split(":")[1]], ["claude", "0190d3e0-0000-7000-a000-0000000000ff", "0190d3e0-0000-7000-a000-000000000001"]);
});

test("where sessions live, and what they're called", () => {
  assert.equal(state.homeOf(SESSION(1)), `project:${state.projects.find((p) => p.name === "atlas-api").key}`);
  assert.equal(state.homeOf(SESSION(2, "codex")), `project:${state.projects.find((p) => p.name === "atlas-api").key}`, "a Codex project's session lives in the folder holding it");
  assert.equal(state.homeOf("claude:nowhere"), null);
  assert.equal(state.titleOf(SESSION(1)), "Add partial refunds to the orders API");
  assert.equal(state.titleOf("core"), "~/Projects");
  assert.equal(state.titleOf("claude:nowhere"), "claude:nowhere");
});

test("the live snapshot leaves out what you've seen, what's archived and what runs on its own", async () => {
  await state.live.start();
  state.live.stop();
  const asking = SESSION(3);
  const stuck = SESSION(4, "codex");
  let snap = state.liveSnapshot();
  const of = (id) => snap.sessions.find((s) => s.id === id);
  assert.equal(of(asking).needsYou.kind, "question");
  assert.ok(of(stuck).stuck.reasons.length > 0);
  const collision = snap.collisions.find((c) => [c.a, c.b].includes(SESSION(1)));
  assert.deepEqual(collision.titles.sort(), ["Add partial refunds to the orders API", "Rate-limit the public orders endpoint"]);
  assert.equal(collision.sameBranch, false);

  store.ack(asking, Date.now());
  snap = state.liveSnapshot();
  assert.equal(of(asking).needsYou, null, "acknowledged");
  store.ack(asking, 0);
  const node = state.index.get(asking);
  node.archived = true;
  state.index.get(stuck).archived = true;
  snap = state.liveSnapshot();
  assert.deepEqual([of(asking).needsYou, of(stuck).stuck], [null, null]);
  node.archived = false;
  state.index.get(stuck).archived = false;
  node.source = "exec";
  snap = state.liveSnapshot();
  assert.equal(of(asking).needsYou, null, "codex exec runs never wait for you");
  delete node.source;
  const gone = state.index.get(SESSION(1));
  state.index.delete(SESSION(1));
  snap = state.liveSnapshot();
  assert.ok(!snap.collisions.some((c) => [c.a, c.b].includes(SESSION(1))), "only sessions on the map collide");
  state.index.set(SESSION(1), gone);
});

test("search, digests, the day's log and a narration", async () => {
  const results = await state.searchTranscripts("partial refunds");
  assert.ok(results.length > 0);
  assert.ok(results.some((r) => r.sessionId === SESSION(1) && r.inGraph && r.title === "Add partial refunds to the orders API"));
  const digest = state.digest(Date.now() - 12 * 3600e3);
  assert.ok(digest.totals.sessions > 0);
  await state.refreshRepos();
  const log = state.dayLog();
  assert.equal(log.hours.length, 24);
  assert.ok(log.uncommitted.some((r) => r.name === "atlas-api" && r.dirty > 0));
  assert.match(await state.narrate("day"), /While you were away/);
});

test("a handoff brief, from what the tailer saw or else the transcript", async () => {
  await assert.rejects(state.handoff("core", "codex"), /unknown session/);
  const seen = await state.handoff(SESSION(1), "codex");
  assert.match(seen.brief, /Continue the work from a Claude Code session titled "Add partial refunds/);
  assert.equal(seen.folder, state.index.get(SESSION(1)).cwd);

  const fresh = new XenoNetState();
  stops.push(() => fresh.stop());
  await fresh.refresh();
  const old = await fresh.handoff(SESSION(7), "claude");
  assert.match(old.brief, /Original goal: Move staging to the new cluster\./);
  assert.match(old.brief, /Where it left off:\nStaging runs on the new cluster/);
});

test("the tray's summary", () => {
  const s = state.summary();
  assert.ok(s.working >= 0 && s.tpm >= 0);
  assert.ok(s.needsYou.every((n) => n.kind !== "review"));
  assert.ok(s.stuck.some((x) => x.id === SESSION(4, "codex")));
  assert.equal(s.url, `http://127.0.0.1:${port}/`);
});

test("started, it follows the agents: live updates, a nudge to commit, and a stop", async () => {
  const live = new XenoNetState();
  stops.push(() => live.stop());
  const events = { live: 0, activity: [] };
  live.on("live", () => events.live++);
  live.on("activity", (a) => events.activity.push(a));
  await live.start();
  assert.ok(live.graph && live.live.status.watching);
  await live.refreshRepos();
  assert.ok([...live.repos.status.values()].some((r) => r.dirty > 0), "the demo leaves work uncommitted");
  const turnEnd = { kind: "turn-end", sessionId: SESSION(1), app: "claude", ts: Date.now() };
  // A turn you stopped is likely half done: no nudge for it.
  live.live.emit("activity", { ...turnEnd, interrupted: true });
  assert.equal(events.activity.filter((a) => a.kind === "ship-it").length, 0);
  live.live.emit("activity", turnEnd);
  live.live.emit("activity", { ...turnEnd, ts: Date.now() + 1 });
  await wait(700);
  const nudges = events.activity.filter((a) => a.kind === "ship-it");
  assert.equal(nudges.length, 1, "once, not on every turn");
  assert.match(nudges[0].detail, /^\d+ uncommitted files? in atlas-api$/);
  assert.ok(events.live >= 1, "a live snapshot follows activity");
  live.stop();
  // Stopped before its first index was due, it doesn't open the search file
  // again afterwards.
  await wait(1000);
  assert.equal(live.search.db, null);
});

test("plan limits name the account they were read for", async () => {
  await state.live.start();
  state.live.stop();
  await state.refresh();
  const { limits } = state.liveSnapshot();
  // The demo's Claude Code is signed in to Nova's personal account, and
  // Codex to its ChatGPT workspace (core/demo.js).
  assert.match(limits.claude.identity, /^claude:0190d3e0-0000-7000-a000-000000000001:/);
  assert.equal(limits.codex.identity, "chatgpt:demo-workspace");
  assert.equal(state.live.limits.codex.identity, undefined, "the tailer's own reading stays as it was");
});
