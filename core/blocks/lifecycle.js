// What every building block has: a name and a kind, start and stop, and its
// health, which /api/health lists and the doctor's xenonet.blocks check reads.
//
//   state      idle       made, not started or used yet
//              running    working, and the last thing it did went well
//              degraded   working, but the last thing it did had a problem
//                         (it serves what it had before, or less than all)
//              failed     not working: the last thing it did failed, and it
//                         has nothing to serve
//              stopped    stopped
//   since      when it came to be in that state
//   lastOk     when it last did its work well
//   lastError  { at, message } of the last problem, or null; it stays after
//              the block recovers, and `state` says whether it's current
//   counters   numbers the kind of block keeps (runs, lines, errors, …)
//   budget     what it may spend (folders, depth, time) and what it spent
//              last, for the blocks that have one; null for the rest
//
// A block that's used without being started (a source run on demand)
// counts as running from its first success.
import { EventEmitter } from "node:events";
import { systemClock } from "./clock.js";

export const STATES = ["idle", "running", "degraded", "failed", "stopped"];

export const messageOf = (err) => String(err?.message ?? err);

export class Block extends EventEmitter {
  constructor({ name, kind, clock = systemClock } = {}) {
    super();
    if (!name || !kind) throw new Error("a block needs a name and a kind");
    this.name = name;
    this.kind = kind;
    this.clock = clock;
    this.state = "idle";
    this.since = clock.now();
    this.lastOk = null;
    this.lastError = null;
    this.counters = { errors: 0 };
    this.budget = null;
    this.started = false;
  }

  // Each start() and stop() moves this on, so a start can tell whether it
  // still stands (see current()).
  #run = 0;

  // Starts the work, through the kind's onStart(run). Starting what's
  // already started does nothing. A start that throws leaves the block
  // failed, and one that reports a problem leaves it degraded, the first
  // start or one after a stop alike: a stopped block is idle again while
  // it starts, since a stopped one stays stopped whatever it reports.
  // Stopped before its start is done, it stays stopped.
  async start() {
    if (this.started) return this;
    this.started = true;
    const run = ++this.#run;
    if (this.state === "stopped") this.setState("idle");
    try {
      await this.onStart?.(run);
    } catch (err) {
      if (this.current(run)) this.fail(err);
      throw err;
    }
    if (this.current(run) && this.state === "idle") this.setState("running");
    return this;
  }

  // Whether the start that's `run` still stands: the block wasn't stopped,
  // or stopped and started again, since. A kind whose onStart() awaits
  // asks after each await, and gives up when it doesn't.
  current(run) {
    return this.started && run === this.#run;
  }

  // Stops the work, through the kind's onStop(). Synchronous, so it can run
  // as the process exits.
  stop() {
    this.started = false;
    this.#run++;
    try {
      this.onStop?.();
    } finally {
      this.setState("stopped");
    }
  }

  setState(state, at = this.clock.now()) {
    if (state === this.state) return;
    const from = this.state;
    this.state = state;
    this.since = at;
    this.emit("state", { name: this.name, from, to: state, at });
  }

  // The work went well: running again after a problem.
  ok(at = this.clock.now()) {
    this.lastOk = at;
    if (this.state !== "running" && this.state !== "stopped") this.setState("running", at);
  }

  // A problem the block works around: degraded until the next success.
  problem(err, at = this.clock.now()) {
    this.#error(err, at);
    if (this.state !== "stopped") this.setState("degraded", at);
  }

  // A problem the block can't work around: failed until the next success.
  fail(err, at = this.clock.now()) {
    this.#error(err, at);
    if (this.state !== "stopped") this.setState("failed", at);
  }

  #error(err, at) {
    this.lastError = { at, message: messageOf(err) };
    this.count("errors");
  }

  count(name, n = 1) {
    this.counters[name] = (this.counters[name] ?? 0) + n;
  }

  health() {
    return {
      state: this.state,
      since: this.since,
      lastOk: this.lastOk,
      lastError: this.lastError && { ...this.lastError },
      counters: { ...this.counters },
      budget: this.budget ? structuredClone(this.budget) : null,
    };
  }
}

// The blocks a part of XenoNet is made of, by name, for health reports. The
// blocks' owners start and stop them; this only lists them.
export class Blocks {
  #list = [];
  // Block name → the source of the map it reads for (XenoNetState.sources),
  // so the doctor doesn't report a source's failure twice.
  #sources = new Map();

  // Adds a block, and gives it back. `source`: the source of the map it
  // reads for, as /api/health's sources name it, if any.
  add(block, { source = null } = {}) {
    if (this.get(block.name) && this.get(block.name) !== block) throw new Error(`there's already a block called ${block.name}`);
    if (!this.#list.includes(block)) this.#list.push(block);
    if (source) this.#sources.set(block.name, source);
    return block;
  }

  remove(name) {
    this.#list = this.#list.filter((b) => b.name !== name);
    this.#sources.delete(name);
  }

  get = (name) => this.#list.find((b) => b.name === name) ?? null;
  list = () => [...this.#list];
  [Symbol.iterator]() {
    return this.#list[Symbol.iterator]();
  }

  // Each block's health, with its name and kind, and the source it reads
  // for when it has one.
  health() {
    return this.#list.map((b) => {
      const source = this.#sources.get(b.name);
      return { name: b.name, kind: b.kind, ...(source ? { source } : {}), ...b.health() };
    });
  }
}
