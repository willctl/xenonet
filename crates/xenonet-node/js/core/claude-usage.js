// core/claude-usage.js in Rust (crates/xenonet-sources/src/claude_usage.rs):
// Claude plan usage, asked of Anthropic the way Claude Code's /usage asks
// (GET api.anthropic.com/api/oauth/usage with Claude Code's own sign-in),
// and Claude desktop's samples.
//
// Rust decides what each answer means: the sign-in a credentials text
// holds, whether it can be used, the windows an answer gives, why there's
// no reading, when to ask again, and the samples a desktop file holds.
// Reading the keychain and the files, the request, the vendor schemas'
// checks and the gatherers' timing stay here, where the tests hand them
// in. The sign-in is read for each request, and never stored, logged or
// refreshed.
import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { native } from "../index.js";
import { Gatherer } from "./blocks/gatherer.js";
import { reader } from "./vendor.js";

// Both answers are read leniently against their schemas (vendor.js).
const anthropicAnswers = reader("anthropic/oauth/usage");
const desktopSamples = reader("anthropic/claude-desktop/plan-usage");

const VERSION = (() => {
  try {
    return JSON.parse(readFileSync(new URL("../../../../package.json", import.meta.url), "utf8")).version;
  } catch {
    return "dev";
  }
})();
const { url: USAGE_URL, everyMs: USAGE_EVERY_MS, keychainService: KEYCHAIN_SERVICE } = native.claudeUsageConstants();

function keychainSecret(service) {
  return new Promise((resolve) => {
    execFile("security", ["find-generic-password", "-s", service, "-w"], { timeout: 5000 }, (err, stdout) => resolve(err ? null : stdout));
  });
}

// { token, expiresAt } for Claude Code's sign-in, or null.
export async function readClaudeToken({ platform = process.platform, env = process.env, homeDir = os.homedir(), keychain = keychainSecret, readFile = (f) => fs.readFile(f, "utf8") } = {}) {
  const sources = [];
  if (platform === "darwin") sources.push(() => keychain(KEYCHAIN_SERVICE));
  const p = platform === "win32" ? path.win32 : path.posix;
  sources.push(() => readFile(p.join(env.CLAUDE_CONFIG_DIR ?? p.join(homeDir, ".claude"), ".credentials.json")).catch(() => null));
  for (const read of sources) {
    try {
      const text = await read();
      const cred = native.claudeUsageTokenOf(text == null ? null : String(text));
      if (cred) return cred;
    } catch {}
  }
  return null;
}

// The windows XenoNet shows, as percentages, with when each resets.
export const parseUsage = (body, now = Date.now()) => native.claudeUsageParse(body, Number(now));

// One reading. `retryAfterMs` says how long to leave it when Anthropic
// asks for a pause; `reason` says why there's no reading.
export async function fetchClaudeUsage({ fetchImpl = globalThis.fetch, readToken = readClaudeToken, now = Date.now(), userAgent = `xenonet/${VERSION}` } = {}) {
  const cred = await readToken();
  const unusable = native.claudeUsageSignInProblem(cred, Number(now));
  if (unusable) return unusable;
  let res;
  try {
    res = await fetchImpl(USAGE_URL, {
      headers: { authorization: `Bearer ${cred.token}`, "anthropic-beta": "oauth-2025-04-20", "content-type": "application/json", "user-agent": userAgent },
      redirect: "error",
      signal: AbortSignal.timeout(8000),
    });
  } catch (err) {
    return native.claudeUsageUnreachable(err.name, err.message);
  }
  // The pause it asks for is in a header, read only when it asks.
  const pause = res.status === 429 || res.status === 403 ? res.headers.get("retry-after") : null;
  const problem = native.claudeUsageStatusProblem(res.status, !!res.ok, pause);
  if (problem) return problem;
  const body = await res.json().catch(() => undefined);
  if (body === undefined) anthropicAnswers.unparsable();
  return native.claudeUsageReading(body === undefined ? null : anthropicAnswers.check(body), Number(now));
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
    nextIn: (r) => native.claudeUsageNextIn(r),
    clock,
  });

// Claude desktop's own samples, which it now writes only while its menu
// bar usage view has been opened lately. Each read, null while there's no
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
      // The file says its format's version, which picks the schema.
      const raw = JSON.parse(text);
      const d = desktopSamples.check(raw, { version: native.claudeUsageDesktopVersion(raw) ?? null });
      return native.claudeUsageDesktopSamples(raw, d);
    },
    keep: true,
    clock,
  });
