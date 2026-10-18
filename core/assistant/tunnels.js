// Tunnels XenoNet sets up and runs for remote actors' connections
// (docs/guide/cloud-assistants.md): for a connection through OpenAI's
// Secure MCP Tunnel, XenoNet finds tunnel-client, writes its profile for the
// connection's grant, keeps the tunnel's runtime key in your computer's
// credential store (core/secret-store.js), and runs the client as a child
// of its service, so the tunnel is up while XenoNet is and you don't keep a
// terminal open for it.
//
// Your choices, all of them, from XenoNet's window (server/routes/
// assistant.js): which tunnel a connection uses (its id, which isn't a
// secret), the key (typed into the window once; XenoNet keeps it in the
// store and never shows it again), whether it starts with XenoNet (off
// unless you turn it on), and starting it. Stopping it is anyone's who is
// you, from a terminal too. A tunnel stops by itself when its grant is
// revoked, expires, or cloud access is turned off.
//
// The key goes only into tunnel-client's own environment
// (CONTROL_PLANE_API_KEY), never into a file, a command line or a log;
// its log lines are read as JSON and anything that looks like a key is
// taken out before XenoNet keeps or shows one.
import fs from "node:fs";
import path from "node:path";
import { spawn as spawnChild, execFile } from "node:child_process";
import { EventEmitter } from "node:events";
import { findClient, clientVersion, findProfiles, profileDirs, profileYaml, commandLine, adapterArgv, logLine, healthOf, TUNNEL_ID, RUNTIME_KEY, RELEASES, TUNNELS_PAGE } from "./tunnel-client.js";
import { TARGET } from "../secret-store.js";
import { GRANT_ID } from "../schema/assistant.js";
import { tilde } from "../paths.js";

export const TUNNEL_TRANSPORT = "openai-secure-mcp-tunnel";
export const GUIDE = "https://developers.openai.com/api/docs/guides/secure-mcp-tunnels";
// The store's item for a tunnel's key, unless you name one you keep already.
export const keyTargetFor = (tunnelId) => `XenoNet/OpenAI tunnel/${tunnelId}`;
const POLL_MS = 5000;
// After it exits by itself: wait, then try again, longer each time; after
// as many exits in ten minutes as there are waits, it stays failed.
const BACKOFF_MS = [5e3, 20e3, 60e3];
const WINDOW_MS = 10 * 60e3;
const LINES = 30;
const HAS_MS = 60e3;

export class TunnelError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}
const fail = (status, code, message) => {
  throw new TunnelError(status, code, message);
};
const parse = (s) => {
  try {
    return s ? JSON.parse(s) : {};
  } catch {
    return {};
  }
};

// `launch`: how the adapter runs ({ bin, port }), as
// XenoNet's own hooks do. `secrets`: the credential store. The rest stand in
// for tests.
export function createTunnels({ connections, store, dir, secrets, launch, home = process.env.HOME ?? process.env.USERPROFILE ?? "", env = process.env, platform = process.platform, find = () => findClient({ env, platform, home }), version = clientVersion, profiles = () => findProfiles(profileDirs({ env, platform, home })), spawn = spawnChild, fetch = globalThis.fetch, now = Date.now, log = () => {}, killTree = defaultKillTree(platform), setTimer = setTimeout, clearTimer = clearTimeout }) {
  const events = new EventEmitter();
  const runs = new Map(); // grant → its run
  const hasCache = new Map(); // key target → { at, has }
  const versions = new Map(); // client path → version
  const tdir = path.join(dir, "tunnels");
  const fileOf = (grant, ext) => path.join(tdir, `${grant}.${ext}`);

  // ---------- what's set up ----------

  const bindingOf = (connectionId) => {
    const b = parse(store.connection(connectionId)?.transport_binding);
    return { tunnelId: TUNNEL_ID.test(b.tunnel_id ?? "") ? b.tunnel_id : null, keyTarget: TARGET.test(b.key_target ?? "") ? b.key_target : null, autostart: b.autostart === true };
  };
  const keyTargetOf = (b) => b.keyTarget ?? (b.tunnelId ? keyTargetFor(b.tunnelId) : null);

  // A connection through a tunnel, by its grant, as connections.js has it.
  function tunneled(grant, { active = false } = {}) {
    const v = GRANT_ID.test(String(grant)) ? connections.view(grant) : null;
    if (!v) fail(404, "not-found", "XenoNet has no such connection");
    if (v.transport_kind !== TUNNEL_TRANSPORT) fail(400, "bad-request", "that connection doesn't go through a tunnel");
    if (active && v.state !== "active") fail(409, "not-active", `that connection is ${v.state}`);
    return v;
  }

  async function hasKey(target) {
    if (!target) return false;
    const had = hasCache.get(target);
    if (had && now() - had.at < HAS_MS) return had.has;
    const has = await secrets.has(target).catch(() => false);
    hasCache.set(target, { at: now(), has });
    return has;
  }

  // Which tunnel a connection uses, where its key is kept, and whether it
  // starts with XenoNet. A change while it runs takes effect when it starts
  // again.
  function configure(grant, { tunnelId, keyTarget, autostart } = {}) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    const next = { ...b };
    if (tunnelId !== undefined) {
      if (!TUNNEL_ID.test(String(tunnelId ?? ""))) fail(400, "bad-request", "a tunnel's id starts tunnel_, as OpenAI's Tunnels page shows it");
      next.tunnelId = tunnelId;
    }
    if (keyTarget !== undefined) {
      if (keyTarget !== null && keyTarget !== "" && !TARGET.test(String(keyTarget))) fail(400, "bad-request", "a credential's name is printable text, without quotes or backslashes");
      next.keyTarget = keyTarget || null;
    }
    if (autostart !== undefined) next.autostart = autostart === true;
    store.setTransportBinding(v.connection_id, JSON.stringify({ tunnel_id: next.tunnelId, key_target: next.keyTarget, autostart: next.autostart }));
    events.emit("changed", { grant });
    return { grant, tunnel_id: next.tunnelId, key_target: keyTargetOf(next), autostart: next.autostart };
  }

  // The tunnel's runtime key, into the store. It's checked to be one, and
  // never comes back out of XenoNet but into tunnel-client.
  async function storeKey(grant, key) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) fail(409, "no-tunnel", "say which tunnel it is first");
    if (!RUNTIME_KEY.test(String(key ?? "").trim())) fail(400, "bad-request", "that isn't an OpenAI API key (sk-…)");
    if (!secrets.available()) fail(501, "no-store", `XenoNet can't reach ${secrets.kind} on this computer`);
    const target = keyTargetOf(b);
    await secrets.set(target, String(key).trim(), { comment: "Runtime API key for an OpenAI Secure MCP Tunnel, kept by XenoNet for a remote actor" }).catch((err) => fail(500, "store", err.message));
    hasCache.set(target, { at: now(), has: true });
    events.emit("changed", { grant });
    return { grant, key: "stored", target };
  }

  // Takes away the key XenoNet made; one you named, kept under a name of
  // your own, is yours to remove.
  async function forgetKey(grant) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) return { grant, key: "missing" };
    if (b.keyTarget) fail(409, "yours", "XenoNet didn't make that item in your credential store; remove it there yourself, or name another");
    const target = keyTargetFor(b.tunnelId);
    await secrets.remove(target).catch(() => false);
    hasCache.set(target, { at: now(), has: false });
    events.emit("changed", { grant });
    return { grant, key: "missing" };
  }

  // ---------- the client ----------

  async function client() {
    const file = find();
    if (!file) return { found: false, path: null, version: null };
    if (!versions.has(file)) versions.set(file, await version(file));
    return { found: true, path: file, version: versions.get(file) };
  }

  // A tunnel-client you run yourself for this grant, from a profile of
  // your own with a fixed health address that answers: XenoNet doesn't
  // start a second.
  async function outside(grant) {
    let found = [];
    try {
      found = profiles().filter((p) => p.grant === grant);
    } catch {}
    for (const p of found) {
      const addr = /^(127\.0\.0\.1|localhost|\[::1\]):(\d{2,5})$/.exec(p.health ?? "");
      if (!addr || addr[2] === "0") continue;
      const up = await probe(`http://${addr[1]}:${addr[2]}/readyz`, 1000);
      if (up) return { profile: p.name, file: tilde(p.file, home) };
    }
    return null;
  }

  async function probe(url, ms) {
    try {
      const r = await fetch(url, { signal: AbortSignal.timeout(ms) });
      return r.ok ? await r.text() : null;
    } catch {
      return null;
    }
  }

  // ---------- running one ----------

  const view = (run) => (run ? { state: run.state, why: run.why ?? null, since: run.since, version: run.version ?? null, polled_at: run.polledAt ?? null, pid: run.child?.pid ?? null, restarts: run.exits.length, log: run.lines.slice(-8) } : { state: "off", why: null, since: null, version: null, polled_at: null, pid: null, restarts: 0, log: [] });

  function setState(run, state, why = null) {
    if (run.state === state && run.why === why) return;
    run.state = state;
    run.why = why;
    run.since = now();
    events.emit("changed", { grant: run.grant, state });
  }

  async function start(grant) {
    if (connections.disabled()) fail(409, "disabled", "cloud access is off; turn it on first");
    const v = tunneled(grant, { active: true });
    const had = runs.get(grant);
    if (had && had.child && !had.stopping) return { grant, ...view(had) };
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) fail(409, "no-tunnel", "say which tunnel it is first");
    const c = await client();
    if (!c.found) fail(409, "no-client", `XenoNet can't find tunnel-client; install it from ${RELEASES}`);
    const there = await outside(grant);
    if (there) fail(409, "running-elsewhere", `a tunnel-client you started yourself already runs this connection (profile ${there.profile}); stop it first, or keep using it`);
    const key = await secrets.get(keyTargetOf(b)).catch(() => null);
    if (!key) fail(409, "no-key", "XenoNet has no runtime key for this tunnel yet");
    const run = had ?? { grant, state: "off", since: now(), exits: [], lines: [], child: null, stopping: false, timer: null, poll: null };
    clearTimer(run.timer);
    runs.set(grant, run);
    launchRun(run, v, b, c.path, key);
    return { grant, ...view(run) };
  }

  function launchRun(run, v, b, file, key) {
    fs.mkdirSync(tdir, { recursive: true, mode: 0o700 });
    const profile = fileOf(run.grant, "yaml");
    const urlFile = fileOf(run.grant, "url");
    const command = commandLine(adapterArgv({ bin: launch.bin, grant: run.grant, port: launch.port }));
    fs.writeFileSync(profile, profileYaml({ tunnelId: b.tunnelId, command, label: v.label }), { mode: 0o600 });
    fs.rmSync(urlFile, { force: true });
    const childEnv = { ...env, CONTROL_PLANE_API_KEY: key, XENONET_PORT: String(launch.port) };
    delete childEnv.OPENAI_API_KEY;
    run.stopping = false;
    setState(run, "starting", "tunnel-client is starting");
    let child;
    try {
      child = spawn(file, ["run", "--profile-file", profile, "--health.url-file", urlFile], { env: childEnv, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    } catch (err) {
      return exited(run, null, `it couldn't start: ${err.message}`);
    }
    run.child = child;
    const read = (stream) => {
      let buf = "";
      stream?.on("data", (d) => {
        buf += d;
        for (let i; (i = buf.indexOf("\n")) >= 0; ) {
          const line = logLine(buf.slice(0, i).trim());
          buf = buf.slice(i + 1);
          if (!line) continue;
          run.lines.push({ at: now(), level: line.level, msg: tilde(line.msg, home) });
          if (run.lines.length > LINES) run.lines.splice(0, run.lines.length - LINES);
          if (line.level === "error" || line.level === "warn") log(`tunnel ${run.grant}: ${line.level}: ${line.msg}`);
        }
        if (buf.length > 1 << 16) buf = "";
      });
    };
    read(child.stdout);
    read(child.stderr);
    child.on("error", (err) => exited(run, child, `it couldn't start: ${err.code ?? err.message}`));
    child.on("exit", (code) => exited(run, child, `it exited${code != null ? ` with code ${code}` : ""}`));
    // Its health, now and then, from the address it wrote when it started.
    const poll = async () => {
      if (run.child !== child || run.stopping) return;
      let url = null;
      try {
        url = fs.readFileSync(urlFile, "utf8").trim();
      } catch {}
      const body = url && /^http:\/\/(127\.0\.0\.1|localhost|\[::1\]):\d+\/?$/.test(url) ? await probe(`${url.replace(/\/$/, "")}/health?details=true`, 3000) : null;
      if (run.child !== child || run.stopping) return;
      let h = null;
      try {
        h = body ? JSON.parse(body) : null;
      } catch {}
      const s = healthOf(h);
      run.version = s.version ?? run.version;
      run.polledAt = s.polledAt ?? run.polledAt;
      setState(run, s.state, s.why);
      run.poll = setTimer(poll, POLL_MS);
    };
    run.poll = setTimer(poll, 1500);
  }

  function exited(run, child, why) {
    if (child && run.child !== child) return;
    run.child = null;
    clearTimer(run.poll);
    if (run.stopping) return setState(run, "off", null);
    const t = now();
    run.exits = run.exits.filter((at) => t - at < WINDOW_MS);
    run.exits.push(t);
    const last = run.lines.filter((l) => l.level === "error").at(-1)?.msg;
    const said = last ? `${why}: ${last}` : why;
    log(`tunnel ${run.grant}: ${said}`);
    if (run.exits.length > BACKOFF_MS.length) return setState(run, "failed", said);
    setState(run, "failed", `${said}; trying again in ${Math.round(BACKOFF_MS[run.exits.length - 1] / 1000)} s`);
    run.timer = setTimer(() => {
      run.timer = null;
      if (run.stopping || run.child) return;
      return start(run.grant).catch((err) => setState(run, "failed", err.message));
    }, BACKOFF_MS[run.exits.length - 1]);
  }

  function stop(grant, { reason = null } = {}) {
    const run = runs.get(grant);
    if (!run) return { grant, ...view(null) };
    run.stopping = true;
    clearTimer(run.timer);
    clearTimer(run.poll);
    run.exits = [];
    if (run.child) {
      setState(run, "stopping", reason);
      killTree(run.child);
    } else setState(run, "off", reason);
    return { grant, ...view(run) };
  }

  // tunnel-client's own checks of the profile, with the key, as you'd run
  // tunnel-client doctor --explain: its lines, with anything like a key
  // taken out.
  async function check(grant) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) fail(409, "no-tunnel", "say which tunnel it is first");
    const c = await client();
    if (!c.found) fail(409, "no-client", `XenoNet can't find tunnel-client; install it from ${RELEASES}`);
    const key = await secrets.get(keyTargetOf(b)).catch(() => null);
    if (!key) fail(409, "no-key", "XenoNet has no runtime key for this tunnel yet");
    fs.mkdirSync(tdir, { recursive: true, mode: 0o700 });
    const profile = fileOf(grant, "check.yaml");
    const command = commandLine(adapterArgv({ bin: launch.bin, grant, port: launch.port }));
    fs.writeFileSync(profile, profileYaml({ tunnelId: b.tunnelId, command, label: v.label }), { mode: 0o600 });
    const out = await new Promise((resolve) => {
      execFile(c.path, ["doctor", "--profile-file", profile, "--explain"], { env: { ...env, CONTROL_PLANE_API_KEY: key, XENONET_PORT: String(launch.port) }, timeout: 60000, windowsHide: true, maxBuffer: 1 << 20 }, (err, stdout, stderr) => resolve({ ok: !err, text: `${stdout ?? ""}\n${stderr ?? ""}` }));
    });
    fs.rmSync(profile, { force: true });
    const lines = out.text
      .split(/\r?\n/)
      .map((l) => tilde(l.replace(/sk-[A-Za-z0-9_-]{8,}/g, "sk-…").replace(/[\u0000-\u0008\u000b-\u001f]/g, ""), home).trimEnd())
      .filter((l) => l.trim())
      .slice(0, 80);
    return { grant, ok: out.ok, lines };
  }

  // ---------- how they stand ----------

  async function status() {
    const c = await client();
    const list = connections.list().connections.filter((x) => x.transport_kind === TUNNEL_TRANSPORT && x.state !== "revoked");
    let found = [];
    try {
      found = profiles();
    } catch {}
    const tunnels = [];
    for (const x of list) {
      const b = bindingOf(x.connection_id);
      const target = keyTargetOf(b);
      const yours = found.find((p) => p.grant === x.grant_id) ?? null;
      tunnels.push({
        grant: x.grant_id,
        connection: x.connection_id,
        actor: x.actor_id,
        label: x.label,
        tunnel_id: b.tunnelId,
        key_target: target,
        key_named: !!b.keyTarget,
        key: target ? ((await hasKey(target)) ? "stored" : "missing") : "missing",
        autostart: b.autostart,
        // A profile of your own for this grant, whose tunnel XenoNet can use.
        profile: yours ? { name: yours.name, tunnel_id: yours.tunnelId } : null,
        ...view(runs.get(x.grant_id)),
      });
    }
    return {
      client: { found: c.found, path: c.path ? tilde(c.path, home) : null, version: c.version },
      store: { kind: secrets.kind, available: secrets.available() },
      links: { releases: RELEASES, tunnels: TUNNELS_PAGE, guide: GUIDE },
      tunnels,
    };
  }

  // ---------- with XenoNet ----------

  // Those set to start with XenoNet, when it starts.
  async function autostart() {
    if (connections.disabled()) return;
    for (const x of connections.list().connections) {
      if (x.transport_kind !== TUNNEL_TRANSPORT || x.state !== "active" || !bindingOf(x.connection_id).autostart) continue;
      await start(x.grant_id).catch((err) => log(`tunnel ${x.grant_id}: didn't start with XenoNet: ${err.message}`));
    }
  }

  connections.on("revoked", ({ grant }) => stop(grant, { reason: "its connection was revoked" }));
  connections.on("expired", ({ grant }) => stop(grant, { reason: "its grant expired" }));
  connections.on("disabled", () => {
    for (const grant of runs.keys()) stop(grant, { reason: "cloud access is off" });
  });
  // A new credential: the adapter it runs holds the old one, so it starts
  // again with the new.
  connections.on("rotated", ({ grant }) => {
    const run = runs.get(grant);
    if (!run?.child) return;
    stop(grant);
    setTimer(() => start(grant).catch((err) => log(`tunnel ${grant}: didn't start again after a new credential: ${err.message}`)), 1500);
  });

  return {
    events,
    on: (...args) => events.on(...args),
    configure,
    storeKey,
    forgetKey,
    start,
    stop,
    check,
    status,
    autostart,
    running: () => [...runs.values()].filter((r) => r.child).map((r) => r.grant),
    stopAll() {
      for (const grant of runs.keys()) stop(grant);
    },
  };
}

// Stops tunnel-client and the adapter it runs: on Windows, the whole tree,
// which a kill of the parent alone would leave.
function defaultKillTree(platform) {
  return (child) => {
    if (platform === "win32" && child.pid) {
      execFile("taskkill", ["/pid", String(child.pid), "/T", "/F"], { windowsHide: true }, () => {});
      return;
    }
    try {
      child.kill("SIGTERM");
    } catch {}
  };
}
