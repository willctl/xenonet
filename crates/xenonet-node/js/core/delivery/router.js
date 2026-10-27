// core/delivery/router.js in Rust (crates/xenonet-engine/src/delivery/router.rs):
// for each message and each session it's for, the fastest way in that's
// allowed, then whether it arrived. The records and every decision are
// Rust's; the twin keeps a JS object for each record, which is what
// route() and list() give and what the delivery events copy, and the
// timers. The engine, policy and ways in are called as the JS called them.
import { EventEmitter } from "node:events";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

export const ACK_MS = native.routerAckMs();

export class DeliveryRouter extends EventEmitter {
  #core = new native.RouterCore();
  #objs = new Map(); // key → its record's object, while Rust keeps it
  #deps;

  constructor({ engine, policy, channels = null, opencode = null, tmux = null, resume = null, notify = () => {}, usage = () => null, reach = () => ({}), now = () => Date.now(), ackMs = ACK_MS }) {
    super();
    Object.assign(this, { engine, policy, channels, opencode, tmux, resume, notify, usage, reach, now, ackMs });
    this.records = new Map(); // `${message}|${agent}` → record
    this.timers = new Map();
    this.#deps = {
      router: this,
      listed: (key, rec) => {
        const obj = { ...rec };
        this.#objs.set(key, obj);
        this.records.set(key, obj);
      },
      unlisted: (key) => this.records.delete(key),
      release: (key) => this.#objs.delete(key),
      sync: (key, rec) => Object.assign(this.#objs.get(key), rec),
      emit: (key) => this.emit("delivery", { ...this.#objs.get(key) }),
      arm: (key, ms) => {
        const timer = setTimeout(() => void this.#go(this.#core.expire(key)), ms);
        timer.unref?.();
        this.timers.set(key, timer);
      },
      clear: (key) => {
        clearTimeout(this.timers.get(key));
        this.timers.delete(key);
      },
    };
    engine.on("message", (m) => void this.route(m).catch((err) => this.emit("error", err)));
    engine.on("seen", ({ agent, ids }) => this.#core.seen(this.#deps, agent, [...ids]));
    engine.on("contact", ({ agent, via }) => via === "hook" && this.#core.contact(this.#deps, agent));
  }

  #go(machine) {
    return drive(machine, calls(this.#deps));
  }

  // Whether a session can be reached without hooks (an OpenCode run XenoNet
  // has a way into), so the engine counts it as reachable.
  reaches(id) {
    return this.#core.reaches(this.#deps, id);
  }

  // The OpenCode sessions you allowed XenoNet to wake.
  known() {
    return this.#core.known(this.#deps);
  }

  // What its own OpenCode says of an OpenCode session, or null.
  async describe(id) {
    return this.#go(this.#core.describe(id));
  }

  // Whether a session that ended can still be messaged.
  resumable(view) {
    return this.#core.resumable(this.#deps, view ?? null);
  }

  forkable(view) {
    return this.#core.forkable(this.#deps, view ?? null);
  }

  // A wake for a session's listening Stop hook to hand a message over.
  wake(from, agent) {
    return this.#core.wake(this.#deps, from, agent);
  }

  // How a message from `from` would reach `a` if sent now, or null.
  way(from, a) {
    return this.#core.way(this.#deps, from, a) ?? null;
  }

  list(limit = 100) {
    return [...this.records.values()].slice(-limit);
  }

  async route(m) {
    const keys = await this.#go(this.#core.route(m));
    return keys.map((k) => this.#objs.get(k) ?? this.records.get(k));
  }
}
