// core/engine/verify.js in Rust (crates/xenonet-engine/src/verify.rs):
// whether a request made in an agent's name comes from that agent, and
// the stricter checks for what's yours, the window's, an actor's and a
// registration's. Every check and verdict is Rust's, over the engine
// store's pins; procs and the clock are called as the JS called them.
import { native } from "../../index.js";
import { drive, calls } from "../../machine.js";
import { STORE } from "./store.js";

export const ownOf = (chain) => chain.slice(0, native.verifyOwnOf(chain));

export const actorRefusal = (chain, { product = null, pid = null, registering = false } = {}) => native.verifyActorRefusal(chain, product, pid, registering) ?? null;

export const bundleOf = (program) => native.verifyBundleOf(program);

export function createVerifier({ procs, store, home = null, now = () => Date.now() }) {
  const core = new native.VerifierCore(store[STORE], home);
  // The parts a check reads: procs, the clock, and the process on the
  // other end of this request's socket.
  const go = (machine, socket) => drive(machine, calls({ procs, now: () => now(), peerOf: () => procs.peer(socket) }));
  // What the checks have counted, read as they stand.
  const state = Object.defineProperties(
    {},
    Object.fromEntries(["checked", "refused", "unchecked", "unseen", "lastRefusal"].map((k) => [k, { enumerable: true, get: () => core.state()[k] }])),
  );
  return {
    check: (socket, agent, { hook = false } = {}) => go(core.check(agent, !!hook), socket),
    person: (socket) => go(core.person(), socket),
    window: (socket) => go(core.window(), socket),
    desktop: (socket) => go(core.desktop(socket != null), socket),
    actor: (socket, who = {}, { pid = null } = {}) => go(core.actor(who, pid), socket),
    register: (socket, { product = null, pid = null } = {}) => go(core.register(product, pid), socket),
    asker: (socket) => go(core.asker(), socket),
    peer: (socket) => go(core.peer(), socket),
    lineage: (pid) => go(core.lineage(pid), null),
    state,
  };
}
