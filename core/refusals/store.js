// Every refusal XenoNet sees, kept on this computer to build a dataset from
// (docs/guide/refusals.md): one folder each under <state>/refusals, with
// record.json (refusal/record/v1) and, when you've turned on keeping
// context, what the model had been given up to it:
//
//   context.jsonl.gz  the transcript's own lines, as the app wrote them:
//                     for Claude Code, the chain of messages back from the
//                     refusal to the last compaction; for Codex, the
//                     session's settings and turn contexts and the items
//                     since its last compaction
//   request.json      the same as a request would carry it, ready to
//                     tokenize: { provider, model, system, messages } for
//                     Claude, { provider, model, instructions, input } for
//                     Codex. What a transcript doesn't hold (the system
//                     prompt and tool definitions Claude Code sends, what
//                     a provider adds on its side) is listed in `missing`
//
// Keeping context is off unless you turn it on: transcripts can hold
// anything typed into them, secrets included, and these copies outlive
// the apps' own. With it off, the record keeps the refusal itself (its
// category, the provider's words, the model and the ids) and where the
// transcript is, and `capture` adds the context later while the
// transcript is still there. Nothing here leaves this computer; export
// writes files for you, scrubbed of home folders and keys if you ask.
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import crypto from "node:crypto";
import os from "node:os";
import { tilde } from "../paths.js";
import { scrub } from "../log.js";
import { keyOf } from "./detect.js";

export const SCHEMA = "refusal/record/v1";
const CONTEXT_MAX = 16 << 20;
const LONG = 20000;

const readJsonl = (file) => {
  const out = [];
  for (const line of fs.readFileSync(file, "utf8").split("\n")) {
    if (!line.trim()) continue;
    try {
      out.push(JSON.parse(line));
    } catch {}
  }
  return out;
};

// Long strings (tool output, file contents) cut to their head and tail,
// so a context fits; says whether anything was cut.
function trimmed(lines) {
  let cut = false;
  const trim = (v) => {
    if (typeof v === "string" && v.length > LONG) {
      cut = true;
      return `${v.slice(0, LONG / 2)}\n[… ${v.length - LONG} characters cut by XenoNet …]\n${v.slice(-LONG / 2)}`;
    }
    if (Array.isArray(v)) return v.map(trim);
    if (v && typeof v === "object") return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, trim(x)]));
    return v;
  };
  return { lines: lines.map(trim), cut };
}

// Claude Code: the chain from the refusal back through parentUuid to the
// last compaction, in order, with the system lines on it.
export function claudeContext(all, sig) {
  const byUuid = new Map(all.filter((l) => l.uuid).map((l) => [l.uuid, l]));
  const end = (sig.uuid && byUuid.get(sig.uuid)) || all.findLast((l) => (sig.requestId && l.requestId === sig.requestId) || (sig.messageId && l.message?.id === sig.messageId));
  if (!end) return null;
  const chain = [];
  const seen = new Set();
  for (let l = end; l && !seen.has(l.uuid); l = l.parentUuid ? byUuid.get(l.parentUuid) : null) {
    seen.add(l.uuid);
    chain.push(l);
    if (l.type === "system" && l.subtype === "compact_boundary") break;
  }
  chain.reverse();
  const messages = chain.filter((l) => (l.type === "user" || l.type === "assistant") && l.message && !(l.type === "assistant" && l.isApiErrorMessage)).map((l) => ({ role: l.message.role ?? l.type, content: l.message.content }));
  const model = sig.model ?? chain.findLast((l) => l.type === "assistant" && l.message?.model && l.message.model !== "<synthetic>")?.message.model ?? null;
  return { lines: chain, request: { provider: "anthropic", model, system: null, messages }, missing: ["system prompt (Claude Code doesn't write it to the transcript)", "tool definitions", "context Claude Code adds at request time (CLAUDE.md, reminders)"] };
}

// Codex: the session's settings, its turn contexts, and what came since
// its last compaction, up to the refusal.
export function codexContext(all, sig) {
  let end = all.findIndex((l) => l.type === "event_msg" && l.payload?.type === (sig.kind === "reroute" ? "model_reroute" : "error") && Date.parse(l.timestamp) === sig.at);
  if (end < 0) end = all.length - 1;
  const upTo = all.slice(0, end + 1);
  const lastCompact = upTo.findLastIndex((l) => l.type === "compacted");
  const meta = upTo.find((l) => l.type === "session_meta");
  const lines = [...(meta && upTo.indexOf(meta) < lastCompact ? [meta] : []), ...upTo.slice(Math.max(0, lastCompact))];
  const input = [...(lastCompact >= 0 ? upTo[lastCompact].payload?.replacement_history ?? [] : []), ...lines.filter((l) => l.type === "response_item").map((l) => l.payload)];
  const ctx = upTo.findLast((l) => l.type === "turn_context")?.payload ?? {};
  return { lines, request: { provider: "openai", model: ctx.model ?? sig.model ?? null, effort: ctx.effort ?? null, instructions: meta?.payload?.base_instructions?.text ?? meta?.payload?.instructions ?? null, input }, missing: ["tool definitions", "what OpenAI adds to the prompt on its side", "encrypted reasoning's content"] };
}

// Whether to keep context: XENONET_REFUSAL_CONTEXT (on or off) when set,
// else your choice, in <state>/refusals/settings.json (off until you turn
// it on: xenonet refusals context on, or the palette).
export function contextOn(dir, env = process.env) {
  if (env.XENONET_REFUSAL_CONTEXT === "on" || env.XENONET_REFUSAL_CONTEXT === "off") return env.XENONET_REFUSAL_CONTEXT === "on";
  try {
    return JSON.parse(fs.readFileSync(path.join(dir, "settings.json"), "utf8")).context === true;
  } catch {
    return false;
  }
}
export function setContext(dir, on) {
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  fs.writeFileSync(path.join(dir, "settings.json"), `${JSON.stringify({ context: !!on })}\n`, { mode: 0o600 });
  return !!on;
}

// `dir`: <state>/refusals. `context()`: whether to keep context.
export function createRefusals({ dir, home = os.homedir(), now = Date.now, context = () => false }) {
  const indexFile = path.join(dir, "index.jsonl");
  let keys = null;
  const ensure = () => fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  function loadKeys() {
    if (keys) return keys;
    keys = new Set();
    try {
      for (const line of fs.readFileSync(indexFile, "utf8").split("\n")) {
        try {
          keys.add(JSON.parse(line).key);
        } catch {}
      }
    } catch {}
    return keys;
  }
  const folderOf = (id) => path.join(dir, id.replace(/[^a-z0-9-]/gi, ""));
  const write = (file, data) => fs.writeFileSync(file, data, { mode: 0o600 });

  function addContext(rec) {
    const file = rec.source?.path;
    if (!file || !fs.existsSync(file)) return { ok: false, why: "its transcript isn't there any more" };
    const all = readJsonl(file);
    const got = rec.provider === "openai" ? codexContext(all, rec.signal) : claudeContext(all, rec.signal);
    if (!got) return { ok: false, why: "its line isn't in the transcript" };
    let { lines, cut } = trimmed(got.lines);
    let text = lines.map((l) => JSON.stringify(l)).join("\n");
    if (Buffer.byteLength(text) > CONTEXT_MAX) {
      lines = lines.slice(-Math.max(1, Math.floor(lines.length / 2)));
      text = lines.map((l) => JSON.stringify(l)).join("\n");
      cut = true;
    }
    const folder = folderOf(rec.id);
    write(path.join(folder, "context.jsonl.gz"), zlib.gzipSync(`${text}\n`));
    write(path.join(folder, "request.json"), `${JSON.stringify({ ...got.request, ...(cut ? { trimmed: true } : {}) }, null, 1)}\n`);
    rec.context = { file: "context.jsonl.gz", request: "request.json", lines: lines.length, bytes: Buffer.byteLength(text), sha256: crypto.createHash("sha256").update(text).digest("hex"), truncated: cut };
    rec.missing = got.missing;
    return { ok: true };
  }

  // A refusal seen in a session's transcript: a new record, or null for
  // one already kept. `at`: { sessionId, app, agentId, file, cwd, model }.
  function record(sig, at = {}) {
    const key = keyOf(sig, at.sessionId);
    if (loadKeys().has(key)) return null;
    ensure();
    const t = now();
    const id = `${new Date(sig.at ?? t).toISOString().slice(0, 10).replace(/-/g, "")}-${crypto.randomBytes(4).toString("hex")}`;
    const folder = folderOf(id);
    fs.mkdirSync(folder, { mode: 0o700 });
    const rec = {
      schema: SCHEMA,
      id,
      key,
      at: sig.at ?? t,
      recordedAt: t,
      provider: sig.provider,
      app: at.app ?? null,
      kind: sig.kind,
      session: { id: at.sessionId ?? null, agentId: at.agentId ?? null, cwd: at.cwd ? tilde(at.cwd, home) : null, model: sig.model ?? at.model ?? null },
      signal: sig,
      source: at.file ? { file: tilde(at.file, home), path: at.file } : null,
      context: null,
      missing: [],
    };
    if (context()) {
      try {
        addContext(rec);
      } catch {}
    }
    write(path.join(folder, "record.json"), `${JSON.stringify(rec, null, 1)}\n`);
    fs.appendFileSync(indexFile, `${JSON.stringify({ id, key, at: rec.at, provider: rec.provider, app: rec.app, kind: rec.kind, category: sig.category, model: rec.session.model, session: rec.session.id, context: !!rec.context })}\n`, { mode: 0o600 });
    keys.add(key);
    return rec;
  }

  function get(id) {
    try {
      return JSON.parse(fs.readFileSync(path.join(folderOf(id), "record.json"), "utf8"));
    } catch {
      return null;
    }
  }

  // The records, newest first: since `since` (ms), of a provider or kind.
  function list({ since = 0, provider = null, kind = null } = {}) {
    let names = [];
    try {
      names = fs.readdirSync(dir).filter((n) => /^\d{8}-[0-9a-f]{8}$/.test(n));
    } catch {
      return [];
    }
    return names
      .map(get)
      .filter((r) => r && r.at >= since && (!provider || r.provider === provider || r.app === provider) && (!kind || r.kind === kind))
      .sort((a, b) => b.at - a.at);
  }

  // Adds context to a record kept without it, from its transcript if it's
  // still there.
  function capture(id) {
    const rec = get(id);
    if (!rec) return { ok: false, why: "no such record" };
    if (rec.context) return { ok: true, already: true };
    const r = addContext(rec);
    if (r.ok) write(path.join(folderOf(id), "record.json"), `${JSON.stringify(rec, null, 1)}\n`);
    return r;
  }

  // The records as JSON lines for a dataset, each with its context's
  // request when asked; scrubbed of home folders and keys when asked.
  function exportLines({ since = 0, provider = null, kind = null, withContext = false, scrubbed = false } = {}) {
    const deep = (v) => (typeof v === "string" ? scrub(v, home) : Array.isArray(v) ? v.map(deep) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, deep(x)])) : v);
    const clean = (v) => (scrubbed ? deep(v) : v);
    return list({ since, provider, kind }).map((r) => {
      const { source, key, ...rest } = r;
      let request = null;
      if (withContext && r.context) {
        try {
          request = JSON.parse(fs.readFileSync(path.join(folderOf(r.id), "request.json"), "utf8"));
        } catch {}
      }
      return clean({ ...rest, source: source ? { file: source.file } : null, ...(withContext ? { request } : {}) });
    });
  }

  const contextFile = (id) => path.join(folderOf(id), "context.jsonl.gz");
  return { record, get, list, capture, exportLines, contextFile, dir };
}
