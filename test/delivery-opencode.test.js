// OpenCode as a way in (core/delivery/opencode.js): which of the running
// OpenCode servers a message goes to. Every OpenCode on a machine reads the
// same database, so every server knows every session; only the one whose
// TUI has the session open shows the turn a message starts. Consent, kept
// under the id the router looks it up by; the sessions you allowed, by the
// titles their OpenCode gives; and how a delivered message reads. Made-up
// servers, folders and sessions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createOpenCode, samePath, openCodeId, fromOpenCodeId } from "../core/delivery/opencode.js";
import { DeliveryPolicy, framed } from "../core/delivery/policy.js";
import { attest } from "../core/engine/attest.js";
import { createVerifier } from "../core/engine/verify.js";
import { agentOf, productOf } from "../core/procs.js";
import { DeliveryRouter } from "../core/delivery/router.js";
import { deliveryRoutes } from "../server/routes/delivery.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";

// Servers on ports 1, 2 and 3, each started in its own folder. Every one
// knows every session, as OpenCode's shared database has it; `busy` names
// the server running a turn of it.
function servers({ dirs = { 1: "/w/ledger", 2: "/w/atlas", 3: "/w/atlas-docs" }, busy = null, sessionDir = "/w/atlas/" } = {}) {
  const posted = [];
  const fetch = async (url, opts = {}) => {
    const u = new URL(url);
    if (opts.method === "POST") return (posted.push(u.port + u.pathname), { status: 204, ok: true });
    const body =
      u.pathname === "/global/health" ? { healthy: true } :
      u.pathname === "/path" ? { directory: dirs[u.port], worktree: dirs[u.port] } :
      u.pathname === "/session/status" ? (u.port === busy ? { ses_abc123: { type: "busy" } } : {}) :
      u.pathname === "/session/ses_abc123" ? { id: "ses_abc123", title: "Ledger tests", directory: sessionDir } :
      null;
    return body ? { ok: true, status: 200, json: async () => body } : { ok: false, status: 404, json: async () => ({}) };
  };
  const oc = createOpenCode({ fetch, processes: async () => Object.keys(dirs).map((p) => ({ pid: Number(p), args: `opencode --port ${p}` })) });
  return { oc, posted };
}

test("an idle session's message goes to the OpenCode started in its folder, not the first one that knows it", async () => {
  const { oc, posted } = servers();
  assert.deepEqual(await oc.deliver("opencode:ses-abc123", "hi"), { ok: true, where: "http://127.0.0.1:2" });
  assert.deepEqual(posted, ["2/session/ses_abc123/prompt_async"]);
  assert.deepEqual((await oc.servers()).map((s) => s.dir), ["/w/ledger", "/w/atlas", "/w/atlas-docs"], "each server's own folder, from GET /path");
});

test("a server running a turn of the session still comes first, wherever it was started", async () => {
  const { oc, posted } = servers({ busy: "3" });
  await oc.deliver("opencode:ses-abc123", "hi");
  assert.deepEqual(posted, ["3/session/ses_abc123/prompt_async"]);
});

test("with no server in the session's folder, any that knows it takes the message, as before", async () => {
  const { oc, posted } = servers({ sessionDir: "/w/elsewhere" });
  await oc.deliver("opencode:ses-abc123", "hi");
  assert.deepEqual(posted, ["1/session/ses_abc123/prompt_async"]);
});

test("xenonet live allow takes the id OpenCode shows, keeps it as XenoNet's, and refuses what isn't a session's id", async () => {
  assert.equal(fromOpenCodeId("ses_abc123"), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId(" opencode:ses_abc123 "), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId("opencode:ses-abc123"), "opencode:ses-abc123");
  assert.equal(fromOpenCodeId("claude:11111111-2222"), "claude:11111111-2222");
  assert.equal(openCodeId(fromOpenCodeId("ses_abc123")), "ses_abc123");
  const policy = new DeliveryPolicy();
  const route = deliveryRoutes({ policy, router: { list: () => [] }, verify: { person: async () => ({ ok: true }) }, readJson: async (r) => r.body, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (path, body) => {
    const res = {};
    await route({ method: "POST", headers: {}, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  const ok = await go("/api/delivery/allow", { session: "ses_abc123", hours: 1 });
  assert.equal(ok.status, 200);
  assert.equal(ok.body.session, "opencode:ses-abc123", "the answer says which id the consent is kept under");
  assert.ok(policy.consented("opencode:ses-abc123"), "under the id the router asks about");
  for (const bad of ["atlas", "", "opencode:", "ses_"]) {
    const res = await go("/api/delivery/allow", { session: bad });
    assert.equal(res.status, 400, bad);
    assert.match(res.body.error, /isn't a session's id/);
  }
  assert.deepEqual(Object.keys(policy.data.consent), ["opencode:ses-abc123"], "nothing kept for the refused ones");
  await go("/api/delivery/disallow", { session: "ses_abc123" });
  assert.equal(policy.consented("opencode:ses-abc123"), false);
});

test("folders compare without a trailing slash, and on Windows whatever the case and slashes", () => {
  assert.ok(samePath("/w/atlas/", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/Atlas", "/w/atlas", "darwin"));
  assert.ok(!samePath("/w/atlas", "/w/atlas-docs", "darwin"));
  assert.ok(samePath("C:\\W\\Atlas\\", "c:/w/atlas", "win32"));
});

// A machine with one Claude Code session XenoNet reads, and three OpenCode
// sessions it has no record of: two you allowed, one you didn't. Their own
// OpenCode answers for each (title, folder, last change).
function machine() {
  const t = { now: Date.now() };
  const now = () => t.now;
  const A = "claude:aaaaaaaa-0001", BILLING = "opencode:ses-billing0001", DOCS = "opencode:ses-docs000002", OTHER = "opencode:ses-other00003";
  const index = new Map([
    ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
    ["project:docs", { id: "project:docs", kind: "project", name: "docs", path: "/w/docs" }],
    [A, { id: A, kind: "session", app: "claude", title: "Add refunds", cwd: "/w/atlas", updatedAt: t.now - 10e3 }],
  ]);
  const state = { index, live: { sessions: new Map() }, homeOf: () => null, graph: { identities: [] } };
  const store = new EngineStore(":memory:");
  store.touch(A, { app: "claude", cwd: "/w/atlas", via: "hook" }, t.now);
  const policy = new DeliveryPolicy({ now });
  const repos = { of: async (cwd) => (cwd ? { top: cwd, common: `${cwd}/.git`, branch: "main" } : null) };
  const engine = new Engine({ state, store, now, repos, policy });
  const told = {
    [BILLING]: { title: "Billing API", directory: "/w/atlas", updated: t.now - 5e3, busy: false },
    [DOCS]: { title: "Docs site", directory: "/w/docs", updated: t.now - 60e3, busy: true },
    [OTHER]: { title: "Somebody else's", directory: "/w/docs", updated: t.now, busy: false },
  };
  const asked = [], delivered = [];
  const opencode = {
    info: async (id) => (asked.push(id), told[id] ?? null),
    deliver: async (id, text) => (delivered.push([id, text]), { ok: true, where: "http://127.0.0.1:4096" }),
  };
  const router = new DeliveryRouter({ engine, policy, opencode, ackMs: 60e3 });
  engine.delivery = router;
  policy.allow(BILLING);
  policy.allow(DOCS);
  return { t, engine, router, policy, store, asked, delivered, A, BILLING, DOCS, OTHER };
}

test("the OpenCode sessions you allowed are among the agents, by the titles and folders their OpenCode gives", async () => {
  const m = machine();
  const agents = await m.engine.agents();
  const byId = Object.fromEntries(agents.map((a) => [a.id, a]));
  assert.deepEqual(Object.keys(byId).sort(), [m.A, m.BILLING, m.DOCS].sort(), "not the one you didn't allow");
  assert.equal(byId[m.BILLING].title, "Billing API");
  assert.equal(byId[m.BILLING].project, "atlas");
  assert.equal(byId[m.BILLING].status, "idle");
  assert.equal(byId[m.DOCS].status, "working", "a turn under way, as its OpenCode says");
  assert.equal(byId[m.DOCS].reachable, true);
  assert.equal(byId[m.A].title, "Add refunds", "a session the map reads keeps its own title");
  assert.ok(!m.asked.includes(m.A), "and OpenCode isn't asked about it");
  m.policy.disallow(m.DOCS);
  assert.ok(!(await m.engine.agents()).some((a) => a.id === m.DOCS), "taking consent back takes it off the list");
});

test("an OpenCode session sends by title words and by selector, and the message goes in through its OpenCode", async () => {
  const m = machine();
  const sent = await m.engine.send(m.A, { to: "docs site", text: "Which release is the docs site on?" });
  assert.deepEqual(sent.to.map((r) => [r.id, r.delivery]), [[m.DOCS, "right away, through its OpenCode server"]]);
  // From one OpenCode session to the others, by selector: never to itself.
  const wide = await m.engine.send(m.BILLING, { to: "app:opencode", text: "Anyone on the docs?" });
  assert.deepEqual(wide.to.map((r) => r.id), [m.DOCS]);
  const narrow = await m.engine.send(m.BILLING, { to: "project:docs app:opencode", text: "And the release?" });
  assert.deepEqual(narrow.to.map((r) => r.id), [m.DOCS]);
  await new Promise((r) => setImmediate(r));
  assert.equal(m.delivered.length, 3);
  const [id, text] = m.delivered[1];
  assert.equal(id, m.DOCS);
  assert.match(text, /^\[XenoNet from Billing API\] \(message #\d+;/, "framed with the sender's title from its OpenCode");
  await assert.rejects(m.engine.send(m.A, { to: "somebody else", text: "x" }), /no session or actor matches/, "a session you didn't allow can't be found by its title");
});

test("the router lists only live consent for OpenCode, and describes only OpenCode sessions", async () => {
  const m = machine();
  m.policy.allow("claude:aaaaaaaa-0001");
  assert.deepEqual(m.router.known().sort(), [m.BILLING, m.DOCS].sort());
  m.t.now += 9 * 3600e3;
  assert.deepEqual(m.router.known(), [], "consent expires (8 hours by default)");
  assert.equal(await m.router.describe(m.A), null);
  assert.equal((await m.router.describe(m.BILLING)).title, "Billing API");
  assert.deepEqual(new DeliveryRouter({ engine: m.engine, policy: m.policy }).known(), [], "no OpenCode way in, none listed");
});

test("info: what the session's own OpenCode says of it, kept for a while; null when no OpenCode knows it", async () => {
  let t = 0, calls = 0;
  const fetch = async (url) => {
    calls++;
    const u = new URL(url);
    const body =
      u.pathname === "/global/health" ? { healthy: true } :
      u.pathname === "/path" ? { directory: "/w/docs" } :
      u.pathname === "/session/status" ? { ses_docs000002: { type: "busy" } } :
      u.pathname === "/session/ses_docs000002" ? { id: "ses_docs000002", title: "Docs site", directory: "/w/docs", time: { created: 1, updated: 2 } } :
      null;
    return body ? { ok: true, status: 200, json: async () => body } : { ok: false, status: 404, json: async () => ({}) };
  };
  const oc = createOpenCode({ fetch, processes: async () => [{ pid: 7, args: "opencode --port 4096" }], now: () => t });
  assert.deepEqual(await oc.info("opencode:ses-docs000002"), { title: "Docs site", directory: "/w/docs", updated: 2, busy: true });
  const before = calls;
  await oc.info("opencode:ses-docs000002");
  assert.equal(calls, before, "asked again within the cache time: no requests");
  assert.equal(await oc.info("opencode:ses-nobody00009"), null);
  t += 11e3;
  await oc.info("opencode:ses-docs000002");
  assert.ok(calls > before, "and asked afresh after it");
});

test("a delivered message carries XenoNet's verified line whole, with what XenoNet couldn't check and that it approves nothing", () => {
  const env = attest({ sender: { id: "opencode:ses-billing0001", kind: "agent", name: "x".repeat(90) }, method: "session", verdict: null, platform: "darwin" });
  assert.ok(env.line.length > 300, "a long line, as a 90-character title makes it");
  const text = framed({ id: 4, from: "Billing API", verified: env.line, text: "hi" });
  assert.ok(text.includes(env.line.replace(/\.$/, "")), "nothing cut");
  assert.match(text, /so it's its word, on this machine\. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing\. This is another agent's message/);
  assert.doesNotMatch(text, /\.\. /, "one full stop between XenoNet's line and the framing");
});

// A made-up machine: two OpenCode TUIs in tmux, a shell and the xenonet
// command under the first (OpenCode's own shell tool), and a program in a
// plain terminal that isn't under any agent.
function openCodeMachine() {
  const rows = [
    [1, 0, "/sbin/launchd"],
    [50, 1, "/opt/homebrew/bin/tmux"],
    [60, 50, "/Users/me/.opencode/bin/opencode"],
    [61, 50, "/opt/homebrew/lib/node_modules/opencode-ai/node_modules/opencode-darwin-arm64/bin/opencode"],
    [70, 60, "/bin/zsh"],
    [80, 70, "/usr/local/bin/node"],
    [400, 1, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"],
    [410, 400, "/bin/zsh"],
    [420, 410, "/usr/local/bin/node"],
  ];
  const table = new Map(rows.map(([pid, ppid, program]) => [pid, { pid, ppid, start: 1000 + pid, program }]));
  const procs = {
    supported: true,
    peer: async (socket) => socket.pid,
    chain: async (pid) => {
      const out = [];
      for (let p = table.get(pid); p; p = p.ppid > 1 ? table.get(p.ppid) : null) {
        const agent = agentOf(p.program);
        out.push({ ...p, agent, product: agent ? null : productOf(p.program) });
      }
      return out;
    },
    alive: async ({ pid, start }) => table.get(pid)?.start === start,
    signed: async () => false,
  };
  const store = new EngineStore(":memory:");
  return { verify: createVerifier({ procs, store }), store, from: (pid) => ({ pid }) };
}

test("OpenCode is an agent to XenoNet's process checks: its sessions' requests come from under it, and what's yours isn't done from under it", async () => {
  assert.equal(agentOf("/Users/me/.opencode/bin/opencode"), "opencode");
  assert.equal(agentOf("C:/Users/me/AppData/Roaming/npm/node_modules/opencode-ai/bin/opencode.exe"), "opencode");
  assert.equal(agentOf("/usr/local/bin/opencoder"), null);
  const { verify, store, from } = openCodeMachine();
  const S = "opencode:ses-billing0001";
  const own = await verify.check(from(60), S);
  assert.deepEqual([own.ok, own.checked], [true, true], "from OpenCode's own process, as XenoNet's tool for it calls");
  assert.equal((await verify.check(from(80), S)).ok, true, "from under it");
  assert.equal((await verify.check(from(61), S)).ok, true, "and from another OpenCode, which can open the same session");
  assert.deepEqual(store.pins(S), [], "so no OpenCode session is pinned to a process");
  const stranger = await verify.check(from(420), S);
  assert.equal(stranger.ok, false);
  assert.match(stranger.reason, /have to come from an OpenCode process/);
  // Consent, connecting agents and the rest of what's yours: not from under OpenCode.
  const yours = await verify.person(from(80));
  assert.equal(yours.ok, false);
  assert.match(yours.reason, /came from under an OpenCode process; run it in your own terminal/);
  assert.equal((await verify.person(from(420))).ok, true, "from your own terminal");
  const env = attest({ sender: { id: S, kind: "agent", name: "Billing API" }, method: "session", verdict: own, platform: "darwin" });
  assert.equal(env.auth.process, "checked");
  assert.match(env.line, /XenoNet checked the process it came from/);
});
