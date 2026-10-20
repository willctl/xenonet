// core/actors/registration.js in Rust (crates/xenonet-actors/src/registration.rs):
// registering actors, their tokens, the gateway's asks and the limits. The
// decisions and what's kept are Rust's, over the engine's store; the actors
// and the clock are called as the JS called them. The twin keeps the
// events and the ActorError class programs test for.
import { EventEmitter } from "node:events";
import os from "node:os";
import { native } from "../../index.js";
import { STORE } from "../engine/store.js";

const C = native.registrationConstants();
export const REQUEST_MS = C.requestMs;
export const TOKEN_UNUSED_MS = C.tokenUnusedMs;
export const tokenExpired = (at, t) => native.tokenExpired(at ?? undefined, t);
export const REGISTERED_MAX = C.registeredMax;
export const WAITING_MAX = C.waitingMax;
export const REQUESTS_PER_HOUR = C.requestsPerHour;
export const CAPABILITIES = C.capabilities;
export const REVOKED = C.revoked;

// An error a program can test: an HTTP status, a code from the interface's
// table, a sentence for a person, and extra fields (suggest, reason,
// retryAfterMs).
export class ActorError extends Error {
  constructor(status, code, message, extra = {}) {
    super(message);
    Object.assign(this, { status, code, extra });
  }
}

export function createRegistration({ actors, now = actors.now, home = os.homedir() }) {
  const events = new EventEmitter();
  const core = new native.RegistrationCore(actors.store[STORE], String(home));
  let api;
  const deps = { actors, now: () => now(), observing: (product) => api.observing(product) };
  // A call into Rust: its answer, with what it told told, and an
  // ActorError as the class.
  const run = (fn) => {
    try {
      return fn();
    } catch (err) {
      if (err?.xenonetActor) throw new ActorError(err.status, err.code, err.message, err.extra);
      throw err;
    } finally {
      for (const [name, what] of core.takeEmits()) events.emit(name, what);
    }
  };
  api = {
    events,
    on: (...args) => events.on(...args),
    waiting: (t = now()) => run(() => core.waiting(deps, t)),
    register: (body, { checked = false, chain = null, cwd = null } = {}) => run(() => core.register(deps, body, checked, chain, cwd)),
    requests: () => run(() => core.requests(deps)),
    decide: (requestId, decision) => run(() => core.decide(deps, requestId, decision)),
    add: (body) => run(() => core.add(deps, body)),
    rotate: (slug) => run(() => core.rotate(deps, slug)),
    remove: (slug, { reason = "removed" } = {}) => run(() => core.remove(deps, slug, String(reason))),
    authenticate: (token) => run(() => core.authenticate(deps, token)),
    gateway: (slug, body = {}) => run(() => core.gateway(deps, slug, body)),
    askGateway: (slug, { checked = false, chain = null, cwd = null } = {}) => run(() => core.askGateway(deps, slug, checked, chain, cwd)),
    dropGateway: (slug) => run(() => core.dropGateway(deps, slug)),
    authenticateGateway: (token) => run(() => core.authenticateGateway(deps, token)),
    productOf: (auth) => run(() => core.productOf(deps, auth)),
    me: (auth, extra = {}) => run(() => core.me(deps, auth, extra)),
    tick: (t = now()) => run(() => core.tick(deps, t)),
    observers: () => run(() => core.observers(deps)),
    observing: (product) => run(() => core.observing(deps, product)),
    observe: (product, on) => run(() => core.observe(deps, product, on)),
    simulate: (kind, { a } = {}) => run(() => core.simulate(deps, kind, a)),
  };
  return api;
}
