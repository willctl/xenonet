// core/actors/observers.js in Rust (crates/xenonet-actors/src/observers.rs):
// the observers of the actor products XenoNet observes, looking together.
// Which observers there are, and their timer, stay here; the look and the
// status are Rust's.
import { systemClock } from "../../../../../core/blocks/clock.js";
import { LOOK_MS } from "./observer.js";
import { HermesObserver } from "./hermes.js";
import { GrokBotObserver } from "./grok-bot.js";
import { MuseObserver } from "./muse.js";
import { ReportedSpawns } from "./attribution.js";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";

export function createObservers({ actors, registration = null, procs, home, env = process.env, clock = systemClock, hermes = {}, grokBot = {}, muse = {} } = {}) {
  if (!actors || actors.demo) return null;
  const observing = (product) => () => registration?.observing(product) ?? true;
  const list = [
    new HermesObserver({ actors, procs, home, env, observing: observing("hermes-agent"), every: null, clock, ...hermes }),
    new GrokBotObserver({ actors, procs, home, observing: observing("grok-bot"), every: null, clock, ...grokBot }),
    new MuseObserver({ actors, procs, home, observing: observing("muse"), every: null, clock, ...muse }),
  ];
  const reported = new ReportedSpawns({ actors, procs });
  const deps = { list, reported, actors, clock };
  let timer = null;
  let running = false;
  const look = () => drive(native.observersLook(), calls(deps));
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
    status: () => native.observersStatus(deps),
  };
}
