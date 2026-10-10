// Tunnels XenoNet runs for remote actors' connections (core/assistant/
// tunnels.js, tunnel-client.js): finding tunnel-client, recognizing the
// profiles you made for it, the profile XenoNet writes, keeping the key in
// the credential store and only in the client's environment, running it
// and reading its health, starting again after it exits and giving up
// after a few, and stopping when its grant goes. tunnel-client, the
// store, the clock and the health checks are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { world, ask, T } from "./fixtures/assistant.js";
import { createTunnels, keyTargetFor, TunnelError } from "../core/assistant/tunnels.js";
import { findClient, clientPlaces, profileDirs, readProfile, findProfiles, commandLine, adapterArgv, profileYaml, logLine, healthOf } from "../core/assistant/tunnel-client.js";
import { createSecretStore } from "../core/secret-store.js";

const TUNNEL = "tunnel_0123456789abcdef0123456789abcdef";
const KEY = "sk-proj-CANARYCANARYCANARY0123456789";

test("tunnel-client is found from XENONET_TUNNEL_CLIENT, PATH, then where its installs put it", () => {
  const win = { LOCALAPPDATA: "C:\\Users\\nova\\AppData\\Local", PATH: "", PATHEXT: ".EXE" };
  const at = path.win32.join(win.LOCALAPPDATA, "Programs", "tunnel-client", "tunnel-client.exe");
  assert.equal(clientPlaces({ env: win, platform: "win32" })[0], at);
  assert.equal(findClient({ env: win, platform: "win32", exists: (f) => f === at }), at);
  assert.equal(findClient({ env: { ...win, XENONET_TUNNEL_CLIENT: "D:\\tc.exe" }, platform: "win32", exists: () => true }), "D:\\tc.exe");
  assert.equal(findClient({ env: win, platform: "win32", exists: () => false }), null);
  const mac = clientPlaces({ env: { HOME: "/Users/nova" }, platform: "darwin" });
  assert.ok(mac.includes("/opt/homebrew/bin/tunnel-client") && mac.includes("/Users/nova/.local/bin/tunnel-client"));
  assert.deepEqual(profileDirs({ env: { APPDATA: "C:\\Users\\nova\\AppData\\Roaming" }, platform: "win32", home: "C:\\Users\\nova" })[0], path.win32.join("C:\\Users\\nova\\AppData\\Roaming", "tunnel-client"));
  assert.deepEqual(profileDirs({ env: { TUNNEL_CLIENT_PROFILE_DIR: "/x" } }), ["/x"]);
});

test("a profile of your own that runs xenonet mcp assistant is recognized, with its tunnel and grant", () => {
  const yours = ['config_version: 1', "control_plane:", '  tunnel_id: "tunnel_5f0e1d2c3b4a59687766554433221100"', '  api_key: "env:CONTROL_PLANE_API_KEY"', "health:", '  listen_addr: "127.0.0.1:8790"', "mcp:", "  commands:", "    - channel: main", '      command: "C:/XenoNet/node.exe C:/XenoNet/cli/xenonet.js mcp assistant --grant gr_0a1b2c3d4e5f60718293a4b5"'].join("\n");
  assert.deepEqual(readProfile(yours), { tunnelId: "tunnel_5f0e1d2c3b4a59687766554433221100", health: "127.0.0.1:8790", grant: "gr_0a1b2c3d4e5f60718293a4b5", xenonet: true });
  assert.equal(readProfile("mcp:\n  commands:\n    - command: other-server --stdio\n").xenonet, false);
  assert.equal(readProfile("control_plane:\n  tunnel_id: not-a-tunnel\n").tunnelId, null);
  const files = { "/p/xenonet.yaml": yours, "/p/other.yml": "mcp:\n  server_url: http://127.0.0.1:3001/mcp\n", "/p/notes.txt": yours };
  const found = findProfiles(["/p", "/missing"], { list: (d) => (d === "/p" ? ["xenonet.yaml", "other.yml", "notes.txt"] : (() => { throw new Error("ENOENT"); })()), read: (f) => files[f.replace(/\\/g, "/")], stat: () => ({ mtimeMs: 1 }) });
  assert.deepEqual(found.map((p) => [p.name, p.grant]), [["xenonet", "gr_0a1b2c3d4e5f60718293a4b5"]]);
});

test("the profile XenoNet writes: its tunnel, the key from the environment only, and the launcher's command in forward slashes", () => {
  const argv = adapterArgv({ exec: "C:\\Users\\nova\\AppData\\Local\\XenoNet\\runtime\\launcher\\node.exe", entry: "C:\\Program Files\\XenoNet\\cli\\xenonet.js", grant: "gr_0a1b2c3d4e5f60718293a4b5", port: 47842 });
  const line = commandLine(argv);
  assert.equal(line, 'C:/Users/nova/AppData/Local/XenoNet/runtime/launcher/node.exe "C:/Program Files/XenoNet/cli/xenonet.js" mcp assistant --grant gr_0a1b2c3d4e5f60718293a4b5 --port 47842');
  assert.ok(!commandLine(adapterArgv({ exec: "/n", entry: "/s.js", grant: "gr_0a1b2c3d4e5f60718293a4b5" })).includes("--port"), "the default port needs no flag");
  assert.throws(() => adapterArgv({ exec: "/n", entry: "/s.js", grant: "gr_x; rm -rf /" }));
  const yaml = profileYaml({ tunnelId: TUNNEL, command: line, label: "My dot" });
  assert.match(yaml, /tunnel_id: 'tunnel_0123456789abcdef0123456789abcdef'/);
  assert.match(yaml, /api_key: "env:CONTROL_PLANE_API_KEY"/);
  assert.match(yaml, /listen_addr: "127\.0\.0\.1:0"/);
  assert.deepEqual(readProfile(yaml), { tunnelId: TUNNEL, health: "127.0.0.1:0", grant: "gr_0a1b2c3d4e5f60718293a4b5", xenonet: true }, "and XenoNet reads its own back");
  assert.throws(() => profileYaml({ tunnelId: "tunnel_x'\nmcp:", command: line }), /not a tunnel id/);
});

test("its log lines and health, as XenoNet reads them: keys taken out, ready only when it's polling and its adapter runs", () => {
  assert.deepEqual(logLine(JSON.stringify({ level: "ERROR", msg: "control plane said 401", error: `bad key ${KEY}` })), { level: "error", msg: "control plane said 401: bad key sk-…" });
  assert.equal(logLine("not json"), null);
  assert.equal(healthOf(null).state, "starting");
  const ok = { live: true, ready: true, runtime: { version: "0.0.16+abc" }, components: { "control-plane": { status: "ok", state: "polling", details: { last_success: "2026-10-07T17:33:40Z" } }, mcp: { status: "unknown", details: { child_state: "running" } } } };
  assert.deepEqual(healthOf(ok), { state: "ready", why: null, version: "0.0.16", polledAt: "2026-10-07T17:33:40Z" });
  assert.equal(healthOf({ ...ok, components: { ...ok.components, "control-plane": { status: "error", state: "backoff", details: { consecutive_failures: 3 } } } }).why, "OpenAI's control plane: backoff, 3 failures in a row");
  assert.equal(healthOf({ ...ok, components: { ...ok.components, mcp: { details: { child_state: "exited" } } } }).state, "degraded");
});

test("the credential store takes a secret on standard input only, never on a command line", async () => {
  const calls = [];
  const run = async (file, args, { input = "" } = {}) => (calls.push({ file, args, input }), { code: 0, stdout: "" });
  await createSecretStore({ platform: "win32", run }).set("XenoNet/OpenAI tunnel/x", KEY);
  await createSecretStore({ platform: "darwin", run }).set("XenoNet/OpenAI tunnel/x", KEY);
  for (const c of calls) {
    assert.ok(!c.args.join(" ").includes(KEY), `${c.file}: not in its arguments`);
    assert.ok(c.input.includes(KEY), `${c.file}: on its input`);
  }
  await assert.rejects(createSecretStore({ platform: "win32", run }).set("bad\"name", KEY), /printable text/);
  await assert.rejects(createSecretStore({ platform: "win32", run }).set("XenoNet/x", 'sk-"; evil'), /isn't a secret/);
  assert.equal(await createSecretStore({ platform: "win32", run: async () => ({ code: 3, stdout: "" }) }).get("XenoNet/x"), null);
});

// A made-up tunnel-client: spawn writes its health address, and the test
// says when it exits; health answers what `health` holds.
function fakeClient() {
  const procs = [];
  const timers = [];
  let health = null;
  // A tunnel-client of your own, answering on its fixed health address.
  const yours = { up: false };
  const spawn = (file, args, opts) => {
    const p = new EventEmitter();
    p.pid = 1000 + procs.length;
    p.stdout = new PassThrough();
    p.stderr = new PassThrough();
    p.file = file;
    p.args = args;
    p.env = opts.env;
    p.profile = fs.readFileSync(args[args.indexOf("--profile-file") + 1], "utf8");
    fs.writeFileSync(args[args.indexOf("--health.url-file") + 1], "http://127.0.0.1:51234\n");
    procs.push(p);
    return p;
  };
  const fetch = async (url) => {
    if (url === "http://127.0.0.1:8790/readyz" && yours.up) return { ok: true, text: async () => "ready" };
    if (!url.startsWith("http://127.0.0.1:51234/health") || !health) throw new Error("ECONNREFUSED");
    return { ok: true, text: async () => JSON.stringify(health) };
  };
  return {
    procs,
    yours,
    spawn,
    fetch,
    setHealth: (h) => (health = h),
    setTimer: (fn, ms) => (timers.push({ fn, ms }), timers.length),
    clearTimer: (id) => id && timers[id - 1] && (timers[id - 1].fn = null),
    // Runs the timers due, as time passing would.
    async tick() {
      const due = timers.splice(0).filter((t) => t.fn);
      for (const t of due) await t.fn();
    },
    killed: [],
  };
}
const READY = { live: true, ready: true, runtime: { version: "0.0.16+abc" }, components: { "control-plane": { status: "ok", state: "polling", details: { last_success: "2026-10-07T17:33:40Z" } }, mcp: { details: { child_state: "running" } } } };

function rig(w, { found = "C:/tc/tunnel-client.exe", profiles = [], kept = new Map() } = {}) {
  const fake = fakeClient();
  const secrets = { kind: "a made-up store", available: () => true, has: async (t) => kept.has(t), get: async (t) => kept.get(t) ?? null, set: async (t, s) => kept.set(t, s), remove: async (t) => kept.delete(t) };
  const logs = [];
  const tunnels = createTunnels({
    connections: w.connections,
    store: w.store,
    dir: w.dir,
    secrets,
    home: "C:\\Users\\nova",
    env: { PATH: "", OPENAI_API_KEY: "sk-should-not-pass-through-0000000000" },
    launch: { exec: "C:\\Users\\nova\\AppData\\Local\\XenoNet\\runtime\\launcher\\node.exe", entry: "C:\\Users\\nova\\AppData\\Local\\XenoNet\\runtime\\launcher\\cli\\xenonet.js", runAsNode: false, port: 47840 },
    find: () => found,
    version: async () => "0.0.16",
    profiles: () => profiles,
    spawn: fake.spawn,
    fetch: fake.fetch,
    now: w.clock.now,
    log: (l) => logs.push(l),
    killTree: (child) => (fake.killed.push(child.pid), setImmediate(() => child.emit("exit", null))),
    setTimer: fake.setTimer,
    clearTimer: fake.clearTimer,
  });
  return { tunnels, fake, kept, logs };
}
const pairIn = (w, over = {}) => w.connections.pair(ask(over), { places: w.places, identities: w.identities });
const settle = () => new Promise((r) => setImmediate(r));

test("set up and run: its tunnel and key kept, the key only in tunnel-client's environment, ready once its health says so", async () => {
  const w = world();
  try {
    const dot = pairIn(w);
    const { tunnels, fake, kept, logs } = rig(w);
    let s = await tunnels.status();
    assert.deepEqual(s.client, { found: true, path: "C:/tc/tunnel-client.exe", version: "0.0.16" });
    assert.deepEqual([s.tunnels[0].grant, s.tunnels[0].tunnel_id, s.tunnels[0].key, s.tunnels[0].state, s.tunnels[0].autostart], [dot.grant_id, null, "missing", "off", false]);

    await assert.rejects(tunnels.start(dot.grant_id), (e) => e instanceof TunnelError && e.code === "no-tunnel");
    await assert.rejects(tunnels.storeKey(dot.grant_id, KEY), /which tunnel/);
    assert.throws(() => tunnels.configure(dot.grant_id, { tunnelId: "tunnel_nope" }), /starts tunnel_/);
    assert.deepEqual(tunnels.configure(dot.grant_id, { tunnelId: TUNNEL }), { grant: dot.grant_id, tunnel_id: TUNNEL, key_target: keyTargetFor(TUNNEL), autostart: false });
    assert.ok(!w.store.connection(dot.connection_id).transport_binding.includes("sk-"), "the binding holds no key");
    await assert.rejects(tunnels.start(dot.grant_id), (e) => e.code === "no-key");
    await assert.rejects(tunnels.storeKey(dot.grant_id, "hunter2"), /isn't an OpenAI API key/);
    assert.deepEqual(await tunnels.storeKey(dot.grant_id, ` ${KEY} `), { grant: dot.grant_id, key: "stored", target: keyTargetFor(TUNNEL) });
    assert.equal(kept.get(keyTargetFor(TUNNEL)), KEY, "in the store, trimmed");

    const started = await tunnels.start(dot.grant_id);
    assert.equal(started.state, "starting");
    const p = fake.procs[0];
    assert.deepEqual(p.args.slice(0, 2), ["run", "--profile-file"]);
    assert.equal(p.env.CONTROL_PLANE_API_KEY, KEY, "the key in its environment");
    assert.equal(p.env.OPENAI_API_KEY, undefined, "and no other key passed on");
    assert.ok(!p.args.join(" ").includes(KEY) && !p.profile.includes(KEY), "never on its command line or in its profile");
    assert.match(p.profile, new RegExp(`--grant ${dot.grant_id}`));
    assert.match(p.profile, /C:\/Users\/nova\/AppData\/Local\/XenoNet\/runtime\/launcher\/node\.exe/);
    assert.equal((await tunnels.start(dot.grant_id)).pid, p.pid, "starting it again keeps the one running");

    p.stdout.write(`${JSON.stringify({ level: "error", msg: "poll failed", error: `401 for ${KEY} at C:\\Users\\nova\\x` })}\n`);
    await settle();
    s = await tunnels.status();
    assert.equal(s.tunnels[0].log.at(-1).msg, "poll failed: 401 for sk-… at ~\\x", "its log, keys and home taken out");
    assert.ok(!JSON.stringify(s).includes(KEY) && !logs.join("\n").includes(KEY), "never the key, in the status or the log");
    await fake.tick();
    assert.equal((await tunnels.status()).tunnels[0].state, "starting", "no health yet");
    fake.setHealth(READY);
    await fake.tick();
    s = await tunnels.status();
    assert.deepEqual([s.tunnels[0].state, s.tunnels[0].version, s.tunnels[0].key], ["ready", "0.0.16", "stored"]);

    const stopped = tunnels.stop(dot.grant_id);
    assert.equal(stopped.state, "stopping");
    await settle();
    assert.deepEqual(fake.killed, [p.pid], "the whole tree");
    assert.equal((await tunnels.status()).tunnels[0].state, "off");
  } finally {
    w.done();
  }
});

test("a tunnel that exits starts again after a wait, gives up after a few, and stops for good when its grant goes", async () => {
  const w = world();
  try {
    const dot = pairIn(w);
    const { tunnels, fake } = rig(w);
    tunnels.configure(dot.grant_id, { tunnelId: TUNNEL, autostart: true });
    await tunnels.storeKey(dot.grant_id, KEY);
    await tunnels.autostart();
    assert.equal(fake.procs.length, 1, "started with XenoNet, as you set it to");
    fake.procs[0].stdout.write(`${JSON.stringify({ level: "error", msg: "tunnel not found" })}\n`);
    await settle();
    fake.procs[0].emit("exit", 1);
    let s = (await tunnels.status()).tunnels[0];
    assert.equal(s.state, "failed");
    assert.match(s.why, /exited with code 1: tunnel not found; trying again in 5 s/);
    for (let i = 1; i <= 3; i++) {
      await fake.tick();
      assert.equal(fake.procs.length, i + 1, `started again (${i})`);
      fake.procs[i].emit("exit", 1);
    }
    s = (await tunnels.status()).tunnels[0];
    assert.equal(s.state, "failed");
    assert.doesNotMatch(s.why, /trying again/, "it gave up");
    await fake.tick();
    assert.equal(fake.procs.length, 4, "and doesn't start again by itself");

    // Started by you again, then revoked: it stops, and can't start.
    await tunnels.start(dot.grant_id);
    assert.equal(fake.procs.length, 5);
    w.connections.revoke(dot.grant_id);
    await settle();
    assert.equal(fake.killed.at(-1), fake.procs[4].pid, "revoking stops it");
    await assert.rejects(tunnels.start(dot.grant_id), (e) => e.code === "not-active");
  } finally {
    w.done();
  }
});

test("turning cloud access off stops every tunnel; a tunnel-client you run yourself for the grant isn't doubled", async () => {
  const w = world();
  try {
    const dot = pairIn(w);
    const other = pairIn(w, { label: "Second dot" });
    const yours = [{ name: "xenonet", file: "C:\\Users\\nova\\AppData\\Roaming\\tunnel-client\\xenonet.yaml", tunnelId: TUNNEL, health: "127.0.0.1:8790", grant: other.grant_id }];
    const { tunnels, fake, kept } = rig(w, { profiles: yours });
    for (const g of [dot.grant_id, other.grant_id]) {
      tunnels.configure(g, { tunnelId: TUNNEL });
      await tunnels.storeKey(g, KEY);
    }
    const st = (await tunnels.status()).tunnels.find((t) => t.grant === other.grant_id);
    assert.deepEqual(st.profile, { name: "xenonet", tunnel_id: TUNNEL }, "your own profile for it is recognized");
    // Your own tunnel-client answers on its fixed health address.
    fake.yours.up = true;
    await assert.rejects(tunnels.start(other.grant_id), (e) => e.code === "running-elsewhere" && /profile xenonet/.test(e.message));
    fake.yours.up = false;

    await tunnels.start(dot.grant_id);
    assert.equal(fake.procs.length, 1);
    w.connections.disable();
    await settle();
    assert.deepEqual(fake.killed, [fake.procs[0].pid]);
    await assert.rejects(tunnels.start(dot.grant_id), (e) => e.code === "disabled");
    // A key you named is yours to remove; XenoNet's own it takes away.
    tunnels.configure(dot.grant_id, { keyTarget: "My/own/item" });
    await assert.rejects(tunnels.forgetKey(dot.grant_id), (e) => e.code === "yours");
    tunnels.configure(dot.grant_id, { keyTarget: null });
    assert.deepEqual(await tunnels.forgetKey(dot.grant_id), { grant: dot.grant_id, key: "missing" });
  } finally {
    w.done();
  }
});

test("a connection that doesn't go through a tunnel has none to set up", async () => {
  const w = world();
  try {
    const local = pairIn(w, { transport: "local-stdio", label: "Local" });
    const { tunnels } = rig(w);
    assert.throws(() => tunnels.configure(local.grant_id, { tunnelId: TUNNEL }), /doesn't go through a tunnel/);
    assert.deepEqual((await tunnels.status()).tunnels, []);
    assert.throws(() => tunnels.configure("gr_000000000000000000000000", {}), (e) => e.status === 404);
  } finally {
    w.done();
  }
});

void T;
