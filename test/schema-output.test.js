// What XenoNet really produces, checked against its schemas: the demo world's
// map, live state, history, identities, a doctor run, the engine's state,
// every built-in theme and state.json, in a demo world of the test's own
// (the demo keeps one per port), and made-up data for the edge cases the
// demo doesn't reach.
//
// Each check is strict, so a field a schema leaves out fails as well as a
// field of the wrong type, and each runs twice: against the definition,
// and against the file in schemas/, read back the way anyone else would
// read it.
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
const store = await import("../core/store.js");
const { world, demoSessions } = await import("../core/demo.js");
const { check, explain } = await import("../core/schema/index.js");
const { runDoctor } = await import("../core/doctor/index.js");
const { doctorContext } = await import("../core/doctor/context.js");
const { Engine } = await import("../core/engine/engine.js");
const { EngineStore } = await import("../core/engine/store.js");
const { themes } = await import("../core/themes.js");
const { readThemes } = await import("../core/themefiles.js");
const z = await import("zod/v4");
const { FIXTURES } = await import("./fixtures/schemas.js");
const { drift } = await import("../core/drift.js");
const { fromFile } = await import("./fixtures/schema-files.js");

const SESSION = (i, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`;
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-schema-output-"));
  made.push(dir);
  return dir;
};
const state = new XenoNetState();
after(() => {
  state.stop();
  for (const dir of made) fs.rmSync(dir, { recursive: true, force: true });
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});

// The data as it goes over the wire.
const wire = (x) => JSON.parse(JSON.stringify(x));

function valid(id, data, what = id) {
  const d = wire(data);
  const r = check(id, d, { strict: true });
  assert.ok(r.ok, `${what} doesn't fit ${id}:\n${explain(r)}`);
  const f = fromFile(id).safeParse(d);
  assert.ok(f.success, `${what} doesn't fit schemas/${id}.json:\n${f.success ? "" : z.prettifyError(f.error)}`);
}

function invalid(id, data, what) {
  const d = wire(data);
  assert.equal(check(id, d).ok, false, `${what} fits ${id}`);
  assert.equal(fromFile(id).safeParse(d).success, false, `${what} fits schemas/${id}.json`);
}

test("the map, as /api/graph sends it, and the sessions and projects it's made of", async () => {
  await state.refresh();
  await state.refreshRepos();
  const graph = await state.refresh();
  valid("graph/v1", graph);
  assert.deepEqual([...new Set(graph.nodes.map((n) => n.kind))].sort(), ["core", "native", "page", "project", "resource", "session", "wiki", "zone"]);
  assert.ok(graph.nodes.some((n) => n.repo?.dirty), "with repo status");
  assert.ok(graph.nodes.some((n) => n.kind === "project" && n.identities.some((l) => l.role === "commits-as")), "and git identities");
  for (const app of ["claude", "codex"]) for (const s of demoSessions(app)) valid("session/v1", s, s.id);
  for (const p of state.projects) valid("project/v1", p, p.rel);
});

test("live state, each live session, and a slice of the history", async () => {
  await state.live.start();
  state.live.stop();
  const snap = state.liveSnapshot();
  valid("live/snapshot/v1", snap);
  assert.ok(snap.sessions.some((s) => s.needsYou) && snap.sessions.some((s) => s.stuck) && snap.collisions.length, "with sessions that need you, seem stuck and collide");
  for (const s of snap.sessions) valid("live/session/v1", s, s.id);
  const history = state.live.historySince(Date.now() - 6 * 3600e3);
  assert.ok(history.length > 20);
  assert.ok(new Set(history.map((e) => e.kind)).size >= 6, "of several kinds");
  for (const e of history) valid("live/activity/v1", e, `a ${e.kind} event`);
});

test("identities, as /api/identities sends them", () => {
  const { identities } = state.identityDetection();
  assert.deepEqual([...new Set(identities.map((i) => i.kind))].sort(), ["chatgpt", "claude", "git", "github"]);
  for (const i of identities) valid("identity/v1", i, i.id);
});

test("the demo's made-up vendor data fits the vendor schemas: the map and the live state read it without drift", () => {
  assert.deepEqual(drift.list().map((r) => [r.source, r.kind, r.path]), []);
});

test("a doctor run, as the demo's server runs it", async () => {
  const ctx = doctorContext({ self: { port: config.port, name: "XenoNet (demo)" }, health: { sources: state.sources }, live: state.liveSnapshot(), drift: () => drift.view() });
  const run = await runDoctor(ctx, { only: ["xenonet.server", "xenonet.sources", "contracts.drift", "claude.transcripts", "codex.sessions", "system.projects", "system.runtime"] });
  assert.equal(run.results.find((r) => r.id === "contracts.drift").status, "ok");
  valid("doctor/run/v1", run);
  for (const r of run.results) valid("doctor/result/v1", r, r.id);
});

test("the engine: its state for the console, agents, a message on its way and in an inbox, and its rows", async () => {
  const engineStore = new EngineStore(path.join(tmp(), "engine.sqlite"));
  const engine = new Engine({ state, store: engineStore });
  const [a, b] = [SESSION(1), SESSION(2, "codex")];
  const events = [];
  engine.on("message", (m) => events.push(m));
  engine.touch(a, { app: "claude", cwd: state.index.get(a).cwd, via: "mcp", pid: process.pid });
  engine.touch(b, { app: "codex", cwd: null, via: "hook" });
  engine.setFocus(a, "Partial refunds");
  await engine.claim(a, { paths: ["src/orders/", "src/orders/refunds.ts"], note: "refunds" });
  const sent = await engine.send(a, { to: b, text: "I'm in the refunds code; stay clear of src/orders for now." });
  try {
    const snap = await engine.snapshot();
    assert.ok(snap.agents.length > 1 && snap.messages.length === 1);
    for (const x of snap.agents) valid("engine/agent/v1", x, `${x.id} in the snapshot`);
    for (const m of snap.messages) valid("engine/message/v1", m, "a message in the snapshot");
    for (const x of await engine.peers(a, { scope: "machine" })) valid("engine/agent/v1", x, `${x.id} among peers`);
    valid("engine/agent/v1", (await engine.whoami(a)).you, "whoami");
    for (const x of sent.to) valid("engine/agent/v1", x, "a recipient");
    valid("engine/message/v1", events[0], "the agent-message event");
    const inbox = await engine.inbox(b);
    assert.equal(inbox.length, 1);
    valid("engine/message/v1", inbox[0], "a message in the inbox");
    valid("engine/contact/v1", engineStore.contact(a), "a contact");
    valid("engine/contact/v1", engineStore.contact(b), "a contact by hook");
    for (const c of engineStore.claims()) valid("engine/claim/v1", c, "a claim's row");
    for (const m of engineStore.recent(0)) valid("engine/message/v1", m, "a message's row");
  } finally {
    engineStore.close();
  }
});

test("every built-in theme, and one of your own with the new tokens", () => {
  for (const [id, theme] of Object.entries(themes())) valid("theme/v1", theme, id);
  const dir = tmp();
  fs.writeFileSync(path.join(dir, "rime.json"), JSON.stringify(FIXTURES.find((f) => f.id === "theme/v1").data));
  const { themes: mine, problems } = readThemes(dir);
  assert.deepEqual(problems, []);
  valid("theme/v1", mine.rime, "a theme of your own, resolved");
});

test("state.json, as the store writes it", async () => {
  store.setLayout({ core: { x: 1, y: 2, z: 3 } });
  store.ack(SESSION(3));
  store.setTheme("phosphor");
  store.setProjectRule(`${config.projectsDir.toLowerCase()}/labs`, { path: path.join(config.projectsDir, "labs"), as: "group", at: Date.now() });
  await state.askCodexAboutProjects((await state.codexOutline()).fingerprint);
  store.flush();
  const file = JSON.parse(fs.readFileSync(path.join(config.stateDir, "state.json"), "utf8"));
  assert.ok(Object.keys(file.projects.codexByRoot).length && Object.keys(file.identities.known).length, "with Codex's picks and the identities XenoNet has seen");
  valid("state/v2", file, "state.json");
  // The demo writes its state as an earlier XenoNet would have, so it was moved up.
  valid("state/v1", JSON.parse(fs.readFileSync(path.join(config.stateDir, "state.v1.json"), "utf8")), "the copy kept of the demo's v1 file");
});

test("made-up edge cases fit, and the ones that mustn't don't", () => {
  for (const f of FIXTURES) (f.valid === false ? invalid : valid)(f.id, f.data, f.name);
});
