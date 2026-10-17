// Builds the Rust core's Node-API addon (crates/xenonet-node) and puts it
// where the twins load it: crates/xenonet-node/xenonet.<platform>-<arch>.node.
// `--debug` builds it unoptimized, for a faster build.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cargo } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const debug = process.argv.includes("--debug");

// Two Rust functions exported under one JS name build fine, and the second
// silently takes the first's place, so the build refuses them.
const exported = new Map();
const src = path.join(root, "crates", "xenonet-node", "src");
for (const file of fs.readdirSync(src).filter((f) => f.endsWith(".rs"))) {
  const lines = fs.readFileSync(path.join(src, file), "utf8").split("\n");
  lines.forEach((line, i) => {
    const attr = /^#\[napi(?:\((.*)\))?\]\s*$/.exec(line.trim());
    const item = attr && lines.slice(i + 1, i + 4).find((l) => /^pub (fn|struct) /.test(l));
    if (!item) return;
    const rust = /^pub (?:fn|struct) (\w+)/.exec(item)[1];
    const name = /js_name = "([^"]+)"/.exec(attr[1] ?? "")?.[1] ?? (item.startsWith("pub fn") ? rust.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase()) : rust);
    if (exported.has(name)) throw new Error(`${name} is exported twice: ${exported.get(name)} and ${file}:${i + 1}`);
    exported.set(name, `${file}:${i + 1}`);
  });
}
cargo(["build", "-p", "xenonet-node", ...(debug ? [] : ["--release"])], { stdio: "inherit" });
const lib = { darwin: "libxenonet_node.dylib", win32: "xenonet_node.dll" }[process.platform] ?? "libxenonet_node.so";
const built = path.join(root, "target", debug ? "debug" : "release", lib);
const out = path.join(root, "crates", "xenonet-node", `xenonet.${process.platform}-${process.arch}.node`);
// Copied, not linked: Node keeps the file it loaded open, and a rebuild
// would write under a running test.
fs.rmSync(out, { force: true });
fs.copyFileSync(built, out);
console.log(`built ${path.relative(root, out)}`);
