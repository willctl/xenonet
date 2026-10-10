// What the inspections share: XenoNet's own schema files and source, the
// stand-in turn's prompt and model, and the steps an inspection reports.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { compareAll } from "./types.mjs";

export const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

// The stand-in turn: what's asked, the model named (the stand-in answers
// any), and the title a rename gives.
export const PROMPT = "Check the plan.";
export const MODEL = "claude-sonnet-4-5";
export const RENAMED = "Renamed by the drift check";

// The newest schema file for a vendor source, as JSON, or null.
export function xenonetSchema(source, dir = path.join(REPO, "schemas")) {
  const folder = path.join(dir, "vendor", ...source.split("/"));
  let names = [];
  try {
    names = fs.readdirSync(folder);
  } catch {
    return null;
  }
  const newest = names
    .map((n) => /^v([1-9]\d*)\.json$/.exec(n))
    .filter(Boolean)
    .sort((a, b) => Number(b[1]) - Number(a[1]))[0];
  return newest ? JSON.parse(fs.readFileSync(path.join(folder, newest[0]), "utf8")) : null;
}

// The text of every module under core/ (what XenoNet runs), for the imports
// and calls it makes.
export function coreSources(dir = path.join(REPO, "core")) {
  const out = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) walk(full);
      else if (e.name.endsWith(".js")) out.push(fs.readFileSync(full, "utf8"));
    }
  };
  walk(dir);
  return out;
}

// The provider's published types against XenoNet's schemas, as findings.
// `docFor(pair)` gives the JSON Schema document a pair's type is in.
export function typeFindings(pairs, docFor) {
  const bySource = new Map();
  for (const p of pairs ?? []) bySource.set(p.source, [...(bySource.get(p.source) ?? []), p]);
  const out = [];
  for (const [source, list] of bySource) out.push(...compareAll(list, { xenonet: () => xenonetSchema(source), provider: docFor, source }).map((f) => ({ ...f, origin: "types" })));
  return out;
}

// The steps an inspection took, each { step, ok, detail }: ok is true,
// false (it failed, and why is in detail) or null (skipped). `note` says
// what a step that went well found, from what it returned.
export function steps() {
  const list = [];
  return {
    list,
    async run(step, fn, note = null) {
      try {
        const value = await fn();
        list.push({ step, ok: true, detail: note ? String(note(value)).slice(0, 200) : null });
        return value;
      } catch (err) {
        list.push({ step, ok: false, detail: String(err?.message ?? err).slice(0, 400) });
        return undefined;
      }
    },
    skip(step, why) {
      list.push({ step, ok: null, detail: why });
    },
  };
}

// The binary a platform package of `prefix` ships for this machine
// (@anthropic-ai/claude-code-linux-x64/claude), or null.
export function platformBinary(dir, prefix, name) {
  const exe = process.platform === "win32" ? `${name}.exe` : name;
  const key = `${process.platform}-${process.arch}`;
  for (const pkg of [`${prefix}-${key}`, `${prefix}-${key}-musl`]) {
    const file = path.join(dir, "node_modules", ...pkg.split("/"), exe);
    if (fs.existsSync(file)) return file;
  }
  return null;
}
