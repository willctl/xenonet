// Grok Build adapter. Grok Build (SpaceXAI's coding CLI, `grok`) keeps each
// session in a folder of its own, ~/.grok/sessions/<encoded cwd>/<session
// id>/ (GROK_HOME moves ~/.grok), and has no listing API to ask: `grok
// sessions list` prints one folder's sessions, for people. So XenoNet reads
// the folders. summary.json is a session's index entry (title, model,
// times, the session it came from), updates.jsonl its conversation as ACP
// session updates, usage.json its tokens and cost, and subagents/*/meta.json
// says which sessions are its subagents. Only summary.json's fields are in
// Grok Build's guide, so each file is read leniently against its schema
// (vendor.js), every field is one XenoNet can do without, and
// docs/design/grok-build.md lists what's still to confirm.
//
// A Grok session is grok:<session id> to the rest of XenoNet. Renaming one
// writes its summary.json as /rename does (the title, marked as yours); a
// desktop app that hosts Grok Build sessions would slot in beside this the
// way Claude desktop does beside Claude Code (claude.js), with its own
// records for titles and "open in app".
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { config } from "./config.js";
import { excerpt } from "./units.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader } from "./vendor.js";
import { readNewLines, grokTime } from "./lines.js";

export { grokCounts, readGrokUsage, grokCost } from "./grok-usage.js";

const summaries = reader("spacexai/grok-build/session-summary");
const subagentMetas = reader("spacexai/grok-build/subagent-meta");

// Where each session's folder is, by its id, as the last listing found
// them: for its transcript, its usage, and renaming it.
const folderOf = new Map();

const toMs = (t) => {
  const ms = grokTime(t);
  return Number.isFinite(ms) ? ms : null;
};
const firstLine = (s, n = 80) => {
  const t = String(s ?? "").trim().split(/\r?\n/)[0].trim();
  return t.length > n ? `${t.slice(0, n - 1)}…` : t;
};
const readJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return undefined;
  }
};
const isDir = (p) => {
  try {
    return fs.statSync(p).isDirectory();
  } catch {
    return false;
  }
};
const dirsIn = (dir) => {
  try {
    return fs.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory() && !e.name.startsWith(".")).map((e) => e.name);
  } catch {
    return [];
  }
};

// The folder a group of sessions works in, from the group's name: Grok
// URL-encodes the working directory, and when that's over 255 bytes uses a
// slug and a hash instead and writes the folder to a .cwd file in the
// group. Null when neither says (each session's summary.json may).
export function groupCwd(groupDir, name = path.basename(groupDir)) {
  try {
    const written = fs.readFileSync(path.join(groupDir, ".cwd"), "utf8").trim();
    if (written) return written;
  } catch {}
  if (!name.includes("%")) return null;
  try {
    const decoded = decodeURIComponent(name);
    return /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(decoded) ? decoded : null;
  } catch {
    return null;
  }
}

// The repo a Grok worktree was made from. Grok makes them under
// ~/.grok/worktrees/<repo>/<name> (and keeps spares in worktree_pool); a
// worktree's .git file points into the repo's .git/worktrees/<name>.
export function worktreeOrigin(cwd, home = config.grokHome) {
  if (!cwd || !home) return null;
  const rel = path.relative(home, cwd);
  const parts = rel.split(/[\\/]/);
  if (rel.startsWith("..") || path.isAbsolute(rel) || !["worktrees", "worktree_pool"].includes(parts[0]) || parts.length < 3) return null;
  const top = path.join(home, ...parts.slice(0, 3));
  try {
    const gitdir = /^gitdir:\s*(.+)$/m.exec(fs.readFileSync(path.join(top, ".git"), "utf8"))?.[1]?.trim();
    const m = gitdir && /^(.*?)[\\/]\.git[\\/]worktrees[\\/][^\\/]+[\\/]?$/.exec(path.resolve(top, gitdir));
    return m ? { repo: m[1], top } : { repo: null, top };
  } catch {
    return { repo: null, top };
  }
}

// A subagent's meta.json, as the session it runs as and what it was asked.
function subagentsOf(sessionDir) {
  const out = [];
  const dir = path.join(sessionDir, "subagents");
  for (const name of dirsIn(dir)) {
    const raw = readJson(path.join(dir, name, "meta.json"));
    if (raw === undefined) continue;
    const m = subagentMetas.sample(raw);
    if (!m) continue;
    const child = [m.child_session_id, m.childSessionId, m.session_id, m.sessionId].find((x) => typeof x === "string" && x);
    if (child) out.push({ child, type: m.subagent_type ?? m.subagentType ?? m.agent_type ?? null, description: m.description ?? null, worktree: m.worktree_path ?? null });
  }
  return out;
}

// Background work: a session Grok marks hidden, or one run headless (`grok
// -p`), as `codex exec` runs are for Codex. Grok's guide doesn't list
// session_kind's values, so the headless ones are told by their name.
const headless = (kind) => /headless|print|exec|batch|automation/i.test(String(kind ?? ""));

// Grok Build's sessions, as XenoNet lists sessions (session/v1).
export function listGrokSessions({ home = config.grokHome } = {}) {
  const root = path.join(home, "sessions");
  const found = [];
  const children = new Map(); // child session id → its parent's and what it was asked
  for (const group of dirsIn(root)) {
    const groupDir = path.join(root, group);
    const cwd = groupCwd(groupDir, group);
    for (const id of dirsIn(groupDir)) {
      const dir = path.join(groupDir, id);
      const raw = readJson(path.join(dir, "summary.json"));
      // A folder without a summary yet is a session being made, unless its
      // conversation is there already.
      if (raw === undefined && !fs.existsSync(path.join(dir, "updates.jsonl"))) continue;
      const s = raw === undefined ? {} : summaries.sample(raw);
      if (!s) continue;
      for (const sub of subagentsOf(dir)) children.set(sub.child, { parent: id, ...sub });
      found.push({ id, dir, cwd, s });
    }
  }
  folderOf.clear();
  return found.map(({ id, dir, cwd, s }) => {
    const info = s.info && typeof s.info === "object" ? s.info : {};
    const rawId = [info.id, info.session_id, info.sessionId].find((x) => typeof x === "string" && x) ?? id;
    folderOf.set(rawId, dir);
    const where = info.cwd ?? cwd ?? null;
    // A session in one of Grok's worktrees belongs to the repo it came from.
    const wt = worktreeOrigin(where, home);
    const origin = wt ? (s.source_workspace_dir && !worktreeOrigin(s.source_workspace_dir, home) ? s.source_workspace_dir : wt.repo) : null;
    const sub = children.get(rawId);
    const parent = sub?.parent ?? s.parent_session_id ?? null;
    let size = null, written = null;
    try {
      const st = fs.statSync(path.join(dir, "updates.jsonl"));
      [size, written] = [st.size, st.mtimeMs];
    } catch {}
    return {
      id: `grok:${rawId}`,
      app: "grok",
      rawId,
      title: firstLine(s.generated_title) || firstLine(s.session_summary) || firstLine(s.agent_name) || "(untitled session)",
      titleManual: s.title_is_manual === true,
      preview: s.last_turn_summary ?? s.last_recap ?? s.session_summary ?? null,
      cwd: origin ?? where,
      worktree: wt ? where : null,
      createdAt: toMs(s.created_at),
      // A session with no summary yet was last written when its updates were.
      updatedAt: toMs(s.last_active_at) ?? toMs(s.updated_at) ?? toMs(s.created_at) ?? written,
      archived: false,
      status: null,
      model: s.current_model_id ?? null,
      branch: s.head_branch ?? null,
      source: s.session_kind ?? null,
      parentId: parent ? `grok:${parent}` : null,
      parentKind: sub ? "subagent" : parent ? "fork" : null,
      agentNickname: sub?.description ?? null,
      agentKind: sub ? (sub.type ?? "spawned") : null,
      background: s.hidden === true || headless(s.session_kind),
      sizeBytes: size,
    };
  });
}

// Grok Build's sessions as a gatherer (core/blocks/gatherer.js): listed
// again on every refresh, timed, and empty after a listing that failed
// (the map keeps the sessions it had; see state.js). `gather` stands in for
// the listing, as the demo's does.
export const grokSessionsGatherer = ({ gather = () => listGrokSessions(), clock } = {}) => new Gatherer({ name: "grok-sessions", gather, fallback: [], clock });

// A session's folder: where the last listing found it, or looked for.
export function grokFolder(rawId, { home = config.grokHome } = {}) {
  const known = folderOf.get(rawId);
  if (known && isDir(known)) return known;
  const root = path.join(home, "sessions");
  for (const group of dirsIn(root)) {
    const dir = path.join(root, group, rawId);
    if (isDir(dir)) return dir;
  }
  return null;
}

// ---------- renaming ----------

// Renames a session where Grok Build keeps its name, as /rename does: the
// summary's title, marked as yours, so Grok's own titling leaves it alone.
// The file is read again just before the write and left alone unless it's
// still this session's. A session open in Grok Build keeps its summary in
// memory and writes it again as it works, so it may show its old name
// until it next starts (docs/design/grok-build.md).
export async function renameGrokSession(rawId, title, { home = config.grokHome } = {}) {
  const dir = grokFolder(rawId, { home });
  if (!dir) throw new Error("XenoNet can't find this Grok Build session's folder");
  const file = path.join(dir, "summary.json");
  const record = JSON.parse(await fsp.readFile(file, "utf8"));
  if (!record || typeof record !== "object" || Array.isArray(record)) throw new Error("Grok Build's summary of this session isn't one XenoNet can change");
  const says = [record.info?.id, record.info?.session_id, record.info?.sessionId].find((x) => typeof x === "string" && x);
  if (says && says !== rawId) throw new Error("Grok Build's summary is of another session now");
  record.generated_title = title;
  record.title_is_manual = true;
  const tmp = `${file}.xenonet-${process.pid}.tmp`;
  await fsp.writeFile(tmp, JSON.stringify(record, null, 2));
  // Windows refuses the rename for a moment while another program has the file open.
  for (let attempt = 0; ; attempt++) {
    try {
      return await fsp.rename(tmp, file);
    } catch (err) {
      if (attempt >= 4 || !["EPERM", "EBUSY", "EACCES"].includes(err.code)) {
        await fsp.rm(tmp, { force: true });
        throw err;
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  }
}

// ---------- the conversation ----------

// A session's updates as the console shows them, oldest first: each run of
// chunks of one kind as one message, and each tool call with what its
// updates said of it.
export function grokEntries(lines) {
  const out = [];
  const calls = new Map();
  let run = null; // the message being put together from chunks
  for (const [n, o] of lines.entries()) {
    const kind = o.kind;
    const ts = Number.isFinite(o.ts) ? o.ts : null;
    const role = kind === "user_message_chunk" ? "user" : kind === "agent_message_chunk" ? "assistant" : kind === "agent_thought_chunk" ? "reasoning" : null;
    if (role) {
      if (run?.role === role) run.text += o.text;
      else out.push((run = { id: `update-${n}`, ts, kind, role, text: o.text }));
      continue;
    }
    run = null;
    if (kind === "tool_call" || kind === "tool_call_update") {
      const u = o.update;
      const id = o.toolCallId ?? `call-${n}`;
      let e = calls.get(id);
      if (!e) {
        // Its title says what it does ("npm test", "Edit refunds.ts"), and
        // the tool's name goes beside it.
        e = { id, ts, kind: "tool_call", role: "tool", title: u.title || o.toolName || "tool", meta: u.title ? o.toolName : null, status: null, text: "" };
        calls.set(id, e);
        out.push(e);
      }
      if (u.status) e.status = u.status === "in_progress" || u.status === "pending" ? e.status : u.status;
      const diffs = o.diffs;
      const input = u.rawInput !== undefined && kind === "tool_call" ? (typeof u.rawInput === "string" ? u.rawInput : JSON.stringify(u.rawInput, null, 2)) : "";
      const said = diffs.length ? diffs.map((d) => `edit ${d.path}`).join("\n") : o.toolText || (typeof u.rawOutput === "string" ? u.rawOutput : u.rawOutput !== undefined ? JSON.stringify(u.rawOutput, null, 2) : "");
      e.text = excerpt([e.text, input, said].filter(Boolean).join("\n"), 2500);
    } else if (kind === "plan") {
      const entries = Array.isArray(o.update.entries) ? o.update.entries : [];
      out.push({ id: `update-${n}`, ts, kind, role: "assistant", title: "plan", text: entries.map((p) => `${p?.status === "completed" ? "✓" : "·"} ${p?.content ?? ""}`).join("\n") });
    }
  }
  return out.filter((e) => e.text || e.title);
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
  const st = await fsp.stat(file);
  let r = last?.file === file ? last : null;
  r ??= last = { file, from: null, lines: [] };
  if (r.from?.offset !== st.size) {
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

// ---------- which sessions are open ----------

// The grok processes running now, with the folder each works in, from the
// process table (core/procs.js): [{ pid, start, cwd }]. A grok under
// another grok (a subagent's, a helper) is its parent's.
export async function grokProcesses(procs) {
  if (!procs?.supported) return [];
  const { table, of } = await procs.census();
  const ours = [...table.values()].filter((p) => of(p).agent === "grok" && of(table.get(p.ppid)).agent !== "grok");
  if (!ours.length) return [];
  const cwds = await procs.cwds(ours.map((p) => p.pid));
  return ours.map((p) => ({ pid: p.pid, start: p.start, cwd: cwds.get(p.pid) ?? null }));
}

// Marks the sessions a running grok has open: in each process's folder,
// the session written to last since the process started (a grok works on
// one session at a time, and writes to it as it goes), else the newest
// one there, for a session just resumed and not written to yet.
export function markRunning(sessions, processes) {
  const key = (p) => (p ? path.resolve(p).replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase() : null);
  for (const s of sessions) s.running = false;
  for (const proc of processes) {
    const here = key(proc.cwd);
    if (!here) continue;
    const mine = sessions.filter((s) => !s.parentKind || s.parentKind === "fork").filter((s) => key(s.worktree ?? s.cwd) === here);
    if (!mine.length) continue;
    const since = mine.filter((s) => (s.updatedAt ?? 0) >= (proc.start ?? 0) - 5000);
    const pick = (since.length ? since : mine).reduce((a, b) => ((b.updatedAt ?? 0) > (a.updatedAt ?? 0) ? b : a));
    pick.running = true;
  }
  return sessions;
}
