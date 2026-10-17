// core/environments.js in Rust (crates/xenonet-app/src/environments.rs):
// the place around the map, chosen apart from its theme.
import { native } from "../index.js";

export const DEFAULT_ENVIRONMENT = native.appDefaultEnvironment();
export const ENVIRONMENTS = Object.freeze(native.appEnvironments());
// Only a string can be an id (JS compared with ===).
export const environmentOf = (id) => ENVIRONMENTS[native.appEnvironmentIndex(typeof id === "string" ? id : null)];
