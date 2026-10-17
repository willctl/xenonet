// core/actors/reports.js in Rust (crates/xenonet-actors/src/reports.rs):
// what a registered actor reports, taken into the registry a part at a
// time. Every check and drop is Rust's; the registry, the actor's
// processes and their lineage are called as the JS called them.
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { ActorError } from "./registration.js";

export function takeHeartbeat(actors, id, body, { now = actors.now() } = {}) {
  try {
    native.reportsHeartbeat({ actors }, id, body, now);
  } catch (err) {
    if (err?.xenonetActor) throw new ActorError(err.status, err.code, err.message, err.extra);
    throw err;
  }
}

// The parts a takeEvents or takeTargets reads besides the registry: the
// actor's processes (a Set) as given, how to read a process's lineage, the
// indexes already dropped, and the registry's awaitSession with `own`.
const depsOf = (actors, { own, lineage, skip }, removeList = () => []) => ({
  actors,
  own,
  lineage: lineage ? (pid) => lineage(pid) : lineage,
  skip,
  awaitSession: (id, app, place, at) => actors.awaitSession(id, { app, place, own, at }),
  removeList,
});

export async function takeEvents(actors, id, events, { now = actors.now(), own = null, lineage = null, skip = null } = {}) {
  const list = Array.from(events.entries(), ([, e]) => e);
  return drive(native.reportsEvents(id, list, now), calls(depsOf(actors, { own, lineage, skip })));
}

export async function takeTargets(actors, id, { targets = [], remove = [], replace = false } = {}, { now = actors.now(), own = null, lineage = null, skip = null } = {}) {
  const list = Array.from(targets.entries(), ([, t]) => t);
  return drive(native.reportsTargets(id, list, !!replace, now), calls(depsOf(actors, { own, lineage, skip }, () => [...remove])));
}
