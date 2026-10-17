// The Rust core's Node-API addon, for the twins in ./core/ (see
// docs/design/rust-core.md). `npm run rust:build` builds it for this
// platform and puts it beside this folder.
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const file = fileURLToPath(new URL(`../xenonet.${process.platform}-${process.arch}.node`, import.meta.url));

function load() {
  try {
    return createRequire(import.meta.url)(file);
  } catch (err) {
    if (err.code !== "MODULE_NOT_FOUND" && err.code !== "ERR_DLOPEN_FAILED") throw err;
    throw new Error(`XenoNet's Rust core isn't built for ${process.platform}-${process.arch}: run npm run rust:build (${err.message})`);
  }
}

export const native = load();
