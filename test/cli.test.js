// The xenonet command (cli/): its arguments and output helpers, the script it
// installs (core/command.js), and each command against a stand-in XenoNet, so
// these run without the desktop app.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "../cli/lib/args.js";
import { table, wrapText, plural, clip } from "../cli/lib/out.js";
import { commandScript, commandState, installCommand, uninstallCommand } from "../core/command.js";
import { HEALTH_MS } from "../core/client.js";
import net from "node:net";
import { stopFile } from "../core/components/runtime.js";
import { rpc } from "../core/components/rpc.js";
import { SERVICE_API } from "../core/components/protocol.js";
import { inOrder, changes } from "../cli/commands/update.js";
import { command } from "./harness/xenonet-bin.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-cli-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

test("arguments: flags anywhere, values, negations and everything after --", () => {
  assert.deepEqual(parse(["doctor", "--only", "claude,codex", "--no-color", "--port=47842", "-h"]), { flags: { only: "claude,codex", color: false, port: "47842", help: true }, rest: ["doctor"] });
  assert.deepEqual(parse(["who", "--json", "src/a.js"]), { flags: { json: true }, rest: ["who", "src/a.js"] });
  assert.deepEqual(parse(["console", "--instance", "dev", "--", "type:--x"]), { flags: { instance: "dev" }, rest: ["console", "type:--x"] });
  assert.deepEqual(parse(["--dry-run"]).flags, { dryRun: true });
});

test("output: columns line up, the last one wraps under itself, and painted text stays painted", () => {
  const out = table([["✓", "short", "one"], ["!", "a longer title", "two words"]], { indent: 2 });
  assert.equal(out, "  ✓  short           one\n  !  a longer title  two words");
  assert.deepEqual(wrapText("the quick brown fox jumps", 10), ["the quick", "brown fox", "jumps"]);
  assert.deepEqual(wrapText("\x1b[2mdim words here\x1b[0m", 9), ["\x1b[2mdim words\x1b[0m", "\x1b[2mhere\x1b[0m"]);
  assert.deepEqual(wrapText("abcdefghij", 4), ["abcd", "efgh", "ij"]);
  assert.equal(plural(1, "check"), "1 check");
  assert.equal(plural(2400, "thread"), "2,400 threads");
  assert.equal(clip("abcdef", 4), "abc…");
});

test("install: writes XenoNet's script, replaces only its own, and takes it out", () => {
  const dir = path.join(tmp(), "bin");
  const env = { PATH: `/usr/bin${path.delimiter}${dir}` };
  assert.equal(commandState({ platform: "linux", dir, env }).installed, false);
  const st = installCommand({ platform: "linux", dir, env, bin: "/opt/xenonet's/launcher/xenonet" });
  assert.deepEqual([st.installed, st.ours, st.onPath, st.runs], [true, true, true, ["/opt/xenonet's/launcher/xenonet"]]);
  assert.equal(st.broken, true, "the binary doesn't exist here");
  if (process.platform !== "win32") assert.equal(fs.statSync(st.file).mode & 0o111, 0o111);
  assert.match(commandScript({ platform: "linux", bin: "/a/xenonet" }), /^#!\/bin\/sh\n# Added by XenoNet.*\nexec '\/a\/xenonet' "\$@"\n$/);
  assert.match(commandScript({ platform: "win32", bin: "C:\\x\\xenonet.exe" }), /\n"C:\\x\\xenonet\.exe" %\*\n$/);
  assert.equal(uninstallCommand({ platform: "linux", dir, env }).installed, false);

  // An earlier XenoNet's script, which ran a node and cli/xenonet.js, is
  // still XenoNet's: read as such, and replaced by installing.
  fs.writeFileSync(path.join(dir, "xenonet"), "#!/bin/sh\n# Added by XenoNet: the xenonet command. xenonet uninstall takes it out.\nELECTRON_RUN_AS_NODE=1 exec '/a/Electron' '/b/xenonet.js' \"$@\"\n");
  assert.deepEqual([commandState({ platform: "linux", dir, env }).ours, commandState({ platform: "linux", dir, env }).runs], [true, ["/a/Electron", "/b/xenonet.js"]]);
  assert.deepEqual(installCommand({ platform: "linux", dir, env, bin: "/b" }).runs, ["/b"]);

  fs.writeFileSync(path.join(dir, "xenonet"), "#!/bin/sh\necho someone else's\n");
  assert.throws(() => installCommand({ platform: "linux", dir, env, bin: "/b" }), /isn't XenoNet's/);
  assert.throws(() => uninstallCommand({ platform: "linux", dir, env }), /leaves it alone/);
  assert.equal(installCommand({ platform: "linux", dir, env, bin: "/b", force: true }).ours, true);
});

// A stand-in XenoNet: `routes` maps "GET /api/x" to (url, body) => [status,
// data], or a promise of it.
async function fakeXenoNet(routes) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", async () => {
      const url = new URL(req.url, "http://x");
      seen.push({ method: req.method, path: url.pathname, query: Object.fromEntries(url.searchParams), body: body ? JSON.parse(body) : null });
      const route = routes[`${req.method} ${url.pathname}`];
      const [status, data] = route ? await route(url, body ? JSON.parse(body) : null) : [404, { error: "not found" }];
      res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => server.close() };
}

// Every run has a home folder of its own, an empty PATH and none of the
// variables that point at real folders, so a command that works here when
// it finds no XenoNet (the doctor, when a stand-in is slow to answer) reads
// nothing of this computer's: no sign-ins, folders or settings. Names are
// matched in any case, as Windows spells PATH Path.
const home = tmp();
const REAL = new Set(["PATH", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "HERMES_HOME", "GH_CONFIG_DIR", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "APPDATA", "LOCALAPPDATA", "XENONET_STATE_DIR", "XENONET_CACHE_DIR", "XENONET_PROJECTS_DIR", "XENONET_WIKI_DIR", "XENONET_CLAUDE_DESKTOP_DIR"]);
function xenonet(args, { cwd = root, input = "", env = {} } = {}) {
  const own = Object.fromEntries(Object.entries(process.env).filter(([name]) => !REAL.has(name.toUpperCase())));
  Object.assign(own, { NO_COLOR: "1", XENONET_PORT: "", HOME: home, USERPROFILE: home, PATH: path.join(home, "bin"), XENONET_CODEX_BIN: path.join(home, "bin", "codex") });
  return new Promise((resolve) => {
    const proc = spawn(...command(args), { cwd, env: { ...own, ...env }, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    // A command that's done before it reads its input (one refused at
    // once) closes the pipe first, which isn't the test's to fail on.
    proc.stdin.on("error", () => {});
    proc.stdin.end(input);
    proc.on("exit", (code) => resolve({ code, stdout, stderr }));
  });
}

// A command against a stand-in XenoNet that has to reach `route` there. One
// that finds no XenoNet in time works here instead (the doctor runs on this
// computer), so the test fails saying that, not on what it printed.
async function via(s, route, args, opts) {
  const from = s.seen.length;
  const out = await xenonet(args, opts);
  assert.ok(s.seen.slice(from).some((r) => `${r.method} ${r.path}` === route), `xenonet ${args.join(" ")} didn't reach the stand-in XenoNet in time`);
  return out;
}

// A port nothing listens on.
async function closedPort() {
  const server = http.createServer();
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const { port } = server.address();
  await new Promise((r) => server.close(r));
  return port;
}

const health = { ok: true, name: "XenoNet Dev", version: "9.9.9", demo: false, sources: [{ name: "projects", ok: true, count: 3 }, { name: "claude", ok: true, count: 12 }, { name: "codex", ok: true, count: 1 }] };
const agent = { id: "claude:aaaaaaaa-0001", app: "claude", title: "Add partial refunds", project: "atlas", branch: "main", status: "working", needsYou: null, focus: "refunds", editing: [{ path: "/w/atlas/src/refunds.ts", at: Date.now() }], claims: [], lastActive: Date.now(), reachable: true, top: "/w/atlas", cwd: "/w/atlas" };
const doctorResult = (status) => ({
  at: Date.now(),
  ms: 420,
  results: [
    { id: "xenonet.server", group: "XenoNet", title: "XenoNet is running", status: "ok", detail: "XenoNet Dev on port 1" },
    { id: "claude.auth", group: "Claude Code", title: "Claude Code is signed in", status, detail: status === "ok" ? "signed in" : "not signed in", fix: "Run claude and sign in with /login." },
  ],
  summary: { ok: status === "ok" ? 2 : 1, info: 0, warn: 0, fail: status === "fail" ? 1 : 0, skip: 0 },
});

test("xenonet status, agents, who and doctor: what a running XenoNet says, and the exit codes", async () => {
  let doctor = doctorResult("ok");
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/summary": () => [200, { working: 1, needsYou: [{ id: "codex:x", app: "codex", title: "Rate-limit the API", kind: "question", since: Date.now() - 60e3 }], review: 0, stuck: [], collisions: 0, actorsNeedYou: [{ id: "actor:wren", name: "Wren", kind: "permission", text: "May I message the team?", since: Date.now() - 120e3 }], requests: 2 }],
    "GET /api/engine/agents": () => [200, [agent]],
    "GET /api/engine/who": (url) => {
      const file = url.searchParams.get("path");
      if (file === "/w/atlas/src/refunds.ts") return [200, [{ agent: { id: agent.id, title: agent.title, app: "claude" }, how: "editing", at: Date.now() }]];
      // An actor's edit XenoNet inferred from what it observes.
      if (file === "/w/atlas/docs/guide.md") return [200, [{ agent: { id: "actor:hermes", title: "Hermes", app: "actor" }, how: "editing", at: Date.now(), observed: true }]];
      return [200, []];
    },
    "GET /api/doctor": () => [200, doctor],
    "POST /api/doctor": () => [200, doctor],
  });
  const port = ["--port", String(s.port)];
  try {
    const status = await via(s, "GET /api/summary", port);
    assert.equal(status.code, 0);
    assert.match(status.stdout, /^● XenoNet Dev 9\.9\.9 on 127\.0\.0\.1:\d+/);
    assert.match(status.stdout, /Reads\s+3 projects, 12 Claude sessions, 1 Codex thread\n/);
    // The longest wait first, as the Needs you panel lists them; programs
    // asking to be actors, by how many, at the end.
    assert.match(status.stdout, /Needs you\s+◇ Wren \(actor\) needs permission\n\s+● Rate-limit the API \(codex\) has a question\n\s+◇ 2 programs ask to join XenoNet; answer in XenoNet's window/);
    assert.deepEqual(JSON.parse((await xenonet(["status", "--json", ...port])).stdout).queue.map((it) => it.kind), ["actor", "question", "requests"]);
    assert.match(status.stdout, /Doctor\s+everything XenoNet needs is in place/);

    const agents = await xenonet(["agents", ...port]);
    assert.match(agents.stdout, /● Add partial refunds  claude · atlas · main\n  says: refunds · editing refunds\.ts\n/);
    assert.deepEqual(JSON.parse((await xenonet(["agents", "--json", ...port])).stdout)[0].id, agent.id);

    // A relative path is taken from where the command runs.
    const who = await xenonet(["who", "src/refunds.ts", ...port], { cwd: fs.mkdtempSync(path.join(tmp(), "w")) });
    assert.match(who.stdout, /No session or actor has a hand in src\/refunds\.ts/);
    assert.ok(s.seen.at(-1).query.path.endsWith(`${path.sep}src${path.sep}refunds.ts`) && path.isAbsolute(s.seen.at(-1).query.path));
    const hand = await xenonet(["who", "/w/atlas/src/refunds.ts", ...port]);
    assert.match(hand.stdout, /Add partial refunds\s+claude\s+is editing it, just now/);
    const seen = await xenonet(["who", "/w/atlas/docs/guide.md", ...port]);
    assert.match(seen.stdout, /Hermes\s+actor\s+is editing it \(observed\), just now/, "an edit XenoNet only observed says so");

    const ok = await via(s, "POST /api/doctor", ["doctor", ...port]);
    assert.equal(ok.code, 0);
    assert.match(ok.stdout, /Everything XenoNet needs is in place · 2 checks in 0\.4 s, run by XenoNet Dev/);
    doctor = doctorResult("fail");
    const bad = await via(s, "POST /api/doctor", ["doctor", ...port]);
    assert.equal(bad.code, 1);
    assert.match(bad.stdout, /✗\s+Claude Code is signed in\s+not signed in\n\s+↳ Run claude and sign in with \/login\./);
    assert.match(bad.stdout, /^1 problem · 2 checks/m);
    const only = JSON.parse((await via(s, "POST /api/doctor", ["doctor", "--only", "xenonet", "--json", ...port])).stdout);
    assert.deepEqual(only.results.map((r) => r.id), ["xenonet.server"]);
  } finally {
    s.close();
  }
});

test("xenonet spend: today's estimate by app and by actor, with the rest unattributed, and a XenoNet from before it", async () => {
  const costs = { today: 12.4, byApp: { claude: 9.1, codex: 3.3 }, perHour: 1.8, unpriced: ["claude-made-up-9"], checked: "2026-10-03" };
  let byActor = {
    at: Date.now(),
    since: Date.now() - 3600e3,
    checked: "2026-10-03",
    total: 12.4,
    actors: [
      { id: "actor:wren", name: "Wren", cost: 2.1, tokens: 90000, sessions: 3, unpriced: [], elsewhere: 0 },
      { id: "actor:kestrel", name: "Kestrel", cost: 0, tokens: 0, sessions: 0, unpriced: [], elsewhere: 2 },
    ],
    unattributed: { cost: 10.3, tokens: 400000, sessions: 9, unpriced: ["claude-made-up-9"] },
  };
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/live": () => [200, { costs }],
    "GET /api/spend/actors": () => (byActor ? [200, byActor] : [404, { error: "not found" }]),
  });
  const port = ["--port", String(s.port)];
  try {
    const out = await via(s, "GET /api/spend/actors", ["spend", ...port]);
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /^About \$12\.40 today at API prices, \$1\.80 an hour at this pace\n  Claude Code {2}\$9\.10\n  Codex {8}\$3\.30\n/);
    assert.match(out.stdout, /By actor\n  Wren {10}\$2\.10 {3}3 sessions\n  Kestrel {7}— {7}2 runs elsewhere, not priced\n  Unattributed {2}\$10\.30 {2}9 sessions not tied to an actor, such as the ones you start\n/);
    assert.match(out.stdout, /Not counted, with no price: claude-made-up-9\. Prices checked 2026-10-03\./);
    assert.deepEqual(JSON.parse((await xenonet(["spend", "--json", ...port])).stdout), { costs, byActor });
    byActor = null;
    const old = await via(s, "GET /api/spend/actors", ["spend", ...port]);
    assert.equal(old.code, 0, old.stderr);
    assert.match(old.stdout, /About \$12\.40 today/);
    assert.match(old.stdout, /This XenoNet doesn't break the day down by actor\./);
    // Grok Build's, at what Grok recorded, or unknown when it recorded
    // none; and a day whose calls have no price at all isn't $0.
    costs.byApp.grok = 0.42;
    assert.match((await via(s, "GET /api/spend/actors", ["spend", ...port])).stdout, /\n  Codex {8}\$3\.30\n  Grok Build {3}\$0\.42\n/);
    costs.byApp.grok = null;
    assert.match((await via(s, "GET /api/spend/actors", ["spend", ...port])).stdout, /\n  Grok Build {3}unknown \(Grok recorded no cost\)\n/);
    Object.assign(costs, { today: null, perHour: 0, byApp: { claude: 0, codex: 0, grok: null }, unpriced: ["grok-build"] });
    const unknown = (await via(s, "GET /api/spend/actors", ["spend", ...port])).stdout;
    assert.match(unknown, /^What today's work would cost isn't known: none of its calls has a price\n/);
    assert.match(unknown, /Not counted, with no price: grok-build\./);
  } finally {
    s.close();
  }
});

test("a busy XenoNet on the port named still counts as running: it has longer than the quick probe", async () => {
  const s = await fakeXenoNet({
    "GET /api/health": () => new Promise((resolve) => setTimeout(() => resolve([200, health]), HEALTH_MS + 500)),
    "GET /api/summary": () => [200, { working: 0, needsYou: [], review: 0, stuck: [], collisions: 0 }],
  });
  try {
    const status = await xenonet(["--json", "--port", String(s.port)]);
    assert.equal(status.code, 0, status.stderr);
    assert.equal(JSON.parse(status.stdout).running, true);
  } finally {
    s.close();
  }
});

test("xenonet connect: shows what will change, needs a yes, then asks XenoNet to do it", async () => {
  let state = "missing";
  const preview = () => ({
    port: 47842,
    demo: false,
    apps: ["claude", "codex"].map((app) => ({ app, name: app === "claude" ? "Claude Code" : "Codex", changes: [{ id: "mcp", what: "The xenonet MCP server", where: "settings", state, detail: "xenonet mcp" }], notes: [], broken: [] })),
  });
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/engine/connect": () => [200, preview()],
    "POST /api/engine/connect": () => ((state = "present"), [200, preview()]),
    "DELETE /api/engine/connect": () => ((state = "missing"), [200, preview()]),
  });
  const port = ["--port", String(s.port)];
  try {
    const asked = await xenonet(["connect", ...port]);
    assert.equal(asked.code, 1);
    assert.match(asked.stdout, /\+ The xenonet MCP server \(to add\)/);
    assert.match(asked.stderr, /Run it again with --yes/);
    assert.ok(!s.seen.some((r) => r.method === "POST"));

    const one = await xenonet(["connect", "codex", "--yes", ...port]);
    assert.equal(one.code, 0, one.stderr);
    assert.deepEqual(s.seen.find((r) => r.method === "POST").body, { apps: ["codex"] });
    assert.match(one.stdout, /Connected Codex to XenoNet Dev/);
    assert.match((await xenonet(["connect", ...port])).stdout, /Already connected to XenoNet Dev/);

    const out = await xenonet(["disconnect", "--yes", ...port]);
    assert.match(out.stdout, /- The xenonet MCP server \(to take out\)/);
    assert.ok(s.seen.some((r) => r.method === "DELETE"));
    assert.equal((await xenonet(["connect", "cursor", ...port])).code, 2);
  } finally {
    s.close();
  }
});

test("xenonet disconnect takes out connect's own entries and leaves what XenoNet's plugin for Claude Code carries", async () => {
  let codex = "present";
  const change = (id, state, via) => ({ id, what: `The ${id}`, where: via ? "XenoNet's plugin for Claude Code (xenonet@xenonet)" : "settings", state, ...(via ? { via } : {}), detail: id });
  const preview = () => ({
    port: 47842,
    demo: false,
    apps: [
      { app: "claude", name: "Claude Code", changes: [change("mcp", "present", "plugin"), change("hooks", "present", "plugin")], notes: [], broken: [] },
      { app: "codex", name: "Codex", changes: [change("mcp", codex), change("hooks", codex)], notes: [], broken: [] },
    ],
  });
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/engine/connect": () => [200, preview()],
    "DELETE /api/engine/connect": () => ((codex = "missing"), [200, preview()]),
  });
  const port = ["--port", String(s.port)];
  try {
    const out = await xenonet(["disconnect", "--yes", ...port]);
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /Took XenoNet out of Codex /);
    assert.doesNotMatch(out.stdout, /^Claude Code$/m, "nothing of the plugin's to take out");
    assert.match(out.stdout, /XenoNet's plugin for Claude Code stays\. \/plugin in Claude Code uninstalls it\./);
    assert.equal(s.seen.filter((r) => r.method === "DELETE").length, 1);
    const again = await xenonet(["disconnect", "--yes", ...port]);
    assert.match(again.stdout, /nothing to take out/);
    assert.match(again.stdout, /XenoNet's plugin for Claude Code stays/);
    assert.equal(s.seen.filter((r) => r.method === "DELETE").length, 1, "nothing more to take out");
  } finally {
    s.close();
  }
});

test("xenonet hook --via=plugin marks the event as the copy XenoNet's plugin for Claude Code sends", async () => {
  const s = await fakeXenoNet({ "POST /api/hook/claude": () => [200, { systemMessage: "from XenoNet" }] });
  try {
    const r = await xenonet(["hook", "claude", `--port=${s.port}`, "--via=plugin"], { input: '{"hook_event_name":"SessionStart"}' });
    assert.deepEqual([r.code, JSON.parse(r.stdout)], [0, { systemMessage: "from XenoNet" }]);
    assert.deepEqual(s.seen.map((x) => [x.path, x.query, x.body]), [["/api/hook/claude", { via: "plugin" }, { hook_event_name: "SessionStart" }]]);
    await xenonet(["hook", "claude", `--port=${s.port}`], { input: "{}" });
    assert.deepEqual(s.seen[1].query, {}, "connect's own carry no mark");
  } finally {
    s.close();
  }
});

test("xenonet hook grok passes Grok Build's events to its own route, as Grok wrote them", async () => {
  const s = await fakeXenoNet({ "POST /api/hook/grok": () => [200, { decision: "block", reason: "a message" }] });
  try {
    const r = await xenonet(["hook", "grok", `--port=${s.port}`], { input: '{"hook_event_name":"Stop","sessionId":"019a3b5c-0000-7000-8000-00000000000a"}' });
    assert.deepEqual([r.code, JSON.parse(r.stdout)], [0, { decision: "block", reason: "a message" }]);
    assert.deepEqual(s.seen.map((x) => [x.path, x.body]), [["/api/hook/grok", { hook_event_name: "Stop", sessionId: "019a3b5c-0000-7000-8000-00000000000a" }]]);
  } finally {
    s.close();
  }
});

test("xenonet hook for an app XenoNet doesn't tail passes the event nowhere, says why in XenoNet's log, and lets the agent carry on", async () => {
  const s = await fakeXenoNet({ "POST /api/hook/codex": () => [200, { systemMessage: "from XenoNet" }], "POST /api/hook/claude": () => [200, { systemMessage: "from XenoNet" }] });
  const state = tmp();
  try {
    for (const app of ["gemini-cli", "Codex"]) {
      const r = await xenonet(["hook", app, `--port=${s.port}`], { input: '{"hook_event_name":"SessionStart"}', env: { XENONET_STATE_DIR: state } });
      assert.deepEqual([r.code, r.stdout, r.stderr], [0, "", ""], app);
    }
    assert.deepEqual(s.seen, [], "nothing reached XenoNet");
    const logged = fs.readFileSync(path.join(state, "logs", "cli.ndjson"), "utf8").trim().split("\n").map((l) => JSON.parse(l));
    assert.deepEqual(logged.map((e) => [e.level, e.code, e.message]), [
      ["warn", "cli.hook", 'xenonet hook takes claude, codex or grok, not "gemini-cli"'],
      ["warn", "cli.hook", 'xenonet hook takes claude, codex or grok, not "Codex"'],
    ]);
    // No app named is Codex, as it always was.
    const none = await xenonet(["hook", `--port=${s.port}`], { input: "{}", env: { XENONET_STATE_DIR: state } });
    assert.deepEqual([none.code, JSON.parse(none.stdout), s.seen.map((x) => x.path)], [0, { systemMessage: "from XenoNet" }, ["/api/hook/codex"]]);
  } finally {
    s.close();
  }
});

test("xenonet diagnose --ask refuses an app XenoNet can't ask, before it writes anything", async () => {
  const state = tmp();
  for (const [args, env, shown] of [
    [["--app", "gemini-cli"], {}, '"gemini-cli"'],
    [[], { XENONET_DIAGNOSE_APP: "cursor" }, '"cursor"'],
    // Grok Build is tailed, but XenoNet doesn't ask it.
    [["--app", "grok"], {}, '"grok"'],
  ]) {
    const r = await xenonet(["diagnose", "--ask", ...args], { env: { XENONET_STATE_DIR: state, ...env } });
    assert.equal(r.code, 2, r.stderr);
    assert.equal(r.stderr, `xenonet: --app takes claude or codex, not ${shown}\n`);
    assert.equal(r.stdout, "");
  }
  assert.equal(fs.existsSync(path.join(state, "diagnostics")), false, "no folder written for it");
});

test("xenonet connect --here: a XenoNet answering through a forwarded port is connected in this computer's settings, never by asking it", async () => {
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "GET /api/engine/connect": () => [200, { port: 1, apps: [] }] });
  try {
    const missing = await xenonet(["connect", "--here"]);
    assert.equal(missing.code, 2);
    assert.match(missing.stderr, /--here needs --port/);
    const r = await xenonet(["connect", "--here", "--port", String(s.port), "--json"]);
    assert.equal(r.code, 0, r.stderr);
    const preview = JSON.parse(r.stdout);
    assert.equal(preview.port, s.port, "that XenoNet's entries, by its port");
    assert.deepEqual(preview.apps.map((a) => a.app), ["claude", "codex", "grok"]);
    assert.ok(preview.apps.every((a) => a.changes.length), "what would be added here");
    assert.deepEqual(s.seen, [], "the XenoNet on that port is never asked: it would write its own computer's settings");
  } finally {
    s.close();
  }
});

test("xenonet root: the projects folder, a check, a change and back", async () => {
  const home = "/home/nova";
  let current = { path: `${home}/Projects`, source: "default" };
  const info = () => ({ ...current, env: null, chosen: current.source === "chosen" ? current.path : null, default: `${home}/Projects`, home, scan: { ok: true, at: Date.now(), projects: current.source === "chosen" ? 4 : 11, groups: 1, truncated: false } });
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/workspace/root": () => [200, info()],
    "POST /api/workspace/check": (_url, b) =>
      b.path === "~/nowhere" ? [200, { ok: false, path: `${home}/nowhere`, error: "~/nowhere doesn't exist.", warnings: [], scan: null }] : [200, { ok: true, path: `${home}/Clients`, error: null, current: false, warnings: ["~/Clients is a repo itself."], scan: { projects: 4, groups: 1, dirsRead: 9, ms: 3 } }],
    "POST /api/workspace/root": (_url, b) => {
      if (b.path === "~/nowhere") return [400, { error: "~/nowhere doesn't exist." }];
      const changed = current.path !== `${home}/Clients`;
      current = { path: `${home}/Clients`, source: "chosen" };
      return [200, { ...info(), changed }];
    },
    "DELETE /api/workspace/root": () => ((current = { path: `${home}/Projects`, source: "default" }), [200, { ...info(), changed: true }]),
  });
  const port = ["--port", String(s.port)];
  try {
    const now = await xenonet(["root", ...port]);
    assert.equal(now.code, 0);
    assert.match(now.stdout, /^● ~\/Projects the default\n {2}11 projects in 1 group \(scanned just now\)\n {2}xenonet root <folder> reads another\.\n$/);

    const check = await xenonet(["root", "--check", "~/Clients", ...port]);
    assert.match(check.stdout, /^✓ ~\/Clients: 4 projects in 1 group \(9 folders read in 3 ms\)\n! ~\/Clients is a repo itself\.\n$/);
    assert.equal(s.seen.at(-1).body.path, "~/Clients", "~ is left for XenoNet to read");
    const missing = await xenonet(["root", "--check", "~/nowhere", ...port]);
    assert.deepEqual([missing.code, missing.stdout], [1, "✗ ~/nowhere doesn't exist.\n"]);

    const moved = await xenonet(["root", "~/Clients", ...port]);
    assert.equal(moved.code, 0, moved.stderr);
    assert.match(moved.stdout, /^✓ XenoNet reads ~\/Clients now: 4 projects in 1 group\n {2}The layout, your rules and Codex's picks for ~\/Projects stay, for when you come back\.\n$/);
    assert.match((await xenonet(["root", "~/Clients", ...port])).stdout, /^· XenoNet reads ~\/Clients already/);
    assert.match((await xenonet(["root", ...port])).stdout, /chosen in XenoNet\n.*\n {2}xenonet root --reset goes back to ~\/Projects\.\n$/);
    const bad = await xenonet(["root", "~/nowhere", ...port]);
    assert.deepEqual([bad.code, bad.stderr], [1, "xenonet: ~/nowhere doesn't exist.\n"]);

    // A relative folder is taken from where the command runs.
    const cwd = fs.mkdtempSync(path.join(tmp(), "w"));
    await xenonet(["root", "--check", "code", ...port], { cwd });
    assert.equal(s.seen.at(-1).body.path, path.join(fs.realpathSync(cwd), "code"));

    const reset = JSON.parse((await xenonet(["root", "--reset", "--json", ...port])).stdout);
    assert.deepEqual([reset.source, reset.changed], ["default", true]);
    assert.equal((await xenonet(["root", "--reset", "~/x", ...port])).code, 2);
    assert.equal((await xenonet(["root", "--check", ...port])).code, 2);
  } finally {
    s.close();
  }
});

// Made-up drift, as drift.json keeps it: one record from before you last
// looked, a change and an addition after.
const DRIFT_LOOKED = 1791230000000;
const driftRecord = (source, kind, at, extra) => ({ schema: "drift/record/v1", source, vendorVersion: null, kind, path: null, shape: null, firstSeen: at, lastSeen: at, count: 2, ...extra });
const DRIFT = [
  driftRecord("openai/codex/rollout-line", "new-field", DRIFT_LOOKED - 1000, { path: "ordinal", shape: "number" }),
  driftRecord("openai/codex/rollout-line", "missing-field", DRIFT_LOOKED + 1000, { path: "payload.cli_version", vendorVersion: "0.151.0" }),
  driftRecord("anthropic/claude-code/transcript-line", "new-variant", DRIFT_LOOKED + 2000, { path: "type", shape: '"progress"' }),
];

test("xenonet drift: what's changed since you last looked, from a running XenoNet, which then marks it seen", async () => {
  const { DriftLog } = await import("../core/drift.js");
  const { driftReport } = await import("../core/vendor.js");
  const log = new DriftLog();
  for (const r of DRIFT) log.record(r, r.firstSeen);
  log.looked(DRIFT_LOOKED);
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/drift": (url) => [200, { ...driftReport({ log, all: url.searchParams.get("all") === "1", readers: [], now: DRIFT_LOOKED + 5000 }), file: "~/Library/Caches/XenoNet Dev/drift.json" }],
    "POST /api/drift/seen": (_url, body) => [200, { lookedAt: body.at }],
  });
  const port = ["--port", String(s.port)];
  try {
    const out = await xenonet(["drift", ...port]);
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /^XenoNet Dev · drift kept in ~\/Library\/Caches\/XenoNet Dev\/drift\.json\n/);
    assert.match(out.stdout, /\n {2}Codex's rollouts {2}vendor\/openai\/codex\/rollout-line\/v1 \(<1\.0\.0\)\n {4}read {5}nothing since XenoNet started\n {4}changed {2}payload\.cli_version is missing {2}\(since 0\.151\.0; 1 time\)\n/);
    assert.match(out.stdout, /\n {4}added {4}type "progress" isn't one the schema lists {2}\(1 time\)\n/);
    assert.match(out.stdout, /2 differences new since you last looked \(.+\), 1 change to what XenoNet reads; 3 known in all\n\nMarked seen/);
    assert.doesNotMatch(out.stdout, /ordinal/, "only what's new");
    assert.deepEqual(s.seen.at(-1), { method: "POST", path: "/api/drift/seen", query: {}, body: { at: DRIFT_LOOKED + 5000 } }, "seen up to when the report was made");
    // For scripts: exit 1 when there's new drift (plain, it's 0, as above),
    // and leave it new, so the next run says so again until you look.
    const marks = () => s.seen.filter((r) => r.path === "/api/drift/seen").length;
    const marked = marks();
    const scripted = await xenonet(["drift", "--exit-code", "--json", ...port]);
    assert.deepEqual([scripted.code, JSON.parse(scripted.stdout).new], [1, 2]);
    const again = await xenonet(["drift", "--exit-code", ...port]);
    assert.equal(again.code, 1);
    assert.match(again.stdout, /\n\nLeft new \(--exit-code\): plain xenonet drift or the console marks it seen\.\n$/);
    assert.equal(marks(), marked, "not marked seen");

    const all = JSON.parse((await xenonet(["drift", "--all", "--json", ...port])).stdout);
    assert.deepEqual([all.by, all.all, all.known, all.sources.find((x) => x.source === "openai/codex/rollout-line").drift.length], ["XenoNet Dev", true, 3, 2]);
    assert.deepEqual(s.seen.find((r) => r.path === "/api/drift" && r.query.all).query, { all: "1" });
  } finally {
    s.close();
  }
  // A XenoNet that can't mark it seen: the report is shown anyway, and the
  // command says what went wrong.
  const busy = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/drift": () => [200, { ...driftReport({ log, readers: [], now: DRIFT_LOOKED + 5000 }), file: "~/drift.json" }],
    "POST /api/drift/seen": () => [503, { error: "XenoNet Dev is restarting" }],
  });
  try {
    const out = await xenonet(["drift", "--port", String(busy.port)]);
    assert.deepEqual([out.code, out.stderr], [1, "xenonet: XenoNet Dev is restarting\n"]);
    assert.match(out.stdout, /payload\.cli_version is missing/);
    assert.doesNotMatch(out.stdout, /Marked seen/);
  } finally {
    busy.close();
  }
});

test("xenonet drift without a XenoNet, or with one from before it: drift.json, marked seen there", async () => {
  const cache = tmp();
  const file = path.join(cache, "drift.json");
  fs.writeFileSync(file, JSON.stringify({ version: 1, lookedAt: DRIFT_LOOKED, records: DRIFT }));
  const env = { XENONET_CACHE_DIR: cache };
  // One from before xenonet drift has no route for it.
  const old = await fakeXenoNet({ "GET /api/health": () => [200, health] });
  try {
    const out = await xenonet(["drift", "--port", String(old.port)], { env });
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /^XenoNet isn't running, so this is the drift it kept in .*drift\.json\n\n/);
    assert.match(out.stdout, /changed {2}payload\.cli_version is missing/);
    assert.doesNotMatch(out.stdout, /read {5}/, "nothing was read here");
    assert.doesNotMatch(out.stdout, /Claude desktop's/, "and a source with nothing to say stays out");
    const looked = JSON.parse(fs.readFileSync(file, "utf8")).lookedAt;
    assert.ok(looked > DRIFT_LOOKED, "marked seen in the file");
  } finally {
    old.close();
  }
  const again = await xenonet(["drift", "--exit-code", "--port", String(await closedPort())], { env });
  assert.match(again.stdout, /0 differences new since you last looked .*\n\nNothing new since you last looked\.\n$/);
  assert.equal(again.code, 0, "nothing new: 0, with --exit-code too");
  assert.equal(JSON.parse(fs.readFileSync(file, "utf8")).records.length, 3, "the records stay");
  // New drift in the file: --exit-code says so and leaves it new there, run
  // after run, until plain xenonet drift marks it seen.
  fs.writeFileSync(file, JSON.stringify({ version: 1, lookedAt: DRIFT_LOOKED, records: DRIFT }));
  const unmarked = fs.readFileSync(file, "utf8");
  const port = await closedPort();
  const scripted = () => xenonet(["drift", "--exit-code", "--port", String(port)], { env });
  assert.deepEqual([(await scripted()).code, (await scripted()).code], [1, 1]);
  assert.equal(fs.readFileSync(file, "utf8"), unmarked, "the file is left as it was");
  assert.equal((await xenonet(["drift", "--port", String(port)], { env })).code, 0);
  assert.ok(JSON.parse(fs.readFileSync(file, "utf8")).lookedAt > DRIFT_LOOKED, "plain, it's marked seen");
  assert.equal((await scripted()).code, 0, "and the script exits 0 again");
});

test("shell completion offers each command's own words, xenonet drift's flags among them", async () => {
  const zsh = await xenonet(["completion", "zsh"]);
  assert.match(zsh.stdout, /\n {6}drift\) compadd -- --all --exit-code ;;\n/);
  const bash = await xenonet(["completion", "bash"]);
  assert.match(bash.stdout, /\n {4}drift\) COMPREPLY=\(\$\(compgen -W "--all --exit-code" -- "\$cur"\)\) ;;\n/);
  assert.equal((await xenonet(["completion", "fish"])).code, 2);
});

test("xenonet update: the Tools before the Service, and only what would change is asked about", () => {
  // As crates/xenonet-cli/src/commands/update.rs's own test has it.
  const service = { component: "service", version: "0.5.3", current: "0.5.2", available: true };
  const tools = { component: "tools", version: "0.5.3", current: "0.5.0", available: true };
  const all = inOrder([service, tools], "all");
  assert.deepEqual(all, [tools, service]);
  assert.deepEqual(inOrder([service, tools], "service"), [service]);
  assert.deepEqual(changes(all), ["Tools 0.5.0 → 0.5.3", "Service 0.5.2 → 0.5.3"]);
  const current = { component: "tools", version: "0.5.3", current: "0.5.3", available: false };
  const missing = { component: "service", version: "0.5.3", current: null, available: true };
  assert.deepEqual(changes([current, missing]), ["Service (not installed) → 0.5.3"]);
});

// A stand-in for the service's own socket (core/components/runtime.js):
// it writes the connection file and answers hello, status and stop,
// refusing a stop while terminals run unless it's forced.
async function componentService(env, { terminals = false } = {}) {
  const token = "ab".repeat(32);
  const s = { stops: 0, sockets: new Set() };
  const server = net.createServer((socket) => {
    s.sockets.add(socket);
    socket.on("close", () => s.sockets.delete(socket));
    rpc(socket, async (method, params) => {
      if (method === "hello") {
        if (params.token !== token) throw new Error("XenoNet refused this connection.");
        return { version: "9.9.9", pid: 1, api: SERVICE_API };
      }
      if (method === "stop") {
        if (terminals && !params?.force) throw new Error("XenoNet has running terminals. Close them first, or use --force.");
        s.stops++;
        return { stopping: true };
      }
      return { version: "9.9.9", pid: 1, terminals: [] };
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  fs.mkdirSync(env.XENONET_RUNTIME_DIR, { recursive: true });
  fs.writeFileSync(path.join(env.XENONET_RUNTIME_DIR, `service-${env.XENONET_PORT}.json`), JSON.stringify({ port: server.address().port, token, pid: 1, version: "9.9.9" }));
  s.close = () => new Promise((r) => {
    for (const socket of s.sockets) socket.destroy();
    server.close(r);
  });
  return s;
}

test("xenonet service stop leaves the stop file a supervisor waiting for the port reads, and takes it away when the stop is refused", async () => {
  const env = { XENONET_RUNTIME_DIR: path.join(tmp(), "runtime"), XENONET_PORT: "47998" };
  const file = path.join(env.XENONET_RUNTIME_DIR, "service-47998.stop");
  assert.equal(stopFile(env), file);
  const s = await componentService(env, { terminals: true });
  try {
    const refused = await xenonet(["service", "stop"], { env });
    assert.notEqual(refused.code, 0);
    assert.match(refused.stderr, /running terminals/);
    assert.equal(fs.existsSync(file), false, "a refused stop leaves no stop file");
    const stopped = await xenonet(["service", "stop", "--force"], { env });
    assert.equal(stopped.code, 0, stopped.stderr);
    assert.deepEqual(JSON.parse(stopped.stdout), { stopping: true });
    assert.equal(s.stops, 1);
    assert.equal(fs.existsSync(file), true, "a supervisor waiting for the port stays stopped");
  } finally {
    await s.close();
  }
  // With no service to stop, there's nothing for a supervisor to wait on.
  fs.rmSync(file);
  fs.rmSync(path.join(env.XENONET_RUNTIME_DIR, "service-47998.json"));
  assert.notEqual((await xenonet(["service", "stop"], { env })).code, 0);
  assert.equal(fs.existsSync(file), false);
});

test("xenonet without a XenoNet: status says so, doctor runs here, and mistakes exit 2", async () => {
  const port = ["--port", String(await closedPort())];
  const status = await xenonet(port);
  assert.equal(status.code, 1);
  assert.match(status.stdout, /XenoNet isn't running/);
  const agents = await xenonet(["agents", ...port]);
  assert.equal(agents.code, 1);
  assert.match(agents.stderr, /^xenonet: XenoNet isn't running/);

  const doctor = JSON.parse((await xenonet(["doctor", "--only", "system.runtime,xenonet.server", "--json", ...port])).stdout);
  assert.deepEqual(doctor.results.map((r) => [r.id, r.status]).sort(), [["system.runtime", "ok"], ["xenonet.server", "warn"]]);
  assert.equal(doctor.by, null, "run here, not by a XenoNet");
  // The projects folder it checks is the one you chose, read from
  // state.json without changing the file.
  const stateDir = tmp();
  const chosen = fs.mkdtempSync(path.join(tmp(), "code-"));
  const saved = JSON.stringify({ version: 2, workspace: { root: chosen } });
  fs.writeFileSync(path.join(stateDir, "state.json"), saved);
  const env = { XENONET_STATE_DIR: stateDir, XENONET_PROJECTS_DIR: "" };
  const projects = JSON.parse((await xenonet(["doctor", "--only", "system.projects", "--json", ...port], { env })).stdout);
  assert.deepEqual([projects.results[0].status, projects.results[0].detail], ["ok", `${chosen}, 0 entries`]);
  assert.deepEqual([fs.readdirSync(stateDir), fs.readFileSync(path.join(stateDir, "state.json"), "utf8")], [["state.json"], saved]);

  const wrong = await xenonet(["frobnicate"]);
  assert.deepEqual([wrong.code, wrong.stderr], [2, 'xenonet: no command "frobnicate". xenonet --help lists them.\n']);
  assert.equal((await xenonet(["who"])).code, 2);
  const version = await xenonet(["-v"]);
  assert.equal(version.stdout, `xenonet ${JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version}\n`);
  const help = await xenonet(["--help"]);
  assert.match(help.stdout, /^ {2}doctor {2,}Check sign-ins/m);
  assert.doesNotMatch(help.stdout, /^ {2}(mcp|hook) /m, "what agents run stays out of the list");
});
