// core/blocks/lifecycle.js in Rust (crates/xenonet-blocks/src/lifecycle.rs):
// what every building block has. Its state and when it began, its last
// success and last error, whether it's started, and its counters live in
// Rust. This keeps the JS API: the EventEmitter a block is, which tells of
// each change of state, and the start that awaits the kind's own.
//
// A twin of a kind of block (scanner.js and the rest) passes its own Rust
// core, which has the same lifecycle under it.
import { EventEmitter } from "node:events";
import { systemClock } from "../../../../../core/blocks/clock.js";
import { native } from "../../index.js";

export const STATES = native.blockStates();

export const messageOf = (err) => String(err?.message ?? err);

// The Rust core under a block, and telling of its changes of state, for
// the twins of the kinds of block.
export const CORE = Symbol("xenonet.core");
export const EMIT = Symbol("xenonet.emit");

// A block's counters, as an object JS code reads and writes as before
// (this.counters.records++), held in Rust.
function countersOf(core) {
  const key = (k) => (typeof k === "string" ? k : null);
  return new Proxy(
    {},
    {
      get: (_, k) => (key(k) === null ? undefined : (core.counter(k) ?? undefined)),
      set: (_, k, v) => (key(k) !== null && core.setCounter(k, Number(v)), true),
      has: (_, k) => key(k) !== null && core.counter(k) != null,
      deleteProperty: (_, k) => (key(k) !== null && core.deleteCounter(k), true),
      ownKeys: () => core.counterNames(),
      getOwnPropertyDescriptor: (_, k) => {
        const v = key(k) === null ? null : core.counter(k);
        return v == null ? undefined : { value: v, writable: true, enumerable: true, configurable: true };
      },
    },
  );
}

export class Block extends EventEmitter {
  #counters = null;

  constructor({ name, kind, clock = systemClock, core = null } = {}) {
    super();
    if (!name || !kind) throw new Error("a block needs a name and a kind");
    this.name = name;
    this.kind = kind;
    this.clock = clock;
    Object.defineProperty(this, CORE, { value: core ?? new native.Lifecycle(clock.now()) });
    // A kind whose budget its core keeps (the scanner) reads it from there.
    if (!("budget" in this)) this.budget = null;
  }

  get state() {
    return this[CORE].state;
  }
  get since() {
    return this[CORE].since;
  }
  get lastOk() {
    return this[CORE].lastOk ?? null;
  }
  set lastOk(at) {
    this[CORE].lastOk = at ?? null;
  }
  get lastError() {
    return this[CORE].lastError ?? null;
  }
  set lastError(e) {
    this[CORE].setLastError(e ? { at: Number(e.at), message: String(e.message) } : null);
  }
  get started() {
    return this[CORE].started;
  }
  set started(v) {
    this[CORE].started = !!v;
  }
  get counters() {
    return (this.#counters ??= countersOf(this[CORE]));
  }
  set counters(c) {
    this[CORE].setCounters(c ?? {});
  }

  // Starts the work, through the kind's onStart(run). Starting what's
  // already started does nothing. Stopped before its start is done, it
  // stays stopped.
  async start() {
    if (this.started) return this;
    const run = this[CORE].beginStart(this.clock.now());
    this[EMIT]();
    try {
      await this.onStart?.(run);
    } catch (err) {
      if (this.current(run)) this.fail(err);
      throw err;
    }
    if (this.current(run) && this.state === "idle") this.setState("running");
    return this;
  }

  current(run) {
    return this[CORE].current(run);
  }

  // Synchronous, so it can run as the process exits.
  stop() {
    this[CORE].beginStop();
    try {
      this.onStop?.();
    } finally {
      this.setState("stopped");
    }
  }

  setState(state, at = this.clock.now()) {
    this[CORE].setState(String(state), at);
    this[EMIT]();
  }

  ok(at = this.clock.now()) {
    this[CORE].ok(at);
    this[EMIT]();
  }

  problem(err, at = this.clock.now()) {
    this[CORE].problem(messageOf(err), at);
    this[EMIT]();
  }

  fail(err, at = this.clock.now()) {
    this[CORE].fail(messageOf(err), at);
    this[EMIT]();
  }

  count(name, n = 1) {
    this[CORE].count(String(name), Number(n));
  }

  health() {
    const h = this[CORE].health();
    h.budget = this.budget ? structuredClone(this.budget) : null;
    return h;
  }

  [EMIT]() {
    for (const c of this[CORE].takeChanges()) this.emit("state", { name: this.name, from: c.from, to: c.to, at: c.at });
  }
}

// The blocks a part of XenoNet is made of, by name, for health reports.
// The names, their order and the source each block reads for are kept in
// Rust; the blocks themselves stay with their owners.
export class Blocks {
  #names = new native.Registry();
  #byName = new Map();

  // Adds a block, and gives it back. `source`: the source of the map it
  // reads for, as /api/health's sources name it, if any.
  add(block, { source = null } = {}) {
    this.#names.add(String(block.name), this.#byName.get(block.name) === block, source ?? null);
    this.#byName.set(block.name, block);
    return block;
  }

  remove(name) {
    this.#names.remove(String(name));
    this.#byName.delete(name);
  }

  get = (name) => this.#byName.get(name) ?? null;
  list = () => this.#names.names().map((n) => this.#byName.get(n));
  [Symbol.iterator]() {
    return this.list()[Symbol.iterator]();
  }

  // Each block's health, with its name and kind, and the source it reads
  // for when it has one.
  health() {
    return this.list().map((b) => {
      const source = this.#names.source(String(b.name));
      return { name: b.name, kind: b.kind, ...(source ? { source } : {}), ...b.health() };
    });
  }
}
