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
// - Surfaces (SURFACES): xenonet-http imports xenonet-api and the base
//   crates and nothing else. xenonet-mcp and xenonet-console have a
//   ceiling like the composition crates', until they reach xenonet-api
//   alone in the phase their entry names.
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
// Test code is left out: what follows a `#[cfg(test)]`, files a test
// module includes (`#[cfg(test)] #[path = "…"] mod …;`), tests/, and
// dev-dependencies. So are attribute lines (`#[serde(…)]`,
// `#[schemars(…)]`) and the model's schema macros (`open!`, `tag!`,
// `constant!`, `pattern!`): there an id is a document's wire name, written
// into its schema, not a branch on a platform.
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(new URL("..", import.meta.url).pathname);
const report = process.argv.includes("--report");
const BASE = new Set(["xenonet-text", "xenonet-os", "xenonet-model"]);
// The composition crates' ceilings, and the phase each dissolves in.
const COMPOSITION = {
  "xenonet-runtime": { ceiling: 11, phase: 5, why: "the engine's drivers go to the engine, the store and the transport" },
  "xenonet-state": { ceiling: 7, phase: 4, why: "the map's state goes into the one store" },
  "xenonet-hosts": { ceiling: 6, phase: 4, why: "deleted: other machines' maps are the network's" },
  "xenonet-assistant": { ceiling: 8, phase: 5, why: "remote actors' connections and tunnels go to the transport" },
  "xenonet-doctor": { ceiling: 8, phase: 5, why: "it runs the checks the crates and adapters expose, through the registry" },
  "xenonet-integrations": { ceiling: 5, phase: 6, why: "the edges: ACP, A2A outbound and MCP's catalog become adapters" },
};
// The surfaces: xenonet-api and the base crates alone, or a ceiling until
// the phase they reach that in.
const SURFACES = {
  "xenonet-http": { only: ["xenonet-api"] },
  "xenonet-mcp": { ceiling: 11, phase: 5, why: "once the composition crates it reaches into have dissolved" },
  "xenonet-console": { ceiling: 4, phase: 5, why: "once the composition crates it reaches into have dissolved" },
};
const ROOT = new Set(["xenonet-service", "xenonet-cli"]);
// What may depend on the demo besides the surfaces and the root: the map's
// demo driver.
const DEMO_USERS = new Set([...Object.keys(SURFACES), ...ROOT, "xenonet-state"]);
// Crates whose literals aren't the core's: the registry and the demo.
const NAMING = new Set(["xenonet-adapter", "xenonet-demo"]);

// Words that are an app's id and also something else: a literal the check
// would count, kept because there it isn't an app. Each entry is a file, the
// exact text on the line, and why. Add one only where the word means
// something other than the app; never for a branch on the app.
const NOT_AN_APP = [
  { file: "crates/xenonet-integrations/src/mcp_info.rs", text: `"cursor": self.state.get("cursor")`, why: "the MCP status listing's page cursor" },
  { file: "crates/xenonet-integrations/src/mcp_info.rs", text: `c.state.insert("cursor".into(), Value::Null);`, why: "the MCP status listing's page cursor" },
  { file: "crates/xenonet-integrations/src/mcp_info.rs", text: `let cursor = c.state.get("cursor").cloned();`, why: "the MCP status listing's page cursor" },
  { file: "crates/xenonet-integrations/src/mcp_info.rs", text: `c.state.insert("cursor".into(), next_cursor`, why: "the MCP status listing's page cursor" },
  { file: "crates/xenonet-integrations/src/runtime/mcp.rs", text: `json!({ "cursor": c })`, why: "an MCP list request's page cursor" },
  { file: "crates/xenonet-integrations/src/runtime/mod.rs", text: `"pi" => {`, why: "the Integrations core's method name for the pi route" },
  { file: "crates/xenonet-assistant/src/service.rs", text: `self.args.get("cursor")`, why: "a read's page cursor argument" },
  { file: "crates/xenonet-assistant/src/runtime/reads.rs", text: `("cursor", &cursor)`, why: "a read's page cursor argument" },
  { file: "crates/xenonet-hosts/src/relay.rs", text: `k == "cursor" && chars_ok`, why: "a relayed read's page cursor parameter" },
  { file: "crates/xenonet-state/src/state.rs", text: `"rules": list, "codex": codex })`, why: "the project detection answer's field for the sorter's picks, a wire name the HTTP goldens pin" },
  { file: "crates/xenonet-state/src/statefile.rs", text: `projects.get("codex")`, why: "state.json v1's field for the projects folder's picks, a stored document's name" },
  { file: "crates/xenonet-state/src/statefile.rs", text: `k != "codex" && k != "codexByRoot"`, why: "state.json v1's field for the projects folder's picks, a stored document's name" },
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
  const src = path.join(c.dir, "src");
  if (!fs.existsSync(src)) continue;
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
