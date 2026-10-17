// core/actors/attribution.js in Rust (crates/xenonet-actors/src/attribution.rs):
// which sessions an observed actor started, from the processes' ancestry
// and then their pins or the map. What's kept and every decision are
// Rust's; the store, the map, procs and the actors are called as the JS
// called them, through the objects given here.
import { native } from "../../index.js";
import { drive, calls } from "../../machine.js";
import { readCensus } from "./observer.js";

const C = native.attributionConstants();
export const SPAWN_WAIT_MS = C.spawnWaitMs;
export const LIVED_MS = C.livedMs;

export function startedUnder({ table, of }, isRoot) {
  return native.attrStartedUnder([...table], (p) => of(p), (p) => !!isRoot(p));
}

// The Rust state under a Spawns, for an observer that keeps one.
export const SPAWNS_CORE = Symbol("xenonet.spawns");
const CORE = SPAWNS_CORE;

export class Spawns {
  constructor({ store, map, procs }) {
    this.store = store;
    this.map = map;
    this.procs = procs;
    this[CORE] = new native.SpawnsCore();
  }

  get pending() {
    return new Map(this[CORE].pending());
  }
  get done() {
    return new Set(this[CORE].done());
  }
  // The sessions matched, which an observer may add to.
  get claimed() {
    const core = this[CORE];
    return {
      add(id) {
        core.claim(String(id));
        return this;
      },
      has: (id) => core.claimed().includes(id),
      get size() {
        return core.claimed().length;
      },
      [Symbol.iterator]: () => core.claimed()[Symbol.iterator](),
    };
  }

  async saw(actor, list, at) {
    return drive(this[CORE].saw(String(actor), [...list], at), calls(this));
  }
  sessionOf(s) {
    return this[CORE].sessionOf(this, s) ?? null;
  }
  settle(at, { alive = null } = {}) {
    return this[CORE].settle(this, at, alive ?? undefined);
  }
}

export class ReportedSpawns {
  constructor({ actors, procs }) {
    this.actors = actors;
    this.procs = procs;
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
  }

  async look(at) {
    const procs = this.procs;
    const deps = {
      actors: this.actors,
      procs,
      store: this.spawns.store,
      map: this.spawns.map,
      // The census, with what each of its processes is.
      census: () => readCensus(procs),
    };
    return drive(native.reportedLook(this.spawns[CORE], at), calls(deps));
  }
}
