// core/delivery/selector.js in Rust (crates/xenonet-engine/src/delivery/selector.rs):
// selectors, which sessions a message goes to or which ones a caller may
// see, and what they match.
import { native } from "../../index.js";

export const KEYS = ["project", "identity", "app", "machine", "status", "by"];

export const parseSelector = (text) => native.selParse(text);
export const isSelector = (text) => native.selIs(text);
export const formatSelector = (sel) => native.selFormat(sel ?? null);
export const matches = (sel, f) => native.selMatches(sel ?? null, f);
export const within = (inner, outer) => native.selWithin(inner, outer ?? null);
