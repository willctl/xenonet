// Grok Build's sessions (its `sessions` capability, core/apps.js). Grok
// Build keeps each session in a folder of its own, ~/.grok/sessions/
// <encoded cwd>/<session id>/ (GROK_HOME moves ~/.grok), and has no listing
// to ask: `grok sessions list` prints one folder's sessions, for people. So
// XenoNet reads the folders: summary.json is a session's index entry (its
// id and folder, title, model, times, and the session it came from), read
// leniently against its schema (vendor.js), and updates.jsonl its
// conversation, which the line readers read (lines.js, events.js). A
// subagent's session is one like any other, whose summary names its parent.
//
// A session is grok:<session id> to the rest of XenoNet. The Rust has a twin
// of the listing (crates/xenonet-sources/src/grok.rs), which the binary
// runs.
import fs from "node:fs";
import path from "node:path";
import { config } from "./config.js";
import { excerpt } from "./units.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader } from "./vendor.js";
import { readNewLines } from "./lines.js";
import { grok } from "./events.js";

const summaries = reader("spacexai/grok-build/session-summary");

// A time as summary.json writes it (RFC 3339), or in Unix seconds or
// milliseconds; null when it isn't one.
export function grokTime(t) {
  const ms = typeof t === "string" ? Date.parse(t) : typeof t === "number" && t > 0 ? (t < 1e12 ? t * 1000 : t) : NaN;
  return Number.isFinite(ms) ? ms : null;
}

// Text's first line, trimmed, at most n characters.
function firstLine(s, n = 80) {
  const t = (typeof s === "string" ? s : "").trim().split(/\r?\n/)[0].trim();
  return t.length > n ? `${t.slice(0, n - 1)}…` : t;
}

const readJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return undefined;
  }
};

// The folders in a folder, sorted, without the hidden ones.
const dirsIn = (dir) => {
  try {
    return fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && !e.name.startsWith("."))
      .map((e) => e.name)
      .sort();
  } catch {
    return [];
  }
};

const absolute = (p) => /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(p);

// The folder a group of sessions works in, from the group's name: Grok
// URL-encodes the working directory, and when that's over 255 bytes uses a
// slug and a hash instead and writes the folder to a .cwd file in the
// group. Null when neither says (each session's summary may).
export function groupCwd(groupDir, name = path.basename(groupDir)) {
  try {
    const written = fs.readFileSync(path.join(groupDir, ".cwd"), "utf8").trim();
    if (written) return written;
  } catch {}
  if (!name.includes("%")) return null;
  try {
    const decoded = decodeURIComponent(name);
    return absolute(decoded) ? decoded : null;
  } catch {
    return null;
  }
}

// A session as the map lists it (session/v1), from its folder's summary
// (`s`, {} when it has none yet), the group's folder and its updates file.
function sessionOf(id, s, cwd, updates) {
  const info = s.info && typeof s.info === "object" && !Array.isArray(s.info) ? s.info : {};
  const rawId = typeof info.id === "string" && info.id ? info.id : id;
  const where = typeof info.cwd === "string" && info.cwd ? info.cwd : cwd;
  // A worktree session works in one of Grok's worktrees, and belongs to the
  // folder its work came from.
  const source = typeof s.source_workspace_dir === "string" && s.source_workspace_dir ? s.source_workspace_dir : null;
  const kind = typeof s.session_kind === "string" ? s.session_kind : null;
  const parent = typeof s.parent_session_id === "string" && s.parent_session_id ? s.parent_session_id : null;
  const subagent = !!kind?.startsWith("subagent");
  return {
    id: `grok:${rawId}`,
    app: "grok",
    rawId,
    title: firstLine(s.generated_title) || firstLine(s.session_summary) || "(untitled session)",
    preview: firstLine(s.session_summary, 200) || null,
    cwd: source ?? where ?? null,
    worktree: source ? (where ?? null) : null,
    createdAt: grokTime(s.created_at),
    // A session with no summary yet was last written when its updates were.
    updatedAt: grokTime(s.last_active_at) ?? grokTime(s.updated_at) ?? grokTime(s.created_at) ?? updates?.mtimeMs ?? null,
    archived: false,
    model: typeof s.current_model_id === "string" ? s.current_model_id : null,
    branch: typeof s.head_branch === "string" ? s.head_branch : null,
    status: null,
    source: kind,
    parentId: parent ? `grok:${parent}` : null,
    parentKind: subagent && parent ? "subagent" : parent ? "fork" : null,
    // Background work: a run Grok marks hidden, or one run headless (grok
    // -p), as `codex exec` runs are for Codex. A subagent is its parent's.
    background: s.hidden === true || kind === "headless",
    sizeBytes: updates?.size ?? null,
  };
}

// Grok Build's sessions, as XenoNet lists sessions (session/v1).
export function listGrokSessions({ home = config.grokHome } = {}) {
  const root = path.join(home, "sessions");
  const out = [];
  for (const group of dirsIn(root)) {
    const groupDir = path.join(root, group);
    const cwd = groupCwd(groupDir, group);
    for (const id of dirsIn(groupDir)) {
      const dir = path.join(groupDir, id);
      let updates = null;
      try {
        updates = fs.statSync(path.join(dir, "updates.jsonl"));
      } catch {}
      const raw = readJson(path.join(dir, "summary.json"));
      // A folder without a summary yet is a session being made, once its
      // conversation is there.
      if (raw === undefined && !updates) continue;
      const s = raw === undefined ? {} : summaries.sample(raw);
      if (!s || typeof s !== "object" || Array.isArray(s)) continue;
      out.push(sessionOf(id, s, cwd, updates));
    }
  }
  return out;
}

// Grok Build's sessions as a gatherer (core/blocks/gatherer.js): listed
// again on every refresh, timed, and empty after a listing that failed
// (the map keeps the sessions it had; see state.js). `gather` stands in for
// the listing, as the demo's does.
export const grokSessionsGatherer = ({ gather = async () => listGrokSessions(), clock } = {}) => new Gatherer({ name: "grok-sessions", gather, fallback: [], clock });

// A session's folder, by its id.
export function grokFolder(rawId, { home = config.grokHome } = {}) {
  const root = path.join(home, "sessions");
  for (const group of dirsIn(root)) {
    const dir = path.join(root, group, rawId);
    try {
      if (fs.statSync(dir).isDirectory()) return dir;
    } catch {}
  }
  return null;
}

// ---------- the conversation ----------

// A session's lines as the console shows them, oldest first, from their
// events (events.js): what you asked, what the agent said and thought (as
// Claude Code's thinking shows, folded), and each tool call with how it
// went. A run of chunks is one entry, at its first chunk's time, with the
// whole text its chunks bring.
export function grokEntries(lines) {
  const out = [];
  const calls = new Map();
  const file = grok.file();
  // The entry of the run of chunks still open, which its later chunks
  // carry on.
  let open = null;
  for (const [n, o] of lines.entries()) {
    const ts = Number.isFinite(o.ts) ? o.ts : null;
    const before = file.run;
    const events = grok.events(o, file);
    if (!file.run || file.run !== before) open = null;
    const entry = (role, text) => {
      const e = { id: `update-${n}`, ts, role, text: excerpt(text, 2500) };
      out.push(e);
      if (file.run) open = e;
    };
    for (const e of events) {
      if (e.kind === "prompt" && !e.meta && e.text.trim()) entry("user", e.text);
      else if (e.kind === "said") {
        const text = e.parts.filter((b) => b.type === "text").map((b) => b.text).join("");
        const thought = e.parts.filter((b) => b.type === "thinking").map((b) => b.thinking).join("");
        if (text.trim()) entry("assistant", text);
        if (thought.trim()) entry("reasoning", thought);
      } else if (e.kind === "more") {
        if (open) open.text = excerpt(e.whole, 2500);
        else if (!e.meta && e.whole.trim()) entry(e.of === "prompt" ? "user" : e.of === "thinking" ? "reasoning" : "assistant", e.whole);
      } else if (e.kind === "tool") {
        const u = o.update;
        const entry = { id: String(e.id ?? `call-${n}`), ts, role: "tool", kind: "tool_use", title: typeof u.title === "string" && u.title ? u.title : e.name, text: "", status: null };
        calls.set(entry.id, entry);
        out.push(entry);
      } else if (e.kind === "result") {
        const entry = calls.get(String(e.id));
        if (entry) entry.status = e.error ? "failed" : "completed";
      }
    }
  }
  return out;
}

// The updates read last, kept while they're paged through and read on from
// where they ended as they grow, as Codex's rollouts are (codex.js).
let last = null;
let reads = Promise.resolve();

// Newest page first; `cursor` is the count of entries already shown from
// the end.
export function readGrokSession(rawId, opts = {}) {
  const run = reads.then(() => page(rawId, opts));
  reads = run.catch(() => {});
  return run;
}

async function page(rawId, { cursor = null, limit = 120, home = config.grokHome } = {}) {
  const dir = grokFolder(rawId, { home });
  if (!dir) throw new Error("XenoNet can't find this Grok Build session's folder");
  const file = path.join(dir, "updates.jsonl");
  const size = fs.statSync(file).size;
  const r = last?.file === file ? last : (last = { file, from: null, lines: [] });
  if (r.from?.offset !== size) {
    const read = await readNewLines(file, "grok", r.from);
    if (read.restarted) r.lines = [];
    r.lines.push(...read.lines);
    r.from = read;
  }
  const all = grokEntries(r.lines);
  const shown = Number(cursor ?? 0);
  const end = Math.max(0, all.length - shown);
  const start = Math.max(0, end - limit);
  return { entries: all.slice(start, end), cursor: start > 0 ? String(all.length - start) : null, total: all.length };
}
