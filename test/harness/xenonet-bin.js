// The XenoNet processes a test starts: the JS ones, or the Rust binary
// (docs/design/rust-core.md) when XENONET_CLI_BIN or XENONET_SERVICE_BIN
// names it. XENONET_CLI_BIN stands in for `node cli/xenonet.js`, the
// command and its MCP servers; XENONET_SERVICE_BIN for `node
// server/index.js`, run as `xenonet serve`. npm run test:bin builds the
// binary and runs the tests that start these with it.
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

export const rustCommand = process.env.XENONET_CLI_BIN || null;
export const rustService = process.env.XENONET_SERVICE_BIN || null;

// [file, args] that run `xenonet <args>`.
export function command(args = []) {
  return rustCommand ? [rustCommand, args] : [process.execPath, [path.join(root, "cli", "xenonet.js"), ...args]];
}

// [file, args] that run the service with `args` (--demo, --dev…).
export function service(args = []) {
  return rustService ? [rustService, ["serve", ...args]] : [process.execPath, [path.join(root, "server", "index.js"), ...args]];
}
