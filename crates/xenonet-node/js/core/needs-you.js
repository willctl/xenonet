// core/needs-you.js in Rust (crates/xenonet-state/src/needs_you.rs): what
// waits on you, as one queue. The page keeps the JS, which it imports
// itself; the defaults stay here, as the JS takes its argument.
import { native } from "../index.js";

export const KINDS = native.needsYouKinds();

export const waitsWords = (it) => native.waitsWords(it);

export function needsYouQueue({ sessions = [], actors = [], requests = [], gatewayAsks = [], canDecide = false, waiting = 0 } = {}) {
  return native.needsYouQueue({ sessions, actors, requests, gatewayAsks, canDecide, waiting });
}

export const order = (a, b) => native.needsYouOrder(a, b);

export const queueOfSummary = (summary) => native.queueOfSummary(summary);

export const countOf = (items) => native.countOf(items);
