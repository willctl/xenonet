// core/version.js in Rust (crates/xenonet-app/src/version.rs): XenoNet's
// version, from its package.json.
import { fileURLToPath } from "node:url";
import { native } from "../index.js";

export const VERSION = native.appVersion(fileURLToPath(new URL("../../../../package.json", import.meta.url)));
