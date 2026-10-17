// core/actors/hermes.js in Rust (crates/xenonet-actors/src/hermes.rs):
// Hermes Agent's observer, one actor per profile. Its look is Rust's: what
// Hermes's processes, ledger, gateways, tickers, jobs and session
// databases say, its presence, status, events and the sessions it started.
// Rust calls the actors, the files, procs and the observer's own keep,
// say, touch, gather and happened through the observer, and Hermes's
// state.db through `db`, opened here.
import fs from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { Observer } from "./observer.js";
import { Spawns, SPAWNS_CORE } from "./attribution.js";
import { PRODUCTS } from "./products.js";
import { hermesSlug } from "./names.js";
import { hermesRoot, stateDbOf, vendor } from "./hermes-files.js";
import { native } from "../../index.js";
import { drive, calls } from "../../machine.js";

const PRODUCT = PRODUCTS["hermes-agent"];
export const HERMES_HEARTBEAT_MS = native.hermesHeartbeatMs();
export const profileSlug = hermesSlug;
export const doingOf = (description) => native.hermesDoingOf(description) ?? null;
export const endingOf = (reason) => native.hermesEndingOf(reason) ?? null;

const CORE = Symbol("xenonet.hermes");

export class HermesObserver extends Observer {
  constructor({ actors, procs, home, env = process.env, files = fs, open, observing, every, clock } = {}) {
    super({ name: "hermes-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.root = hermesRoot(home, env);
    this.files = files;
    this.open = open;
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
    // What the readers of Hermes's files call (hermes-files.js).
    this.vendor = vendor;
    this.db = stateDbOf((f) => (this.open ? this.open(f) : new DatabaseSync(f, { readOnly: true })));
    this[CORE] = new native.HermesCore(this.spawns[SPAWNS_CORE], this.root, String(home ?? ""));
  }

  dirs(dir) {
    return this.files.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name);
  }
  dateParse(s) {
    return Date.parse(s);
  }

  async look(at = this.clock.now()) {
    return drive(this[CORE].look(at), calls(this));
  }

  status() {
    return this[CORE].status(this);
  }
}
