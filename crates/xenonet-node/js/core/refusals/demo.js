// core/refusals/demo.js in Rust (crates/xenonet-sources/src/refusals/demo.rs):
// the demo's refusals, made up.
import { native } from "../../index.js";

export function demoRefusals(now = Date.now()) {
  return native.demoRefusals(Number(now));
}
