// core/actors/adapters.js in Rust (crates/xenonet-actors/src/adapters.rs):
// remote provider adapters, which transports each product reaches XenoNet
// through, and what its provider can prove (nothing, for any of them yet).
import { native } from "../../index.js";

export const TRANSPORTS = Object.fromEntries(native.adapterTransports().map((t) => [t.id, t]));

const unavailable = (reason) => async () => ({ state: "unavailable", reason });

export const ADAPTERS = Object.fromEntries(
  native.adapterList().map(({ key, unavailable: why, capabilities, ...a }) => [key, { ...a, verifyAccount: unavailable(why), verifyActor: unavailable(why), capabilities: () => [...capabilities] }]),
);

export function adapterFor(product, adapters = ADAPTERS) {
  const list = Object.values(adapters);
  const i = native.adapterFor(product, list.map((a) => a?.product), adapters.custom != null);
  return i === -2 ? adapters.custom : i === -1 ? null : list[i];
}

// What an adapter's verification said, checked in Rust: its own answer
// when it stands, else what it comes to.
export async function verified(fn, evidence) {
  let out;
  try {
    out = await fn(evidence);
  } catch {
    return native.adapterVerified(undefined, true);
  }
  return native.adapterVerified(out, false) ?? out;
}
