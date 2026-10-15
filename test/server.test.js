// The HTTP API from the outside: a demo server (made-up data, see demo.js)
// in a process of its own, as XenoNet runs it, with a home folder of the
// test's own so nothing it writes lands in yours.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { vendorSources } from "../core/schema/lenient.js";
import { createProcs } from "../core/procs.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const freePort = () =>
  new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
const port = await freePort();
const base = `http://127.0.0.1:${port}`;
const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-server-home-"));
const env = { ...process.env, HOME: home, USERPROFILE: home, LOCALAPPDATA: path.join(home, "AppData", "Local"), APPDATA: path.join(home, "AppData", "Roaming"), XENONET_HOT: "0", XENONET_CONTROL: "", XENONET_INSTANCE: "" };
const worldOf = (p) => path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${p}`);
const SESSION = (i, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`;
let server;
let output = "";

function start(args, p, extra = {}) {
  const proc = spawn(process.execPath, ["server/index.js", ...args], { cwd: root, env: { ...env, XENONET_PORT: String(p), ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  proc.stdout.on("data", (d) => (output += d));
  proc.stderr.on("data", (d) => (output += d));
  return proc;
}

async function call(method, route, body, headers = {}) {
  const init = { method, headers };
  if (body !== undefined) Object.assign(init, { body: typeof body === "string" ? body : JSON.stringify(body), headers: { "content-type": "application/json", ...headers } });
  const res = await fetch(base + route, init);
  const text = await res.text();
  let data = text;
  try {
    data = JSON.parse(text);
  } catch {}
  return { status: res.status, data, type: res.headers.get("content-type") };
}
const get = (route) => call("GET", route);
const post = (route, body = {}) => call("POST", route, body);
const del = (route) => call("DELETE", route);

// The event stream, for reading until a pattern turns up. Each read waits
// `ms` at most and says whether it came: the server sends nothing while
// nothing changes (a ping every 25 s), so waiting on the next chunk alone
// could run far past the time given. A read that times out leaves its
// chunk pending for the next one.
async function eventStream() {
  const ctl = new AbortController();
  const reader = (await fetch(`${base}/api/events`, { signal: ctl.signal })).body.getReader();
  const decoder = new TextDecoder();
  let pending = null;
  const stream = {
    text: "",
    async read(pattern, ms = 10000) {
      const until = Date.now() + ms;
      while (!pattern.test(stream.text)) {
        const left = until - Date.now();
        if (left <= 0) return false;
        if (!pending) (pending = reader.read()).catch(() => {});
        let timer;
        const next = await Promise.race([pending, new Promise((resolve) => (timer = setTimeout(resolve, left, null)))]);
        clearTimeout(timer);
        if (!next) return false;
        pending = null;
        if (next.done) return pattern.test(stream.text);
        stream.text += decoder.decode(next.value, { stream: true });
      }
      return true;
    },
    close: () => ctl.abort(),
  };
  return stream;
}

// Your standing settings (the projects folder, your rules, identities, the
// theme) are yours: on macOS the server refuses them to a process under an
// agent or an actor product, as these tests often are. Which this one is,
// read as the server reads it; refused() checks the answer then.
const procs = createProcs();
const refusedHere = procs.supported && (await procs.chain(process.pid)).some((p) => p.agent || p.product);
async function refused(answer) {
  const res = await answer;
  assert.deepEqual([res.status, res.data.code], [403, "yours"], JSON.stringify(res.data));
  assert.match(res.data.error, /^this is yours to do, and it came from under /);
}

// A request fetch can't make: another Host header.
const raw = (method, route, headers) =>
  new Promise((resolve, reject) => {
    const req = http.request({ host: "127.0.0.1", port, method, path: route, headers }, (res) => {
      res.resume();
      res.on("end", () => resolve(res.statusCode));
    });
    req.on("error", reject);
    req.end();
  });

before(async () => {
  server = start(["--demo", "--dev"], port);
  for (const until = Date.now() + 60000; Date.now() < until; await new Promise((r) => setTimeout(r, 100))) {
    try {
      const { data } = await get("/api/graph");
      if (data.nodes?.some((n) => n.kind === "session")) return;
    } catch {}
  }
  throw new Error(`the demo server didn't start:\n${output}`);
}, { timeout: 70000 });

after(async () => {
  server.kill("SIGTERM");
  await new Promise((r) => (server.exitCode !== null ? r() : server.once("exit", r)));
  fs.rmSync(worldOf(port), { recursive: true, force: true, maxRetries: 10 });
  fs.rmSync(home, { recursive: true, force: true });
});

test("only this machine's own pages get in, and health says which XenoNet it is", async () => {
  assert.equal(await raw("GET", "/api/health", { host: "evil.example" }), 421);
  assert.equal(await raw("POST", "/api/ack", { host: `127.0.0.1:${port}`, origin: "http://evil.example" }), 403);
  const { data } = await get("/api/health");
  assert.deepEqual([data.ok, data.name, data.demo], [true, "XenoNet (demo)", true]);
  assert.ok(data.sources.some((s) => s.name === "projects" && s.ok));
  // And how each of its parts is doing (core/blocks).
  const names = data.blocks.map((b) => b.name);
  for (const name of ["state", "engine", "transcripts", "claude-lines", "codex-lines", "claude-sessions", "codex-threads", "codex-projects", "codex-app-server", "identities", "git-status", "claude-desktop-usage", "projects"]) assert.ok(names.includes(name), name);
  assert.equal(new Set(names).size, names.length);
  for (const b of data.blocks) assert.deepEqual(Object.keys(b).filter((k) => k !== "source").sort(), ["counters", "kind", "lastError", "lastOk", "name", "since", "state"], b.name);
  // The ones that read for a source of the map say which, so the doctor
  // reports a source that fails once.
  const sourced = Object.fromEntries(data.blocks.filter((b) => b.source).map((b) => [b.name, b.source]));
  assert.deepEqual(sourced, { "claude-sessions": "claude", "codex-threads": "codex", "codex-projects": "codex-projects", "codex-app-server": "codex", transcripts: "live", identities: "identities", projects: "projects" });
  assert.ok(Object.values(sourced).every((name) => data.sources.some((s) => s.name === name)), "each one a source the map has");
  assert.ok(data.blocks.every((b) => ["idle", "running"].includes(b.state)), JSON.stringify(data.blocks.filter((b) => !["idle", "running"].includes(b.state))));
  assert.equal((await get("/api/nothing-here")).status, 404);
  const page = await get("/");
  assert.equal(page.status, 200);
  assert.match(page.data, /<html/i);
});

test("the map, its live state, history, summary and digests", async () => {
  const graph = (await get("/api/graph")).data;
  assert.ok(graph.nodes.find((n) => n.id === SESSION(1)));
  const live = (await get("/api/live")).data;
  assert.ok(live.sessions.length > 0 && Array.isArray(live.collisions));
  const history = (await get("/api/history?since=0")).data;
  assert.ok(history.events.length > 0 && history.now > 0);
  const summary = (await get("/api/summary")).data;
  assert.equal(summary.url, `http://127.0.0.1:${port}/`);
  assert.ok(summary.needsYou.some((n) => n.kind === "question"));
  assert.ok((await get(`/api/digest?since=${Date.now() - 6 * 3600e3}`)).data.totals.sessions > 0);
  const log = (await get("/api/daylog")).data;
  assert.equal(log.hours.length, 24);
  // What a poster or film gets: star names, never the projects' own, and
  // no hours; the names only when asked for.
  const stars = (await get("/api/constellation")).data;
  assert.ok(stars.stars.length > 0 && stars.stars.every((s) => /^[A-Z][a-z]+( \d+)?$/.test(s.name) && !("project" in s)));
  assert.ok(!log.topProjects.some((p) => stars.stars.some((s) => s.name === p.name)));
  assert.ok(!("hours" in stars) && !("busiestHour" in stars) && !("sessions" in stars));
  assert.deepEqual((await get("/api/constellation?names=1")).data.stars.map((s) => s.project), log.topProjects.map((p) => p.name));
  assert.deepEqual((await get("/api/constellation")).data.stars, stars.stars, "the same names again");
  assert.match((await post("/api/narrate", { kind: "day" })).data.text, /While you were away/);
  assert.match((await post("/api/narrate", {})).data.text, /While you were away/);
});

test("sessions: by words or folder, their stats, transcripts and handoffs", async () => {
  const found = (await get("/api/sessions?q=partial%20refunds")).data;
  assert.ok(found.some((s) => s.id === SESSION(1) && s.title === "Add partial refunds to the orders API"));
  const graph = (await get("/api/graph")).data;
  const atlas = graph.nodes.find((n) => n.id === SESSION(1)).cwd;
  const inAtlas = (await get(`/api/sessions?cwd=${encodeURIComponent(atlas)}&limit=50`)).data;
  assert.ok(inAtlas.length >= 2 && inAtlas.every((s) => s.cwd === atlas));
  assert.equal((await get("/api/sessions?limit=1")).data.length, 1);
  assert.deepEqual((await get(`/api/sessions?q=${SESSION(1).split(":")[1]}`)).data.map((s) => s.id), [SESSION(1)]);

  const stats = await get(`/api/session/${encodeURIComponent(SESSION(1))}/stats`);
  assert.equal(stats.status, 200);
  assert.ok(stats.data.totals.calls > 0 && stats.data.title);
  assert.equal((await get(`/api/session/${encodeURIComponent("claude:nope")}/stats`)).status, 404);
  const transcript = await get(`/api/session/${encodeURIComponent(SESSION(1))}/transcript?limit=20`);
  assert.ok(transcript.data.entries.length > 0);
  assert.equal((await get(`/api/session/${encodeURIComponent("core")}/transcript`)).status, 404);

  assert.equal((await get("/api/handoff?nodeId=core")).status, 400);
  const brief = (await get(`/api/handoff?nodeId=${encodeURIComponent(SESSION(1))}&target=codex`)).data;
  assert.equal(brief.target, "codex");
  assert.match(brief.brief, /Continue the work from a Claude Code session titled "Add partial refunds/);
  assert.equal((await post("/api/handoff", { nodeId: SESSION(1), target: "codex", brief: "" })).status, 400);
  assert.equal((await post("/api/handoff", { nodeId: SESSION(1), target: "codex", brief: "go on" })).status, 409, "demo sessions can't leave XenoNet");
  assert.equal((await post("/api/open", { nodeId: SESSION(1), action: "nope" })).status, 400);
  assert.equal((await post("/api/open", { nodeId: SESSION(1), action: "open-app" })).status, 409);
});

test("acks, collisions and the layout", async () => {
  assert.equal((await post("/api/ack", { sessionId: "claude:nope" })).status, 400);
  assert.deepEqual((await post("/api/ack", { sessionId: SESSION(3) })).data, { ok: true });
  const collision = (await get(`/api/collision?a=${SESSION(1)}&b=${SESSION(2, "codex")}`)).data;
  assert.equal(collision.a.title, "Add partial refunds to the orders API");
  assert.ok(Array.isArray(collision.files));
  await post("/api/layout", { positions: { core: { x: 1, y: 2, z: 3 } } });
  assert.deepEqual((await get("/api/layout")).data.core, { x: 1, y: 2, z: 3 });
  await del("/api/layout");
  assert.deepEqual((await get("/api/layout")).data, {});
});

test("themes: the list, a switch and a copy of your own", async () => {
  const { data } = await get("/api/themes");
  assert.ok(data.themes.length > 1 && data.current);
  const other = data.themes.find((t) => t.id !== data.current).id;
  const copy = await post("/api/themes/copy", { from: other, id: "mine", name: "Mine" });
  assert.ok(copy.data.file.endsWith("mine.json"));
  assert.equal((await post("/api/themes/copy", { from: other, id: "mine" })).status, 409);
  if (refusedHere) {
    await refused(post("/api/theme", { id: other }));
    assert.equal((await get("/api/theme")).data.id, data.current, "and the theme stays");
    return;
  }
  assert.equal((await post("/api/theme", { id: "no-such-theme" })).status, 404);
  assert.equal((await post("/api/theme", { id: other })).data.id, other);
  assert.equal((await get("/api/theme")).data.id, other);
  await post("/api/theme", { id: data.current });
});

test("a theme whole, built in or yours, for looking at it before switching", async () => {
  const { data } = await get("/api/themes");
  const built = (await get(`/api/themes/${data.themes.find((t) => t.builtIn).id}`)).data;
  assert.ok(built.colors.actor && built.stars.shape && built.sound.chords.length, "every field, for drawing it");
  await post("/api/themes/copy", { from: "rime", id: "frost", name: "Frost" });
  const yours = await get("/api/themes/frost");
  assert.deepEqual([yours.status, yours.data.name, yours.data.builtIn, yours.data.stars.shape], [200, "Frost", false, "flake"]);
  assert.equal((await get("/api/themes/no-such-theme")).status, 404);
  // Shown in the file manager by id only, and the demo leaves other apps alone.
  const shown = await post("/api/themes/reveal", { id: "frost" });
  assert.deepEqual([shown.status, shown.data.shown, path.basename(shown.data.file)], [200, false, "frost.json"]);
  assert.equal((await post("/api/themes/reveal", { id: "rime" })).status, 404, "a built-in has no file");
  assert.equal((await post("/api/themes/reveal", { id: "../state" })).status, 404);
});

test("how the projects folder is read: rules, a fresh scan and the Codex pass", async () => {
  const detection = (await get("/api/projects")).data;
  assert.ok(detection.scan.ok && detection.root);
  const labs = path.join(detection.root, "labs");
  if (refusedHere) {
    await refused(post("/api/projects/rule", { path: labs, as: "project" }));
    await refused(del("/api/projects/rules?all=1"));
    assert.deepEqual((await get("/api/projects")).data.rules, [], "no rule was set");
  } else {
    const ruled = (await post("/api/projects/rule", { path: labs, as: "project" })).data;
    assert.deepEqual(ruled.rules.map((r) => [r.as, path.basename(r.path)]), [["project", "labs"]]);
    assert.equal((await post("/api/projects/rule", { path: labs, as: "planet" })).status, 400);
    assert.equal((await post("/api/projects/rule", { path: os.tmpdir(), as: "project" })).status, 400);
    assert.equal((await post("/api/projects/rule", { path: path.join(detection.root, "gone"), as: "group" })).status, 400);
    assert.deepEqual((await del("/api/projects/rules")).data.rules, []);
    assert.deepEqual((await del("/api/projects/rules?all=1")).data.rules, [], "every folder's");
  }
  assert.ok((await post("/api/projects/rescan")).data.scan.ok);

  const outline = (await get("/api/projects/codex")).data;
  assert.ok(outline.outline.length > 0 && outline.fingerprint && outline.demo);
  // Asking Codex spends your Codex usage, and its picks change what the
  // map shows: yours, as your rules are.
  if (refusedHere) {
    await refused(post("/api/projects/codex", { fingerprint: outline.fingerprint }));
    await refused(del("/api/projects/codex"));
    assert.equal((await get("/api/projects")).data.codex, null, "Codex wasn't asked");
    return;
  }
  assert.equal((await post("/api/projects/codex", {})).status, 400);
  assert.equal((await post("/api/projects/codex", { fingerprint: "stale" })).status, 409);
  const asked = (await post("/api/projects/codex", { fingerprint: outline.fingerprint })).data;
  assert.equal(asked.codex.picks[0].path, "labs/snippets");
  assert.equal(asked.codex.stale, false);
  assert.equal((await del("/api/projects/codex")).data.codex, null);
});

test("the projects folder: where it comes from, a check, a change and back", async () => {
  const before = (await get("/api/workspace/root")).data;
  assert.deepEqual([before.source, path.basename(before.path), before.scan.ok], ["default", "Projects", true]);
  const check = (await post("/api/workspace/check", { path: "~/Clients" })).data;
  assert.deepEqual([check.ok, check.current, check.scan.projects, check.scan.groups], [true, false, 4, 1]);
  assert.match((await post("/api/workspace/check", { path: "~" })).data.error, /your home folder/);
  assert.equal((await get("/api/workspace/nothing")).status, 404);
  if (refusedHere) {
    await refused(post("/api/workspace/root", { path: "~/Clients" }));
    await refused(del("/api/workspace/root"));
    assert.equal((await get("/api/workspace/root")).data.path, before.path, "the folder stays");
    return;
  }
  assert.equal((await post("/api/workspace/root", { path: "~/nowhere" })).status, 400);
  assert.equal((await post("/api/workspace/root", {})).status, 400, "no folder named");

  const moved = (await post("/api/workspace/root", { path: "~/Clients" })).data;
  assert.deepEqual([moved.source, moved.changed, path.basename(moved.path), moved.scan.projects], ["chosen", true, "Clients", 4]);
  assert.equal((await get("/api/graph")).data.nodes.find((n) => n.id === "core").name, "~/Clients", "the map follows at once");
  assert.equal((await get("/api/projects")).data.root, moved.path);
  const back = (await del("/api/workspace/root")).data;
  assert.deepEqual([back.source, back.changed, back.path], ["default", true, before.path]);
});

test("identities: a name, hiding one, pinning work to one, and detecting again", async () => {
  const { data } = await get("/api/identities");
  assert.ok(data.enabled && data.identities.length > 0);
  const id = data.identities[0].id;
  if (refusedHere) {
    await refused(post("/api/identities/name", { id, name: "Work" }));
    await refused(post("/api/identities/hidden", { id, hidden: true }));
    await refused(post("/api/identities/pin", { nodeId: SESSION(1), identity: id }));
    const after = (await get("/api/identities")).data;
    assert.deepEqual([after.identities.find((i) => i.id === id).name ?? null, !!after.identities.find((i) => i.id === id).hidden, after.pins.length], [null, false, 0], "nothing changed");
    assert.ok((await post("/api/identities/rescan")).data.identities.length > 0, "detecting again is anyone's");
    return;
  }
  assert.equal((await post("/api/identities/name", { id: "github:nowhere/nobody", name: "X" })).status, 400);
  assert.equal((await post("/api/identities/name", { id, name: " Work\u0007 " })).data.identities.find((i) => i.id === id).name, "Work");
  assert.equal((await post("/api/identities/name", { id, name: null })).data.identities.find((i) => i.id === id).name, null);
  assert.equal((await post("/api/identities/hidden", { id, hidden: true })).data.identities.find((i) => i.id === id).hidden, true);
  await post("/api/identities/hidden", { id, hidden: false });
  const pinned = (await post("/api/identities/pin", { nodeId: SESSION(1), identity: id })).data;
  assert.equal(pinned.pins.find((p) => p.node === SESSION(1)).identity, id);
  assert.equal((await post("/api/identities/pin", { nodeId: "core", identity: id })).status, 400);
  assert.deepEqual((await post("/api/identities/pin", { nodeId: SESSION(1), identity: null })).data.pins, []);
  assert.ok((await post("/api/identities/rescan")).data.identities.length > 0);
});

test("renaming a session: the demo keeps the name, the map goes out with it, and what isn't a name or a session is refused", async () => {
  const id = SESSION(1);
  if (refusedHere) return refused(post("/api/session/rename", { nodeId: id, title: "Refunds" }));
  const before = (await get("/api/graph")).data.nodes.find((n) => n.id === id).title;
  const events = await eventStream();
  try {
    const done = await post("/api/session/rename", { nodeId: id, title: "  Refunds,\n partial\u0007  " });
    assert.equal(done.status, 200, JSON.stringify(done.data));
    assert.deepEqual([done.data.id, done.data.title, done.data.demo, done.data.saved], [id, "Refunds, partial", true, []], "one line, without control characters");
    assert.ok(await events.read(/event: graph\ndata: .*Refunds, partial/), "the map goes out with the new name");
  } finally {
    events.close();
  }
  assert.equal((await get("/api/graph")).data.nodes.find((n) => n.id === id).title, "Refunds, partial");
  assert.equal((await get("/api/graph")).data.demo, true);
  assert.equal((await post("/api/session/rename", { nodeId: id, title: " \n\t " })).status, 400);
  assert.equal((await post("/api/session/rename", { nodeId: id })).status, 400);
  assert.equal((await post("/api/session/rename", { nodeId: "claude:nobody", title: "x" })).status, 404);
  assert.equal((await post("/api/session/rename", { nodeId: "core", title: "x" })).status, 404, "only a session has a name to change");
  assert.equal((await post("/api/session/rename", { nodeId: id, title: "x".repeat(500) })).data.title.length, 120);
  await post("/api/session/rename", { nodeId: id, title: before });
});

test("the doctor and the xenonet command", async () => {
  const first = (await get("/api/doctor")).data;
  assert.ok(first.results.some((r) => r.id === "xenonet.server"));
  assert.equal(first.results.find((r) => r.id === "xenonet.blocks").status, "ok");
  assert.ok((await post("/api/doctor")).data.summary);
  assert.equal((await get("/api/command")).data.installed, false);
  const installed = (await post("/api/command")).data;
  assert.ok(installed.installed && installed.ours && installed.file.startsWith(home));
});

test("drift: the report xenonet drift shows, and marking it seen", async () => {
  const report = (await get("/api/drift")).data;
  assert.equal(report.sources.length, vendorSources().length, "every source with a schema");
  assert.deepEqual([report.new, report.changes, report.all], [0, 0, false], "the demo's made-up data fits the schemas");
  assert.ok(report.at > 0 && report.file.endsWith("drift.json"));
  assert.ok(report.sources.find((s) => s.source === "anthropic/claude-code/transcript-line").read.records > 0, "what this XenoNet has read");
  assert.equal((await get("/api/drift?all=1")).data.all, true);
  assert.deepEqual((await post("/api/drift/seen", { at: report.at })).data, { lookedAt: report.at });
  assert.equal((await get("/api/drift")).data.since, report.at);
  // Never ahead of now, so drift that turns up later is new; and a time
  // that isn't one is the client's mistake.
  assert.ok((await post("/api/drift/seen", { at: Date.now() + 3600e3 })).data.lookedAt <= Date.now());
  assert.equal((await post("/api/drift/seen", { at: "soon" })).status, 400);
  assert.equal((await post("/api/drift/seen", "null")).status, 200, "no body is now");
  assert.equal((await get("/api/drift/nothing")).status, 404);
});

test("Claude Code's hooks honor the sender's process identity", async () => {
  const hook = (body) => fetch(`${base}/api/hook/claude`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  assert.equal((await hook({ session_id: "not-a-session" })).status, 204);
  const id = "0190d3e0-0000-7000-8000-0000000000aa";
  const res = await hook({ session_id: id, hook_event_name: "Notification", message: "Claude needs your permission to use Bash" });
  // On a Mac CI runner, this is plain Node, not a child of Claude Code.
  // The same check accepts a test started from inside Claude Code.
  const fromClaude = procs.supported && (await procs.chain(process.pid)).some((p) => p.agent === "claude");
  if (procs.supported && !fromClaude) {
    assert.equal(res.status, 403);
    assert.match((await res.json()).error, /have to come from a Claude Code process/);
    assert.equal((await hook({ session_id: id, hook_event_name: "SessionStart", cwd: root })).status, 403);
    return;
  }
  assert.ok([200, 204].includes(res.status));
  assert.equal((await hook({ session_id: id, hook_event_name: "SessionStart", cwd: root })).ok, true);
});

test("XenoNet Dev's simulations, its control channel and captures", async () => {
  for (const kind of ["needs-you", "collision", "stuck", "turn-end", "wiki-write"]) {
    assert.equal((await post("/api/dev/simulate", { kind })).data.kind, kind, kind);
    if (kind === "needs-you") assert.ok((await get("/api/live")).data.sessions.some((s) => s.needsYou?.text?.startsWith("Should I run the migration")));
    if (kind === "stuck") assert.ok((await get("/api/live")).data.sessions.some((s) => s.stuck?.reasons.includes("4 failed tool calls in a row")));
  }
  assert.equal((await post("/api/dev/simulate", { kind: "doctor" })).data.kind, "doctor");
  assert.ok((await get("/api/doctor")).data.results.some((r) => r.id === "codex.auth" && r.status === "fail"));
  assert.equal((await post("/api/dev/simulate", { kind: "clear" })).data.kind, "clear");
  assert.equal((await post("/api/dev/simulate", { kind: "earthquake" })).status, 400);

  assert.equal((await post("/api/dev/control", { op: "ping" })).status, 503);
  assert.equal((await post("/api/dev/capture", { dataUrl: "nope" })).status, 400);
  const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
  const name = `server-test-${process.pid}`;
  const shot = (await call("POST", `/api/dev/capture?name=${name}`, { dataUrl: png })).data;
  assert.ok(shot.ok && fs.existsSync(shot.file));
  fs.rmSync(shot.file);
});

test("the event stream sends what changes", async () => {
  const stream = await eventStream();
  try {
    assert.ok(await stream.read(/retry: 3000/));
    await post("/api/ack", { sessionId: SESSION(1) });
    assert.ok(await stream.read(/event: live/));
    // A read gives up at its time, whether or not anything comes meanwhile.
    const started = Date.now();
    assert.equal(await stream.read(/event: nothing-sends-this/, 300), false);
    assert.ok(Date.now() - started < 5000, `${Date.now() - started} ms`);
  } finally {
    stream.close();
  }
});

test("actors: the list, one by its slug, events since a time, XenoNet Dev's simulations, and the events that carry changes", async () => {
  const { data } = await get("/api/actors");
  assert.equal(data.schema, "actor/list/v1");
  assert.deepEqual(data.actors.map((a) => a.id), ["actor:you", "actor:wren", "actor:kestrel", "actor:moth"]);
  assert.equal((await get("/api/actors/kestrel")).data.name, "Kestrel");
  assert.deepEqual(await get("/api/actors/nobody").then((r) => [r.status, r.data.code]), [404, "not-found"]);
  const events = (await get(`/api/actors/events?since=${Date.now() - 3600e3}`)).data;
  assert.equal(events.schema, "actor/events/v1");
  assert.ok(events.events.length > 0);
  // You: the window's to report, with a key no other program has.
  assert.deepEqual(await post("/api/actors/you", { presence: "working" }).then((r) => [r.status, r.data.code]), [403, "yours"]);
  assert.deepEqual(await call("POST", "/api/actors/you", { presence: "working" }, { "x-xenonet-window": "0".repeat(64) }).then((r) => [r.status, r.data.code]), [403, "yours"]);
  assert.deepEqual(await call("GET", "/api/actors/you", undefined, { "x-xenonet-window": "0".repeat(64) }).then((r) => r.status), 403);
  assert.equal((await get("/api/actors/you")).data.name, "You", "without a key, what the list says (the demo's made-up you)");
  const stream = await eventStream();
  try {
    for (const kind of ["actor-spawn", "actor-edit", "actor-message", "actor-offline", "actor-online"]) assert.equal((await post("/api/dev/simulate", { kind, a: "actor:moth" })).data.kind, kind, kind);
    assert.ok(await stream.read(/event: actor-event\ndata: \{"schema":"actor\/event\/v1"/), "each event as it happens");
    assert.ok(await stream.read(/event: actors\ndata: \{"schema":"actor\/list\/v1"/), "and the list after");
  } finally {
    stream.close();
  }
  assert.equal((await post("/api/dev/simulate", { kind: "actor-spawn", a: "actor:nobody" })).status, 400);
  // One waiting on you, which clear, undoing every simulation, ends too.
  const waiting = async () => (await get("/api/actors")).data.actors.find((a) => a.id === "actor:wren").needsYou ?? null;
  assert.equal((await post("/api/dev/simulate", { kind: "actor-needs-you", a: "actor:wren" })).data.kind, "actor-needs-you");
  assert.equal((await waiting())?.kind, "question");
  assert.equal((await post("/api/dev/simulate", { kind: "clear" })).data.kind, "clear");
  assert.equal(await waiting(), null, "clear takes the actor's wait away");
});

test("registering actors over HTTP: a token that waits, the window's routes behind its key, and the demo's made-up request", async () => {
  // On macOS the process check refuses a program under an agent, and these
  // tests often run under one; otherwise the registration waits.
  const asked = await post("/api/actors/register", { name: "Kite", capabilities: ["run-commands"] });
  if (asked.status === 403) assert.equal(asked.data.code, "wrong-process");
  else assert.deepEqual([asked.status, asked.data.state, asked.data.id], [202, "waiting", "actor:kite"]);
  const taken = await post("/api/actors/register", { name: "Wren" });
  assert.ok(taken.status === 403 || (taken.status === 409 && taken.data.code === "taken" && taken.data.suggest === "wren-2"), "the demo's Wren has that slug");
  for (const [method, route, body] of [["GET", "/api/actors/requests"], ["POST", "/api/actors", { name: "Kite" }], ["POST", "/api/actors/moth/token"]]) assert.deepEqual(await call(method, route, body).then((r) => [r.status, r.data.code]), [403, "yours"], `${method} ${route}: the window's alone`);
  const unknown = `xenonet_actor_${"0".repeat(64)}`;
  assert.deepEqual(await get(`/api/actors/me?token=${unknown}`).then((r) => [r.status, r.data.code]), [400, "bad-request"], "never in a URL");
  assert.deepEqual(await call("GET", "/api/actors/me", undefined, { authorization: `Bearer ${unknown}` }).then((r) => [r.status, r.data.code]), [401, "bad-token"]);
  assert.deepEqual(await get("/api/actors/me").then((r) => [r.status, r.data.code]), [401, "no-token"]);
  assert.deepEqual((await get("/api/actors/observers")).data.observers.map((o) => [o.product, o.on]), [["hermes-agent", true], ["grok-bot", true], ["muse", true]]);

  const stream = await eventStream();
  let sim;
  try {
    sim = await post("/api/dev/simulate", { kind: "actor-request" });
    assert.deepEqual([sim.status, sim.data.ok, sim.data.a], [200, true, "actor:heron"]);
    assert.match(sim.data.token, /^xenonet_actor_[0-9a-f]{64}$/);
    assert.ok(await stream.read(/event: actor-requests\ndata: \{"at":\d+,"waiting":[12]\}/), "the window hears how many wait, and reads them with its key");
  } finally {
    stream.close();
  }
  assert.doesNotMatch(stream.text, /heron|xenonet_actor_/i, "never who asked, nor a token, on the stream");
  const me = await call("GET", "/api/actors/me", undefined, { authorization: `Bearer ${sim.data.token}` });
  assert.ok((me.status === 200 && me.data.state === "waiting") || (me.status === 403 && me.data.code === "wrong-process"), JSON.stringify(me.data));
  assert.equal((await get("/api/actors")).data.actors.some((a) => a.id === "actor:heron"), false, "not on the list until you allow it");
});

test("the dev server keeps Vite's prebundled dependencies in this checkout, where git ignores them", async () => {
  // Not in node_modules/.vite: worktrees share the main checkout's
  // node_modules, and this server would rebuild XenoNet Dev's cache there.
  const { default: config } = await import("../vite.config.js");
  const dir = path.relative(root, config.cacheDir);
  assert.ok(dir && !dir.startsWith("..") && !path.isAbsolute(dir), `${config.cacheDir} is in ${root}`);
  assert.ok(!dir.split(path.sep).includes("node_modules"), dir);
  assert.equal(spawnSync("git", ["check-ignore", "-q", path.join(config.cacheDir, "deps")], { cwd: root }).status, 0, `git ignores ${dir}`);
});

test("a body that isn't JSON, or is too big, is the client's error", async () => {
  const before = output.length;
  assert.equal((await post("/api/layout", "{ not json")).status, 400);
  assert.equal((await post("/api/layout", JSON.stringify({ positions: {}, pad: "x".repeat(1 << 20) }))).status, 413);
  assert.doesNotMatch(output.slice(before), /SyntaxError|body too large/, "not logged as a server error");
});

test("a second server on a port in use says so and stops", async () => {
  const busy = await freePort();
  const holder = net.createServer().listen(busy, "127.0.0.1");
  await new Promise((r) => holder.once("listening", r));
  const before = output.length;
  const proc = start(["--demo"], busy);
  const code = await new Promise((r) => proc.once("exit", r));
  holder.close();
  fs.rmSync(worldOf(busy), { recursive: true, force: true, maxRetries: 10 });
  assert.equal(code, 1);
  assert.match(output.slice(before), new RegExp(`port ${busy} is already in use`));
});
