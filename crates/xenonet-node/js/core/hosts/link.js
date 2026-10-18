// core/hosts/link.js in Rust (crates/xenonet-hosts/src/link.rs): one host's
// link, the console's end of `ssh <host> xenonet relay`. Its state, what a
// frame from the host may be, the clock offset, the backoff and why a
// connection ended are the Rust's, which answers each thing that happens
// with what to do: start the program, write to it, end it, emit, set or
// clear a timer, settle a get. The program, its pipes, the timers and the
// gets' promises are here. Everything a host sends is untrusted, and the
// Rust checks it as the JS did.
import { EventEmitter } from "node:events";
import { spawn as childSpawn } from "node:child_process";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { settle } from "../../settle.js";
import fs from "node:fs";
import { pathStyle } from "../paths.js";
import { xenonetBin } from "../components/runtime.js";
import { MAX_FRAME, lineSplitter } from "./frames.js";

const { systemClock } = await import(peer("blocks/clock.js"));
const { check, explain } = await import(peer("schema/check.js"));
const { HostHello } = await import(peer("schema/host.js"));

const C = native.hostsLinkConstants();

export const HELLO_MS = C.helloMs;
export const STALE_MS = C.staleMs;
export const BACKOFF = { min: C.backoff.min, max: C.backoff.max };
export const SSH_OPTIONS = C.sshOptions;

// The JS service in a checkout with no binary built yet (npm test, before
// cargo build) runs the checkout's own relay on this Node.
function localRun({ root, env, bin }) {
  if (bin != null) return { bin: String(bin), entry: null };
  const built = xenonetBin({ env, root });
  const js = root ? pathStyle(root).join(String(root), "cli", "xenonet.js") : null;
  return !fs.existsSync(built) && js && fs.existsSync(js) ? { bin: process.execPath, entry: js } : { bin: built, entry: null };
}

const optionsOf = ({ sshBin = "ssh", sshConfig = null, root, env = process.env, bin = null } = {}) => ({ sshBin: [].concat(sshBin).map(String), sshConfig: sshConfig == null ? null : String(sshConfig), ...localRun({ root, env, bin }), env: { ...env } });

export const transport = (entry, options = {}) => settle(native.hostsTransport(entry, optionsOf(options)));

export const shellPath = (p) => native.hostsShellPath(String(p));

export const trustCommand = (id, sshConfig, tilde = (p) => p) => native.hostsTrustCommand(String(id), sshConfig ? String(sshConfig) : null, (p) => String(tilde(p)));

const spawnText = (err) => (err ? String(err.code ?? err.message) : null);

export function whyEnded({ entry, exitCode = null, signal = null, stderr = "", bye = null, spawnError = null, sshConfig = null, tilde = (p) => p }) {
  return native.hostsWhyEnded({ entry, exitCode, signal, stderr: String(stderr), bye, spawnError: spawnText(spawnError), sshConfig: sshConfig ? String(sshConfig) : null }, (p) => String(tilde(p)));
}

// What the schema says is wrong with a hello, or null.
const checkHello = (frame) => {
  const checked = check(HostHello, frame);
  return checked.ok ? null : explain(checked);
};

// Timers whose names start so are intervals; the rest are timeouts. The
// stale check, the retry and the time asks don't keep XenoNet running.
const INTERVALS = /^(stale|time:\d+)$/;
const UNREF = /^(stale|retry|time:\d+)$/;

export class HostLink extends EventEmitter {
  #core;
  #timers = new Map();
  #children = new Map();
  #gets = new Map();
  #unstarted = new Set();

  constructor({ entry, sshConfig = null, sshBin = "ssh", root, env = process.env, bin = null, spawn = childSpawn, clock = systemClock, random = Math.random, helloMs = HELLO_MS, staleMs = STALE_MS, backoff = BACKOFF, getMs = C.getMs, demoOnly = false, once = false, tilde = (p) => p }) {
    super();
    this.options = { sshConfig, sshBin, root, env, bin };
    Object.assign(this, { spawn, clock, random, tilde });
    this.#core = new native.HostsLink({ ...entry }, optionsOf(this.options), { helloMs, staleMs, backoff: { min: backoff?.min, max: backoff?.max }, getMs, demoOnly: Boolean(demoOnly), once: Boolean(once) }, clock.now());
  }

  get entry() {
    return this.#core.entry;
  }
  get state() {
    return this.#core.state;
  }
  get hello() {
    return this.#core.hello;
  }
  get lastHeard() {
    return this.#core.lastHeard;
  }
  set lastHeard(at) {
    this.#core.lastHeard = Number.isFinite(at) ? at : null;
  }
  get stopped() {
    return this.#core.stopped;
  }
  get getMs() {
    return this.#core.getMs;
  }
  get error() {
    return this.#core.info().error;
  }

  #asks() {
    return [() => Number(this.random()), checkHello, (p) => String(this.tilde(p))];
  }

  start() {
    this.#do(this.#core.start(this.clock.now(), ...this.#asks()));
    return this;
  }

  stop() {
    this.#do(this.#core.stop(this.clock.now()));
  }

  pause(message = "taken offline") {
    this.#do(this.#core.pause(String(message), this.clock.now()));
  }

  resume() {
    return this.start();
  }

  reconnect() {
    this.#do(this.#core.reconnect(this.clock.now(), ...this.#asks()));
    return this;
  }

  get(route, { timeoutMs = this.getMs } = {}) {
    const r = this.#core.get(route, Number(timeoutMs));
    if (r.error) return Promise.reject(Object.assign(new Error(r.error.message), { status: r.error.status }));
    return new Promise((resolve, reject) => {
      this.#gets.set(r.id, { resolve, reject, ms: Number(timeoutMs) });
      this.#do(r.effects);
    });
  }

  info() {
    return this.#core.info();
  }

  // ---------- what the Rust says to do ----------

  #do(effects) {
    for (const fx of effects) {
      switch (fx.do) {
        case "spawn":
          this.#spawn(fx);
          break;
        case "write":
          try {
            this.#children.get(fx.run)?.stdin.write(fx.text);
          } catch {}
          break;
        case "kill": {
          const child = this.#children.get(fx.run);
          if (child && child.exitCode == null && !child.killed) child.kill("SIGTERM");
          break;
        }
        case "fail": {
          const child = this.#children.get(fx.run);
          if (child && child.exitCode == null && !child.killed) child.kill("SIGTERM");
          else this.#end(fx.run, {});
          break;
        }
        case "emit":
          this.emit(fx.name, fx.data);
          break;
        case "timer":
        case "interval":
          // A program that couldn't start has no hello to wait for.
          if (fx.name.startsWith("hello:") && this.#unstarted.delete(Number(fx.name.slice(6)))) break;
          this.#set(fx.name, fx.ms);
          break;
        case "clear":
          this.#clear(fx.name);
          break;
        case "resolve": {
          const g = this.#gets.get(fx.id);
          this.#gets.delete(fx.id);
          g?.resolve(fx.value);
          break;
        }
        case "reject": {
          const g = this.#gets.get(fx.id);
          this.#gets.delete(fx.id);
          g?.reject(Object.assign(new Error(fx.message), { status: fx.status }));
          break;
        }
      }
    }
  }

  #set(name, ms) {
    this.#clear(name);
    const interval = INTERVALS.test(name);
    const fire = () => {
      if (!interval) this.#timers.delete(name);
      this.#fire(name);
    };
    const handle = interval ? this.clock.setInterval(fire, ms) : this.clock.setTimeout(fire, ms);
    if (UNREF.test(name)) handle?.unref?.();
    this.#timers.set(name, { handle, interval });
  }

  #clear(name) {
    const t = this.#timers.get(name);
    if (!t) return;
    this.#timers.delete(name);
    if (t.interval) this.clock.clearInterval(t.handle);
    else this.clock.clearTimeout(t.handle);
  }

  #fire(name) {
    const now = this.clock.now();
    const [kind, n] = name.split(":");
    if (kind === "stale") this.#do(this.#core.checkStale(now));
    else if (kind === "retry") this.#do(this.#core.retry(now, ...this.#asks()));
    else if (kind === "hello") this.#do(this.#core.helloTimeout(Number(n), now, ...this.#asks()));
    else if (kind === "time") this.#do(this.#core.timeTick(Number(n), now));
    else if (kind === "get") this.#do(this.#core.getTimeout(Number(n), this.#gets.get(Number(n))?.ms ?? this.getMs));
  }

  #spawn({ run, file, args, env }) {
    let child;
    try {
      child = this.spawn(file, args, { stdio: ["pipe", "pipe", "pipe"], env, windowsHide: true });
    } catch (err) {
      this.#unstarted.add(run);
      return this.#end(run, { spawnError: err });
    }
    this.#children.set(run, child);
    child.stdout.on("data", lineSplitter({ max: MAX_FRAME, onLine: (l) => this.#do(this.#core.line(run, l, this.clock.now(), ...this.#asks())), onDrop: () => this.#core.tooBig(run, this.clock.now()) }));
    child.stderr.on("data", (d) => this.#core.stderr(run, String(d)));
    child.stdin.on("error", () => {});
    child.on("error", (err) => {
      // A program that couldn't start has no exit to wait for.
      if (!child.pid) this.#end(run, { spawnError: err });
    });
    child.on("close", (exitCode, signal) => {
      this.#end(run, { exitCode, signal });
      if (this.#children.get(run) === child) this.#children.delete(run);
    });
  }

  #end(run, { exitCode = null, signal = null, spawnError = null }) {
    this.#do(this.#core.end(run, { exitCode: Number.isFinite(exitCode) ? exitCode : null, signal: signal == null ? null : String(signal), spawnError: spawnText(spawnError) }, this.clock.now(), ...this.#asks()));
  }
}
