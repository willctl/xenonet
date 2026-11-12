// The Rust core's layering (docs/design/direction.md, "The target shape";
// docs/design/rust-core.md, "The layering"). Every crate in crates/ is in
// one class, and each class has its rules:
//
// - Base crates (BASE): xenonet-text, xenonet-os and xenonet-model, the
//   formats, the machine and the shared vocabulary every crate uses. They
//   count toward no limit.
// - Core library crates: every crate in crates/ not classed below, a new
//   one included. Each has at most three internal dependencies besides the
//   base crates.
// - Composition crates (COMPOSITION): each holds a domain's runtime and
//   wires several domains together, on its way to dissolving in the phase
//   of docs/design/direction.md's order of work its entry names. Each has a
//   ceiling at its count when the ceilings were set: the check fails when
//   it goes over, and when it drops below without its ceiling being
//   lowered here, so a ceiling only ever comes down.
// - Surfaces (SURFACES): xenonet-http, xenonet-mcp and xenonet-console
//   import xenonet-api and the base crates and nothing else. What else a
//   surface needs, the root gives it (a trait or the data it reads), or it
//   asks the xenonet binary.
// - The composition root (ROOT): xenonet-service (compose/, serve.rs, the
//   gateway, A2A and the other servers) and xenonet-cli (the binary, whose
//   `serve()` composes). No limit, and no other crate depends on either.
//
// And for every crate but the surfaces and the root: it never depends on a
// surface crate or an adapter crate, and never holds a registry id literal
// (an app's or an actor product's id), since what it knows of a platform
// comes through the adapter registry. The demo's made-up world
// (xenonet-demo) is wired in by the surfaces, the root and the map's demo
// driver alone. Dependencies count as Cargo.toml lists them, a target's
// included and dev-dependencies left out.
//
//   node scripts/rust-layers.mjs            fails on a breach
//   node scripts/rust-layers.mjs --report   lists every breach and each
//                                           crate's count, without failing
//
// xenonet-adapter is the registry itself, and adapters (adapters/*) may
// name their own platform; the demo names the apps it makes sessions for.
// A generic adapter (GENERIC: one protocol, for whichever apps the
// registry names, as xenonet-acp and xenonet-a2a are) may not: it's held
// to the core's rule on literals, and depends on no platform's adapter,
// surface or root.
// Test code is left out: what follows a `#[cfg(test)]`, files a test
// module includes (`#[cfg(test)] #[path = "…"] mod …;`), tests/, and
// dev-dependencies. So are attribute lines (`#[serde(…)]`,
// `#[schemars(…)]`) and the model's schema macros (`open!`, `tag!`,
// `constant!`, `pattern!`): there an id is a document's wire name, written
// into its schema, not a branch on a platform.
//
// Xenolith (XENOLITH: xenolith/rust, its own workspace) takes XenoNet's
// libraries, never its app: its crates, and its workspace's dependency
// table, name no XenoNet crate but xenonet-id, -log, -authz and -net and the
// base crates. No crate of XenoNet's depends on a crate of Xenolith's.
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const report = process.argv.includes("--report");
const BASE = new Set(["xenonet-text", "xenonet-os", "xenonet-model"]);
// The composition crates' ceilings, and the phase each dissolves in.
const COMPOSITION = {
  "xenonet-state": { ceiling: 7, phase: 4, why: "the map's state goes into the one store" },
  "xenonet-doctor": { ceiling: 7, phase: 5, why: "it runs the checks the crates and adapters expose, through the registry" },
};
// The surfaces: xenonet-api and the base crates alone.
const SURFACES = {
  "xenonet-http": { only: ["xenonet-api"] },
  "xenonet-mcp": { only: ["xenonet-api"] },
  "xenonet-console": { only: ["xenonet-api"] },
};
const ROOT = new Set(["xenonet-service", "xenonet-cli"]);
// What may depend on the demo besides the surfaces and the root: the map's
// demo driver.
const DEMO_USERS = new Set([...Object.keys(SURFACES), ...ROOT, "xenonet-state"]);
// Crates whose literals aren't the core's: the registry and the demo.
const NAMING = new Set(["xenonet-adapter", "xenonet-demo"]);
// The generic adapters, by package name.
const GENERIC = new Set(["xenonet-acp", "xenonet-a2a", "xenonet-mcp-catalog"]);
// What Xenolith may take of XenoNet's: the libraries, and the base crates
// they bring.
const XENOLITH = { dir: "xenolith/rust", may: new Set(["xenonet-id", "xenonet-log", "xenonet-authz", "xenonet-net", ...BASE]) };

// Words that are an app's id and also something else: a literal the check
// would count, kept because there it isn't an app. Each entry is a file, the
// exact text on the line, and why. Add one only where the word means
// something other than the app; never for a branch on the app.
const NOT_AN_APP = [
  { file: "crates/xenonet-net/src/svc2.rs", text: `"cursor" => (1..=200)`, why: "a transcript page's cursor parameter in a window read" },
  { file: "adapters/mcp-catalog/src/info.rs", text: `"cursor": self.state.get("cursor")`, why: "the MCP status listing's page cursor" },
  { file: "adapters/mcp-catalog/src/info.rs", text: `c.state.insert("cursor".into(), Value::Null);`, why: "the MCP status listing's page cursor" },
  { file: "adapters/mcp-catalog/src/info.rs", text: `let cursor = c.state.get("cursor").cloned();`, why: "the MCP status listing's page cursor" },
  { file: "adapters/mcp-catalog/src/info.rs", text: `c.state.insert("cursor".into(), next_cursor`, why: "the MCP status listing's page cursor" },
  { file: "adapters/mcp-catalog/src/catalog.rs", text: `json!({ "cursor": c })`, why: "an MCP list request's page cursor" },
  { file: "crates/xenonet-actors/src/assistant/service.rs", text: `self.args.get("cursor")`, why: "a read's page cursor argument" },
  { file: "crates/xenonet-state/src/state.rs", text: `"rules": list, "codex": codex })`, why: "the project detection answer's field for the sorter's picks, a wire name the HTTP goldens pin" },
];

const apps = JSON.parse(fs.readFileSync(path.join(root, "crates/xenonet-adapter/src/apps.json"), "utf8"));
const ids = apps.map((a) => a.id);
const products = ["hermes-agent", "grok-bot", "muse", "dots"];
const literals = new RegExp(`"(${[...ids, ...products].map((s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})"`, "g");

function crates(dir) {
  const at = path.join(root, dir);
  if (!fs.existsSync(at)) return [];
  return fs.readdirSync(at).filter((n) => fs.existsSync(path.join(at, n, "Cargo.toml"))).map((n) => ({ name: n, dir: path.join(at, n) }));
}

// A crate's dependencies on XenoNet's own crates (or, with prefix
// "xenolith", Xenolith's), dev-dependencies left out unless `dev`.
function deps(cargoToml, { prefix = "xenonet", dev = false } = {}) {
  const text = fs.readFileSync(cargoToml, "utf8");
  const out = new Set();
  let section = "";
  for (const line of text.split("\n")) {
    const t = line.trim();
    const head = /^\[(.+)\]$/.exec(t);
    if (head) section = head[1];
    if (!/(^|\.)dependencies$/.test(section) || (!dev && /dev-dependencies$/.test(section))) continue;
    const m = new RegExp(`^(${prefix}-[a-z-]+)\\s*(\\.workspace|=)`).exec(t);
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

// The files a crate's test modules pull in by path: test code too.
function testFiles(src) {
  const out = new Set();
  for (const f of rustFiles(src)) {
    const text = fs.readFileSync(f, "utf8");
    for (const m of text.matchAll(/#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*#\[path\s*=\s*"([^"]+)"\]/g)) {
      out.add(path.resolve(path.dirname(f), m[1]));
    }
  }
  return out;
}

const breaches = [];
const counts = [];
const names = new Set(crates("crates").map((c) => c.name));
for (const n of [...BASE, ...Object.keys(COMPOSITION), ...Object.keys(SURFACES), ...ROOT]) {
  if (!names.has(n)) breaches.push(`${n} is classed in scripts/rust-layers.mjs, and there's no such crate: take it out`);
}
for (const c of crates("crates")) {
  const all = deps(path.join(c.dir, "Cargo.toml"));
  const counted = [...all].filter((d) => !BASE.has(d)).sort();
  const n = counted.length;
  if (ROOT.has(c.name)) {
    counts.push(`${c.name}: ${n} (the root, no limit)`);
    continue;
  }
  for (const d of all) {
    if (ROOT.has(d)) breaches.push(`${c.name} depends on the composition root's ${d}`);
  }
  const surface = SURFACES[c.name];
  const kept = surface ?? COMPOSITION[c.name];
  if (surface?.only) {
    const others = counted.filter((d) => !surface.only.includes(d));
    if (others.length) breaches.push(`${c.name} is a surface on ${surface.only.join(", ")} and the base crates alone, and depends on ${others.join(", ")}`);
    counts.push(`${c.name}: ${n} (surface: ${surface.only.join(", ")} and the base crates only)`);
  } else if (kept) {
    const kind = surface ? "surface" : "composition";
    if (n > kept.ceiling) breaches.push(`${c.name} has ${n} internal dependencies besides the base crates, over its ceiling of ${kept.ceiling} (${counted.join(", ")})`);
    if (n < kept.ceiling) breaches.push(`${c.name} has ${n} internal dependencies besides the base crates, under its ceiling of ${kept.ceiling}: lower the ceiling to ${n} in scripts/rust-layers.mjs`);
    counts.push(`${c.name}: ${n} (${kind}, ceiling ${kept.ceiling} until phase ${kept.phase}: ${kept.why})`);
  } else if (BASE.has(c.name)) {
    counts.push(`${c.name}: ${n} (base)`);
  } else {
    if (n > 3) breaches.push(`${c.name} has ${n} internal dependencies besides the base crates, and a core crate has at most 3 (${counted.join(", ")})`);
    counts.push(`${c.name}: ${n} (core, at most 3)`);
  }
  if (surface) continue;
  for (const d of all) {
    if (SURFACES[d]) breaches.push(`${c.name} depends on the surface crate ${d}`);
    if (ADAPTERS.has(d) && c.name !== "xenonet-adapter") breaches.push(`${c.name} depends on the adapter crate ${d}`);
    if (d === "xenonet-demo" && !DEMO_USERS.has(c.name)) breaches.push(`${c.name} depends on the demo's world (xenonet-demo)`);
  }
  if (NAMING.has(c.name)) continue;
  literalBreaches(c.dir);
}

for (const n of GENERIC) {
  if (!ADAPTERS.has(n)) breaches.push(`${n} is classed as a generic adapter in scripts/rust-layers.mjs, and there's no such crate: take it out`);
}
for (const c of crates("adapters")) {
  const name = packageName(c.dir);
  if (!GENERIC.has(name)) continue;
  for (const d of deps(path.join(c.dir, "Cargo.toml"))) {
    if (ADAPTERS.has(d)) breaches.push(`${name} is a generic adapter, and depends on the adapter crate ${d}`);
    if (SURFACES[d] || ROOT.has(d)) breaches.push(`${name} is a generic adapter, and depends on ${d}`);
  }
  literalBreaches(c.dir);
}

// Xenolith: the libraries and the base crates, never the app, and nothing
// of XenoNet's depends on it. Its dev-dependencies count too: a test that
// links the service links the app.
{
  const may = [...XENOLITH.may].join(", ");
  const manifests = [{ name: `${XENOLITH.dir}'s workspace`, file: path.join(root, XENOLITH.dir, "Cargo.toml") }, ...crates(`${XENOLITH.dir}/crates`).map((c) => ({ name: packageName(c.dir), file: path.join(c.dir, "Cargo.toml") }))];
  for (const m of manifests) {
    const all = deps(m.file, { dev: true });
    const others = [...all].filter((d) => !XENOLITH.may.has(d)).sort();
    if (others.length) breaches.push(`${m.name} is Xenolith's, which takes ${may} alone of XenoNet's, and depends on ${others.join(", ")}`);
    if (!m.name.endsWith("workspace")) counts.push(`${m.name}: ${[...all].filter((d) => !BASE.has(d)).length} (Xenolith: ${may} only)`);
  }
  for (const c of [...crates("crates"), ...crates("adapters")]) {
    const theirs = [...deps(path.join(c.dir, "Cargo.toml"), { prefix: "xenolith", dev: true })];
    if (theirs.length) breaches.push(`${packageName(c.dir)} depends on Xenolith's ${theirs.join(", ")}: Xenolith takes XenoNet's libraries, never the other way`);
  }
}

// A crate's registry id literals outside its tests, as breaches.
function literalBreaches(dir) {
  const src = path.join(dir, "src");
  if (!fs.existsSync(src)) return;
  const tests = testFiles(src);
  for (const f of rustFiles(src)) {
    if (tests.has(f)) continue;
    const text = fs.readFileSync(f, "utf8");
    const code = text.split(/^#\[cfg\(test\)\]/m)[0];
    let n = 0;
    for (const line of code.split("\n")) {
      const t = line.trim();
      if (t.startsWith("//") || /^#\[(serde|schemars)\(/.test(t) || /^(crate::)?(open|tag|constant|pattern)!\(/.test(t)) continue;
      const rel = path.relative(root, f);
      if (NOT_AN_APP.some((a) => a.file === rel && line.includes(a.text))) continue;
      n += (line.match(literals) || []).length;
    }
    if (n) breaches.push(`${path.relative(root, f)}: ${n} registry id literal${n === 1 ? "" : "s"}`);
  }
}

if (report) process.stderr.write(`Internal dependencies besides the base crates:\n  ${counts.join("\n  ")}\n`);
if (breaches.length) {
  process.stderr.write(`${breaches.length} layering breach${breaches.length === 1 ? "" : "es"}:\n  ${breaches.join("\n  ")}\n`);
  if (!report) process.exit(1);
} else {
  process.stderr.write("Every crate keeps to its class: the core depends on no surface and names no platform, and no crate is over its limit.\n");
}
