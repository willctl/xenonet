// The Rust core's layering (docs/design/direction.md, "The target shape"):
// a core crate never depends on a surface crate, and never holds a
// registry id literal (an app's or an actor product's id), since what it
// knows of a platform comes through the adapter registry.
//
//   node scripts/rust-layers.mjs            fails on a breach
//   node scripts/rust-layers.mjs --report   lists every breach without failing
//
// Surfaces: the command, the MCP servers and the service. Adapters
// (adapters/*) may name their own platform. Test modules (after
// `#[cfg(test)]`) and tests/ may use any literal.
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const report = process.argv.includes("--report");
const SURFACES = new Set(["xenonet-cli", "xenonet-mcp", "xenonet-service"]);

const apps = JSON.parse(fs.readFileSync(path.join(root, "crates/xenonet-adapter/src/apps.json"), "utf8"));
const ids = apps.map((a) => a.id);
const products = ["hermes-agent", "grok-bot", "muse", "dots"];
const literals = new RegExp(`"(${[...ids, ...products].map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})"`, "g");

function crates(dir) {
  const at = path.join(root, dir);
  if (!fs.existsSync(at)) return [];
  return fs.readdirSync(at).filter((n) => fs.existsSync(path.join(at, n, "Cargo.toml"))).map((n) => ({ name: n, dir: path.join(at, n) }));
}

function deps(cargoToml) {
  const text = fs.readFileSync(cargoToml, "utf8");
  const out = new Set();
  for (const line of text.split("\n")) {
    const m = /^(xenonet-[a-z-]+)\s*(\.workspace|=)/.exec(line.trim());
    if (m) out.add(m[1]);
  }
  return out;
}

function* rustFiles(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* rustFiles(p);
    else if (e.name.endsWith(".rs")) yield p;
  }
}

const breaches = [];
for (const c of crates("crates")) {
  if (SURFACES.has(c.name)) continue;
  for (const d of deps(path.join(c.dir, "Cargo.toml"))) {
    if (SURFACES.has(d)) breaches.push(`${c.name} depends on the surface crate ${d}`);
  }
  const src = path.join(c.dir, "src");
  if (!fs.existsSync(src)) continue;
  for (const f of rustFiles(src)) {
    const text = fs.readFileSync(f, "utf8");
    const code = text.split(/^#\[cfg\(test\)\]/m)[0];
    let n = 0;
    for (const line of code.split("\n")) {
      if (line.trim().startsWith("//")) continue;
      n += (line.match(literals) || []).length;
    }
    if (n) breaches.push(`${path.relative(root, f)}: ${n} registry id literal${n === 1 ? "" : "s"}`);
  }
}

if (breaches.length) {
  process.stderr.write(`${breaches.length} layering breach${breaches.length === 1 ? "" : "es"}:\n  ${breaches.join("\n  ")}\n`);
  if (!report) process.exit(1);
} else {
  process.stderr.write("The core depends on no surface and names no platform.\n");
}
