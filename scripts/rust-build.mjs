// Builds the Rust core's Node-API addon (crates/xenonet-node) and puts it
// where the twins load it: crates/xenonet-node/xenonet.<platform>-<arch>.node.
// `--debug` builds it unoptimized, for a faster build.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const debug = process.argv.includes("--debug");
execFileSync("cargo", ["build", "-p", "xenonet-node", ...(debug ? [] : ["--release"])], { cwd: root, stdio: "inherit" });
const lib = { darwin: "libxenonet_node.dylib", win32: "xenonet_node.dll" }[process.platform] ?? "libxenonet_node.so";
const built = path.join(root, "target", debug ? "debug" : "release", lib);
const out = path.join(root, "crates", "xenonet-node", `xenonet.${process.platform}-${process.arch}.node`);
// Copied, not linked: Node keeps the file it loaded open, and a rebuild
// would write under a running test.
fs.rmSync(out, { force: true });
fs.copyFileSync(built, out);
console.log(`built ${path.relative(root, out)}`);
