// One host's link (docs/design/hosts.md): the console's end of
// `ssh <host> xenonet relay`, or of another XenoNet's relay on this machine
// (the local transport, for tests and for watching XenoNet from XenoNet Dev).
// It starts the transport, reads the frames, keeps the clock offset, sends
// gets with time limits, and keeps the host's state:
//
//   connecting  starting
//   live        frames are arriving
//   stale       no frame for 45 seconds: three missed pings
//   offline     the connection ended or never came up; it tries again
//               with backoff, from 2 to 60 seconds, with jitter
//   error       it needs you: the host key, a refused key, no relay, a
//               protocol this XenoNet doesn't speak, or another XenoNet
//               answering in the host's place; tried again every minute
//
// Everything a host sends is untrusted: frames are capped in size and
// checked, and the merging (merge.js) qualifies every id. Nothing a link
// reads can make it write anywhere but to the relay's standard input.
import { EventEmitter } from "node:events";
import { spawn as childSpawn } from "node:child_process";
import path from "node:path";
import { xenonetBin } from "../components/runtime.js";
import { systemClock } from "../blocks/clock.js";
import { check, explain } from "../schema/check.js";
import { HostHello, XENONET_COMMAND, SINCE } from "../schema/host.js";
import { isHostId } from "../schema/ids.js";
import { PROTOCOL, MAX_FRAME, FROM_RELAY, lineSplitter, parseFrame, encode, shallow } from "./frames.js";

export const HELLO_MS = 20e3;
export const STALE_MS = 45e3;
export const BACKOFF = { min: 2e3, max: 60e3 };
const GET_MS = 20e3;
const TIME_EVERY_MS = 5 * 60e3;

// ssh as hosts.md sets it: no terminal, no agent or X11 forwarding, never a
// prompt, a host key that's known and unchanged even if your own config
// would take a new one, none of your forwards, compression, and a
// connection that gives up on a silent host in about 45 seconds.
export const SSH_OPTIONS = ["-T", "-a", "-x", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ClearAllForwardings=yes", "-o", "ConnectTimeout=10", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", "-C"];

// What a link runs, as { file, args, env }: fixed words, and values
// hostEntry checked, checked again here, since the host's shell parses the
// remote command. The ssh destination is the host's id, an alias in
// XenoNet's own ssh config, and comes after --, so it can't be an option.
// The local transport runs only XenoNet's own relay, with none of this
// XenoNet's XENONET_ settings (a demo world's port, its state folder).
// `bin`: the xenonet binary a local link runs `relay` with, xenonetBin's by
// default.
export function transport(entry, { sshBin = "ssh", sshConfig = null, root, env = process.env, bin = null } = {}) {
  if (!isHostId(entry?.id)) throw new Error(`not a host id: ${JSON.stringify(entry?.id)}`);
  if (!Number.isInteger(entry.port) || entry.port < 1 || entry.port > 65535) throw new Error(`not a port: ${JSON.stringify(entry.port)}`);
  if (!SINCE.test(String(entry.since))) throw new Error(`not a time: ${JSON.stringify(entry.since)}`);
  const relay = ["relay", "--port", String(entry.port), "--since", entry.since, ...(entry.detail === true ? ["--detail"] : []), ...(entry.start === true ? ["--start"] : [])];
  if (entry.transport === "local") {
    const clean = Object.fromEntries(Object.entries(env).filter(([k]) => !k.startsWith("XENONET_")));
    return { file: bin ?? xenonetBin({ env, root }), args: relay, env: clean };
  }
  if (entry.transport !== "ssh") throw new Error(`not a transport: ${JSON.stringify(entry.transport)}`);
  if (!XENONET_COMMAND.test(String(entry.command))) throw new Error(`not a xenonet command: ${JSON.stringify(entry.command)}`);
  if (!sshConfig) throw new Error("ssh needs XenoNet's own ssh config");
  const [ssh, ...pre] = [].concat(sshBin);
  return { file: ssh, args: [...pre, "-F", sshConfig, ...SSH_OPTIONS, "--", entry.id, entry.command, ...relay], env };
}

// A path as a word a shell takes as it is, for a command a person pastes
// into a terminal: under the home folder as ~/ and the rest in quotes,
// since a shell expands ~ only unquoted and ssh doesn't expand it in -F at
// all; any other path in quotes whole. `p` comes with the home folder
// already written as ~ (tilde in core/paths.js).
const quote = (s) => (/^[\w@%+=:,./-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`);
export const shellPath = (p) => (String(p).startsWith("~/") ? `~/${quote(String(p).slice(2))}` : quote(String(p)));

// The command that checks a host's key, and accepts it once, in a
// terminal: ssh with XenoNet's own ssh config, where the host is, and
// `tilde` to write the config's path without the home folder.
export const trustCommand = (id, sshConfig, tilde = (p) => p) => (sshConfig ? `ssh -F ${shellPath(tilde(sshConfig))} ${id}` : `ssh ${id}`);

// Why a connection ended, from what ssh and the relay said:
// { state, code, message, fix? }.
export function whyEnded({ entry, exitCode = null, signal = null, stderr = "", bye = null, spawnError = null, sshConfig = null, tilde = (p) => p }) {
  const id = entry.id;
  const ssh = entry.transport === "ssh";
  // ssh names the machine and the user it logs in as, which often hold a
  // person's name: what it says is shown with the host's id in their place.
  const named = (text) => {
    let out = String(text);
    for (const [name, as] of [[entry.ssh?.user && `${entry.ssh.user}@`, ""], [entry.ssh?.hostname, id]]) if (name) out = out.split(name).join(as);
    return out;
  };
  const said = named(String(stderr).trim().split(/\r?\n/).filter(Boolean).at(-1)?.slice(0, 200) ?? "");
  const tryIt = trustCommand(id, sshConfig, tilde);
  if (spawnError) return { state: "error", code: "no-ssh", message: `${ssh ? "ssh" : "the runtime"} couldn't start here (${spawnError.code ?? spawnError.message})`, fix: ssh ? "Install OpenSSH's client, or put ssh on XenoNet's PATH." : "Restart XenoNet." };
  if (bye?.code === "no-xenonet") return { state: "offline", code: "no-xenonet", message: `no XenoNet is running on ${id}`, fix: `Start XenoNet on ${id}, or add it again with the port its XenoNet uses (xenonet hosts add ${id} … --port).` };
  if (bye?.code === "xenonet-gone") return { state: "offline", code: "xenonet-gone", message: `the XenoNet on ${id} stopped` };
  if (bye?.code === "old-xenonet") return { state: "error", code: "old-xenonet", message: `the XenoNet on ${id} is too old to be a host`, fix: `Update XenoNet on ${id}.` };
  if (bye?.code) return { state: "offline", code: "relay", message: `the relay on ${id} stopped: ${named(String(bye.message ?? bye.code).slice(0, 200))}` };
  if (ssh && exitCode === 255) {
    if (/host key verification failed|remote host identification has changed|no \S+ host key is known|host key for .* has changed/i.test(stderr)) return { state: "error", code: "host-key", message: `ssh doesn't know ${id}'s host key, or it changed`, fix: `Check it and accept it once in a terminal: ${tryIt}` };
    if (/permission denied|too many authentication failures|no supported authentication methods/i.test(stderr)) return { state: "error", code: "refused", message: `${id} refused the key`, fix: `Install the key in ${id}'s ~/.ssh/authorized_keys, then try ${tryIt}` };
    return { state: "offline", code: "unreachable", message: `ssh couldn't reach ${id}${said ? `: ${said.replace(/^ssh: /, "")}` : ""}`, fix: `Check that ${id} is on and reachable: ${tryIt}` };
  }
  if (exitCode === 127 || /(^|\s)xenonet: (command )?not found|command not found: xenonet|xenonet: No such file or directory/i.test(stderr)) return { state: "error", code: "no-relay", message: `${id} has no xenonet command`, fix: `Install XenoNet on ${id} and its command (xenonet install), or add it again with the command's full path (--command).` };
  return { state: "offline", code: "ended", message: `the relay on ${id} stopped${exitCode != null ? ` (exit ${exitCode})` : signal ? ` (${signal})` : ""}${said ? `: ${said}` : ""}` };
}

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);

export class HostLink extends EventEmitter {
  constructor({ entry, sshConfig = null, sshBin = "ssh", root, env = process.env, bin = null, spawn = childSpawn, clock = systemClock, random = Math.random, helloMs = HELLO_MS, staleMs = STALE_MS, backoff = BACKOFF, getMs = GET_MS, demoOnly = false, once = false, tilde = (p) => p }) {
    super();
    this.entry = { ...entry };
    this.options = { sshConfig, sshBin, root, env, bin };
    // `once`: a single try, for xenonet hosts test, never again after it ends.
    Object.assign(this, { spawn, clock, random, helloMs, staleMs, backoff, getMs, demoOnly, oneShot: once, tilde });
    this.state = "offline";
    this.since = clock.now();
    this.lastHeard = null;
    this.offsetMs = 0;
    this.rttMs = null;
    this.hello = null;
    this.error = null;
    this.nextTryAt = null;
    this.attempt = 0;
    this.counts = { connects: 0, frames: 0, dropped: 0 };
    this.lastDrop = null;
    this.seq = 0;
    this.run = null;
    this.stopped = true;
  }

  start() {
    if (!this.stopped) return this;
    this.stopped = false;
    this.staleTimer = this.clock.setInterval(() => this.#checkStale(), 5e3);
    this.staleTimer?.unref?.();
    this.#connect();
    return this;
  }

  stop() {
    this.stopped = true;
    this.clock.clearInterval(this.staleTimer);
    this.clock.clearTimeout(this.retryTimer);
    const run = this.run;
    this.run = null;
    if (run) this.#close(run, "the link stopped");
    this.nextTryAt = null;
  }

  // Down until resume(), as if the host went away: XenoNet Dev's
  // simulations, to see how an offline host looks.
  pause(message = "taken offline") {
    this.stop();
    this.error = { code: "paused", message };
    this.#set("offline");
  }

  resume() {
    return this.start();
  }

  // Now, rather than when the backoff says.
  reconnect() {
    if (this.stopped) return this.start();
    this.attempt = 0;
    this.clock.clearTimeout(this.retryTimer);
    const run = this.run;
    this.run = null;
    if (run) this.#close(run, "connecting again");
    this.#connect();
    return this;
  }

  // A read through the relay (its allowlist decides what it may be):
  // { status, body }, or an error with a status once the time is up or
  // the connection ends.
  get(route, { timeoutMs = this.getMs } = {}) {
    const run = this.run;
    if (!run?.snapshot) return Promise.reject(Object.assign(new Error(`${this.entry.id} isn't connected (${this.state})`), { status: 503 }));
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      const timer = this.clock.setTimeout(() => {
        run.pending.delete(id);
        reject(Object.assign(new Error(`${this.entry.id} didn't answer in ${Math.round(timeoutMs / 1000)} s`), { status: 504 }));
      }, timeoutMs);
      run.pending.set(id, { resolve, reject, timer });
      this.#write(run, { t: "get", id, path: route });
    });
  }

  // Where the host stands, for /api/hosts and the doctor.
  info() {
    const e = this.entry;
    return {
      id: e.id,
      transport: e.transport,
      target: e.transport === "local" ? `this machine, port ${e.port}` : `${e.ssh?.user ? `${e.ssh.user}@` : ""}${e.ssh?.hostname ?? e.id}${e.ssh?.port ? `:${e.ssh.port}` : ""}`,
      port: e.port,
      state: this.state,
      since: this.since,
      lastHeard: this.lastHeard,
      offsetMs: this.rttMs == null ? null : this.offsetMs,
      rttMs: this.rttMs,
      hello: this.hello,
      error: this.error,
      nextTryAt: this.state === "offline" || this.state === "error" ? this.nextTryAt : null,
      counts: { ...this.counts },
      lastDrop: this.lastDrop,
    };
  }

  // ---------- one connection ----------

  #connect() {
    this.clock.clearTimeout(this.retryTimer);
    if (this.stopped) return;
    let cmd;
    try {
      cmd = transport(this.entry, this.options);
    } catch (err) {
      return this.#ended({ state: "error", code: "bad-entry", message: `XenoNet can't reach ${this.entry.id} as it's kept: ${err.message}`, fix: `Remove it and add it again (xenonet hosts remove ${this.entry.id}).` });
    }
    this.counts.connects++;
    this.#set("connecting");
    const run = { child: null, hello: null, snapshot: false, bye: null, stderr: "", problem: null, ended: false, pending: new Map(), times: new Map() };
    this.run = run;
    let child;
    try {
      child = this.spawn(cmd.file, cmd.args, { stdio: ["pipe", "pipe", "pipe"], env: cmd.env, windowsHide: true });
    } catch (err) {
      return this.#end(run, { spawnError: err });
    }
    run.child = child;
    child.stdout.on("data", lineSplitter({ max: MAX_FRAME, onLine: (l) => this.#line(run, l), onDrop: () => this.#drop(run, "a frame over 16 MB") }));
    child.stderr.on("data", (d) => (run.stderr = (run.stderr + d).slice(-4096)));
    child.stdin.on("error", () => {});
    child.on("error", (err) => {
      // A program that couldn't start has no exit to wait for.
      if (!child.pid) this.#end(run, { spawnError: err });
    });
    child.on("close", (exitCode, signal) => this.#end(run, { exitCode, signal }));
    run.helloTimer = this.clock.setTimeout(() => {
      if (run.snapshot || run.ended) return;
      this.#fail(run, { state: "offline", code: run.hello ? "no-snapshot" : "no-hello", message: run.hello ? `${this.entry.id} said hello but sent no map in ${this.helloMs / 1000} s` : `${this.entry.id} didn't answer in ${this.helloMs / 1000} s` });
    }, this.helloMs);
  }

  #line(run, line) {
    if (this.run !== run || run.ended) return;
    const { frame, error } = parseFrame(line, FROM_RELAY);
    this.counts.frames++;
    if (!frame) return this.#drop(run, error);
    if (!shallow(frame)) return this.#drop(run, "a frame nested too deep");
    if (!run.hello && frame.t !== "hello" && frame.t !== "bye") return this.#drop(run, `a ${frame.t} before the hello`);
    this.lastHeard = this.clock.now();
    switch (frame.t) {
      case "hello":
        return this.#hello(run, frame);
      case "snapshot":
        return this.#snapshot(run, frame);
      case "event":
        if (typeof frame.name !== "string" || frame.name.length > 40 || !isObject(frame.data)) return this.#drop(run, "an event that isn't one");
        if (!run.snapshot) return;
        this.#heard();
        return this.emit("event", { name: frame.name, data: frame.data });
      case "res": {
        const p = run.pending.get(frame.id);
        if (!p) return this.#drop(run, "an answer to nothing asked");
        run.pending.delete(frame.id);
        this.clock.clearTimeout(p.timer);
        this.#heard();
        return p.resolve({ status: Number.isInteger(frame.status) && frame.status >= 100 && frame.status < 600 ? frame.status : 502, body: frame.body ?? null });
      }
      case "time":
        return this.#timed(run, frame);
      case "ping":
        return this.#heard();
      case "bye":
        run.bye = { code: String(frame.code ?? "").slice(0, 40), message: String(frame.message ?? "").slice(0, 300) };
        return;
    }
  }

  #hello(run, f) {
    if (run.hello) return this.#drop(run, "a second hello");
    const id = this.entry.id;
    const checked = check(HostHello, f);
    if (!checked.ok) return this.#fail(run, { state: "error", code: "bad-hello", message: `${id} answered with something that isn't a relay's hello (${explain(checked).split("\n")[0].slice(0, 160)})`, fix: `Update XenoNet on ${id} and here to the same version.` });
    if (f.protocol !== PROTOCOL) return this.#fail(run, { state: "error", code: "protocol", message: `${id} runs XenoNet ${f.version}, whose relay speaks protocol ${f.protocol}; this XenoNet speaks ${PROTOCOL}`, fix: "Update the older of the two." });
    if (this.demoOnly && !f.demo) return this.#fail(run, { state: "error", code: "not-demo", message: `${id} shows real sessions, and this XenoNet shows made-up data only`, fix: "Watch it from a XenoNet that shows your own sessions, or run a demo XenoNet there." });
    // The key a host made once says it's the same XenoNet as last time. A
    // demo world is made afresh at every start, key and all, and its data
    // is made up, so its new key is taken.
    if (this.entry.key && f.host.key !== this.entry.key && !f.demo) return this.#fail(run, { state: "error", code: "key-changed", message: `another XenoNet answered as ${id}: its key isn't the one ${id} had`, fix: `If ${id} was set up again, or its alias now points elsewhere on purpose, add it again (xenonet hosts add ${id} … ) to accept the new one.` });
    run.hello = f;
    this.hello = { id: f.host.id, key: f.host.key, version: f.version, platform: f.platform, demo: f.demo, protocol: f.protocol, sinceMs: f.sinceMs, detail: f.detail };
    if (f.host.key !== this.entry.key || (f.host.id ?? null) !== (this.entry.name ?? null)) {
      this.entry.key = f.host.key;
      this.entry.name = f.host.id && f.host.id !== id ? f.host.id : null;
      this.emit("learn", { key: f.host.key, name: f.host.id ?? null });
    }
    this.#time(run);
    run.timeTimer = this.clock.setInterval(() => this.#time(run), TIME_EVERY_MS);
    run.timeTimer?.unref?.();
  }

  #snapshot(run, f) {
    if (!isObject(f.graph) || !Array.isArray(f.graph.nodes) || !isObject(f.live) || (f.engine != null && !isObject(f.engine))) return this.#drop(run, "a snapshot that isn't one");
    const first = !run.snapshot;
    run.snapshot = true;
    this.clock.clearTimeout(run.helloTimer);
    this.attempt = 0;
    this.error = null;
    this.#set("live");
    this.emit("snapshot", { graph: f.graph, live: f.live, engine: f.engine ?? null, at: Number.isFinite(f.at) ? f.at : null, first });
  }

  // The offset: the host's time minus the midpoint of the round trip, from
  // the quickest round trip so far, so a slow moment doesn't skew it.
  #time(run) {
    const id = ++this.seq;
    run.times.set(id, this.clock.now());
    this.#write(run, { t: "time", id });
  }

  #timed(run, f) {
    const sent = run.times.get(f.id);
    if (sent == null || !Number.isFinite(f.now)) return this.#drop(run, "a time frame for no question");
    run.times.delete(f.id);
    const back = this.clock.now();
    const rtt = back - sent;
    if (this.rttMs == null || rtt <= Math.max(this.rttMs * 2, 250)) {
      this.offsetMs = Math.round(f.now - (sent + back) / 2);
      this.rttMs = this.rttMs == null ? rtt : Math.min(this.rttMs, rtt);
      this.emit("offset", this.offsetMs);
    }
    this.#heard();
  }

  #heard() {
    if (this.state === "stale" && this.run?.snapshot) this.#set("live");
  }

  #checkStale() {
    if (this.state === "live" && this.lastHeard != null && this.clock.now() - this.lastHeard > this.staleMs) this.#set("stale");
  }

  #drop(run, why) {
    this.counts.dropped++;
    this.lastDrop = { at: this.clock.now(), why: String(why).slice(0, 120) };
  }

  #write(run, frame) {
    try {
      run.child?.stdin.write(encode(frame));
    } catch {}
  }

  // Ends this connection for a reason found here (a bad hello, silence).
  #fail(run, why) {
    run.problem ??= why;
    if (run.child && run.child.exitCode == null && !run.child.killed) run.child.kill("SIGTERM");
    else this.#end(run, {});
  }

  #close(run, reason) {
    run.problem ??= { state: "offline", code: "stopped", message: reason };
    run.ended = true;
    this.#settle(run, reason);
    if (run.child && run.child.exitCode == null && !run.child.killed) run.child.kill("SIGTERM");
  }

  #settle(run, reason) {
    this.clock.clearTimeout(run.helloTimer);
    this.clock.clearInterval(run.timeTimer);
    for (const p of run.pending.values()) {
      this.clock.clearTimeout(p.timer);
      p.reject(Object.assign(new Error(`${this.entry.id}: ${reason}`), { status: 503 }));
    }
    run.pending.clear();
  }

  #end(run, { exitCode = null, signal = null, spawnError = null }) {
    if (run.ended) return;
    run.ended = true;
    this.#settle(run, "the connection ended");
    if (this.run !== run) return;
    this.run = null;
    const why = run.problem ?? whyEnded({ entry: this.entry, exitCode, signal, stderr: run.stderr, bye: run.bye, spawnError, sshConfig: this.options.sshConfig, tilde: this.tilde });
    this.#ended(why);
  }

  #ended(why) {
    this.error = { code: why.code, message: why.message, ...(why.fix ? { fix: why.fix } : {}) };
    this.nextTryAt = null;
    if (!this.stopped && !this.oneShot) {
      const { min, max } = this.backoff;
      const delay = why.state === "error" ? max : Math.min(max, min * 2 ** this.attempt) * (0.75 + this.random() * 0.5);
      this.attempt = Math.min(this.attempt + 1, 16);
      this.nextTryAt = this.clock.now() + Math.round(delay);
      this.retryTimer = this.clock.setTimeout(() => this.#connect(), delay);
      this.retryTimer?.unref?.();
    }
    this.#set(why.state);
  }

  #set(state) {
    if (state === this.state && state !== "offline" && state !== "error") return;
    const changed = state !== this.state;
    this.state = state;
    if (changed) this.since = this.clock.now();
    if (state === "live" || state === "connecting") this.nextTryAt = null;
    this.emit("state", this.info());
  }
}
