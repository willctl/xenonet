// core/rename.js in Rust (crates/xenonet-state/src/rename.rs): a session's
// name as you give it in XenoNet, before it's saved in its own app.
import { native } from "../index.js";

export const MAX_TITLE = native.maxTitle();

export const cleanTitle = (title) => (typeof title === "string" ? native.cleanTitle(title) : "");

export const renamable = (n) => native.renamable(n);
