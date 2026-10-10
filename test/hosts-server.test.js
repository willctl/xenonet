// Hosts from the outside: two demo XenoNets on spare ports, each with a
// made-up world of its own and the same repos. The console runs with
// XENONET_HOSTS=1 and shows the other through its relay (the local
// transport, added by XenoNet Dev's simulation, since adding a host for real
// is refused from under an agent); the other runs without the flag, as any
// host would. Nothing here uses SSH: a stand-in for it
// (test/fixtures/fake-ssh.mjs) checks what a link would give ssh and runs
// the relay here.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { check, explain } from "../core/schema/index.js";
import { HostLink } from "../core/hosts/link.js";
import { HostsConfig } from "../core/hosts/config.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const freePort = () =>
  new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
const [consolePort, hostPort] = [await freePort(), await freePort()];
const home = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-hosts-server-home-"));
const env = { ...process.env, HOME: home, USERPROFILE: home, XENONET_HOT: "0", XENONET_CONTROL: "", XENONET_INSTANCE: "", XENONET_EXEC: "" };
const worldOf = (p) => path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${p}`);
const SESSION = (i, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`;
const REMOTE = (id) => `@demo-b:${id}`;
const servers = {};
let output = "";

function start(name, port, extra = {}, args = ["--demo"]) {
  const proc = spawn(process.execPath, ["server/index.js", ...args], { cwd: root, env: { ...env, XENONET_PORT: String(port), ...extra }, stdio: ["ignore", "pipe", "pipe"] });
  proc.stdout.on("data", (d) => (output += `[${name}] ${d}`));
  proc.stderr.on("data", (d) => (output += `[${name}] ${d}`));
  servers[name] = proc;
  return proc;
}
async function stop(name) {
  const proc = servers[name];
  if (!proc || proc.exitCode !== null) return;
  proc.kill("SIGTERM");
  await new Promise((r) => (proc.exitCode !== null ? r() : proc.once("exit", r)));
}

async function call(port, method, route, body) {
  const init = { method, headers: {} };
  if (body !== undefined) Object.assign(init, { body: JSON.stringify(body), headers: { "content-type": "application/json" } });
  const res = await fetch(`http://127.0.0.1:${port}${route}`, init);
  const text = await res.text();
  let data = text;
  try {
    data = JSON.parse(text);
  } catch {}
  return { status: res.status, data };
}
const get = (route, port = consolePort) => call(port, "GET", route);
const post = (route, body = {}, port = consolePort) => call(port, "POST", route, body);

async function until(what, fn, ms = 20000) {
  let last;
  for (const end = Date.now() + ms; Date.now() < end; await new Promise((r) => setTimeout(r, 150))) {
    try {
      last = await fn();
      if (last) return last;
    } catch (err) {
      last = err;
    }
  }
  throw new Error(`${what}: not in ${ms / 1000} s (last: ${JSON.stringify(last)?.slice(0, 300)})\n${output.slice(-3000)}`);
}
const ready = (port) => until(`the XenoNet on ${port}`, async () => (await get("/api/graph", port)).data.nodes?.some((n) => n.kind === "session"), 60000);
const hostState = async () => (await get("/api/hosts")).data.hosts?.find((h) => h.id === "demo-b");
const valid = (id, data, what) => {
  const c = check(id, data, { strict: true });
  assert.ok(c.ok, `${what} doesn't fit ${id}:\n${explain(c)}`);
};

before(async () => {
  start("console", consolePort, { XENONET_HOSTS: "1" }, ["--demo", "--dev"]);
  start("demo-b", hostPort);
  await Promise.all([ready(consolePort), ready(hostPort)]);
}, { timeout: 90000 });

after(async () => {
  await stop("console");
  await stop("demo-b");
  for (const p of [consolePort, hostPort]) fs.rmSync(worldOf(p), { recursive: true, force: true, maxRetries: 10 });
  fs.rmSync(home, { recursive: true, force: true });
});

test("a XenoNet without the flag is a host all the same, and shows only its own", async () => {
  const off = await get("/api/hosts", hostPort);
  assert.equal(off.status, 404);
  assert.match(off.data.error, /XENONET_HOSTS=1/);
  const self = (await get("/api/hosts/self", hostPort)).data;
  assert.match(self.key, /^[0-9a-f]{32}$/);
  assert.equal(self.id, null);
  const graph = (await get("/api/graph", hostPort)).data;
  assert.ok(!("hosts" in graph) && !graph.nodes.some((n) => n.kind === "host" || n.id.startsWith("@")));
  // The console, with no host yet, shows its own map as it was.
  const alone = (await get("/api/graph")).data;
  assert.ok(!("hosts" in alone) && !alone.nodes.some((n) => n.kind === "host"));
  assert.deepEqual((await get("/api/hosts")).data.hosts, []);
});

test("the console shows the other XenoNet's map: its host node, its projects and sessions under @demo-b:, bridges between the two atlas-apis", { timeout: 60000 }, async () => {
  const added = await post("/api/dev/simulate", { kind: "host-add", id: "demo-b", port: hostPort });
  assert.deepEqual(added.data, { ok: true, kind: "host-add", id: "demo-b" });
  assert.equal((await post("/api/dev/simulate", { kind: "host-add", id: "demo-b", port: hostPort })).status, 400, "once");
  const live = await until("demo-b live", async () => ((await hostState())?.state === "live" ? hostState() : null));
  assert.deepEqual([live.transport, live.demo, live.simulated, live.version], ["local", true, true, JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version]);
  assert.ok(Math.abs(live.offsetMs) < 2000);

  const graph = await until("the merged map", async () => {
    const g = (await get("/api/graph")).data;
    return g.nodes.some((n) => n.id === REMOTE(SESSION(1))) ? g : null;
  });
  valid("graph/v1", graph, "the merged map");
  const host = graph.nodes.find((n) => n.id === "@demo-b:core");
  assert.deepEqual([host.kind, host.state, host.path, host.demo], ["host", "live", "~/Projects", true]);
  assert.ok(graph.links.some((l) => l.source === "core" && l.target === "@demo-b:core" && l.kind === "host"));
  assert.ok(graph.nodes.some((n) => n.id === "@demo-b:project:~/projects/atlas-api" && n.host === "demo-b"));
  const atlas = graph.nodes.find((n) => n.kind === "project" && n.name === "atlas-api" && !n.host);
  assert.ok(graph.relations.some((r) => r.kind === "same-repo" && r.a === atlas.id && r.b === "@demo-b:project:~/projects/atlas-api"));
  assert.deepEqual(graph.hosts.map((h) => [h.id, h.state]), [["demo-b", "live"]]);
  assert.ok(!JSON.stringify(graph.nodes.filter((n) => n.host)).includes(worldOf(hostPort)), "no home folder from the other machine");
  assert.ok(!(await get("/api/graph?hosts=0")).data.nodes.some((n) => n.host), "this machine's own view, for its own relay");

  const merged = await until("remote live sessions", async () => {
    const l = (await get("/api/live")).data;
    return l.sessions.some((s) => s.id.startsWith("@demo-b:")) ? l : null;
  });
  valid("live/snapshot/v1", merged, "the merged live state");
  assert.ok(merged.sessions.filter((s) => s.host === "demo-b").every((s) => s.lastMessage === null), "titles, states and counts only, by default");
});

test("a remote transcript and stats come through the relay; nothing opens, dismisses or hands off a remote session", async () => {
  const id = REMOTE(SESSION(1));
  const transcript = await get(`/api/session/${encodeURIComponent(id)}/transcript?limit=5`);
  assert.equal(transcript.status, 200);
  assert.ok(transcript.data.entries.length > 0);
  assert.equal(transcript.data.host, "demo-b");
  const stats = await get(`/api/session/${encodeURIComponent(id)}/stats`);
  assert.equal(stats.status, 200);
  assert.deepEqual([stats.data.id, stats.data.host], [id, "demo-b"]);
  assert.ok(stats.data.totals.calls > 0);

  assert.equal((await post("/api/open", { nodeId: id, action: "open-app" })).status, 400);
  assert.equal((await post("/api/open", { nodeId: "@demo-b:project:~/projects/atlas-api", action: "open-folder" })).status, 400);
  assert.equal((await post("/api/ack", { sessionId: id })).status, 400);
  assert.equal((await post("/api/handoff", { nodeId: id, target: "codex", brief: "go on" })).status, 400);
  // Pinning is yours (verify.person), so under an agent, as these tests
  // often run, it's refused before the remote id is.
  const pin = await post("/api/identities/pin", { nodeId: id, identity: "claude:x" });
  assert.ok(pin.status === 400 || (pin.status === 403 && pin.data.code === "yours"), JSON.stringify(pin));
  assert.equal((await get(`/api/session/${encodeURIComponent("@demo-b:claude:nope")}/stats`)).status, 404);
  const detail = (await get("/api/hosts/demo-b")).data;
  assert.equal(detail.id, "demo-b");
  assert.ok(Array.isArray(detail.engine?.agents));
  const tried = (await post("/api/hosts/demo-b/test")).data;
  assert.equal(tried.ok, true, JSON.stringify(tried));
  assert.ok(tried.snapshot.sessions > 0 && tried.hello.demo === true);
});

test("the tray's summary counts the live host's alerts, each with its host and title", async () => {
  const summary = await until("demo-b's alerts in the summary", async () => {
    const data = (await get("/api/summary")).data;
    return data.needsYou?.some((n) => n.host === "demo-b") ? data : null;
  });
  const theirs = summary.needsYou.filter((n) => n.host === "demo-b");
  assert.ok(theirs.every((n) => n.id.startsWith("@demo-b:") && n.title && !n.title.startsWith("@")), JSON.stringify(theirs));
  assert.ok((await get("/api/summary", hostPort)).data.needsYou.every((n) => !("host" in n)), "a XenoNet's own alerts have no host");
});

test("the event stream carries the other host's map, but not to a relay's stream", { timeout: 30000 }, async () => {
  const read = async (route, pattern, ms) => {
    const ctl = new AbortController();
    const res = await fetch(`http://127.0.0.1:${consolePort}${route}`, { signal: ctl.signal });
    const reader = res.body.getReader();
    let text = "";
    const end = Date.now() + ms;
    let pending = reader.read();
    try {
      while (Date.now() < end && !pattern.test(text)) {
        // Keep the same read when the timer wins. Abandoning a read drops
        // the next event when that pending promise eventually resolves.
        const chunk = await Promise.race([pending, new Promise((r) => setTimeout(() => r(null), 500))]);
        if (!chunk) continue;
        if (chunk.done) break;
        if (chunk.value) text += new TextDecoder().decode(chunk.value);
        pending = reader.read();
      }
    } finally {
      ctl.abort();
      await pending.catch(() => {});
    }
    return text;
  };
  await post("/api/dev/simulate", { kind: "host-skew", a: "demo-b", ms: 1000 });
  const hostsEvent = await read("/api/events", /event: hosts\ndata: \[/, 10000);
  assert.match(hostsEvent, /"id":"demo-b"/);
  assert.doesNotMatch(hostsEvent.slice(hostsEvent.indexOf("event: hosts")).split("\n\n")[0], /"target"|"key"|"command"/, "the page hears how a host stands, not how it's reached");
  const own = await read("/api/events?hosts=0", /event: live\n/, 10000);
  assert.match(own, /event: live/);
  assert.doesNotMatch(own, /@demo-b:|event: hosts/);
  await post("/api/dev/simulate", { kind: "host-skew", a: "demo-b", ms: 0 });
});

test("the host's clock three minutes off: the doctor says so", { timeout: 30000 }, async () => {
  await post("/api/dev/simulate", { kind: "host-skew", a: "demo-b", ms: 180e3 });
  const run = (await post("/api/doctor")).data;
  const check = run.results.find((r) => r.id === "hosts.demo-b");
  assert.deepEqual([check.group, check.status], ["Hosts", "warn"]);
  assert.match(check.detail, /clock 3 min ahead of this one's/);
  await post("/api/dev/simulate", { kind: "host-skew", a: "demo-b", ms: 0 });
  assert.equal((await post("/api/doctor")).data.results.find((r) => r.id === "hosts.demo-b").status, "ok");
});

test("through a stand-in for ssh, a link reads the other XenoNet's real relay", { timeout: 60000 }, async () => {
  const state = fs.mkdtempSync(path.join(home, "console-state-"));
  const hosts = new HostsConfig({ stateDir: state });
  const entry = hosts.add({ id: "demo-b", target: "lab@127.0.0.1", port: hostPort });
  const log = path.join(state, "fake-ssh.log");
  const link = new HostLink({ entry, sshConfig: hosts.sshConfig, sshBin: [process.execPath, path.join(root, "test", "fixtures", "fake-ssh.mjs")], root, env: { ...process.env, FAKE_SSH_LOG: log } });
  const snapshot = new Promise((resolve) => link.once("snapshot", resolve));
  link.start();
  try {
    const snap = await snapshot;
    assert.equal(link.state, "live");
    assert.equal(link.info().hello.demo, true);
    const atlas = snap.graph.nodes.find((n) => n.kind === "project" && n.name === "atlas-api");
    assert.deepEqual([atlas.id, atlas.repoKey], ["project:~/projects/atlas-api", "github.com/atlas-co/atlas-api"], "path keys tilded and repo keys computed on the host");
    assert.equal(snap.graph.nodes.find((n) => n.name === "route-sim").repoKey, "github.com/orbitworks/route-sim", "with the host's own SSH alias");
    assert.ok(!JSON.stringify(snap).includes(worldOf(hostPort)), "no home folder crosses");
    const stats = await link.get(`/api/session/${SESSION(1)}/stats`);
    assert.equal(stats.status, 200);
    assert.equal((await link.get("/api/open")).status, 403);
    assert.deepEqual(JSON.parse(fs.readFileSync(log, "utf8").trim()), { alias: "demo-b", command: "xenonet", words: ["relay", "--port", String(hostPort), "--since", "7d"] });
  } finally {
    link.stop();
  }
});

test("when the other XenoNet stops, the host goes offline with its nodes kept and marked stale, and comes back when it does", { timeout: 120000 }, async () => {
  await stop("demo-b");
  const off = await until("demo-b offline", async () => ((await hostState())?.state === "offline" ? hostState() : null), 10000);
  assert.equal(off.error.code, "xenonet-gone");
  const graph = (await get("/api/graph")).data;
  valid("graph/v1", graph, "the map with an offline host");
  const theirs = graph.nodes.filter((n) => n.host === "demo-b");
  assert.ok(theirs.length > 5 && theirs.every((n) => n.stale === true), "kept, and marked stale");
  assert.equal(graph.nodes.find((n) => n.id === "@demo-b:core").state, "offline");
  const live = (await get("/api/live")).data;
  assert.ok(live.sessions.filter((s) => s.host === "demo-b").every((s) => s.status === "idle" && s.stale));
  assert.equal((await get(`/api/session/${encodeURIComponent(REMOTE(SESSION(1)))}/stats`)).status, 503, "a transcript needs the host there");
  const doctor = (await post("/api/doctor")).data.results.find((r) => r.id === "hosts.demo-b");
  assert.equal(doctor.status, "warn");
  assert.match(doctor.detail, /^offline/);

  // Back: the same port, a world made afresh (and so a new key, which a
  // demo world may have).
  fs.rmSync(worldOf(hostPort), { recursive: true, force: true, maxRetries: 10 });
  start("demo-b", hostPort);
  await ready(hostPort);
  await post("/api/hosts/demo-b/reconnect");
  await until("demo-b live again", async () => (await hostState())?.state === "live", 20000);
  const back = (await get("/api/graph")).data;
  assert.ok(back.nodes.filter((n) => n.host === "demo-b").every((n) => !n.stale));
  assert.equal(new Set(back.nodes.map((n) => n.id)).size, back.nodes.length, "no node twice");

  // And XenoNet Dev's simulation of the same.
  await post("/api/dev/simulate", { kind: "host-offline", a: "demo-b" });
  assert.equal((await hostState()).state, "offline");
  await post("/api/dev/simulate", { kind: "host-online", a: "demo-b" });
  await until("demo-b live after the simulation", async () => (await hostState())?.state === "live", 20000);
});
