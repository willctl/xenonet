// XenoNet's error log: what went wrong in each part of XenoNet, kept where you
// can find it (docs/guide/troubleshooting.md). Each part writes its own
// file of JSON lines, log/entry/v1, under <state>/logs:
//
//   service.ndjson   the background service
//   desktop.ndjson   the desktop app, and the page in it
//   cli.ndjson       the xenonet command, its hooks and MCP servers
//
// An entry has a code that says what it is (service.port-in-use,
// doctor.claude.auth, page.uncaught), XenoNet's words, the fix when there's
// one, and a stack for what XenoNet didn't expect. Everything is scrubbed as
// it's written: the home folder becomes ~ and anything that looks like a
// key is taken out, so the log is safe to read out to an agent or to
// copy into an issue. A file over 1 MB moves aside, and two are kept; the
// same code with the same words is written at most every ten seconds,
// with how many times it happened meanwhile.
//
// xenonet logs reads them, with XenoNet running or not (cli/commands/logs.js),
// and so do GET /api/logs, the doctor's Recent errors and xenonet diagnose.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { tilde } from "./paths.js";

export const LEVELS = ["error", "warn", "info"];
export const COMPONENTS = ["service", "desktop", "cli"];
const MAX_BYTES = 1 << 20;
const KEEP = 2;
const AGAIN_MS = 10e3;
const MESSAGE_MAX = 2000;
const STACK_MAX = 4000;

// Words that start the way keys and tokens do, bearer tokens and JWTs:
// taken out wherever they turn up.
const SECRETS = /(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)[A-Za-z0-9_-]{6,}|\bBearer\s+[A-Za-z0-9._~+/=-]{8,}|\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}|-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g;

// Text as the log keeps it: the home folder as ~ (in its path forms, and
// as Claude Code writes it into folder names, C--Users-nova), keys out,
// control characters but line breaks out.
export function scrub(text, home = os.homedir()) {
  if (text == null) return text;
  let out = tilde(String(text), home);
  const h = String(home ?? "").replace(/[\\/]+$/, "");
  if (h.length > 2) out = out.replace(new RegExp(h.replace(/[\\/.:]/g, "-").replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "(?![\\w.])", "gi"), "~");
  return out.replace(SECRETS, "[key]").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");
}

// Whether text still holds the home folder or a key, after scrubbing:
// what leaves XenoNet (xenonet diagnose) is checked with this last.
export function unsafe(text, home = os.homedir()) {
  const h = String(home ?? "").replace(/[\\/]+$/, "");
  // And as JSON writes it, backslashes doubled: what's checked is often a
  // bundle as JSON, where C:\Users\nova reads C:\\Users\\nova.
  const forms = h.length > 2 ? [h, h.replace(/\\/g, "/"), h.replace(/\//g, "\\"), h.replace(/\\/g, "\\\\")] : [];
  const lower = String(text).toLowerCase();
  if (forms.some((f) => lower.includes(f.toLowerCase()))) return "the home folder";
  SECRETS.lastIndex = 0;
  if (SECRETS.test(String(text))) return "something that looks like a key";
  return null;
}

const scrubAll = (v, home, depth = 0) => {
  if (typeof v === "string") return scrub(v, home).slice(0, 500);
  if (Array.isArray(v)) return depth > 3 ? [] : v.slice(0, 20).map((x) => scrubAll(x, home, depth + 1));
  if (v && typeof v === "object") return depth > 3 ? {} : Object.fromEntries(Object.entries(v).slice(0, 30).map(([k, x]) => [k, scrubAll(x, home, depth + 1)]));
  return v;
};

export const logDir = (stateDir) => path.join(stateDir, "logs");
const fileOf = (dir, component) => path.join(dir, `${component}.ndjson`);

// A log for one part of XenoNet. `dir` is <state>/logs; null logs nothing,
// as in the demo. Never throws: a log that can't be written is let go.
export function createLog({ dir, component, instance = null, version = null, home = os.homedir(), now = Date.now, maxBytes = MAX_BYTES, mirror = null }) {
  const seen = new Map(); // code|message → { at, repeats }
  let ready = false;
  function write(level, code, message, { err = null, fix = null, ctx = null } = {}) {
    const text = scrub(String(message ?? err?.message ?? code), home).slice(0, MESSAGE_MAX);
    const key = `${code}|${text}`;
    const t = now();
    const had = seen.get(key);
    if (had && t - had.at < AGAIN_MS) {
      had.repeats++;
      return null;
    }
    seen.set(key, { at: t, repeats: 0 });
    if (seen.size > 500) seen.delete(seen.keys().next().value);
    const entry = {
      schema: "log/entry/v1",
      at: t,
      component,
      ...(instance ? { instance } : {}),
      ...(version ? { version } : {}),
      pid: process.pid,
      level,
      code: String(code),
      message: text,
      ...(had?.repeats ? { repeats: had.repeats } : {}),
      ...(fix ? { fix: scrub(fix, home).slice(0, 500) } : {}),
      ...(err?.stack && level === "error" ? { stack: scrub(err.stack, home).slice(0, STACK_MAX) } : {}),
      ...(err?.code && typeof err.code === "string" ? { errno: err.code } : {}),
      ...(ctx ? { ctx: scrubAll(ctx, home) } : {}),
    };
    mirror?.(entry);
    if (!dir) return entry;
    try {
      if (!ready) fs.mkdirSync(dir, { recursive: true, mode: 0o700 }), (ready = true);
      const file = fileOf(dir, component);
      let size = 0;
      try {
        size = fs.statSync(file).size;
      } catch {}
      if (size > maxBytes) rotate(file);
      fs.appendFileSync(file, `${JSON.stringify(entry)}\n`, { mode: 0o600 });
    } catch {}
    return entry;
  }
  return {
    error: (code, message, opts) => write("error", code, message, opts),
    warn: (code, message, opts) => write("warn", code, message, opts),
    info: (code, message, opts) => write("info", code, message, opts),
    file: dir ? fileOf(dir, component) : null,
  };
}

function rotate(file) {
  for (let i = KEEP; i >= 1; i--) {
    const from = i === 1 ? file : file.replace(/\.ndjson$/, `.${i - 1}.ndjson`);
    const to = file.replace(/\.ndjson$/, `.${i}.ndjson`);
    try {
      fs.renameSync(from, to);
    } catch {}
  }
}

// The entries in `dir`, newest first: since `since` (ms), at `level` or
// worse, of a component or a code (a prefix: doctor. is every check's).
export function readLogs(dir, { since = 0, level = "info", component = null, code = null, limit = 500 } = {}) {
  const worst = LEVELS.indexOf(level);
  const out = [];
  let names = [];
  try {
    names = fs.readdirSync(dir).filter((n) => /^[a-z]+(\.\d+)?\.ndjson$/.test(n));
  } catch {
    return [];
  }
  for (const n of names) {
    if (component && !n.startsWith(`${component}.`)) continue;
    let text = "";
    try {
      text = fs.readFileSync(path.join(dir, n), "utf8");
    } catch {
      continue;
    }
    for (const line of text.split("\n")) {
      if (!line) continue;
      let e;
      try {
        e = JSON.parse(line);
      } catch {
        continue;
      }
      if (e?.schema !== "log/entry/v1" || e.at < since) continue;
      if (LEVELS.indexOf(e.level) > worst) continue;
      if (code && !String(e.code).startsWith(code)) continue;
      out.push(e);
    }
  }
  return out.sort((a, b) => b.at - a.at).slice(0, limit);
}

// Entries by what they are: each code once, with how often, when first and
// last, and its latest words and fix; the worst and most recent first.
export function summarize(entries) {
  const by = new Map();
  for (const e of entries) {
    const k = `${e.component}|${e.code}`;
    const had = by.get(k);
    const n = 1 + (e.repeats ?? 0);
    if (!had) by.set(k, { code: e.code, component: e.component, level: e.level, count: n, first: e.at, last: e.at, message: e.message, fix: e.fix ?? null, version: e.version ?? null });
    else {
      had.count += n;
      had.first = Math.min(had.first, e.at);
      if (e.at > had.last) Object.assign(had, { last: e.at, message: e.message, fix: e.fix ?? had.fix, version: e.version ?? had.version });
      if (LEVELS.indexOf(e.level) < LEVELS.indexOf(had.level)) had.level = e.level;
    }
  }
  return [...by.values()].sort((a, b) => LEVELS.indexOf(a.level) - LEVELS.indexOf(b.level) || b.last - a.last);
}
