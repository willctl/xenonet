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
const SURFACES = new Set(["xenonet-cli", "xenonet-mcp", "xenonet-service", "xenonet-console"]);
// What may depend on the demo besides the surfaces: the map's demo driver.
const DEMO_USERS = new Set([...SURFACES, "xenonet-state"]);
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

if (breaches.length) {
  process.stderr.write(`${breaches.length} layering breach${breaches.length === 1 ? "" : "es"}:\n  ${breaches.join("\n  ")}\n`);
  if (!report) process.exit(1);
} else {
  process.stderr.write("The core depends on no surface and names no platform.\n");
}
