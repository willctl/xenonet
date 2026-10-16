// The observers of the actor products XenoNet observes (products.js, and the
// decisions at merge point A): Hermes Agent's (hermes.js), Grok Bot's
// (grok-bot.js) and Muse's (muse.js). Each is on unless you've turned its
// product's observer off in the Actors panel or with xenonet actor observe
// (registration.observing), and reads nothing while it's off. There are
// none at all in the demo and test instances, whose actors are made up and
// whose machine XenoNet mustn't read, nor with XENONET_ACTORS=off, which turns
// the whole layer off.
//
// They look together every 5 seconds, one after the other, so they share
// one read of the process table (procs keeps it for a moment); Grok Bot's
// also looks at what its app runs every 3 seconds by itself. After them,
// the registered actors' spawned reports that named no session look for
// the session that starts under their processes (ReportedSpawns), with the
// same read, and only while one waits.
import { systemClock } from "../blocks/clock.js";
import { LOOK_MS } from "./observer.js";
import { HermesObserver } from "./hermes.js";
import { GrokBotObserver } from "./grok-bot.js";
import { MuseObserver } from "./muse.js";
import { NumbatObserver } from "./numbat.js";
import { ReportedSpawns } from "./attribution.js";

// The observers for `actors`, or null where there are none. `procs` is the
// process table (core/procs.js), shared with the process checks; `home`
// the folder the products keep their state under; `hermes`, `grokBot` and
// `muse`, anything more for each observer (where its app may be, say);
// `numbat`, { file }: the numbat bridge, off unless a file is named.
export function createObservers({ actors, registration = null, procs, home, env = process.env, clock = systemClock, hermes = {}, grokBot = {}, muse = {}, numbat = null } = {}) {
  if (!actors || actors.demo) return null;
  const observing = (product) => () => registration?.observing(product) ?? true;
  const list = [
    new HermesObserver({ actors, procs, home, env, observing: observing("hermes-agent"), every: null, clock, ...hermes }),
    new GrokBotObserver({ actors, procs, home, observing: observing("grok-bot"), every: null, clock, ...grokBot }),
    new MuseObserver({ actors, procs, home, observing: observing("muse"), every: null, clock, ...muse }),
    // numbat's records, only when you named its file (opt-in).
    ...(numbat?.file ? [new NumbatObserver({ actors, observing: observing("numbat"), every: null, clock, ...numbat })] : []),
  ];
  const reported = new ReportedSpawns({ actors, procs });
  let timer = null;
  let running = false;
  const look = async () => {
    for (const o of list) await o.tick();
    // Before XenoNet's first look at the map, no session could be named. A
    // look that fails leaves the reports as they were told.
    if (actors.map?.ready?.() === false) return;
    await reported.look(clock.now()).catch(() => {});
  };
  return {
    list,
    reported,
    look,
    start() {
      if (running) return this;
      running = true;
      for (const o of list) o.start();
      const run = async () => {
        if (!running) return;
        await look();
        if (!running) return;
        timer = clock.setTimeout(run, LOOK_MS);
        timer?.unref?.();
      };
      timer = clock.setTimeout(run, 0);
      timer?.unref?.();
      return this;
    },
    stop() {
      running = false;
      clock.clearTimeout(timer);
      timer = null;
      for (const o of list) o.stop();
    },
    // What the doctor shows of each: whether its product is installed and
    // its observer on, what it reads and whether it could, and the process
    // check's self-test.
    status: () => list.map((o) => ({ ...o.status(), on: o.observing() })),
  };
}
