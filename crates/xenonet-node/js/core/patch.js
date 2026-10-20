// core/patch.js in Rust (crates/xenonet-base/src/patch.rs): an apply_patch
// text read for the files it touches.
import { native } from "../index.js";

export const patchHeaders = (text) => native.patchHeaders(String(text));

export const patchPaths = (text, { moves = true } = {}) => native.patchPaths(String(text), !!moves);
