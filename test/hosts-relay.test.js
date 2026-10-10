// xenonet relay (core/hosts/relay.js) against a stand-in XenoNet: a made-up map
// served on 127.0.0.1 with XenoNet's own Host check, a home folder of the
// test's own, and the event stream held open so the test can send events.
// What leaves must carry no home folder, nothing from another host, and
// only titles, states and counts unless the console asked for detail; and
// nothing a console sends may write to XenoNet.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import nativePath from "node:path";
const path = nativePath.posix;
import { PassThrough } from "node:stream";
import { fileURLToPath } from "node:url";
import { runRelay, homeScrubber, userFolderOf, userFolders, allowed, shapeGraph, shapeLive, shapeEngine, parseSince, fromOriginalCommand, EXIT } from "../core/hosts/relay.js";
import { lineSplitter, encode } from "../core/hosts/frames.js";
import { check } from "../core/schema/index.js";
import { pathKey } from "../core/paths.js";
import { fakeClock } from "./harness/clock.js";

const root = nativePath.resolve(nativePath.dirname(fileURLToPath(import.meta.url)), "..");
const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

// A home folder of the test's own, with an SSH alias the relay resolves.
const HOME = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-relay-home-"));
made.push(HOME);
fs.mkdirSync(path.join(HOME, ".ssh"));
fs.writeFileSync(path.join(HOME, ".ssh", "config"), "Host gh-orbit\n  HostName github.com\n");
const P = path.join(HOME, "Projects");
const KEY = (p) => pathKey(p);
const NOW = Date.now();
const DAY = 86400e3;
const S = (n, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(n).padStart(12, "0")}`;
const placed = { fx: null, fy: null, fz: null, pinned: false, actions: [] };
const rolled = { sessionCount: 0, activeCount: 0, lastActivity: null, lastSessionAt: null };
const session = (n, extra = {}) => ({ id: S(n), kind: "session", app: "claude", rawId: S(n).slice(7), title: `Session ${n}`, preview: `the first prompt of session ${n}, with a secret plan`, cwd: path.join(P, "atlas-api"), createdAt: NOW - 3600e3, updatedAt: NOW - 60e3, archived: false, model: null, branch: "main", parentId: null, parentKind: null, activity: "active", ...placed, ...extra });

function made_up_graph() {
  const atlas = `project:${KEY(path.join(P, "atlas-api"))}`;
  const sim = `project:${KEY(path.join(P, "route-sim"))}`;
  return {
    generatedAt: NOW,
    nodes: [
      { id: "core", kind: "core", name: "~/Projects", path: P, home: HOME, ...placed, ...rolled },
      { id: atlas, kind: "project", name: "atlas-api", path: path.join(P, "atlas-api"), key: KEY(path.join(P, "atlas-api")), git: { branch: "main", origin: "https://github.com/atlas-co/atlas-api.git", worktreeOf: null, submoduleOf: null }, group: false, satelliteOf: null, repo: null, ...placed, ...rolled },
      { id: sim, kind: "project", name: "route-sim", path: path.join(P, "route-sim"), key: KEY(path.join(P, "route-sim")), git: { branch: "main", origin: "git@gh-orbit:orbitworks/route-sim.git", worktreeOf: null, submoduleOf: null }, group: false, satelliteOf: null, repo: null, ...placed, ...rolled },
      session(1),
      session(2, { updatedAt: NOW - 9 * DAY, activity: "idle" }),
      session(3, { parentId: S(2), parentKind: "spawn" }),
      session(4, { archived: true, activity: "recent" }),
      // What a XenoNet that shows other hosts itself would have in its map.
      { ...session(5), id: `@elsewhere:${S(5)}` },
      { id: "@elsewhere:core", kind: "host", name: "elsewhere", ...placed, ...rolled },
      { id: "page", kind: "page", name: "Refunds", slug: "refunds", summary: "What we decided about refunds", path: path.join(HOME, "wiki", "pages", "refunds.md"), ...placed, ...rolled },
    ],
    links: [
      { source: "core", target: atlas, kind: "contains" },
      { source: "core", target: sim, kind: "contains" },
      { source: atlas, target: S(1), kind: "session" },
      { source: atlas, target: S(2), kind: "session" },
      { source: S(2), target: S(3), kind: "lineage" },
      { source: atlas, target: S(4), kind: "session" },
      { source: "core", target: "@elsewhere:core", kind: "host" },
      { source: "@elsewhere:core", target: `@elsewhere:${S(5)}`, kind: "session" },
      { source: "core", target: "page", kind: "page" },
    ],
    relations: [{ a: "page", b: atlas, kind: "mentions" }, { a: atlas, b: `@elsewhere:${S(5)}`, kind: "same-repo" }],
    stats: { wikiPages: 1, dirtyRepos: 0, projects: 2, groups: 0, sessions: 5, claude: 5, codex: 0, active: 3, archived: 1 },
    identities: [
      { id: "claude:a1:o1", kind: "claude", handle: "nova@example.com", signedIn: true, sources: [path.join(HOME, ".claude.json")] },
      { id: "@elsewhere:openai:api-key", kind: "openai", handle: null, signedIn: true, sources: [] },
    ],
    hosts: [{ id: "elsewhere", state: "live" }],
    self: "lab",
    sources: [],
  };
}

const liveSession = (n, extra = {}) => ({ id: S(n), app: "claude", status: "tool", tpm: 10, lastMessage: `I changed ${path.join(P, "atlas-api", "src", "refunds.ts")} as you asked`, needsYou: { kind: "question", text: "Should I drop the staging database?", since: NOW }, lastTool: { name: "Bash", category: "shell", detail: "npm test -- refunds", ts: NOW }, files: [{ path: path.join(P, "atlas-api", "src", "refunds.ts"), ts: NOW }], stuck: { reasons: ["ran npm test -- refunds 4 times in a row", "silent for 9 min"] }, subagents: [{ id: "agent-1", label: "Rewrite the refunds memo for th", startedAt: NOW, lastAt: NOW, tools: 2, done: false, active: true }], lastEventAt: NOW, ...extra });
const madeUpLive = () => ({
  at: NOW,
  sessions: [liveSession(1), liveSession(2), { ...liveSession(5), id: `@elsewhere:${S(5)}` }],
  series: { claude: [1, 2, 3], codex: [0, 0, 0] },
  tpm: 30,
  total24h: 100,
  today: 50,
  limits: { claude: null, codex: null, others: [{ app: "claude", host: "elsewhere" }] },
  limitForecast: {},
  collab: [{ a: S(1), b: S(3), kind: "file", files: [path.join(P, "atlas-api", "src", "refunds.ts")] }],
  collisions: [{ a: S(1), b: S(3), files: [path.join(P, "atlas-api", "src", "refunds.ts")], paths: ["src/refunds.ts"], ts: NOW, branches: ["main", "main"], sameBranch: true, titles: ["Session 1", "Session 3"] }],
  resources: [],
  status: { watching: true, files: 3, lines: 9, errors: 0 },
});
const madeUpEngine = () => ({
  agents: [{ id: S(1), title: "Session 1", app: "claude", project: "atlas-api", branch: "main", status: "working", focus: "refunds", editing: 1, claims: [{ pattern: path.join(P, "atlas-api", "src") + "/", note: "moving refunds", until: NOW + 3600e3 }], reachable: true }, { id: `@elsewhere:${S(5)}`, title: "x", app: "claude" }],
  messages: [{ id: 7, from: S(1), target: "repo", text: "I'm moving refunds into a module", at: NOW, to: [{ id: S(3), seen: false }] }],
});

// The stand-in XenoNet. `self` null answers 404 for /api/hosts/self, as an
// older XenoNet would; `graph` and `live` make what it answers with.
async function standIn({ self = { id: "lab", key: "9a2b0000000000000000000000000002", home: HOME }, demo = false, graph = made_up_graph, live = madeUpLive } = {}) {
  const seen = [];
  const streams = new Set();
  const server = http.createServer((req, res) => {
    const { port } = server.address();
    seen.push({ method: req.method, url: req.url });
    if (req.headers.host !== `127.0.0.1:${port}`) return res.writeHead(421).end("{}");
    const url = new URL(req.url, "http://x");
    const json = (status, body) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    if (req.method !== "GET") return json(405, { error: "the relay never writes" });
    if (url.pathname === "/api/health") return json(200, { ok: true, name: "XenoNet", version: "0.4.0", demo });
    if (url.pathname === "/api/hosts/self") return self ? json(200, self) : json(404, { error: "not found" });
    if (url.pathname === "/api/graph") return json(200, graph());
    if (url.pathname === "/api/live") return json(200, live());
    if (url.pathname === "/api/engine/state") return json(200, madeUpEngine());
    if (url.pathname === `/api/session/${S(1)}/transcript`) return json(200, { entries: [{ role: "user", text: `look at ${path.join(HOME, "notes.md")}` }], cursor: null, limit: url.searchParams.get("limit") });
    if (url.pathname === "/api/events") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.write("retry: 3000\n\n");
      streams.add(res);
      req.on("close", () => streams.delete(res));
      return;
    }
    json(404, { error: "not found" });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return {
    port: server.address().port,
    seen,
    emit: (name, data) => streams.forEach((res) => res.write(`event: ${name}\ndata: ${JSON.stringify(data)}\n\n`)),
    endStreams: () => streams.forEach((res) => res.end()),
    streaming: () => streams.size,
    close: () => (streams.forEach((res) => res.destroy()), new Promise((r) => server.close(r))),
  };
}

// A relay over in-memory pipes: frames it writes, a way to send it frames,
// and its exit.
function relayTo(xenonet, opts = {}) {
  const input = new PassThrough();
  const output = new PassThrough();
  const frames = [];
  const waiters = [];
  output.on("data", lineSplitter({ onLine: (l) => (frames.push(JSON.parse(l)), waiters.forEach((w) => w())) }));
  // Frames written just before the end are read a moment later.
  const exit = runRelay({ port: xenonet.port, input, output, ...opts }).then(async (code) => (await new Promise((r) => setImmediate(r)), code));
  const until = async (pred, ms = 5000) => {
    const deadline = Date.now() + ms;
    for (;;) {
      const hit = frames.find(pred);
      if (hit) return hit;
      if (Date.now() > deadline) throw new Error(`no such frame in ${ms} ms; got ${frames.map((f) => f.t + (f.name ? `:${f.name}` : "")).join(", ")}`);
      await new Promise((r) => {
        waiters.push(r);
        setTimeout(r, 50);
      });
    }
  };
  return { input, output, frames, exit, until, send: (f) => input.write(encode(f)) };
}

// No form of the home folder anywhere: as given, as a path key, or as
// Claude Code would name its folder.
function homeless(value, what) {
  const text = JSON.stringify(value).toLowerCase();
  for (const form of [HOME, pathKey(HOME), HOME.replace(/[\\/.:]/g, "-")]) assert.ok(!text.includes(form.toLowerCase()), `${what} carries ${form}`);
  assert.ok(!text.includes(path.basename(HOME).toLowerCase()), `${what} carries the home folder's name`);
}

test("hello first, then a snapshot: paths as ~, nothing from another host, old and archived sessions out, repo keys in, titles, states and counts only", async () => {
  const xenonet = await standIn();
  const r = relayTo(xenonet);
  try {
    const hello = await r.until((f) => f.t === "hello");
    assert.equal(r.frames[0], hello, "the hello comes first");
    const c = check("host/hello/v1", hello, { strict: true });
    assert.ok(c.ok, JSON.stringify(c));
    assert.deepEqual([hello.protocol, hello.host, hello.demo, hello.sinceMs, hello.detail], [1, { id: "lab", key: "9a2b0000000000000000000000000002" }, false, 7 * DAY, false]);

    const { graph, live, engine } = await r.until((f) => f.t === "snapshot");
    homeless(graph, "the map");
    homeless(live, "the live state");
    homeless(engine, "the engine's state");
    const ids = graph.nodes.map((n) => n.id);
    assert.ok(ids.includes(S(1)) && ids.includes(S(3)));
    assert.ok(!ids.includes(S(2)), "not updated in the last 7 days");
    assert.ok(!ids.includes(S(4)), "archived");
    assert.ok(!ids.some((id) => id.startsWith("@")) && !graph.nodes.some((n) => n.kind === "host"), "another host's nodes never pass on");
    assert.ok(!graph.links.some((l) => [l.source, l.target].some((x) => x.startsWith("@") || x === S(2))));
    assert.deepEqual(graph.relations.map((x) => x.kind), ["mentions"]);
    assert.ok(!("hosts" in graph) && !("self" in graph));
    assert.deepEqual(graph.identities.map((i) => i.id), ["claude:a1:o1"]);
    const atlas = graph.nodes.find((n) => n.name === "atlas-api");
    assert.equal(atlas.id, "project:~/projects/atlas-api", "path keys are tilded on the host");
    assert.equal(atlas.path, "~/Projects/atlas-api");
    assert.deepEqual(graph.links.find((l) => l.target === S(3)), { source: atlas.id, target: S(3), kind: "session" }, "a session kept under one left out moves up to its project");
    assert.equal(atlas.repoKey, "github.com/atlas-co/atlas-api");
    assert.equal(graph.nodes.find((n) => n.name === "route-sim").repoKey, "github.com/orbitworks/route-sim", "the host's own SSH alias, resolved there");
    assert.equal(graph.nodes.find((n) => n.id === S(1)).preview, null, "a first prompt is what's said, not a title");
    assert.equal(graph.nodes.find((n) => n.id === S(1)).title, "Session 1");
    assert.equal(graph.nodes.find((n) => n.kind === "page").summary, "");
    assert.deepEqual([graph.stats.sessions, graph.stats.archived], [2, 0]);

    assert.deepEqual(live.sessions.map((s) => s.id), [S(1)]);
    const s1 = live.sessions[0];
    assert.deepEqual([s1.status, s1.tpm, s1.lastMessage, s1.needsYou, s1.lastTool.detail, s1.files], ["tool", 10, null, { kind: "question", text: "", since: NOW }, "", []]);
    assert.deepEqual(s1.stuck.reasons, ["ran the same command 4 times in a row", "silent for 9 min"]);
    assert.deepEqual(s1.subagents.map((a) => [a.id, a.label, a.tools]), [["agent-1", "subagent", 2]], "a subagent's label is the start of its task");
    assert.deepEqual(live.collisions.map((c) => [c.a, c.b, c.files, c.paths, c.titles]), [[S(1), S(3), [], [], ["Session 1", "Session 3"]]], "a collision crosses as a state, without its files");
    assert.deepEqual(live.collab.map((c) => c.files), [[]]);
    assert.ok(!("others" in live.limits));
    assert.deepEqual(engine.agents.map((a) => a.id), [S(1)]);
    assert.deepEqual(engine.agents[0].claims, [{ pattern: "", note: null, until: NOW + 3600e3 }]);
    assert.equal(engine.messages[0].text, "");
    assert.ok(xenonet.seen.every((x) => x.method === "GET"));
    assert.ok(xenonet.seen.some((x) => x.url === "/api/graph?hosts=0") && xenonet.seen.some((x) => x.url === "/api/events?hosts=0"), "its own view, not one with other hosts in it");
  } finally {
    r.input.end();
    assert.equal(await r.exit, EXIT.done);
    await xenonet.close();
  }
});

test("events follow, shaped the same way, and only those a console needs", async () => {
  const xenonet = await standIn();
  const r = relayTo(xenonet);
  try {
    await r.until((f) => f.t === "snapshot");
    xenonet.emit("theme", { id: "phosphor" });
    xenonet.emit("activity", { sessionId: `@elsewhere:${S(5)}`, app: "claude", kind: "prompt", detail: "not ours", ts: NOW });
    xenonet.emit("activity", { sessionId: S(1), app: "claude", kind: "prompt", detail: "Please read my diary", ts: NOW });
    xenonet.emit("activity", { sessionId: S(1), app: "claude", kind: "tool", tool: "Edit", category: "edit", detail: path.join(P, "atlas-api", "x.ts"), files: [path.join(P, "atlas-api", "x.ts")], server: null, ts: NOW });
    xenonet.emit("activity", { kind: "identity", app: "claude", identity: "claude:a1:o1", before: null, ts: NOW });
    xenonet.emit("activity", { sessionId: S(1), app: "claude", kind: "subagent-start", agentId: "agent-2", label: "Find every caller of refund()", ts: NOW });
    xenonet.emit("tokens", { sessionId: S(1), app: "claude", n: 120, ts: NOW });
    xenonet.emit("pulse", { nodeId: S(1), reason: "updated" });
    xenonet.emit("agent-message", { id: 8, from: S(1), to: [S(3)], target: null, text: "secret", ts: NOW });
    xenonet.emit("doctor", { at: NOW, summary: { ok: 1, info: 0, warn: 1, fail: 0, skip: 0 }, results: [{ id: "codex.auth", title: "Codex is signed in", status: "warn", detail: `odd file ${path.join(HOME, ".codex")}` }, { id: "xenonet.server", status: "ok" }] });
    xenonet.emit("graph", { ...made_up_graph(), nodes: made_up_graph().nodes.filter((n) => n.id !== S(3)) });
    xenonet.emit("live", madeUpLive());
    const graph = await r.until((f) => f.t === "event" && f.name === "graph");
    await r.until((f) => f.t === "event" && f.name === "live");
    const events = r.frames.filter((f) => f.t === "event");
    homeless(events, "the events");
    assert.deepEqual(events.map((e) => e.name), ["activity", "activity", "activity", "tokens", "pulse", "agent-message", "doctor", "graph", "live"], "not the theme, not another host's activity, not a sign-in");
    const [prompt, tool, subagent] = events;
    assert.deepEqual([prompt.data.detail, tool.data.detail, tool.data.files], ["", "", []]);
    assert.deepEqual([subagent.data.kind, subagent.data.agentId, subagent.data.label], ["subagent-start", "agent-2", "subagent"]);
    assert.equal(events[5].data.text, "");
    assert.deepEqual(events[6].data, { at: NOW, summary: { ok: 1, info: 0, warn: 1, fail: 0, skip: 0 }, problems: [{ id: "codex.auth", title: "Codex is signed in", status: "warn", detail: "odd file ~/.codex" }] });
    assert.ok(!graph.data.nodes.some((n) => n.id.startsWith("@")));
  } finally {
    r.input.end();
    await r.exit;
    await xenonet.close();
  }
});

test("a console reads only the allowlist, and never writes", async () => {
  const xenonet = await standIn();
  const r = relayTo(xenonet);
  try {
    await r.until((f) => f.t === "snapshot");
    const refused = ["/api/open", "/api/ack", "/api/dev/control", "/api/dev/simulate", "/api/engine/connect", "/api/session/../../../etc/passwd", "/api/session/claude:x/../../identities", "/api/session/%2e%2e/stats", "/api/session/claude%3Ax/stats", "//api/graph", "/api/graph?hosts=1", "/api/graph#x", "/api\\graph", "/api/history", "/api/identities", "/api/search?q=password", "http://evil.example/api/graph", "/api/session/claude:x/transcript?limit=5&extra=1", "/api/session/claude:x/transcript?limit=lots", "/api/collision?a=x&b=y", "", 7];
    // Keys every object has: refused like any other, never a crash.
    refused.push("/api/graph?constructor=1", `/api/session/${S(1)}/transcript?__proto__=1`, `/api/collision?a=${S(1)}&b=${S(2)}&toString=x`);
    refused.forEach((p, i) => r.send({ t: "get", id: 100 + i, path: p }));
    r.send({ t: "post", id: 200, path: "/api/open", body: { nodeId: S(1), action: "open-app" } });
    r.send({ t: "get", id: 201, path: "/api/open", method: "POST" });
    r.send({ t: "exec", cmd: "touch /tmp/pwned" });
    r.input.write("not json at all\n");
    r.send({ t: "get", id: 1, path: `/api/session/${S(1)}/transcript?limit=5` });
    r.send({ t: "get", id: 2, path: "/api/graph" });
    r.send({ t: "time", id: 3 });
    const transcript = await r.until((f) => f.t === "res" && f.id === 1);
    assert.equal(transcript.status, 200);
    assert.deepEqual(transcript.body, { entries: [{ role: "user", text: "look at ~/notes.md" }], cursor: null, limit: "5" }, "transcripts cross when opened, as ~");
    const map = await r.until((f) => f.t === "res" && f.id === 2);
    assert.ok(map.body.nodes.every((n) => !n.id.startsWith("@")));
    homeless(map.body, "a map read through the relay");
    const time = await r.until((f) => f.t === "time" && f.id === 3);
    assert.ok(Math.abs(time.now - Date.now()) < 5000);
    for (let i = 0; i < refused.length; i++) {
      const res = await r.until((f) => f.t === "res" && f.id === 100 + i);
      assert.equal(res.status, 403, String(refused[i]));
    }
    assert.equal((await r.until((f) => f.t === "res" && f.id === 201)).status, 403, "a get is a get, whatever else it says");
    assert.ok(!r.frames.some((f) => f.id === 200), "a frame that isn't a get or a time is dropped");
    assert.ok(xenonet.seen.every((x) => x.method === "GET"), "nothing but GETs reach XenoNet");
    assert.ok(!xenonet.seen.some((x) => /\/api\/(open|ack|dev|engine\/connect|identities|history|search)/.test(x.url)), "and none of the refused routes");
  } finally {
    r.input.end();
    await r.exit;
    await xenonet.close();
  }
});

test("asked for detail, the live detail crosses too, still as ~", async () => {
  const xenonet = await standIn();
  const r = relayTo(xenonet, { detail: true, sinceMs: 30 * DAY });
  try {
    const hello = await r.until((f) => f.t === "hello");
    assert.deepEqual([hello.detail, hello.sinceMs], [true, 30 * DAY]);
    const { graph, live } = await r.until((f) => f.t === "snapshot");
    homeless(live, "the live state");
    assert.ok(graph.nodes.some((n) => n.id === S(2)), "30 days back");
    const s1 = live.sessions.find((s) => s.id === S(1));
    assert.equal(s1.lastMessage, "I changed ~/Projects/atlas-api/src/refunds.ts as you asked");
    assert.equal(s1.needsYou.text, "Should I drop the staging database?");
    assert.deepEqual(s1.files, [{ path: "~/Projects/atlas-api/src/refunds.ts", ts: NOW }]);
    assert.equal(s1.subagents[0].label, "Rewrite the refunds memo for th");
  } finally {
    r.input.end();
    await r.exit;
    await xenonet.close();
  }
});

test("the relay stops when the console goes away, and says why when the XenoNet does", async () => {
  const gone = await standIn();
  const r = relayTo(gone);
  await r.until((f) => f.t === "snapshot");
  gone.endStreams();
  assert.equal(await r.exit, EXIT.xenonetGone);
  assert.deepEqual(r.frames.at(-1), { t: "bye", code: "xenonet-gone", message: `the XenoNet on 127.0.0.1:${gone.port} stopped` });
  await gone.close();

  // No XenoNet at all, and one too old to be a host.
  const free = await new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
  const none = relayTo({ port: free });
  assert.equal(await none.exit, EXIT.noXenoNet);
  assert.deepEqual(none.frames, [{ t: "bye", code: "no-xenonet", message: `no XenoNet answers on 127.0.0.1:${free} on this machine` }]);
  const old = await standIn({ self: null });
  const o = relayTo(old);
  assert.equal(await o.exit, EXIT.oldXenoNet);
  assert.equal(o.frames[0].code, "old-xenonet");
  await old.close();
});

test("a projects folder in a folder named for a person, outside the home folder, goes with ~ for the name", async () => {
  // Windows's side of WSL, with a Windows name the home folder doesn't have.
  const WIN = "/mnt/c/Users/Wren";
  const root = `${WIN}/Projects`;
  const atlas = `project:${KEY(`${root}/atlas-api`)}`;
  const scratch = `${WIN}/Downloads/scratch`;
  const graph = () => ({
    ...made_up_graph(),
    nodes: [
      { id: "core", kind: "core", name: "Projects", path: root, home: HOME, ...placed, ...rolled },
      { id: atlas, kind: "project", name: "atlas-api", path: `${root}/atlas-api`, key: KEY(`${root}/atlas-api`), git: { branch: "main", origin: "https://github.com/atlas-co/atlas-api.git", worktreeOf: null, submoduleOf: null }, group: false, satelliteOf: null, repo: null, ...placed, ...rolled },
      { id: `zone:folder:${KEY(scratch)}`, kind: "zone", name: "scratch", path: scratch, app: null, external: true, ...placed, ...rolled, active: false },
      session(1, { cwd: `${root}/atlas-api` }),
    ],
    links: [{ source: "core", target: atlas, kind: "contains" }, { source: atlas, target: S(1), kind: "session" }, { source: "core", target: `zone:folder:${KEY(scratch)}`, kind: "zone" }],
    relations: [],
    identities: [],
  });
  const live = () => ({ ...madeUpLive(), sessions: [liveSession(1, { lastMessage: `wrote ${HOME}/.claude/projects/-mnt-c-Users-Wren-Projects-atlas-api/1.jsonl`, files: [{ path: `${root}/atlas-api/src/refunds.ts`, ts: NOW }] })], collab: [], collisions: [] });
  const xenonet = await standIn({ graph, live });
  const r = relayTo(xenonet, { detail: true });
  try {
    const snap = await r.until((f) => f.t === "snapshot");
    xenonet.emit("activity", { sessionId: S(1), app: "claude", kind: "tool", tool: "Edit", category: "edit", detail: `${root}/atlas-api/x.ts`, files: [`${root.toLowerCase()}/atlas-api/x.ts`], server: null, ts: NOW });
    const tool = await r.until((f) => f.t === "event" && f.name === "activity");
    for (const f of [snap, tool]) assert.doesNotMatch(JSON.stringify(f), /wren/i, "the Windows name stays on its machine");
    homeless(snap, "the snapshot");
    assert.deepEqual(snap.graph.nodes.map((n) => n.path ?? n.cwd), ["/mnt/c/Users/~/Projects", "/mnt/c/Users/~/Projects/atlas-api", "/mnt/c/Users/~/Downloads/scratch", "/mnt/c/Users/~/Projects/atlas-api"]);
    assert.equal(snap.graph.nodes[1].id, "project:/mnt/c/users/~/projects/atlas-api", "path keys too, never mistaken for the home folder's");
    assert.equal(snap.live.sessions[0].lastMessage, "wrote ~/.claude/projects/-mnt-c-Users-~-Projects-atlas-api/1.jsonl");
    assert.deepEqual([tool.data.detail, tool.data.files], ["/mnt/c/Users/~/Projects/atlas-api/x.ts", ["/mnt/c/users/~/projects/atlas-api/x.ts"]]);
  } finally {
    r.input.end();
    await r.exit;
    await xenonet.close();
  }
});

test("nothing but the hello goes before the first snapshot", async () => {
  const xenonet = await standIn();
  const written = [];
  const output = { write: (text) => (written.push(JSON.parse(text)), true), on() {}, off() {} };
  // What the event stream says while the relay reads the map, which the
  // console would drop: it takes no event before a snapshot.
  const follow = async (_port, _route, { onEvent }) => {
    onEvent("activity", { sessionId: S(1), app: "claude", kind: "turn-end", duration: 1, ts: NOW });
    onEvent("live", madeUpLive());
    return { close() {} };
  };
  const input = new PassThrough();
  const exit = runRelay({ port: xenonet.port, input, output, follow });
  try {
    for (const until = Date.now() + 5000; !written.some((f) => f.t === "snapshot") && Date.now() < until; ) await new Promise((res) => setTimeout(res, 20));
    assert.deepEqual(written.map((f) => f.t), ["hello", "snapshot"]);
  } finally {
    input.end();
    await exit;
    await xenonet.close();
  }
});

test("quiet, it pings every 15 seconds; a slow console gets the newest map and live state, and a fresh snapshot after a gap", async () => {
  const xenonet = await standIn();
  const clock = fakeClock(NOW);
  const r = relayTo(xenonet, { clock });
  await r.until((f) => f.t === "snapshot");
  await clock.advance(16e3);
  assert.ok(r.frames.some((f) => f.t === "ping"));
  r.input.end();
  await r.exit;

  // An output that's full until the test lets it drain, and events handed
  // straight to the relay, so nothing here waits on timing.
  const written = [];
  let full = false;
  let drain = null;
  const output = { write: (text) => (written.push(JSON.parse(text)), !full), on: (ev, fn) => ev === "drain" && (drain = fn), off: () => {} };
  let emit = null;
  const follow = async (_port, _route, { onEvent }) => ((emit = onEvent), { close() {} });
  const input = new PassThrough();
  const exit = runRelay({ port: xenonet.port, input, output, follow });
  for (const until = Date.now() + 5000; !written.some((f) => f.t === "snapshot") && Date.now() < until; ) await new Promise((res) => setTimeout(res, 20));
  full = true;
  written.length = 0;
  emit("pulse", { nodeId: S(1), reason: "updated" }); // the write that finds it full
  for (const n of [1, 2, 3]) {
    emit("live", { ...madeUpLive(), tpm: n });
    emit("activity", { sessionId: S(1), app: "claude", kind: "turn-end", duration: n, ts: NOW });
  }
  emit("graph", made_up_graph());
  assert.deepEqual(written.map((f) => f.name), ["pulse"], "nothing more while it's full");
  full = false;
  drain();
  assert.deepEqual(written.map((f) => f.name), ["pulse", "activity", "activity", "activity", "live", "graph"], "events in order, then only the newest live state and map");
  assert.equal(written.find((f) => f.name === "live").data.tpm, 3);

  // A flood past what waits: dropped, then a fresh snapshot.
  written.length = 0;
  full = true;
  emit("pulse", { nodeId: S(1), reason: "updated" });
  const big = "x".repeat(1 << 20);
  for (let i = 0; i < 6; i++) emit("activity", { sessionId: S(1), app: "claude", kind: "turn-end", duration: i, ts: NOW, note: big });
  full = false;
  drain();
  for (const until = Date.now() + 5000; !written.some((f) => f.t === "snapshot") && Date.now() < until; ) await new Promise((res) => setTimeout(res, 20));
  assert.equal(written.filter((f) => f.name === "activity").length, 3, "what didn't fit was dropped");
  assert.ok(written.some((f) => f.t === "snapshot"), "and a snapshot puts the console right");
  input.end();
  await exit;
  await xenonet.close();
});

test("the home folder goes as ~ in every form, in strings and keys, and only whole", () => {
  const scrub = homeScrubber("/private/var/folders/ab/T/xenonet-demo-4711/home");
  assert.deepEqual(
    scrub({
      path: "/private/var/folders/ab/T/xenonet-demo-4711/home/Projects/atlas",
      id: "project:/var/folders/ab/t/xenonet-demo-4711/home/projects/atlas",
      claudeDir: "/x/.claude/projects/-private-var-folders-ab-T-xenonet-demo-4711-home-Projects-atlas/1.jsonl",
      ["/private/var/folders/ab/T/xenonet-demo-4711/home/x"]: 1,
      list: ["cd /private/var/folders/ab/T/xenonet-demo-4711/home && ls", 3, null, true],
      near: "/private/var/folders/ab/T/xenonet-demo-4711/homework",
    }),
    { path: "~/Projects/atlas", id: "project:~/projects/atlas", claudeDir: "/x/.claude/projects/~-Projects-atlas/1.jsonl", "~/x": 1, list: ["cd ~ && ls", 3, null, true], near: "/private/var/folders/ab/T/xenonet-demo-4711/homework" },
  );
  const nova = homeScrubber("/Users/nova/");
  assert.equal(nova("/Users/Nova/Projects and /users/nova/projects and -Users-nova-Projects-x and /Users/novak"), "~/Projects and ~/projects and ~-Projects-x and /Users/novak");
  assert.equal(homeScrubber(null)("/Users/nova"), "/Users/nova");

  // Other folders named for a person: the name goes, the rest stays.
  const both = homeScrubber("/home/nova", { also: ["C:\\Users\\Wren", "/mnt/d/Users/Wren"] });
  assert.deepEqual(both(["C:\\Users\\Wren\\Projects", "c:/users/wren/projects", "C--Users-Wren-Projects-atlas", "/mnt/d/Users/Wren/x", "/mnt/d/Users/Wrenna/x", "/home/nova/x"]), ["C:\\Users\\~\\Projects", "c:/users/~/projects", "C--Users-~-Projects-atlas", "/mnt/d/Users/~/x", "/mnt/d/Users/Wrenna/x", "~/x"]);
  assert.equal(homeScrubber(null, { also: ["/home/wren"] })("/home/wren/a"), "/home/~/a");
  for (const [p, home, folder] of [
    ["/mnt/c/Users/Wren/Projects", "/home/nova", "/mnt/c/Users/Wren"],
    ["C:\\Users\\Wren\\Projects", "C:\\Users\\nova", "C:\\Users\\Wren"],
    ["/c/Users/Wren/x", "/home/nova", "/c/Users/Wren"],
    ["/home/nova2/x", "/home/nova", "/home/nova2"],
    ["/Users/Shared/Projects", "/Users/nova", "/Users/Shared"],
    ["/home/nova/Projects", "/home/nova", null],
    ["/Users/Nova/Projects", "/Users/nova", null],
    ["/srv/projects", "/home/nova", null],
    ["/data/home/x", "/home/nova", null],
  ])
    assert.equal(userFolderOf(p, home), folder, p);
  const map = { nodes: [{ kind: "core", path: "/mnt/c/Users/Wren/Projects" }, { kind: "zone", path: "/mnt/c/users/wren/tmp" }, { kind: "native", roots: ["/home/nova/x", "/Users/Kit/y"] }, { kind: "project", path: "/Users/Lee/z" }, { id: "@vm1:zone:folder:/users/ash/x", kind: "zone", path: "/Users/Ash/x" }] };
  assert.deepEqual(userFolders(map, "/home/nova"), ["/Users/Kit", "/mnt/c/Users/Wren"], "the places' folders, each once, and not another host's");
});

test("a map is shaped in one pass: what goes is copied once, by the scrubber, what's left out never is, and XenoNet's own is left as it was", () => {
  const g = made_up_graph();
  const before = JSON.stringify(g);
  const scrub = homeScrubber(HOME);
  const handed = [];
  const { graph, dropped } = shapeGraph(g, { now: NOW, copy: (v) => (handed.push(v), scrub(v)) });
  assert.equal(handed.length, 1, "one copy");
  const copied = handed[0].nodes.map((n) => n.id);
  for (const id of [S(2), S(4), `@elsewhere:${S(5)}`, "@elsewhere:core"]) assert.ok(!copied.includes(id), `${id} is left out before the copy`);
  assert.deepEqual([...dropped].sort(), [S(2), S(4)]);
  assert.equal(JSON.stringify(g), before, "the map it was given is unchanged");
  assert.ok(graph.nodes.every((n) => !g.nodes.includes(n)) && graph.links.every((l) => !g.links.includes(l)), "nothing shared with it");
  homeless(graph, "the shaped map");
  assert.equal(graph.nodes.find((n) => n.name === "atlas-api").repoKey, "github.com/atlas-co/atlas-api");
  assert.equal(graph.nodes.find((n) => n.id === S(1)).preview, null);

  // The live state and the engine's the same way.
  const live = madeUpLive();
  const liveBefore = JSON.stringify(live);
  const seen = [];
  const l = shapeLive(live, { dropped, copy: (v) => (seen.push(v), scrub(v)) });
  assert.equal(seen.length, 1);
  assert.deepEqual(seen[0].sessions.map((s) => s.id), [S(1)], "only the sessions that go are copied");
  assert.equal(JSON.stringify(live), liveBefore);
  assert.deepEqual([l.sessions[0].lastMessage, "others" in l.limits], [null, false]);
  const engine = madeUpEngine();
  const e = shapeEngine(engine, { copy: scrub });
  assert.deepEqual([e.agents.map((a) => a.id), engine.agents[0].claims[0].note], [[S(1)], "moving refunds"]);
  // Without a scrubber, still a copy, so shaping never changes its input.
  const plain = { a: [{ b: 1 }] };
  assert.ok(homeScrubber(null)(plain) !== plain && homeScrubber(null)(plain).a[0] !== plain.a[0]);
  assert.deepEqual(homeScrubber(null)(plain), plain);
});

test("the allowlist, and the rest of what the relay takes", () => {
  assert.equal(allowed(`/api/session/${S(1)}/transcript?limit=120&cursor=abc`).path, `/api/session/${S(1)}/transcript?limit=120&cursor=abc`);
  assert.ok(allowed(`/api/session/${S(1, "codex")}/stats`).route);
  assert.ok(allowed(`/api/collision?a=${S(1)}&b=${S(2)}`).route);
  for (const p of ["/api/live", "/api/graph", "/api/engine/state"]) assert.ok(allowed(p).route, p);
  for (const p of ["/api/live/", "/API/live", "/api/live?x", "/api/session/claude:x y/stats", "/api/session/claude:é/stats", "/api/session/@vm1:claude:x/stats"]) assert.ok(allowed(p).error, p);
  for (const k of ["constructor", "__proto__", "toString", "hasOwnProperty"]) assert.equal(allowed(`/api/graph?${k}=1`).error, `/api/graph doesn't take ${k}=1`, k);
  assert.equal(parseSince("7d"), 7 * DAY);
  assert.equal(parseSince("90m"), 90 * 60e3);
  for (const bad of ["7", "d", "1w", "-1d", "99999d", null]) assert.equal(parseSince(bad), null, String(bad));
  assert.deepEqual(fromOriginalCommand("xenonet relay --port 47840 --since 12h --detail"), { sinceMs: 12 * 3600e3 }, "only --since, never the port or detail");
  assert.deepEqual(fromOriginalCommand("sh -c 'rm -rf ~'"), {});
  const { graph } = shapeGraph({ nodes: [], links: [] }, {});
  assert.deepEqual([graph.nodes, graph.links, graph.relations, graph.identities], [[], [], [], []]);
});

test("loading the relay never builds a demo world, whatever the environment says", () => {
  // A port below 1024, which nothing here binds, whose world folder may
  // still be there: another test's at the same moment, or one left from an
  // earlier run. Only one the relay made counts.
  const port = 1 + (process.pid % 1000);
  const world = path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`);
  const there = fs.existsSync(world);
  const out = spawnSync(process.execPath, ["--input-type=module", "-e", "await import('./core/hosts/relay.js'); await import('./cli/commands/relay.js')"], { cwd: root, env: { ...process.env, XENONET_DEMO: "1", XENONET_PORT: String(port) }, encoding: "utf8", input: "" });
  assert.equal(out.status, 0, out.stderr);
  if (!there) assert.equal(fs.existsSync(world), false, "the relay runs beside a demo console, whose world it mustn't wipe");
});
