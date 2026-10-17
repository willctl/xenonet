// core/actors/grok-bot.js in Rust (crates/xenonet-actors/src/grok_bot.rs):
// Grok Bot's observer, presence and process ancestry. Its look and its
// poll are Rust's, calling the actors, its app, procs, its Spawns and the
// observer's own keep, say, gather, touch and happened through the
// observer; the poll's timer stays here.
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { Spawns, SPAWNS_CORE } from "./attribution.js";
import { PRODUCTS } from "./products.js";
import { AppBundle } from "./app-bundle.js";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

const PRODUCT = PRODUCTS["grok-bot"];
const BUNDLE = "Grok Bot.app";
const P = native.observerProducts();
export const BUNDLE_ID = new RegExp(P.grokBundleId);
export const POLL_MS = P.grokPollMs;

// Whether a process still runs: signal 0 only checks.
const running = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
};

const CORE = Symbol("xenonet.grok-bot");

export class GrokBotObserver extends Observer {
  constructor({ actors, procs, home, apps = ["/Applications", path.join(home ?? "", "Applications")].map((d) => path.join(d, BUNDLE)), files = fs, alive = running, binary, pollMs = POLL_MS, observing, every, clock } = {}) {
    super({ name: "grok-bot-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.alive = alive;
    this.app = new AppBundle({ apps, id: BUNDLE_ID, files, ...(binary ? { binary } : {}) });
    this.pollMs = pollMs;
    this.id = `actor:${PRODUCT.slug}`;
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
    this[CORE] = new native.GrokBotCore(this.spawns[SPAWNS_CORE]);
    this.pollTimer = null;
    this.polling = null;
  }

  // The app's commands and what they run, by pid, and the app's own
  // processes: copies, for the tests that look in.
  get tracked() {
    return new Map(this[CORE].tracked());
  }
  get appPids() {
    return new Set(this[CORE].appPids());
  }

  async look(at = this.clock.now()) {
    return drive(this[CORE].look(at), calls(this));
  }

  // Every 3 seconds while the app runs; one under way is the poll.
  async poll(at = this.clock.now()) {
    if (!this[CORE].mayPoll(this) || this.polling) return;
    this.polling = drive(this[CORE].poll(at), calls(this))
      .catch((err) => this.problem(err))
      .finally(() => (this.polling = null));
    return this.polling;
  }

  onStart() {
    super.onStart();
    const run = async () => {
      if (!this.started) return;
      await this.poll();
      if (!this.started) return;
      this.pollTimer = this.clock.setTimeout(run, this.pollMs);
      this.pollTimer?.unref?.();
    };
    this.pollTimer = this.clock.setTimeout(run, this.pollMs);
    this.pollTimer?.unref?.();
  }

  onStop() {
    super.onStop();
    this.clock.clearTimeout(this.pollTimer);
    this.pollTimer = null;
  }

  status() {
    return this[CORE].status(this);
  }
}
