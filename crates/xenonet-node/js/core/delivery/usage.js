// core/delivery/usage.js in Rust (crates/xenonet-engine/src/delivery/usage.rs):
// how much of its plan a provider has used, from 0 to 1, or null.
import { native } from "../../index.js";

export const planUsage = (limits, app, now = Date.now()) => native.planUsage(limits, app, now) ?? null;
