// core/pricing.js in Rust (crates/xenonet-base/src/pricing.rs): what a
// call cost at API prices on the day it was made.
import { native } from "../index.js";

export const PRICES_CHECKED = native.pricesChecked();

// Arguments as JS would read them: a string, and a time that's a number,
// except a missing one, which means now.
const str = (v) => String(v ?? "");
const time = (at) => (at === undefined ? undefined : Number(at));
const num = (v) => (v == null ? undefined : Number(v));

export const priorityTier = (tier) => native.priorityTier(str(tier));

export const plainModel = (model) => native.plainModel(str(model));

export const priceOf = (model, at) => native.priceOf(str(model), time(at)) ?? null;

export function costParts(model, at, u, { fast = false, priority = false } = {}) {
  const tokens = { input: num(u?.input), cacheRead: num(u?.cacheRead), cacheWrite5m: num(u?.cacheWrite5m), cacheWrite1h: num(u?.cacheWrite1h), output: num(u?.output) };
  return native.costParts(str(model), time(at), tokens, !!fast, !!priority) ?? null;
}

export const costOf = (model, at, u, opts) => costParts(model, at, u, opts)?.total ?? null;
