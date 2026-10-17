// core/actors/muse.js in Rust (crates/xenonet-actors/src/muse.rs): Muse's
// observer, presence alone. Its look is Rust's, calling the actors, its
// app, procs and the observer's own keep and say through the observer.
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { PRODUCTS } from "./products.js";
import { AppBundle } from "./app-bundle.js";
import { native } from "../../index.js";
import { drive, calls } from "../../machine.js";

const PRODUCT = PRODUCTS.muse;
const BUNDLE = "Muse.app";
export const BUNDLE_ID = new RegExp(native.observerProducts().museBundleId);

const CORE = Symbol("xenonet.muse");

export class MuseObserver extends Observer {
  constructor({ actors, procs, home, apps = ["/Applications", path.join(home ?? "", "Applications")].map((d) => path.join(d, BUNDLE)), files = fs, binary, observing, every, clock } = {}) {
    super({ name: "muse-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.app = new AppBundle({ apps, id: BUNDLE_ID, files, ...(binary ? { binary } : {}) });
    this.id = `actor:${PRODUCT.slug}`;
    this[CORE] = new native.MuseCore();
  }

  async look(at = this.clock.now()) {
    return drive(this[CORE].look(at), calls(this));
  }

  status() {
    return this[CORE].status(this);
  }
}
