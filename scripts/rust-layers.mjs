// The Rust core's layering (docs/design/direction.md, "The target shape"):
// a core crate never depends on a surface crate or an adapter crate, and
// never holds a registry id literal (an app's or an actor product's id),
// since what it knows of a platform comes through the adapter registry.
// The demo's made-up world (xenonet-demo) is wired in by the surfaces and
// the map's demo driver alone.
//
//   node scripts/rust-layers.mjs            fails on a breach
//   node scripts/rust-layers.mjs --report   lists every breach without failing
//
// Surfaces: the command, the MCP servers, the service and the console.
// xenonet-adapter is the registry itself, and adapters (adapters/*) may
// name their own platform; the demo names the apps it makes sessions for.
// Test modules (after `#[cfg(test)]`), tests/ and dev-dependencies are
// left out.
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const report = process.argv.includes("--report");
const SURFACES = new Set(["xenonet-cli", "xenonet-mcp", "xenonet-service", "xenonet-console"]);
// What may depend on the demo besides the surfaces: the map's demo driver.
const DEMO_USERS = new Set([...SURFACES, "xenonet-state"]);
// Crates whose literals aren't the core's: the registry and the demo.
const NAMING = new Set(["xenonet-adapter", "xenonet-demo"]);

const apps = JSON.parse(fs.readFileSync(path.join(root, "crates/xenonet-adapter/src/apps.json"), "utf8"));
const ids = apps.map((a) => a.id);
const products = ["hermes-agent", "grok-bot", "muse", "dots"];
const literals = new RegExp(`"(${[...ids, ...products].map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})"`, "g");

function crates(dir) {
  const at = path.join(root, dir);
  if (!fs.existsSync(at)) return [];
  return fs.readdirSync(at).filter((n) => fs.existsSync(path.join(at, n, "Cargo.toml"))).map((n) => ({ name: n, dir: path.join(at, n) }));
}

// A crate's dependencies on XenoNet's own crates, dev-dependencies left out.
function deps(cargoToml) {
  const text = fs.readFileSync(cargoToml, "utf8");
  const out = new Set();
  let section = "";
  for (const line of text.split("\n")) {
    const t = line.trim();
    const head = /^\[(.+)\]$/.exec(t);
    if (head) section = head[1];
    if (!/(^|\.)dependencies$/.test(section) || /dev-dependencies$/.test(section)) continue;
    const m = /^(xenonet-[a-z-]+)\s*(\.workspace|=)/.exec(t);
    if (m) out.add(m[1]);
  }
  return out;
}

const packageName = (dir) => /^name\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(path.join(dir, "Cargo.toml"), "utf8"))?.[1] ?? path.basename(dir);
const ADAPTERS = new Set(crates("adapters").map((c) => packageName(c.dir)));

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
    if (ADAPTERS.has(d) && c.name !== "xenonet-adapter") breaches.push(`${c.name} depends on the adapter crate ${d}`);
    if (d === "xenonet-demo" && !DEMO_USERS.has(c.name)) breaches.push(`${c.name} depends on the demo's world (xenonet-demo)`);
  }
  if (NAMING.has(c.name)) continue;
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
