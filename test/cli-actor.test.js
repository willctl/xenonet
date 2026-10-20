// xenonet actor (cli/commands/actor.js) and token files
// (core/actors/token-file.js), against a stand-in XenoNet that records what
// reaches it, headers included: listing and showing actors, registering and
// waiting for your Allow, a token written only where it should be and read
// only from a file only you can read, an actor's reports with its token in
// the Authorization header and never on the command line, and removing and
// observing as yours. Made-up actors, paths and tokens.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { otherReaders, protectFile } from "../core/private-file.js";
import { execFileSync } from "node:child_process";
import { parse } from "../cli/lib/args.js";
import { isShell, defaultPid } from "../cli/commands/actor.js";
import { readTokenFile, tokenFromEnv, writeTokenFile, tokenFileRefusal, gitTreeOf } from "../core/actors/token-file.js";
import { command } from "./harness/xenonet-bin.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-actor-cli-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

const TOKEN = `xenonet_actor_${"ab".repeat(32)}`;
const OTHER = `xenonet_actor_${"cd".repeat(32)}`;
const health = { ok: true, name: "XenoNet Dev", version: "9.9.9", demo: false, sources: [] };
const T = Date.now();
const wren = {
  schema: "actor/snapshot/v1",
  id: "actor:wren",
  kind: "agent",
  name: "Wren",
  product: null,
  host: null,
  via: ["registered"],
  presence: "working",
  presenceSince: T - 120e3,
  lastSeen: T - 5e3,
  heartbeatMs: 60000,
  status: "Rebuilding the docs site",
  capabilities: ["run-commands", "edit-repos"],
  targets: [{ id: "project:/users/nova/projects/docs-site", relation: "works-on", since: T - 3600e3, lastAt: T - 60e3 }],
  recent: [{ schema: "actor/event/v1", id: "wren:1", actor: "actor:wren", at: T - 60e3, kind: "ran", target: "project:/users/nova/projects/docs-site", detail: "Ran 2 commands in docs-site", via: "registered" }],
  counts: { events24h: 1, spawned24h: 0, messages24h: 0 },
  identity: null,
};
const placeholder = { ...wren, id: "actor:you", kind: "human", name: "You", via: ["observed"], presence: "unknown", status: null, targets: [], recent: [], heartbeatMs: null, capabilities: [] };

// A stand-in XenoNet: `routes` maps "GET /api/x" to (req) => [status, data].
async function fakeXenoNet(routes) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const url = new URL(req.url, "http://x");
      const r = { method: req.method, path: url.pathname, query: Object.fromEntries(url.searchParams), headers: req.headers, body: body ? JSON.parse(body) : null };
      seen.push(r);
      const route = routes[`${req.method} ${url.pathname}`];
      const [status, data] = route ? route(r) : [404, { error: "not found" }];
      res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => server.close() };
}

function xenonet(args, { env = {}, cwd = root } = {}) {
  return new Promise((resolve) => {
    const proc = spawn(...command(args), { cwd, env: { ...process.env, NO_COLOR: "1", XENONET_PORT: "", XENONET_ACTOR_TOKEN_FILE: "", XENONET_ACTOR_TOKEN: "", XENONET_ACTOR_WAIT_MS: "30", ...env }, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    proc.stdin.end();
    proc.on("exit", (code) => resolve({ code, stdout, stderr, pid: proc.pid }));
  });
}

// A token file only you can read.
function tokenFile(token = TOKEN, mode = 0o600) {
  const file = path.join(tmp(), "wren.token");
  fs.writeFileSync(file, `${token}\n`, { mode });
  if (mode === 0o600) protectFile(file);
  else if (process.platform === "win32") execFileSync("icacls", [file, "/grant", "*S-1-1-0:R", "/q"], { windowsHide: true });
  else if (mode === 0o600) protectFile(file);
  else if (process.platform === "win32") execFileSync("icacls", [file, "/grant", "*S-1-1-0:R", "/q"], { windowsHide: true });
  else fs.chmodSync(file, mode);
  return file;
}

test("the actor command's flags take values, and --add, --remove and --paths may come again", () => {
  const { flags, rest } = parse(["actor", "targets", "--add", "/a=watches", "--add", "claude:x=spawned", "--remove", "/b", "--replace"]);
  assert.deepEqual([rest, flags], [["actor", "targets"], { add: ["/a=watches", "claude:x=spawned"], remove: ["/b"], replace: true }]);
  assert.deepEqual(parse(["actor", "heartbeat", "--status", "", "--presence", "idle", "--token-file=x"]).flags, { status: "", presence: "idle", tokenFile: "x" });
  assert.deepEqual(parse(["actor", "event", "edited", "--paths", "a.ts", "--paths", "b.ts", "--count", "2"]).flags, { paths: ["a.ts", "b.ts"], count: "2" });
});

test("token files: only yours, never inside a git working tree or the projects folder, and read only when others can't", () => {
  const dir = tmp();
  const file = writeTokenFile(path.join(dir, "config", "wren", "xenonet.token"), TOKEN);
  assert.deepEqual(otherReaders(file), []);
  if (process.platform !== "win32") assert.equal(fs.statSync(path.dirname(file)).mode & 0o777, 0o700, "a folder made for it is yours alone too");
  assert.equal(readTokenFile(file), TOKEN);
  assert.match(tokenFileRefusal(file), /is there already; pass --force/);
  fs.chmodSync(file, 0o644);
  assert.equal(writeTokenFile(file, OTHER, { force: true }), file);
  assert.deepEqual(otherReaders(file), []);
  assert.equal(readTokenFile(file), OTHER);

  const repo = path.join(dir, "repo");
  fs.mkdirSync(path.join(repo, ".git"), { recursive: true });
  assert.equal(gitTreeOf(path.join(repo, "deep", "er")), repo);
  assert.match(tokenFileRefusal(path.join(repo, "deep", "xenonet.token")), /inside a git working tree/);
  assert.throws(() => writeTokenFile(path.join(repo, "xenonet.token"), TOKEN), /inside a git working tree/);
  assert.equal(tokenFileRefusal(path.join(repo, "xenonet.token"), { force: true }), null);
  assert.match(tokenFileRefusal(path.join(dir, "Projects", "x", "xenonet.token"), { projectsDir: path.join(dir, "Projects") }), /inside the projects folder/);
  assert.throws(() => writeTokenFile(path.join(dir, "t"), "hunter2"), /isn't an actor's token/);

  const loose = tokenFile(TOKEN, 0o644);
  assert.throws(() => readTokenFile(loose), /other users can read/);
  assert.throws(() => readTokenFile(loose, { platform: "win32", run: () => assert.fail("no icacls here") }), /couldn't tell who else can read/, "on Windows the file's ACL decides, and one XenoNet can't read isn't trusted (test/token-files.test.js has the rest)");
  assert.throws(() => readTokenFile(path.join(dir, "nowhere")), /no token file/);
  assert.throws(() => readTokenFile(tokenFile("not a token")), /doesn't hold an actor's token/);
  assert.deepEqual(tokenFromEnv({ XENONET_ACTOR_TOKEN_FILE: file }), { token: OTHER, file });
  assert.deepEqual(tokenFromEnv({ XENONET_ACTOR_TOKEN: ` ${TOKEN} ` }), { token: TOKEN, file: null });
  assert.throws(() => tokenFromEnv({ XENONET_ACTOR_TOKEN: "x" }), /doesn't hold/);
  assert.equal(tokenFromEnv({}), null);
});

test("xenonet actors and xenonet actor show: the list, one actor and what it did lately", async () => {
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/actors": () => [200, { schema: "actor/list/v1", at: T, actors: [placeholder, wren] }],
    "GET /api/actors/wren": () => [200, wren],
    "GET /api/actors/kite": () => [200, { ...wren, id: "actor:kite", name: "Kite", product: { id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" } }],
    "GET /api/actors/gizmo": () => [200, { ...wren, id: "actor:gizmo", name: "Gizmo", product: { id: "gizmo", vendor: null, version: null } }],
    "GET /api/actors/hermes": () => [200, { ...wren, id: "actor:hermes", name: "Hermes", via: ["observed", "attributed"], product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" } }],
    "GET /api/actors/observers": () => [200, { at: T, observers: [{ product: "hermes-agent", name: "Hermes Agent", on: true, actors: [] }, { product: "grok-bot", name: "Grok Bot", on: false, actors: [] }] }],
  });
  const port = ["--port", String(s.port)];
  try {
    const list = await xenonet(["actors", ...port]);
    assert.equal(list.code, 0, list.stderr);
    assert.match(list.stdout, /^● {2}Wren {2}actor:wren {2}registered {2}working {2}Rebuilding the docs site\n$/);
    assert.doesNotMatch(list.stdout, /You/, "you, a placeholder outside the window, aren't listed");
    assert.deepEqual(JSON.parse((await xenonet(["actor", "list", "--json", ...port])).stdout).map((a) => a.id), ["actor:you", "actor:wren"]);
    const one = await xenonet(["actor", "show", "actor:wren", ...port]);
    assert.match(one.stdout, /^● Wren {2}actor:wren\n/);
    assert.match(one.stdout, /\n {2}Status +Rebuilding the docs site\n/);
    assert.match(one.stdout, /\n {2}Acts on +docs-site \(works-on\)\n/);
    assert.match(one.stdout, /Lately\n {2}\d+ min ago {2}Ran 2 commands in docs-site/);
    assert.doesNotMatch(one.stdout, /Product/, "a custom actor with no product says none");
    // A product XenoNet doesn't know is the actor's own word, quoted and said
    // to be; one it knows goes by its own id and maker.
    assert.match((await xenonet(["actor", "show", "kite", ...port])).stdout, /\n {2}Product +“acme-sync” by “Acme Tools” 2\.1\.0 \(its own words\)\n/);
    assert.match((await xenonet(["actor", "show", "gizmo", ...port])).stdout, /\n {2}Product +“gizmo” \(its own word\)\n/);
    assert.match((await xenonet(["actor", "show", "hermes", ...port])).stdout, /\n {2}Product +hermes-agent by Nous Research 0\.21\.5\n/);
    assert.equal((await xenonet(["actor", "show", ...port])).code, 2);
    assert.match((await xenonet(["actor", "observe", ...port])).stdout, /^on {3}Hermes Agent {2}hermes-agent\noff {2}Grok Bot {6}grok-bot\n$/);
    assert.equal((await xenonet(["actor", "dance", ...port])).code, 2);
  } finally {
    s.close();
  }
});

test("xenonet actor register: asks, waits for your Allow, and writes the token where only you can read it", async () => {
  let asks = 0;
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/workspace/root": () => [200, { path: "/nowhere/Projects" }],
    "POST /api/actors/register": (r) => [202, { schema: "actor/credentials/v1", id: `actor:${r.body.slug ?? r.body.name.toLowerCase()}`, token: r.body.name === "Heron" ? OTHER : TOKEN, state: "waiting", expiresAt: Date.now() + 600e3, ignored: ["fly"] }],
    "GET /api/actors/me": (r) => {
      if (r.headers.authorization === `Bearer ${OTHER}`) return [401, { error: "that registration was denied in XenoNet's window", code: "revoked", reason: "denied" }];
      asks++;
      return [200, { schema: "actor/me/v1", id: "actor:wren", state: asks < 3 ? "waiting" : "allowed", presence: "unknown", heartbeatMs: 30000, unread: 0 }];
    },
  });
  const port = ["--port", String(s.port)];
  try {
    const file = path.join(tmp(), "config", "wren.token");
    const out = await xenonet(["actor", "register", "--name", "Wren", "--capabilities", "run-commands,edit-repos,fly", "--heartbeat", "30", "--token-file", file, ...port]);
    assert.equal(out.code, 0, out.stderr);
    assert.match(out.stdout, /^Waiting for you to allow "Wren" in XenoNet Dev's window \(it asks for 10 minutes\)…\n✓ actor:wren is allowed\. Its token is in .*wren\.token, readable only by you\.\nXenoNet doesn't know these capabilities, so it left them out: fly\n$/);
    assert.ok(!out.stdout.includes(TOKEN), "a token written to a file isn't printed");
    assert.equal(fs.readFileSync(file, "utf8"), `${TOKEN}\n`);
    assert.deepEqual(otherReaders(file), []);
    const asked = s.seen.find((r) => r.path === "/api/actors/register");
    assert.deepEqual(asked.body, { schema: "actor/registration/v1", name: "Wren", capabilities: ["run-commands", "edit-repos", "fly"], heartbeatMs: 30000 });
    assert.equal(asked.headers.authorization, undefined, "asking takes no token");
    assert.ok(s.seen.filter((r) => r.path === "/api/actors/me").every((r) => r.headers.authorization === `Bearer ${TOKEN}`));

    const printed = await xenonet(["actor", "register", "--name", "Wren", ...port]);
    assert.match(printed.stdout, new RegExp(`Its token, shown once:\\n${TOKEN}\\n`), "without a file, it's printed once");

    const denied = await xenonet(["actor", "register", "--name", "Heron", ...port]);
    assert.deepEqual([denied.code, denied.stderr], [1, "xenonet: actor:heron wasn't allowed: that registration was denied in XenoNet's window.\n"]);

    const repo = tmp();
    fs.mkdirSync(path.join(repo, ".git"));
    const before = s.seen.length;
    const inRepo = await xenonet(["actor", "register", "--name", "Kite", "--token-file", path.join(repo, "kite.token"), ...port]);
    assert.equal(inRepo.code, 1);
    assert.match(inRepo.stderr, /inside a git working tree/);
    assert.ok(!s.seen.slice(before).some((r) => r.path === "/api/actors/register"), "refused before asking, so no token is made for nothing");
    assert.equal((await xenonet(["actor", "register", ...port])).code, 2, "a name is needed");
    assert.equal((await xenonet(["actor", "register", "--name", "Kite", "--heartbeat", "soon", ...port])).code, 2);
  } finally {
    s.close();
  }
});

test("an actor's reports carry its token as Authorization: Bearer, from a file only it can read", async () => {
  const me = (state = "allowed", extra = {}) => ({ schema: "actor/me/v1", id: "actor:wren", state, presence: "working", heartbeatMs: 60000, unread: 2, ...extra });
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "POST /api/actors/heartbeat": () => [200, me()],
    "GET /api/actors/me": () => [200, me()],
    "POST /api/actors/events": (r) => [200, me("allowed", r.body.events[0].kind === "said" ? { accepted: 0, dropped: [{ index: 0, reason: "XenoNet doesn't take that kind in a report" }] } : { accepted: 1, dropped: [] })],
    "POST /api/actors/targets": () => [200, me("allowed", { accepted: 2, dropped: [{ index: 1, reason: "spawned needs the spawn-sessions capability, which it wasn't allowed" }] })],
    "POST /api/engine/inbox": () => [200, [{ id: 7, from: { id: "claude:aaaaaaaa-0001", title: "Add partial refunds", app: "claude", project: "atlas-api", branch: null }, to: "you", text: "OK, staying out of dist/.", at: Date.now() - 60e3, replyTo: null }]],
    "POST /api/engine/send": () => [200, { id: 9, to: [{ id: "claude:aaaaaaaa-0001", title: "Add partial refunds", app: "claude", project: "atlas-api", branch: null, delivery: "during its current turn" }], alsoTo: "sessions that start work in this repo in the next half hour" }],
    "DELETE /api/actors/me": () => [200, { removed: "actor:wren" }],
  });
  const port = ["--port", String(s.port)];
  const env = { XENONET_ACTOR_TOKEN_FILE: tokenFile() };
  const cwd = tmp();
  try {
    const beat = await xenonet(["actor", "heartbeat", "--presence", "working", "--status", "Rebuilding the docs site", "--instance", "cli", ...port], { env, cwd });
    assert.equal(beat.code, 0, beat.stderr);
    assert.equal(beat.stdout, "● actor:wren is working; 2 messages wait (xenonet actor inbox)\n");
    const hb = s.seen.find((r) => r.path === "/api/actors/heartbeat");
    assert.equal(hb.headers.authorization, `Bearer ${TOKEN}`);
    assert.equal(hb.headers["x-xenonet-agent"], undefined, "never both");
    // Its parent, the process that ran the command, isn't a shell; where ps
    // can't say what a process runs (Windows), it names none.
    const parent = process.platform === "win32" ? {} : { pid: process.pid };
    assert.deepEqual(hb.body, { schema: "actor/report/v1", presence: "working", status: "Rebuilding the docs site", instance: "cli", cwd, ...parent }, "its folder, and its parent as its process");
    await xenonet(["actor", "beat", "--cwd", "/somewhere/else", "--pid", "4242", ...port], { env, cwd });
    assert.deepEqual([s.seen.at(-1).body.cwd, s.seen.at(-1).body.pid], [path.resolve(cwd, "/somewhere/else"), 4242]);
    assert.equal((await xenonet(["actor", "heartbeat", "--presence", "away", ...port], { env })).code, 2, "only XenoNet decides away");

    assert.match((await xenonet(["actor", "status", ...port], { env })).stdout, /^● actor:wren is allowed, and working; 2 messages wait/);
    const ran = await xenonet(["actor", "event", "ran", "--path", "docs-site", "--count", "3", "--ref", "run-118", ...port], { env, cwd });
    assert.equal(ran.stdout, "✓ ran\n");
    assert.deepEqual(s.seen.at(-1).body, { schema: "actor/report/v1", events: [{ kind: "ran", path: path.join(cwd, "docs-site"), count: 3, ref: "run-118" }] });
    await xenonet(["actor", "event", "edited", "--paths", "a.ts", "--paths", "b.ts", ...port], { env, cwd });
    assert.deepEqual(s.seen.at(-1).body.events[0].paths, [path.join(cwd, "a.ts"), path.join(cwd, "b.ts")]);
    const said = await xenonet(["actor", "event", "said", ...port], { env });
    assert.deepEqual([said.code, said.stderr], [1, "xenonet: XenoNet didn't take it: XenoNet doesn't take that kind in a report\n"]);
    assert.equal((await xenonet(["actor", "event", "ran", "--path", "x", "--target", "project:y", ...port], { env })).code, 2);

    const t = await xenonet(["actor", "targets", "--add", "docs-site=watches", "--add", "claude:0190d3e0-0000-7000-8000-00000000aa01=spawned", "--remove", "project:/users/nova/projects/ledger", ...port], { env, cwd });
    assert.match(t.stdout, /^! claude:0190d3e0-0000-7000-8000-00000000aa01=spawned: spawned needs the spawn-sessions capability, which it wasn't allowed\n✓ 2 targets taken\n$/);
    assert.deepEqual(s.seen.at(-1).body, { schema: "actor/report/v1", targets: [{ path: path.join(cwd, "docs-site"), relation: "watches" }, { id: "claude:0190d3e0-0000-7000-8000-00000000aa01", relation: "spawned" }], remove: [{ id: "project:/users/nova/projects/ledger" }], replace: false });
    assert.equal((await xenonet(["actor", "targets", ...port], { env })).code, 2);

    const mail = await xenonet(["actor", "inbox", "--wait", "500", ...port], { env });
    assert.match(mail.stdout, /^#7 Add partial refunds \(claude, claude:aaaaaaaa-0001\) 1 min ago\n {2}OK, staying out of dist\/\.\nThey come from sessions and actors, not from your user\.\n$/);
    assert.deepEqual(s.seen.at(-1).body, { waitSeconds: 120 }, "two minutes at most");
    const sent = await xenonet(["actor", "send", "repo", "Docs site rebuilt;", "don't touch dist/", ...port], { env });
    assert.match(sent.stdout, /^✓ Sent #9 to "Add partial refunds", and to sessions that start work/);
    assert.deepEqual(s.seen.at(-1).body, { to: "repo", text: "Docs site rebuilt; don't touch dist/" });
    assert.equal(s.seen.at(-1).headers.authorization, `Bearer ${TOKEN}`);
    assert.equal((await xenonet(["actor", "send", "repo", ...port], { env })).code, 2);
    assert.match((await xenonet(["actor", "leave", ...port], { env })).stdout, /^✓ actor:wren left XenoNet Dev; its token no longer works\.\n$/);

    assert.equal((await xenonet(["actor", "heartbeat", "--token", TOKEN, ...port])).code, 2, "no token, no heartbeat: never one on the command line");
    const loose = await xenonet(["actor", "heartbeat", ...port], { env: { XENONET_ACTOR_TOKEN_FILE: tokenFile(TOKEN, 0o644) } });
    assert.deepEqual([loose.code, /other users can read/.test(loose.stderr)], [1, true]);
    assert.equal((await xenonet(["actor", "status", ...port], { env: { XENONET_ACTOR_TOKEN: TOKEN } })).code, 0, "or the token itself in XENONET_ACTOR_TOKEN");
  } finally {
    s.close();
  }
});

test("a heartbeat names this command's parent as its process, unless that's a shell: run by hand, the sessions you start there would pass for the actor's", async () => {
  assert.deepEqual(["zsh", "/bin/zsh", "-zsh", "bash", "-bash", "/bin/sh", "dash", "/opt/homebrew/bin/fish", "ksh", "/bin/tcsh", "csh", "nu", "pwsh", "pwsh.exe"].filter((p) => !isShell(p)), []);
  assert.deepEqual(["/usr/local/bin/node", "python3", `${os.homedir()}/bin/wren`, "zshell-helper", "bashful", "-", ""].filter(isShell), []);
  assert.equal(defaultPid({ ppid: 4242, program: () => "/usr/local/bin/node" }), 4242, "a program that runs it, which goes offline when it ends");
  assert.equal(defaultPid({ ppid: 4242, program: () => "-zsh" }), null, "your login shell, run by hand");
  assert.equal(defaultPid({ ppid: 4242, program: () => "/bin/bash" }), null);
  assert.equal(defaultPid({ ppid: 4242, program: () => null }), null, "where what it runs can't be told, none");
  assert.equal(defaultPid({ ppid: 1, program: () => "/sbin/launchd" }), null);
});

test("run from a shell, a heartbeat names no process; with --pid, the one it names", { skip: process.platform === "win32" }, async () => {
  const me = { schema: "actor/me/v1", id: "actor:wren", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 0 };
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "POST /api/actors/heartbeat": () => [200, me] });
  const env = { ...process.env, NO_COLOR: "1", XENONET_PORT: "", XENONET_ACTOR_TOKEN: "", XENONET_ACTOR_TOKEN_FILE: tokenFile() };
  // The shell waits for the command, as yours does, rather than becoming it.
  const viaShell = (args) =>
    new Promise((resolve) => {
      const quote = (a) => `'${String(a).replace(/'/g, "'\\''")}'`;
      const line = command([...args, "--port", s.port]).flat().map(quote).join(" ");
      const proc = spawn("/bin/sh", ["-c", `${line}; exit $?`], { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
      let stderr = "";
      proc.stderr.on("data", (d) => (stderr += d));
      proc.on("exit", (code) => resolve({ code, stderr }));
    });
  try {
    const plain = await viaShell(["actor", "heartbeat"]);
    assert.equal(plain.code, 0, plain.stderr);
    assert.equal("pid" in s.seen.at(-1).body, false, "no process named");
    await viaShell(["actor", "heartbeat", "--pid", "4242"]);
    assert.equal(s.seen.at(-1).body.pid, 4242);
  } finally {
    s.close();
  }
});

test("xenonet actor remove and observe: yours, with no token sent, and remove asks first", async () => {
  const hermes = { ...wren, id: "actor:hermes", name: "Hermes", product: { id: "hermes-agent", vendor: "Nous Research", version: null }, via: ["observed", "attributed", "registered"] };
  const bot = { ...wren, id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: "0.70.4" }, via: ["observed", "attributed"] };
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/actors/wren": () => [200, wren],
    "GET /api/actors/hermes": () => [200, hermes],
    "GET /api/actors/grok-bot": () => [200, bot],
    "DELETE /api/actors/wren": () => [200, { removed: "actor:wren" }],
    "DELETE /api/actors/hermes": () => [200, { removed: "actor:hermes", observed: true }],
    "POST /api/actors/observers": (r) => [200, { product: r.body.product, on: r.body.on, actors: [] }],
  });
  const port = ["--port", String(s.port)];
  const env = { XENONET_ACTOR_TOKEN_FILE: tokenFile() };
  try {
    const asked = await xenonet(["actor", "remove", "wren", ...port], { env });
    assert.equal(asked.code, 1);
    assert.match(asked.stderr, /Remove actor:wren from XenoNet Dev\? Its token stops working and its record goes\./);
    assert.match(asked.stderr, /Run it again with --yes/);
    assert.match((await xenonet(["actor", "remove", "hermes", ...port])).stderr, /Its token stops working, and XenoNet Dev keeps observing it\./);
    assert.ok(!s.seen.some((r) => r.method === "DELETE"));
    const gone = await xenonet(["actor", "remove", "actor:wren", "--yes", ...port], { env });
    assert.equal(gone.stdout, "✓ Removed actor:wren.\n");
    assert.equal(s.seen.at(-1).headers.authorization, undefined, "yours, so never an actor's token");
    assert.equal((await xenonet(["actor", "remove", "hermes", "--yes", ...port])).stdout, "✓ Removed actor:hermes; XenoNet Dev still observes it.\n");
    assert.equal((await xenonet(["actor", "remove", ...port])).code, 2);
    const observed = await xenonet(["actor", "remove", "grok-bot", "--yes", ...port]);
    assert.deepEqual([observed.code, observed.stderr], [1, "xenonet: XenoNet Dev only observes Grok Bot, so it can't be removed. Turn off its observer instead: xenonet actor observe grok-bot off, or its switch in XenoNet Dev's Actors panel.\n"]);
    assert.ok(!s.seen.some((r) => r.method === "DELETE" && r.path === "/api/actors/grok-bot"), "and asks nothing of XenoNet but what it is");
    const you = await xenonet(["actor", "remove", "actor:you", "--yes", ...port]);
    assert.deepEqual([you.code, you.stderr], [1, "xenonet: You can't be removed: turning off Show me on the map, in XenoNet's Actors panel, deletes your record.\n"]);
    assert.ok(!s.seen.some((r) => r.path === "/api/actors/you"), "nothing asked of XenoNet");
    const off = await xenonet(["actor", "observe", "grok-bot", "off", ...port], { env });
    assert.equal(off.stdout, "✓ XenoNet Dev no longer observes grok-bot.\n");
    assert.deepEqual([s.seen.at(-1).body, s.seen.at(-1).headers.authorization], [{ product: "grok-bot", on: false }, undefined]);
    assert.equal((await xenonet(["actor", "observe", "grok-bot", "maybe", ...port])).code, 2);
  } finally {
    s.close();
  }
});

test("xenonet agents marks the files XenoNet saw an actor edit, which it inferred rather than heard of", async () => {
  const hermes = { id: "actor:hermes", app: "actor", title: "Hermes", project: "atlas-api", branch: "main", status: "working", needsYou: null, focus: "Editing files in atlas-api", editing: [{ path: "/Users/nova/Projects/atlas-api/src/app.ts", at: T, observed: true }], claims: [], lastActive: T, reachable: false, top: "/Users/nova/Projects/atlas-api", cwd: "/Users/nova/Projects/atlas-api" };
  const moth = { ...hermes, id: "actor:moth", title: "Moth", editing: [{ path: "/Users/nova/Projects/atlas-api/README.md", at: T }], reachable: true };
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "GET /api/engine/agents": () => [200, [hermes, moth]] });
  try {
    const out = await xenonet(["agents", "--port", String(s.port)]);
    assert.match(out.stdout, /^Actors\n● Hermes {2}atlas-api · main\n {2}says: Editing files in atlas-api · editing app\.ts \(observed\)\n/);
    assert.match(out.stdout, /● Moth {2}atlas-api · main\n {2}says: Editing files in atlas-api · editing README\.md\n/);
  } finally {
    s.close();
  }
});

// A stand-in XenoNet with an actor's stream: it tells of one message waiting,
// whose inbox answers it once.
async function streamingXenoNet() {
  const seen = [];
  let taken = false;
  const message = { id: 7, from: { id: "claude:abc", title: "Fix the build", app: "claude" }, to: "you", text: "the build is green", at: T, replyTo: null, envelope: null };
  const server = http.createServer((req, res) => {
    seen.push({ method: req.method, path: req.url, authorization: req.headers.authorization });
    if (req.url === "/api/health") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(health));
    if (req.url === "/api/actors/me/stream") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      return res.write(`retry: 5000\n\n: still here\n\nevent: inbox\ndata: {"unread":1}\n\n`);
    }
    if (req.url === "/api/engine/inbox") {
      const list = taken ? [] : [message];
      taken = true;
      return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(list));
    }
    res.writeHead(404).end("{}");
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => (server.closeAllConnections(), server.close()) };
}

// Runs xenonet until `done(stdout)` says it has what it needs, then stops it.
function xenonetUntil(args, done, { env = {} } = {}) {
  return new Promise((resolve) => {
    const proc = spawn(...command(args), { cwd: root, env: { ...process.env, NO_COLOR: "1", XENONET_PORT: "", XENONET_ACTOR_TOKEN_FILE: "", XENONET_ACTOR_TOKEN: "", ...env }, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    const check = () => done(stdout, stderr) && proc.kill();
    proc.stdout.on("data", (d) => ((stdout += d), check()));
    proc.stderr.on("data", (d) => ((stderr += d), check()));
    const timer = setTimeout(() => proc.kill(), 15000);
    proc.on("exit", () => (clearTimeout(timer), resolve({ stdout, stderr })));
  });
}

test("xenonet actor listen hears of a message over the actor's stream and reads it at once, with its token in the header", async () => {
  const s = await streamingXenoNet();
  try {
    const out = await xenonetUntil(["actor", "listen", "--json", "--port", String(s.port)], (o) => o.includes("\n"), { env: { XENONET_ACTOR_TOKEN: TOKEN } });
    assert.deepEqual(JSON.parse(out.stdout.split("\n")[0]), { id: 7, from: { id: "claude:abc", title: "Fix the build", app: "claude" }, to: "you", text: "the build is green", at: T, replyTo: null, envelope: null });
    const stream = s.seen.find((r) => r.path === "/api/actors/me/stream");
    assert.equal(stream.authorization, `Bearer ${TOKEN}`, "its token in the header, never the URL");
    assert.ok(s.seen.some((r) => r.method === "POST" && r.path === "/api/engine/inbox"));
  } finally {
    s.close();
  }
});

test("xenonet actor listen --exec gives each message to the program on its standard input, without the actor's token", { skip: process.platform === "win32" ? "a shell script stands in for the program" : false }, async () => {
  const s = await streamingXenoNet();
  const dir = tmp();
  const got = path.join(dir, "got.json");
  const program = path.join(dir, "on-message.sh");
  fs.writeFileSync(program, `#!/bin/sh\ncat > "${got}.part"\nprintf '%s|%s|%s' "$XENONET_MESSAGE_ID" "$XENONET_MESSAGE_FROM" "\${XENONET_ACTOR_TOKEN:-none}" > "${got}.env"\nmv "${got}.part" "${got}"\n`, { mode: 0o755 });
  try {
    await xenonetUntil(["actor", "listen", "--exec", program, "--port", String(s.port)], () => fs.existsSync(got), { env: { XENONET_ACTOR_TOKEN: TOKEN } });
    assert.equal(JSON.parse(fs.readFileSync(got, "utf8")).text, "the build is green");
    assert.equal(fs.readFileSync(`${got}.env`, "utf8"), "7|claude:abc|none");
  } finally {
    s.close();
  }
  const missing = await xenonet(["actor", "listen", "--exec", path.join(dir, "nope"), "--port", "1"], { env: { XENONET_ACTOR_TOKEN: TOKEN } });
  assert.equal(missing.code, 2);
});

test("xenonet actor wake: how one is woken, a test of its webhook, and off; yours, never with a token", async () => {
  const at = Date.now() - 120e3;
  const wake = { actor: "actor:kite", kind: "webhook", set: true, host: "hooks.example.com", signed: false, listening: 1, lastAt: at, last: { ok: true, at, status: 204, why: null }, failures: 0 };
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/actors/wakes": () => [200, { at: T, wakes: [wake] }],
    "POST /api/actors/kite/wake/test": () => [200, { ok: false, status: 401, why: "hooks.example.com answered HTTP 401", wake }],
    "DELETE /api/actors/kite/wake": () => [200, { ...wake, kind: "none", host: null }],
  });
  const port = ["--port", String(s.port)];
  try {
    const shown = await xenonet(["actor", "wake", "kite", ...port]);
    assert.match(shown.stdout, /^actor:kite is woken by a webhook to hooks\.example\.com, with its key \(kept in your credential store\)\n {2}● a program of its own is listening, so messages reach it at once\n {2}✓ last wake 2 min ago: answered HTTP 204\n$/);
    const tested = await xenonet(["actor", "wake", "kite", "test", ...port]);
    assert.equal(tested.stdout, "✗ hooks.example.com answered HTTP 401\n");
    assert.equal((await xenonet(["actor", "wake", "kite", "off", ...port])).stdout, "✓ actor:kite is woken no more; it reads its inbox when it checks\n");
    assert.ok(s.seen.filter((r) => r.path.includes("/wake")).every((r) => r.headers.authorization === undefined));
    assert.equal((await xenonet(["actor", "wake", ...port])).code, 2);
    assert.equal((await xenonet(["actor", "wake", "kite", "now", ...port])).code, 2);
  } finally {
    s.close();
  }
});
