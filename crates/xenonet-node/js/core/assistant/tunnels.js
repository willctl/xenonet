// core/assistant/tunnels.js in Rust (crates/xenonet-assistant/src/
// tunnels.rs): tunnels XenoNet runs for remote actors' connections through
// OpenAI's Secure MCP Tunnel. Bindings and their checks, each run's state,
// its backoff after an exit, its log as XenoNet shows it and the client's
// own environment are the Rust's; the processes, timers, credential store,
// health probes and files are here.
//
// The key goes only into tunnel-client's own environment
// (CONTROL_PLANE_API_KEY), never into a file, a command line or a log.
import fs from "node:fs";
import path from "node:path";
import { spawn as spawnChild, execFile } from "node:child_process";
import { EventEmitter } from "node:events";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { envOf } from "../../machine.js";
import { findClient, clientVersion, findProfiles, profileDirs, profileYaml, commandLine, adapterArgv, RELEASES, TUNNELS_PAGE } from "./tunnel-client.js";

const { TARGET } = await import(peer("secret-store.js"));
const { GRANT_ID } = await import(peer("schema/assistant.js"));
const { tilde } = await import(peer("paths.js"));

const T = native.asstConstants().tunnel;

export const TUNNEL_TRANSPORT = T.transport;
export const GUIDE = T.guide;
export const keyTargetFor = (tunnelId) => native.asstKeyTargetFor(String(tunnelId));

export class TunnelError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}
const fail = (status, code, message) => {
  throw new TunnelError(status, code, message);
};
const refused = (r) => {
  if (r.error) fail(r.error.status, r.error.code, r.error.message);
  return r.ok;
};
const targetOk = (s) => TARGET.test(s);

export function createTunnels({ connections, store, dir, secrets, launch, home = process.env.HOME ?? process.env.USERPROFILE ?? "", env = process.env, platform = process.platform, find = () => findClient({ env, platform, home }), version = clientVersion, profiles = () => findProfiles(profileDirs({ env, platform, home })), spawn = spawnChild, fetch = globalThis.fetch, now = Date.now, log = () => {}, killTree = defaultKillTree(platform), setTimer = setTimeout, clearTimer = clearTimeout }) {
  const events = new EventEmitter();
  const core = new native.AsstTunnels();
  // A run's program and timers, by grant.
  const live = new Map();
  const versions = new Map();
  const tdir = path.join(dir, "tunnels");
  const fileOf = (grant, ext) => path.join(tdir, `${grant}.${ext}`);
  const handles = (grant) => live.get(grant) ?? live.set(grant, { child: null, timer: null, poll: null }).get(grant);

  // ---------- what's set up ----------

  const bindingOf = (connectionId) => native.asstBindingOf(store.connection(connectionId)?.transport_binding ?? null, targetOk);
  const keyTargetOf = (b) => native.asstKeyTargetOf(b) ?? null;

  function tunneled(grant, { active = false } = {}) {
    const v = GRANT_ID.test(String(grant)) ? connections.view(grant) : null;
    if (!v) fail(404, "not-found", "XenoNet has no such connection");
    if (v.transport_kind !== TUNNEL_TRANSPORT) fail(400, "bad-request", "that connection doesn't go through a tunnel");
    if (active && v.state !== "active") fail(409, "not-active", `that connection is ${v.state}`);
    return v;
  }

  async function hasKey(target) {
    if (!target) return false;
    const had = core.hasKey(target, now());
    if (had != null) return had;
    const has = await secrets.has(target).catch(() => false);
    core.setHas(target, !!has, now());
    return has;
  }

  function configure(grant, { tunnelId, keyTarget, autostart } = {}) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    const next = refused(native.asstConfigure(b, tunnelId, keyTarget, autostart, targetOk));
    store.setTransportBinding(v.connection_id, next.text);
    events.emit("changed", { grant });
    return { grant, tunnel_id: next.binding.tunnelId, key_target: keyTargetOf(next.binding), autostart: next.binding.autostart };
  }

  async function storeKey(grant, key) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    const k = refused(native.asstRuntimeKey(b, key));
    if (!secrets.available()) fail(501, "no-store", `XenoNet can't reach ${secrets.kind} on this computer`);
    const target = keyTargetOf(b);
    await secrets.set(target, k, { comment: "Runtime API key for an OpenAI Secure MCP Tunnel, kept by XenoNet for a remote actor" }).catch((err) => fail(500, "store", err.message));
    core.setHas(target, true, now());
    events.emit("changed", { grant });
    return { grant, key: "stored", target };
  }

  async function forgetKey(grant) {
    const v = tunneled(grant);
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) return { grant, key: "missing" };
    if (b.keyTarget) fail(409, "yours", "XenoNet didn't make that item in your credential store; remove it there yourself, or name another");
    const target = keyTargetFor(b.tunnelId);
    await secrets.remove(target).catch(() => false);
    core.setHas(target, false, now());
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
  // your own with a fixed health address that answers.
  async function outside(grant) {
    let found = [];
    try {
      found = profiles().filter((p) => p.grant === grant);
    } catch {}
    for (const p of found) {
      const url = native.asstReadyzOf(p.health ?? null);
      if (!url) continue;
      if (await probe(url, 1000)) return { profile: p.name, file: tilde(p.file, home) };
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

  const view = (grant) => core.view(grant, live.get(grant)?.child?.pid ?? null);
  const changed = (grant, state) => events.emit("changed", { grant, ...(state ? { state } : {}) });
  const stateOf = (grant) => view(grant).state;

  async function start(grant) {
    if (connections.disabled()) fail(409, "disabled", "cloud access is off; turn it on first");
    const v = tunneled(grant, { active: true });
    const h = live.get(grant);
    if (core.hasRun(grant) && h?.child && !core.stopping(grant)) return { grant, ...view(grant) };
    const b = bindingOf(v.connection_id);
    if (!b.tunnelId) fail(409, "no-tunnel", "say which tunnel it is first");
    const c = await client();
    if (!c.found) fail(409, "no-client", `XenoNet can't find tunnel-client; install it from ${RELEASES}`);
    const there = await outside(grant);
    if (there) fail(409, "running-elsewhere", `a tunnel-client you started yourself already runs this connection (profile ${there.profile}); stop it first, or keep using it`);
    const key = await secrets.get(keyTargetOf(b)).catch(() => null);
    if (!key) fail(409, "no-key", "XenoNet has no runtime key for this tunnel yet");
    clearTimer(handles(grant).timer);
    launchRun(grant, v, b, c.path, key);
    return { grant, ...view(grant) };
  }

  function launchRun(grant, v, b, file, key) {
    fs.mkdirSync(tdir, { recursive: true, mode: 0o700 });
    const profile = fileOf(grant, "yaml");
    const urlFile = fileOf(grant, "url");
    const command = commandLine(adapterArgv({ bin: launch.bin, grant, port: launch.port }));
    fs.writeFileSync(profile, profileYaml({ tunnelId: b.tunnelId, command, label: v.label }), { mode: 0o600 });
    fs.rmSync(urlFile, { force: true });
    const childEnv = native.asstChildEnv(envOf(env), key, launch.port, true);
    if (core.launching(grant, now())) changed(grant, "starting");
    const h = handles(grant);
    let child;
    try {
      child = spawn(file, ["run", "--profile-file", profile, "--health.url-file", urlFile], { env: childEnv, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
    } catch (err) {
      return exited(grant, null, `it couldn't start: ${err.message}`);
    }
    h.child = child;
    const read = (stream) => {
      let buf = "";
      stream?.on("data", (d) => {
        buf += d;
        for (let i; (i = buf.indexOf("\n")) >= 0; ) {
          const said = core.line(grant, buf.slice(0, i), now(), String(home ?? ""));
          buf = buf.slice(i + 1);
          if (said) log(said);
        }
        if (buf.length > 1 << 16) buf = "";
      });
    };
    read(child.stdout);
    read(child.stderr);
    child.on("error", (err) => exited(grant, child, `it couldn't start: ${err.code ?? err.message}`));
    child.on("exit", (code) => exited(grant, child, `it exited${code != null ? ` with code ${code}` : ""}`));
    // Its health, now and then, from the address it wrote when it started.
    const poll = async () => {
      if (h.child !== child || core.stopping(grant)) return;
      let written = null;
      try {
        written = fs.readFileSync(urlFile, "utf8");
      } catch {}
      const url = written == null ? null : native.asstHealthUrl(written);
      const body = url ? await probe(url, 3000) : null;
      if (h.child !== child || core.stopping(grant)) return;
      let health = null;
      try {
        health = body ? JSON.parse(body) : null;
      } catch {}
      if (core.health(grant, health, now())) changed(grant, stateOf(grant));
      h.poll = setTimer(poll, T.pollMs);
    };
    h.poll = setTimer(poll, T.firstPollMs);
  }

  function exited(grant, child, why) {
    const h = handles(grant);
    if (child && h.child !== child) return;
    h.child = null;
    clearTimer(h.poll);
    const a = core.exited(grant, why, now());
    if (a.log) log(a.log);
    if (a.changed) changed(grant, stateOf(grant));
    if (a.retryMs == null) return;
    h.timer = setTimer(() => {
      h.timer = null;
      if (core.stopping(grant) || h.child) return;
      return start(grant).catch((err) => {
        if (core.failed(grant, err.message, now())) changed(grant, "failed");
      });
    }, a.retryMs);
  }

  function stop(grant, { reason = null } = {}) {
    if (!core.hasRun(grant)) return { grant, ...view(grant) };
    const h = handles(grant);
    clearTimer(h.timer);
    clearTimer(h.poll);
    if (core.stop(grant, reason, !!h.child, now())) changed(grant, stateOf(grant));
    if (h.child) killTree(h.child);
    return { grant, ...view(grant) };
  }

  // tunnel-client's own checks of the profile, with the key: its lines,
  // with anything like a key taken out.
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
    const childEnv = native.asstChildEnv(envOf(env), key, launch.port, false);
    const out = await new Promise((resolve) => {
      execFile(c.path, ["doctor", "--profile-file", profile, "--explain"], { env: childEnv, timeout: 60000, windowsHide: true, maxBuffer: 1 << 20 }, (err, stdout, stderr) => resolve({ ok: !err, text: `${stdout ?? ""}\n${stderr ?? ""}` }));
    });
    fs.rmSync(profile, { force: true });
    return { grant, ok: out.ok, lines: native.asstCheckLines(out.text, String(home ?? "")) };
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
      const stored = target ? !!(await hasKey(target)) : false;
      tunnels.push(native.asstStatusEntry(x, b, stored, yours, view(x.grant_id)));
    }
    return {
      client: { found: c.found, path: c.path ? tilde(c.path, home) : null, version: c.version },
      store: { kind: secrets.kind, available: secrets.available() },
      links: { releases: RELEASES, tunnels: TUNNELS_PAGE, guide: GUIDE },
      tunnels,
    };
  }

  // ---------- with XenoNet ----------

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
    for (const grant of core.grants()) stop(grant, { reason: "cloud access is off" });
  });
  // A new credential: the adapter it runs holds the old one, so it starts
  // again with the new.
  connections.on("rotated", ({ grant }) => {
    if (!live.get(grant)?.child) return;
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
    running: () => core.grants().filter((g) => live.get(g)?.child),
    stopAll() {
      for (const grant of core.grants()) stop(grant);
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
