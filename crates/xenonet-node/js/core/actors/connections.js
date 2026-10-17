// core/actors/connections.js in Rust (crates/xenonet-actors/src/connections.rs):
// remote actors' connections and their grants, over the engine's store.
// Every check, what a grant may read and how it stands are Rust's; the
// actors, the credentials' files (core/assistant/credentials.js) and the
// adapters are called as the JS called them. The twin keeps the events and
// the ConnectionError class programs test for.
import os from "node:os";
import { EventEmitter } from "node:events";
import { native } from "../../index.js";
import { STORE } from "../engine/store.js";
import { ADAPTERS, adapterFor } from "./adapters.js";
import { LinkError } from "./account-links.js";
import * as disk from "../../../../../core/assistant/credentials.js";

const C = native.connectionsConstants();
export const GRANT_DAYS = C.grantDays;
export const DEFAULT_GRANT_DAYS = C.defaultGrantDays;
export const LOOKBACK_DAYS = C.lookbackDays;
export const REMOTE_ACTORS_MAX = C.remoteActorsMax;
export const CONNECTIONS_MAX = C.connectionsMax;
export const PROJECTS_MAX = C.projectsMax;

export class ConnectionError extends Error {
  constructor(status, code, message, extra = {}) {
    super(message);
    Object.assign(this, { status, code, extra });
  }
}

export const fingerprintOf = ({ stateDir, host = os.hostname(), user = os.userInfo().username }) => native.connectionsFingerprint(host, user, stateDir);

export const launchFor = (grant, { port = 47840 } = {}) => native.connectionsLaunch(grant, port);

// A call into Rust, with its refusals as the classes they are.
const run = (fn) => {
  try {
    return fn();
  } catch (err) {
    if (err?.xenonetConnection) throw new ConnectionError(err.status, err.code, err.message, err.extra);
    if (err?.xenonetLink) throw new LinkError(err.code, err.message);
    throw err;
  }
};

export function createConnections({ actors, store = actors.store, dir, fingerprint = "unchecked", now = () => actors.now(), adapters = ADAPTERS, describe = () => null, taken = () => false, port = 47840, credentialIo = {} }) {
  const events = new EventEmitter();
  const core = new native.ConnectionsCore(store[STORE], dir, fingerprint, port);
  // The adapter a pairing picked, whose capabilities it reads.
  let picked = null;
  const base = {
    actors,
    disk,
    now: () => now(),
    wall: () => Date.now(),
    emit: (name, what) => events.emit(name, what),
    describe: (place) => describe(place),
    taken: (slug) => taken(slug),
    adapterFor(product) {
      picked = adapterFor(product, adapters);
      return picked ? { id: picked.id, product: picked.product, provider: picked.provider, transports: picked.transports } : picked;
    },
    get picked() {
      return picked;
    },
    writeCredential: (credential) => disk.writeCredential(dir, credential, credentialIo),
  };
  // The parts one call adds.
  const withAlso = (also) => Object.assign(Object.create(base), also);

  actors.beforeRemove.push((slug) => run(() => core.beforeRemove(base, slug)));
  run(() => core.start(base));

  return {
    events,
    on: (...args) => events.on(...args),
    device: () => run(() => core.device(base)),
    idKey: () => run(() => core.idKey(base)),
    disabled: () => run(() => core.disabled(base)),
    pair: (ask = {}, { places = new Map(), identities = new Set() } = {}) => run(() => core.pair(withAlso({ places, identities }), ask)),
    update: (grantId, { projects, labels } = {}, { places = new Map() } = {}) => run(() => core.update(withAlso({ places }), grantId, { projects, labels })),
    rotate: (grantId) => run(() => core.rotate(base, grantId)),
    revoke: (grantId, { reason = "revoked" } = {}) => run(() => core.revoke(base, grantId, reason)),
    disable: () => run(() => core.disable(base)),
    enable: () => run(() => core.enable(base)),
    authenticate: ({ grant, token } = {}) => run(() => core.authenticate(base, { grant, token })),
    recheck: (grantId, revision) => run(() => core.recheck(base, grantId, revision)),
    policy: (s) => {
      const p = run(() => core.policy(base, s));
      return { ...p, places: new Map(p.places) };
    },
    used: (grantId, t = now()) => run(() => core.used(grantId, t)),
    bridge: (connectionId, delta) => run(() => core.bridge(base, connectionId, delta)),
    list: () => run(() => core.list(base)),
    view: (grantId) => run(() => core.view(base, grantId)),
    tick: (t = now()) => run(() => core.tick(base, t)),
    sync: (t = now()) => run(() => core.sync(base, t)),
    // Whether any grant could be used now: the private endpoint listens
    // only then (server/assistant-endpoint.js).
    anyActive: () => run(() => core.anyActive(base)),
    audit: (opts) => store.audit(opts),
    dir,
  };
}
