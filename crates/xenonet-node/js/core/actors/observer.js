// core/actors/observer.js in Rust (crates/xenonet-actors/src/observer.rs):
// what every observer of an actor product shares. What it said, touched and
// recorded, and the minutes it gathered, are Rust's; Rust calls the actors,
// the product, the clock and the observer's own observing(), look(), ok()
// and problem() through the observer, as the JS did. The twin keeps the
// Block it is and its timer.
import { native } from "../../index.js";
import { drive, calls } from "../../machine.js";
import { Block } from "../blocks/lifecycle.js";

const C = native.observerConstants();
export const LOOK_MS = C.lookMs;
export const KEEPALIVE_MS = C.keepaliveMs;

const CORE = Symbol("xenonet.observer");

// procs.census() as Rust reads it: the table's [pid, row] pairs, and what
// each process is.
export async function readCensus(procs) {
  const c = await procs.census();
  const table = [...c.table];
  return { table, of: table.map(([, p]) => c.of(p)) };
}

export class Observer extends Block {
  constructor({ name, product, actors, observing = () => true, every = LOOK_MS, clock }) {
    super({ name, kind: "observer", clock });
    this.product = product;
    this.actors = actors;
    this.observing = observing;
    this.every = every;
    this[CORE] = new native.ObserverCore();
    this.timer = null;
    this.looking = null;
  }

  keep(id, options) {
    this[CORE].keep(this, id, options);
  }
  say(id, options) {
    return this[CORE].say(this, id, options);
  }
  happened(id, options) {
    return this[CORE].happened(this, id, options) ?? null;
  }
  gather(id, options) {
    this[CORE].gather(id, options);
  }
  flush(now = this.clock.now()) {
    this[CORE].flush(this, now);
  }
  touch(id, nodeId, relation, at = this.clock.now()) {
    this[CORE].touch(this, id, nodeId, relation, at);
  }

  // The census, with what each of its processes is, for a look in Rust.
  async __census() {
    return readCensus(this.procs);
  }

  // One look (look(), each kind's own), unless its observer is off or the
  // map isn't there yet; one under way is the look.
  async tick(at = this.clock.now()) {
    if (!this[CORE].mayLook(this)) return false;
    if (this.looking) return this.looking;
    this.looking = drive(this[CORE].tick(at), calls(this)).finally(() => (this.looking = null));
    return this.looking;
  }

  onStart() {
    if (!this.every) return;
    const run = async () => {
      if (!this.started) return;
      await this.tick();
      if (!this.started) return;
      this.timer = this.clock.setTimeout(run, this.every);
      this.timer?.unref?.();
    };
    this.timer = this.clock.setTimeout(run, 0);
    this.timer?.unref?.();
  }

  onStop() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
