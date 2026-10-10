// Claude plan usage, asked of Anthropic the way Claude Code's /usage asks:
// GET api.anthropic.com/api/oauth/usage with Claude Code's own sign-in.
//
// Claude desktop's plan-usage-history.json used to cover this, but since
// October 2026 it polls only while its menu bar usage view has been opened
// lately, and Claude Code hands usage only to terminal status lines, not to
// the sessions the desktop app runs. So XenoNet asks itself.
//
// The sign-in is read where Claude Code keeps it (the "Claude
// Code-credentials" item in the login keychain on macOS, else
// .credentials.json in Claude's config folder) for each request, and is
// never stored, logged or refreshed. An expired token waits for Claude Code
// to refresh it, so XenoNet can't sign anyone out. The same item holds other
// tokens (MCP servers'); only the Claude one is kept.
import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Gatherer } from "./blocks/gatherer.js";
import { reader } from "./vendor.js";

// Both answers are read leniently against their schemas (vendor.js).
const anthropicAnswers = reader("anthropic/oauth/usage");
const desktopSamples = reader("anthropic/claude-desktop/plan-usage");

const VERSION = (() => {
  try {
    return JSON.parse(readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "package.json"), "utf8")).version;
  } catch {
    return "dev";
  }
})();
const USAGE_URL = "https://api.anthropic.com/api/oauth/usage";
// Asked at most this often; a reading this old says nothing about the
// windows now (see live.js).
const USAGE_EVERY_MS = 5 * 60e3;
const KEYCHAIN_SERVICE = "Claude Code-credentials";

function keychainSecret(service) {
  return new Promise((resolve) => {
    execFile("security", ["find-generic-password", "-s", service, "-w"], { timeout: 5000 }, (err, stdout) => resolve(err ? null : stdout));
  });
}

// { token, expiresAt } for Claude Code's sign-in, or null.
export async function readClaudeToken({ platform = process.platform, env = process.env, homeDir = os.homedir(), keychain = keychainSecret, readFile = (f) => fs.readFile(f, "utf8") } = {}) {
  const sources = [];
  if (platform === "darwin") sources.push(() => keychain(KEYCHAIN_SERVICE));
  sources.push(() => readFile((platform === "win32" ? path.win32 : path.posix).join(env.CLAUDE_CONFIG_DIR ?? (platform === "win32" ? path.win32 : path.posix).join(homeDir, ".claude"), ".credentials.json")).catch(() => null));
  for (const read of sources) {
    try {
      const oauth = JSON.parse((await read()) ?? "null")?.claudeAiOauth;
      if (typeof oauth?.accessToken === "string" && oauth.accessToken) return { token: oauth.accessToken, expiresAt: Number(oauth.expiresAt) || null };
    } catch {}
  }
  return null;
}

// The windows XenoNet shows, as percentages, with when each resets.
export function parseUsage(body, now = Date.now()) {
  const window = (w) => {
    const pct = Number(w?.utilization);
    if (!Number.isFinite(pct)) return { pct: null, resetsAt: null };
    const resetsAt = w.resets_at ? Date.parse(w.resets_at) : NaN;
    return { pct, resetsAt: Number.isFinite(resetsAt) ? resetsAt : null };
  };
  const five = window(body?.five_hour), seven = window(body?.seven_day);
  if (five.pct == null && seven.pct == null) return null;
  return { fiveHour: five.pct, sevenDay: seven.pct, resets: { fiveHour: five.resetsAt, sevenDay: seven.resetsAt }, observedAt: now, source: "anthropic" };
}

// One reading. `retryAfterMs` says how long to leave it when Anthropic
// asks for a pause; `reason` says why there's no reading.
export async function fetchClaudeUsage({ fetchImpl = globalThis.fetch, readToken = readClaudeToken, now = Date.now(), userAgent = `xenonet/${VERSION}` } = {}) {
  const cred = await readToken();
  if (!cred) return { reason: "no Claude Code sign-in found" };
  if (cred.expiresAt && cred.expiresAt <= now) return { reason: "Claude Code's sign-in has expired; it refreshes on its next request" };
  let res;
  try {
    res = await fetchImpl(USAGE_URL, {
      headers: { authorization: `Bearer ${cred.token}`, "anthropic-beta": "oauth-2025-04-20", "content-type": "application/json", "user-agent": userAgent },
      redirect: "error",
      signal: AbortSignal.timeout(8000),
    });
  } catch (err) {
    return { reason: `couldn't reach Anthropic (${err.name === "TimeoutError" ? "timed out" : err.message})` };
  }
  if (res.status === 429 || res.status === 403) {
    const after = Number(res.headers.get("retry-after"));
    return { reason: `Anthropic asked to wait (${res.status})`, retryAfterMs: Number.isFinite(after) && after > 0 ? after * 1000 : 15 * 60e3 };
  }
  if (res.status === 401) return { reason: "Claude Code's sign-in was refused; it refreshes on its next request" };
  if (!res.ok) return { reason: `Anthropic answered ${res.status}` };
  const body = await res.json().catch(() => undefined);
  if (body === undefined) anthropicAnswers.unparsable();
  const usage = parseUsage(body === undefined ? null : anthropicAnswers.check(body), now);
  return usage ? { usage } : { reason: "Anthropic's answer had no usage windows" };
}

// Readings as gatherers (core/blocks/gatherer.js), which the live state
// asks every 30 seconds (see live.js):
//
// Anthropic's, at most every 5 minutes, or later when it asks for a pause.
// `fetch` gives { usage } or { reason, retryAfterMs? }; one that throws
// gives its error as the reason.
export const claudeUsageGatherer = ({ fetch = fetchClaudeUsage, clock } = {}) =>
  new Gatherer({
    name: "claude-usage",
    gather: async () => {
      try {
        return await fetch();
      } catch (err) {
        return { reason: String(err?.message ?? err) };
      }
    },
    everyMs: USAGE_EVERY_MS,
    nextIn: (r) => (r.usage ? null : r.retryAfterMs),
    clock,
  });

// Claude desktop's own samples ({t, u: {fh, sd}}: five-hour and seven-day
// percentages; undocumented), which it now writes only while its menu bar
// usage view has been opened lately. Each read, null while there's no
// file; a file that can't be read keeps the samples there were.
export const desktopUsageGatherer = ({ dir, clock } = {}) =>
  new Gatherer({
    name: "claude-desktop-usage",
    gather: async () => {
      let text;
      try {
        text = await fs.readFile(path.join(dir, "plan-usage-history.json"), "utf8");
      } catch (err) {
        if (err.code === "ENOENT") return null;
        throw err;
      }
      // The file says its format's version, which picks the schema. One
      // with nothing to read (not an object, or samples of another sort)
      // keeps the samples there were.
      const raw = JSON.parse(text);
      const d = desktopSamples.check(raw, { version: raw?.version != null ? String(raw.version) : null });
      if (!d || (raw.samples !== undefined && d.samples === undefined)) throw new Error("plan-usage-history.json has no samples XenoNet can read");
      return (d.samples ?? []).filter((x) => x.u).map((x) => ({ ts: x.t, fiveHour: x.u.fh ?? null, sevenDay: x.u.sd ?? null, resets: null, source: "claude-desktop" }));
    },
    keep: true,
    clock,
  });
