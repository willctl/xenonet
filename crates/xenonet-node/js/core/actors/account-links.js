// core/actors/account-links.js in Rust (crates/xenonet-actors/src/account_links.rs):
// how sure XenoNet is of the account a remote actor works under, kept in
// the engine's store. The twin keeps the LinkError class programs test for.
import { native } from "../../index.js";
import { STORE } from "../engine/store.js";

export class LinkError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

const run = (fn) => {
  try {
    return fn();
  } catch (err) {
    if (err?.xenonetLink) throw new LinkError(err.code, err.message);
    throw err;
  }
};

export function assertLink(store, { slug, identity, provider, known, now = Date.now() }) {
  return run(() => native.linkAssert(store[STORE], slug, identity, provider, known ? [...known] : undefined, now));
}

export function recordVerified(store, { slug, facts, known = new Set(), now = Date.now() }) {
  return run(() => native.linkRecordVerified(store[STORE], slug, facts, [...known], now));
}

export const linkState = (store, slug, now = Date.now()) => native.linkState(store[STORE], slug, now);

export function addExternalRef(store, { slug, adapterId, facts, now = Date.now() }) {
  return run(() => native.linkAddExternalRef(store[STORE], slug, adapterId, facts, now));
}
