// Muse's observer (docs/design/actors/muse.md; interface.md, sections 5
// and 7): presence, nothing more. Muse is Meta's personal agent, and it
// works in a cloud VM of its own whether or not its Mac app is open. What
// it does on this Mac (reading files and app data, driving apps, computer
// use) leaves nothing XenoNet could attribute to it without reading
// protected system data, which it won't. So:
//
//   idle      its app runs
//   unknown   its app isn't running: Muse goes on in the cloud with it
//             closed, and nothing here can say it stopped (heartbeatMs is
//             null), so it's never offline, and it stays listed for as
//             long as it's installed, as Grok Bot does
//
// Never working, since XenoNet can't see its work, and no events or targets
// beyond presence.
//
// The app is Meta's only when its bundle id says so (com.meta.endo):
// other Mac apps are called Muse. Its process is its main program,
// Contents/MacOS/Muse, as the process table has it and core/procs.js
// vouches for by that id; its crash reporter and updater don't keep it
// open. Of its files, XenoNet reads only whether the app is there, and two
// keys of its Info.plist, its version and bundle id: never its data,
// caches, web storage, preferences or sign-in, which muse.md (section 2)
// says to treat as secret.
//
// Muse doesn't report itself to XenoNet, and can't be paired as a remote
// actor: its custom connectors take an API key or OAuth for a service's
// hosts, not an MCP server (muse.md, section 8).
import fs from "node:fs";
import path from "node:path";
import { Observer } from "./observer.js";
import { PRODUCTS } from "./products.js";
import { AppBundle } from "./app-bundle.js";

const PRODUCT = PRODUCTS.muse;
const BUNDLE = "Muse.app";
export const BUNDLE_ID = /^com\.meta\.endo(\.|$)/;
const MAIN = `/${BUNDLE}/Contents/MacOS/Muse`;
const STRAY_MS = 60e3;
// What Muse can do, as Meta's docs have it: browse, message people (email
// and messages as you), keep scheduled tasks, and use the computer on a
// Mac with computer use turned on.
const CAPABILITIES = ["browse", "message-people", "schedule", "use-computer"];

export class MuseObserver extends Observer {
  // `apps`: where the app may be installed. `binary(file, key)`: how a key
  // is read from an Info.plist written as binary, which Muse's is.
  constructor({ actors, procs, home, apps = ["/Applications", path.join(home ?? "", "Applications")].map((d) => path.join(d, BUNDLE)), files = fs, binary, observing, every, clock } = {}) {
    super({ name: "muse-observer", product: PRODUCT, actors, observing, every, clock });
    this.procs = procs;
    this.home = home;
    this.app = new AppBundle({ apps, id: BUNDLE_ID, files, ...(binary ? { binary } : {}) });
    this.id = `actor:${PRODUCT.slug}`;
    this.wasInstalled = false;
    this.tableState = "ok";
  }

  async look(at = this.clock.now()) {
    const known = this.actors.has(this.id) && !this.actors.info(this.id)?.via.includes("demo");
    // Whether the app where apps go is Meta's, from its Info.plist (read
    // again only when it changes).
    await this.app.read();
    // Not installed where apps go and not seen before: the process table
    // is read only once a minute, for an app run from somewhere else.
    if (!known && !this.app.installed()) {
      if (at - (this.lookedAt ?? -Infinity) < STRAY_MS) return;
      this.lookedAt = at;
    }
    let census;
    try {
      census = await this.procs.census();
      this.tableState = this.procs.supported === false ? "unsupported" : "ok";
    } catch (err) {
      this.tableState = "unreadable";
      throw err;
    }
    const { table, of } = census;
    const open = [...table.values()].some((p) => of(p).product?.product === PRODUCT.id && of(p).product.role === "app" && String(p.program ?? "").endsWith(MAIN));
    const installed = open || this.app.installed();
    this.wasInstalled = installed;
    if (!installed && !known) return;
    const version = this.app.ours() ? this.app.plist?.version ?? null : null;
    this.keep(this.id, { name: PRODUCT.name, version, via: ["observed"], heartbeatMs: null, capabilities: CAPABILITIES, installed, at });
    if (!this.actors.enabled(this.id)) return;
    if (open) this.say(this.id, { presence: "idle", status: "Its app is open on this Mac; it works in Meta's cloud", at });
    else this.say(this.id, { presence: "unknown", status: "Its app on this Mac is closed; it may still work in Meta's cloud", at });
  }

  // What the doctor shows: whether the app is installed, and what this
  // observer reads: the processes, whether the app is there and Meta's, and
  // its version. Asked of where apps go each time, its observer on or off,
  // so the doctor can tell an observer you turned off from an app that
  // isn't here; one its last look saw running from elsewhere counts too.
  status() {
    return {
      product: PRODUCT.id,
      name: PRODUCT.name,
      installed: this.app.installed() || this.wasInstalled,
      sources: [{ name: "processes", file: null, state: this.tableState }, ...this.app.sources(this.home)],
      recognized: null,
    };
  }
}
