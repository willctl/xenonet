// xenonet status and xenonet agents (cli/commands/status.js, agents.js) with
// actors at work: the engine counts them among the agents, and these
// commands count them apart, the sessions connected to XenoNet included.
// Against a stand-in XenoNet with made-up sessions and actors.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// A stand-in XenoNet: `routes` maps "GET /api/x" to the data it answers.
async function fakeXenoNet(routes) {
  const server = http.createServer((req, res) => {
    const route = routes[`${req.method} ${new URL(req.url, "http://x").pathname}`];
    res.writeHead(route ? 200 : 404, { "content-type": "application/json" }).end(JSON.stringify(route ?? { error: "not found" }));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, close: () => server.close() };
}

function xenonet(args) {
  return new Promise((resolve) => {
    const proc = spawn(process.execPath, [path.join(root, "cli", "xenonet.js"), ...args], { cwd: root, env: { ...process.env, NO_COLOR: "1", XENONET_PORT: "" }, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.on("exit", (code) => resolve({ code, stdout }));
  });
}

const now = Date.now();
const health = { ok: true, name: "XenoNet Dev", version: "9.9.9", demo: false, sources: [{ name: "projects", ok: true, count: 2 }] };
const summary = { working: 1, needsYou: [], review: 0, stuck: [], collisions: 0 };
const session = (id, title, extra = {}) => ({ id, app: id.split(":")[0], title, project: "atlas-api", branch: "main", status: "working", needsYou: null, focus: null, editing: [], claims: [], lastActive: now, reachable: false, top: "/w/atlas-api", cwd: "/w/atlas-api", ...extra });
const A = session("claude:aaaaaaaa-0001", "Add partial refunds");
const B = session("codex:bbbbbbbb-0002", "Rate-limit the API", { status: "idle" });
// Wren reports itself, so messages reach it; Hermes XenoNet only observes.
const WREN = session("actor:wren", "Wren", { app: "actor", reachable: true, focus: "Rebuilding the docs" });
const HERMES = session("actor:hermes", "Hermes", { app: "actor", status: "idle", branch: null, editing: [{ path: "/w/atlas-api/src/app.ts", at: now, observed: true }] });
const snapshot = (id, name, presence) => ({ schema: "actor/snapshot/v1", id, kind: id === "actor:you" ? "human" : "agent", name, product: null, host: null, via: ["registered"], presence, presenceSince: null, lastSeen: null, heartbeatMs: null, status: null, capabilities: [], targets: [], recent: [], counts: { events24h: 0, spawned24h: 0, messages24h: 0 }, identity: null });
const actorList = { schema: "actor/list/v1", at: now, actors: [snapshot("actor:you", "You", "unknown"), snapshot("actor:wren", "Wren", "working"), snapshot("actor:hermes", "Hermes", "idle"), snapshot("actor:grok-bot", "Grok Bot", "offline")] };

test("xenonet status counts the actors apart from the agents, and never as sessions connected to XenoNet", async () => {
  const s = await fakeXenoNet({ "GET /api/health": health, "GET /api/summary": summary, "GET /api/engine/agents": [A, B, WREN, HERMES], "GET /api/actors": actorList });
  try {
    const out = await xenonet(["status", "--port", String(s.port)]);
    assert.equal(out.code, 0);
    assert.match(out.stdout, /Agents\s+1 working, 1 idle\n/, "two sessions, neither connected");
    assert.match(out.stdout, /Actors\s+1 working, 1 idle, 1 offline\n/, "you, a placeholder here, aren't counted");
    const data = JSON.parse((await xenonet(["status", "--json", "--port", String(s.port)])).stdout);
    assert.deepEqual(data.agents.map((a) => a.id), [A.id, B.id]);
    assert.deepEqual(data.actors, [
      { id: "actor:wren", name: "Wren", presence: "working", status: null },
      { id: "actor:hermes", name: "Hermes", presence: "idle", status: null },
      { id: "actor:grok-bot", name: "Grok Bot", presence: "offline", status: null },
    ]);
  } finally {
    s.close();
  }
  // With actors off, or none, no line for them.
  const quiet = await fakeXenoNet({ "GET /api/health": health, "GET /api/summary": summary, "GET /api/engine/agents": [{ ...A, reachable: true }] });
  try {
    const out = await xenonet(["status", "--port", String(quiet.port)]);
    assert.match(out.stdout, /Agents\s+1 working, 0 idle, 1 connected to XenoNet\n/);
    assert.doesNotMatch(out.stdout, /Actors/);
  } finally {
    quiet.close();
  }
});

test("xenonet agents lists the actors apart, after the sessions, and counts only sessions as connected", async () => {
  const s = await fakeXenoNet({ "GET /api/health": health, "GET /api/engine/agents": [WREN, A, HERMES, B] });
  try {
    const out = await xenonet(["agents", "--port", String(s.port)]);
    assert.match(out.stdout, /^● Add partial refunds {2}claude · atlas-api · main\n○ Rate-limit the API {2}codex · atlas-api · main · just now\n\nActors\n● Wren {2}atlas-api · main\n {2}says: Rebuilding the docs\n○ Hermes {2}atlas-api · just now\n {2}editing app\.ts \(observed\)\n/);
    assert.match(out.stdout, /None of these sessions is connected to XenoNet/, "Wren reaches XenoNet with its token, which isn't a session connected");
    assert.equal(JSON.parse((await xenonet(["agents", "--json", "--port", String(s.port)])).stdout).length, 4, "the full list, actors included");
  } finally {
    s.close();
  }
  const actorsOnly = await fakeXenoNet({ "GET /api/health": health, "GET /api/engine/agents": [WREN] });
  try {
    const out = await xenonet(["agents", "--port", String(actorsOnly.port)]);
    assert.equal(out.stdout, "Actors\n● Wren  atlas-api · main\n  says: Rebuilding the docs\n", "no sessions, so nothing to say of their connections");
  } finally {
    actorsOnly.close();
  }
});
