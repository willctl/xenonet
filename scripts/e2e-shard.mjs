// Split the end-to-end suite (test/e2e) into shards that take about the
// same time, so CI can run them on parallel runners. Each file's time comes
// from test/e2e/timings.json, measured on CI's Linux runner; a file not
// listed there counts as the median.
//
//   node scripts/e2e-shard.mjs 2/4             the files of shard 2 of 4
//   node scripts/e2e-shard.mjs --measure <log> timings.json from a CI log
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DIR = path.join(root, "test", "e2e");
const TIMINGS = path.join(DIR, "timings.json");

export const suiteFiles = (dir = DIR) => fs.readdirSync(dir).filter((f) => f.endsWith(".test.js")).sort();

// Each file goes, longest first, to the shard with the least time so far
// (the lowest-numbered on a tie), so the same files and timings always
// give the same shards. Returns the files of each shard, in name order.
export function shards(files, timings, total) {
  const known = files.map((f) => timings[f]).filter((s) => typeof s === "number").sort((a, b) => a - b);
  const median = known.length ? known[Math.floor(known.length / 2)] : 1;
  const weight = (f) => (typeof timings[f] === "number" ? timings[f] : median);
  const order = [...files].sort((a, b) => weight(b) - weight(a) || a.localeCompare(b));
  const out = Array.from({ length: total }, () => ({ files: [], secs: 0 }));
  for (const f of order) {
    const least = out.reduce((best, s, i) => (s.secs < out[best].secs ? i : best), 0);
    out[least].files.push(f);
    out[least].secs += weight(f);
  }
  return out.map((s) => s.files.sort());
}

export function parseShard(arg) {
  const m = /^(\d+)\/(\d+)$/.exec(arg ?? "");
  const [index, total] = m ? [Number(m[1]), Number(m[2])] : [0, 0];
  if (!m || total < 1 || index < 1 || index > total) throw new Error(`a shard is <index>/<total>, such as 2/4, not ${arg}`);
  return { index, total };
}

// Each file's seconds from a CI log of the whole suite: the spec reporter
// prints each test's title and time, and a title belongs to the file whose
// source has it.
export function measure(log, files, source) {
  const secs = {};
  for (const raw of log.split("\n")) {
    const line = raw.replace(/^.*?\dZ /, "");
    const m = /^\s*\S*\s*(.+?) \(([\d.]+)ms\)/.exec(line);
    if (!m) continue;
    const key = m[1].replace(/^[^A-Za-z0-9"(]+/, "").slice(0, 40);
    const file = files.find((f) => source(f).includes(key));
    if (file) secs[file] = (secs[file] ?? 0) + Number(m[2]) / 1000;
  }
  return Object.fromEntries(Object.entries(secs).sort(([a], [b]) => a.localeCompare(b)).map(([f, s]) => [f, Math.round(s)]));
}

export const readTimings = () => (fs.existsSync(TIMINGS) ? JSON.parse(fs.readFileSync(TIMINGS, "utf8")) : {});

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [arg, log] = process.argv.slice(2);
  if (arg === "--measure") {
    const files = suiteFiles();
    const timings = measure(fs.readFileSync(log, "utf8"), files, (f) => fs.readFileSync(path.join(DIR, f), "utf8"));
    fs.writeFileSync(TIMINGS, `${JSON.stringify(timings, null, 2)}\n`);
    console.log(`${Object.keys(timings).length} of ${files.length} files timed`);
  } else {
    const { index, total } = parseShard(arg);
    for (const f of shards(suiteFiles(), readTimings(), total)[index - 1]) console.log(path.join("test", "e2e", f));
  }
}
