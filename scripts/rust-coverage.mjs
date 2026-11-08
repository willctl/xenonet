// How much of the Rust core its tests run, measured with cargo-llvm-cov:
// lines, functions and regions, crate by crate.
//
//   npm run coverage:rust              cargo test, then the table
//   npm run coverage:rust -- --html    and a report in target/llvm-cov/html
//   npm run coverage:rust -- --no-run  the table (and report) of the last run
//   npm run coverage:rust -- --json    the table as JSON
//
// It needs cargo-llvm-cov and the pinned toolchain's llvm-tools:
//   cargo install --locked cargo-llvm-cov
//   rustup component add llvm-tools --toolchain <the pin in rust-toolchain.toml>
//
// The instrumented build goes to target/llvm-cov-target, apart from the
// usual one, so cargo test and the lint don't build again after it.
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cargoCommand, pinned } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const html = args.includes("--html");
const asJson = args.includes("--json");
const run = !args.includes("--no-run");

function cargo(argv, { capture = false, env = process.env } = {}) {
  const [file, list, environment] = cargoCommand(argv, { ...env, CARGO_PROFILE_DEV_DEBUG: "0" });
  const r = spawnSync(file, list, { cwd: root, env: environment, windowsHide: true, encoding: "utf8", maxBuffer: 256 << 20, stdio: ["ignore", capture ? "pipe" : "inherit", "inherit"] });
  if (r.status !== 0) {
    process.stderr.write(`\ncargo ${argv.join(" ")} failed.\n`);
    if (r.status === 101 && argv[0] === "llvm-cov") process.stderr.write(`(if cargo llvm-cov isn't installed: cargo install --locked cargo-llvm-cov, and rustup component add llvm-tools --toolchain ${pinned() ?? "stable"})\n`);
    process.exit(r.status ?? 1);
  }
  return r.stdout;
}

if (run) cargo(["llvm-cov", "--workspace", "--locked", "--no-report"]);
const summary = JSON.parse(cargo(["llvm-cov", "report", "--json", "--summary-only"], { capture: true }));

// The files' counts, added up by the crate each is in.
const crates = new Map();
const add = (into, s) => {
  for (const k of ["lines", "functions", "regions"]) {
    into[k] ??= { count: 0, covered: 0 };
    into[k].count += s[k].count;
    into[k].covered += s[k].covered;
  }
};
for (const f of summary.data[0].files) {
  const rel = path.relative(root, f.filename).split(path.sep);
  const name = rel[0] === "crates" ? rel[1] : rel[0];
  if (!crates.has(name)) crates.set(name, {});
  add(crates.get(name), f.summary);
}
const total = {};
for (const c of crates.values()) add(total, c);

const pct = (k) => (k.count ? (100 * k.covered) / k.count : 100);
const rows = [...crates.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([name, c]) => ({ name, ...c }));
if (asJson) {
  console.log(JSON.stringify({ crates: rows, total }, null, 2));
} else {
  // Each crate's lines, and the share of its lines, functions and regions
  // the tests ran.
  const width = Math.max(5, ...rows.map((r) => r.name.length));
  const cell = (k, w) => `${pct(k).toFixed(1)}%`.padStart(w);
  console.log(`\n${"crate".padEnd(width)}  ${"lines".padStart(7)}  ${"run".padStart(6)}  ${"functions".padStart(9)}  ${"regions".padStart(7)}`);
  for (const r of [...rows, { name: "total", ...total }]) {
    if (r.name === "total") console.log("-".repeat(width + 39));
    console.log(`${r.name.padEnd(width)}  ${String(r.lines.count).padStart(7)}  ${cell(r.lines, 6)}  ${cell(r.functions, 9)}  ${cell(r.regions, 7)}`);
  }
}

if (html) {
  cargo(["llvm-cov", "report", "--html", "--output-dir", path.join(root, "target", "llvm-cov")]);
  console.log(`\nThe report: ${path.join("target", "llvm-cov", "html", "index.html")}`);
}
