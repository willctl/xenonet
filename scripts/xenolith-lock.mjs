// Xenolith (xenolith/rust) has its own workspace and Cargo.lock, and takes XenoNet's libraries
// (xenonet-id, -log, -authz, -net and the base crates they bring) as path dependencies. Their tests
// run only here, under the root's lock, so Xenolith must build them on the same packages. For every
// package either workspace reaches from those libraries (normal and build dependencies, every
// target), this checks:
//
// - versions: both lockfiles hold the same versions of it, from the same source, with the same
//   checksum;
// - features: Xenolith's build turns on no feature of it that the root's build doesn't, since the
//   libraries' tests have never run with that feature. Features only the root turns on are fine: the
//   root's other crates ask for them, and the libraries' own needs are in both. A feature Xenolith
//   adds for its own crates' sake (the tray's Windows APIs, say) goes in ONLY_XENOLITH below, with
//   why it can't change what a library does.
//
// Features are the ones `cargo tree` resolves for each workspace's whole build, as cargo builds it.
//
//   node scripts/xenolith-lock.mjs         fails on a disagreement, naming each
//   node scripts/xenolith-lock.mjs --fix   first moves Xenolith's lock to the root's versions
//
// npm run lint:rust runs it, and so does Xenolith's CI (.github/workflows/xenolith.yml).
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cargoCommand } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const xenolith = path.join(root, "xenolith", "rust");
const LIBS = ["xenonet-id", "xenonet-log", "xenonet-authz", "xenonet-net"];

// Features Xenolith's own crates turn on in a package the libraries reach, and the root's build
// doesn't, each with why the libraries behave the same with it. Another feature of the same package
// still fails.
const ONLY_XENOLITH = [
  { name: "windows-sys", features: /^(Win32|Wdk)(_[A-Za-z0-9]+)*$/, why: "Windows API declarations only; the tray and the service call more of them" },
  { name: "windows", features: /^(Win32|Wdk)(_[A-Za-z0-9]+)*$/, why: "Windows API declarations only, for the tray's window (wry, tao)" },
  { name: "winapi", features: /^[a-z0-9]+$/, why: "Windows API declarations only (one module per feature)" },
  { name: "objc2", features: /^(disable-encoding-assertions|exception)$/, why: "the tray's macOS menu (tao, muda): how Objective-C exceptions and type checks surface, in calls the libraries never make" },
  { name: "regex-automata", features: /^(nfa|perf)$/, why: "Xenolith's regex asks for perf and its engines: the same matches, found faster" },
  { name: "time", features: /^macros$/, why: "the tray's webview (wry, through cookie): the macros only, re-exported" },
  { name: "time-macros", features: /^(formatting|parsing)$/, why: "a proc macro, run at build time: what the macros may expand to, not what time does" },
  { name: "smallvec", features: /^union$/, why: "the tray on Linux (glib): a smaller layout for the same vector" },
  { name: "futures-executor", features: /^default$/, why: "the tray on Linux (glib): its executor, which the libraries never start" },
  { name: "dlopen2", features: /^(default|derive|dlopen2_derive|symbor|wrapper)$/, why: "the tray (tao) loads the system's libraries with it; the libraries never call it" },
  { name: "foldhash", features: /^(default|std)$/, why: "the tray's webview (wry, through dom_query): a seed per process for its hashers, which no library's results depend on" },
  { name: "stable_deref_trait", features: /^(alloc|default|std)$/, why: "the tray's webview (wry, through servo_arc): the trait for Box, Vec and String too" },
  { name: "toml_datetime", features: /^serde$/, why: "Xenolith's own TOML (the CLI's Codex config) and embed-resource's build script" },
];

function cargo(args, cwd) {
  const [file, argv, env] = cargoCommand(args);
  const r = spawnSync(file, argv, { cwd, env, encoding: "utf8", maxBuffer: 1 << 28, windowsHide: true });
  if (r.status !== 0) throw new Error(`cargo ${args.join(" ")} in ${path.relative(root, cwd) || "."} failed:\n${r.stderr}`);
  return r.stdout;
}

// A Cargo.lock's [[package]] entries, with the text before the first.
function entries(dir) {
  const [head, ...rest] = fs.readFileSync(path.join(dir, "Cargo.lock"), "utf8").split(/^\[\[package\]\]\n/m);
  return {
    head,
    packages: rest.map((text) => {
      const field = (k) => new RegExp(`^${k} = "([^"]*)"`, "m").exec(text)?.[1] ?? null;
      return { text, name: field("name"), version: field("version"), source: field("source"), checksum: field("checksum") };
    }),
  };
}

// name@version -> the features cargo turns on in it, over the workspace's whole build (every unit of
// it, host and target).
function features(dir) {
  const out = new Map();
  for (const line of cargo(["tree", "--workspace", "--locked", "--target", "all", "-e", "normal,build", "-f", "{p}|{f}", "--prefix", "none"], dir).split("\n")) {
    const m = /^(\S+) v(\S+)[^|]*\|([^ ]*)/.exec(line);
    if (!m) continue;
    const key = `${m[1]}@${m[2]}`;
    const set = out.get(key) ?? new Set();
    for (const f of m[3].split(",").filter(Boolean)) set.add(f);
    out.set(key, set);
  }
  return out;
}

// What one workspace reaches from the libraries: name -> [{version, source, checksum, features}].
function reached(dir) {
  const meta = JSON.parse(cargo(["metadata", "--locked", "--format-version", "1"], dir));
  const sums = new Map(entries(dir).packages.map((p) => [`${p.name}@${p.version}|${p.source ?? ""}`, p.checksum]));
  const on = features(dir);
  const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const start = meta.packages.filter((p) => LIBS.includes(p.name) && p.source === null).map((p) => p.id);
  const missing = LIBS.filter((n) => !meta.packages.some((p) => p.name === n && p.source === null));
  if (missing.length) throw new Error(`${path.relative(root, dir) || "the root"} doesn't reach ${missing.join(", ")} as a path dependency`);
  const seen = new Set(start);
  const queue = [...start];
  while (queue.length) {
    for (const d of nodes.get(queue.shift())?.deps ?? []) {
      if (seen.has(d.pkg) || !d.dep_kinds.some((k) => k.kind !== "dev")) continue;
      seen.add(d.pkg);
      queue.push(d.pkg);
    }
  }
  const out = new Map();
  for (const id of seen) {
    const p = pkgs.get(id);
    const source = p.source ?? "";
    const entry = { version: p.version, source, checksum: p.source ? (sums.get(`${p.name}@${p.version}|${source}`) ?? null) : null, features: [...(on.get(`${p.name}@${p.version}`) ?? [])].sort() };
    out.set(p.name, [...(out.get(p.name) ?? []), entry].sort((a, b) => a.version.localeCompare(b.version)));
  }
  return out;
}

const show = (e) => `${e.version}${e.source.startsWith("registry+") ? "" : ` (${e.source || "path"})`}`;

function compare() {
  const a = reached(root);
  const b = reached(xenolith);
  const problems = [];
  let versions = 0;
  for (const name of [...a.keys()].filter((n) => b.has(n)).sort()) {
    const ours = a.get(name);
    const theirs = b.get(name);
    const key = (e) => `${e.version}|${e.source}|${e.checksum}`;
    if (ours.map(key).join(",") !== theirs.map(key).join(",")) {
      versions++;
      const same = ours.length === theirs.length && ours.every((e, i) => e.version === theirs[i].version);
      problems.push(`${name}: the root's lock has ${ours.map(show).join(", ")}, Xenolith's ${theirs.map(show).join(", ")}${same ? " (the same versions from another source or with another checksum)" : ""}`);
      continue;
    }
    for (const [i, o] of ours.entries()) {
      const allowed = ONLY_XENOLITH.find((x) => x.name === name);
      const added = theirs[i].features.filter((f) => !o.features.includes(f) && !allowed?.features.test(f));
      if (added.length) problems.push(`${name} ${o.version}: only Xenolith's build turns on ${added.join(", ")}`);
    }
  }
  return { problems, versions, shared: [...a.keys()].filter((n) => b.has(n)).length };
}

// Every registry package both lockfiles hold takes the root's entries (all its versions), and cargo
// then settles Xenolith's lock around them: what only Xenolith uses keeps its versions where it can.
// One cargo update --precise at a time can't do it: a family like icu_* moves together.
function fix() {
  const ours = entries(root);
  const theirs = entries(xenolith);
  const shared = new Set(ours.packages.filter((p) => p.source).map((p) => p.name));
  const xen = new Set(theirs.packages.filter((p) => p.source).map((p) => p.name));
  const merged = [...theirs.packages.filter((p) => !p.source || !shared.has(p.name)), ...ours.packages.filter((p) => p.source && xen.has(p.name))];
  fs.writeFileSync(path.join(xenolith, "Cargo.lock"), theirs.head + merged.map((p) => `[[package]]\n${p.text}`).join(""));
  cargo(["metadata", "--format-version", "1"], xenolith);
}

const fixing = process.argv.includes("--fix");
let result = compare();
if (fixing && result.versions) {
  process.stderr.write("Moving xenolith/rust/Cargo.lock to the root's versions of the packages both hold\n");
  fix();
  result = compare();
}
if (result.problems.length) {
  process.stderr.write(
    `${result.problems.length} disagreement${result.problems.length === 1 ? "" : "s"} between Cargo.lock and xenolith/rust/Cargo.lock, in what XenoNet's libraries reach:\n  ${result.problems.join("\n  ")}\n` +
      (result.versions && !fixing ? "node scripts/xenolith-lock.mjs --fix moves Xenolith's lock to the root's versions.\n" : "") +
      (result.problems.length > result.versions ? "A feature only Xenolith turns on: turn it off in Xenolith's Cargo.toml, or add it to ONLY_XENOLITH in scripts/xenolith-lock.mjs with why it can't change what a library does.\n" : ""),
  );
  process.exitCode = 1;
} else {
  process.stderr.write(`Both lockfiles agree on the ${result.shared} packages XenoNet's libraries reach: the same versions, sources and checksums, and no feature only Xenolith turns on.\n`);
}
