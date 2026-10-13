import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { clean, fail } from "./common.js";

const number = (v) => Number.isFinite(Number(v)) && Number(v) >= 0 ? Number(v) : 0;
const time = (v) => number(typeof v === "string" ? Date.parse(v) : v);
const idOf = (v) => createHash("sha256").update(v).digest("hex").slice(0, 24);
export function piExportMetadata(value) {
  const run = value?.run;
  if (!run || typeof run.id !== "string" || !run.id || typeof run.status !== "string") fail("Choose a pi-harness JSON run export.");
  return { id: `pi-export:${idOf(run.id)}`, kind: "pi-export", externalId: clean(run.id), status: clean(run.status, 40), at: time(run.finishedAt ?? run.startedAt), startedAt: time(run.startedAt), model: clean(run.model), provider: clean(run.provider), tokens: number(run.usage?.totalTokens), cost: number(run.usage?.estimatedCost), artifacts: Array.isArray(value.artifacts) ? value.artifacts.length : 0, diagnostics: Array.isArray(value.diagnostics) ? value.diagnostics.length : 0, checkpoints: Array.isArray(value.trace?.checkpoints) ? value.trace.checkpoints.length : 0 };
}
export function piSessionMetadata(content, file) {
  let skipped = 0;
  const entries = content.split(/\r?\n/).flatMap((line) => { if (!line.trim()) return []; try { const e = JSON.parse(line); return e && typeof e === "object" ? [e] : []; } catch { skipped++; return []; } });
  const header = entries[0];
  if (header?.type !== "session" || typeof header.id !== "string") fail("Not a Pi session.");
  if (number(header.version) > 3) fail("This Pi session version is newer than XenoNet supports.");
  const byId = new Map(entries.filter((e) => e.id).map((e) => [e.id, e]));
  const branch = [], seen = new Set();
  let leaf = entries.at(-1);
  if (leaf?.parentId !== undefined) {
    while (leaf && !seen.has(leaf.id) && leaf.type !== "session") { seen.add(leaf.id); branch.unshift(leaf); leaf = byId.get(leaf.parentId); }
  } else branch.push(...entries.slice(1));
  let model = "", provider = "", tokens = 0, cost = 0, tools = 0, failures = 0, checkpoints = 0;
  const pending = new Set();
  for (const entry of branch) {
    if (entry.type === "model_change") { model = clean(entry.modelId); provider = clean(entry.provider); }
    if (entry.type === "compaction" || entry.type === "branch_summary") checkpoints++;
    const m = entry.message;
    if (m?.role === "assistant") {
      model = clean(m.model || model); provider = clean(m.provider || provider);
      tokens += number(m.usage?.totalTokens); cost += number(m.usage?.cost?.total);
      for (const c of Array.isArray(m.content) ? m.content : []) if (c.type === "toolCall" && c.id) { tools++; pending.add(c.id); }
    }
    if (m?.role === "toolResult") { pending.delete(m.toolCallId); if (m.isError === true) failures++; }
  }
  return { id: `pi-session:${idOf(file)}`, kind: "pi-session", externalId: clean(header.id), status: pending.size ? "awaiting-result" : "recorded", at: time(branch.at(-1)?.timestamp ?? header.timestamp), startedAt: time(header.timestamp), cwd: clean(header.cwd, 1024), model, provider, tokens, cost, tools, failures, checkpoints, skipped, pending: pending.size, note: "Read-only recorded metadata for the latest branch. A pending result does not establish a live or stuck process." };
}
export async function scanPi(root, { maxFiles = 500, maxBytes = 8 * 1024 * 1024 } = {}) {
  const records = [], warnings = [];
  let visited = 0;
  async function walk(dir, depth) {
    let entries;
    try { entries = await fs.readdir(dir, { withFileTypes: true }); } catch { if (!depth) warnings.push("Pi sessions directory is unavailable."); return; }
    for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      if (++visited > maxFiles * 4) { warnings.push("Directory scan limit reached."); return; }
      if (records.length >= maxFiles) { warnings.push("Session limit reached."); return; }
      const file = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory() && depth < 3) await walk(file, depth + 1);
      else if (entry.isFile() && entry.name.endsWith(".jsonl")) {
        let handle;
        try {
          // Open the descriptor once and read at most the bound, even if a
          // session grows during the read. Sources are never changed.
          handle = await fs.open(file, "r");
          const st = await handle.stat();
          if (!st.isFile() || st.size > maxBytes) { warnings.push("A session exceeded the file size limit."); continue; }
          const buffer = Buffer.alloc(maxBytes + 1);
          const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
          if (bytesRead > maxBytes) { warnings.push("A session grew beyond the file size limit."); continue; }
          records.push(piSessionMetadata(buffer.subarray(0, bytesRead).toString("utf8"), file));
        } catch { warnings.push("A session could not be read or has an unsupported format."); }
        finally { await handle?.close(); }
      }
    }
  }
  await walk(root, 0);
  return { records, warnings: [...new Set(warnings)] };
}
