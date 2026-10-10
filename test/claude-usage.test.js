import { test } from "node:test";
import assert from "node:assert/strict";
import { readClaudeToken, fetchClaudeUsage, parseUsage } from "../core/claude-usage.js";

const now = Date.parse("2026-10-03T12:00:00Z");
const creds = (extra = {}) => JSON.stringify({ mcpOAuth: { "some-server": { accessToken: "mcp-token" } }, claudeAiOauth: { accessToken: "claude-token", refreshToken: "never-used", expiresAt: now + 3600e3, ...extra } });
const answer = (status, body, headers = {}) => ({ status, ok: status >= 200 && status < 300, headers: { get: (k) => headers[k.toLowerCase()] ?? null }, json: async () => body });

test("Claude Code's sign-in: the keychain on macOS, else .credentials.json; only the Claude token is kept", async () => {
  assert.deepEqual(await readClaudeToken({ platform: "darwin", keychain: async () => creds() }), { token: "claude-token", expiresAt: now + 3600e3 });
  const files = { "/cfg/.credentials.json": creds() };
  assert.equal((await readClaudeToken({ platform: "linux", env: { CLAUDE_CONFIG_DIR: "/cfg" }, readFile: async (f) => files[f] })).token, "claude-token");
  assert.equal(await readClaudeToken({ platform: "darwin", keychain: async () => null, readFile: async () => { throw new Error("ENOENT"); } }), null);
  assert.equal(await readClaudeToken({ platform: "darwin", keychain: async () => "not json", readFile: async () => null }), null);
});

test("usage is read as percentages with reset times", () => {
  const u = parseUsage({ five_hour: { utilization: 10, resets_at: "2026-10-03T14:00:00+00:00" }, seven_day: { utilization: 6.5, resets_at: null }, seven_day_opus: null }, now);
  assert.deepEqual(u, { fiveHour: 10, sevenDay: 6.5, resets: { fiveHour: Date.parse("2026-10-03T14:00:00Z"), sevenDay: null }, observedAt: now, source: "anthropic" });
  assert.equal(parseUsage({ extra_usage: {} }, now), null);
});

test("one request with the Claude token; nothing sent when the token is missing or expired", async () => {
  const seen = [];
  const fetchImpl = async (url, opts) => (seen.push({ url, auth: opts.headers.authorization }), answer(200, { five_hour: { utilization: 3, resets_at: null }, seven_day: { utilization: 1, resets_at: null } }));
  const ok = await fetchClaudeUsage({ fetchImpl, readToken: async () => ({ token: "claude-token", expiresAt: now + 60e3 }), now });
  assert.equal(ok.usage.fiveHour, 3);
  assert.deepEqual(seen, [{ url: "https://api.anthropic.com/api/oauth/usage", auth: "Bearer claude-token" }]);

  const none = await fetchClaudeUsage({ fetchImpl, readToken: async () => null, now });
  const expired = await fetchClaudeUsage({ fetchImpl, readToken: async () => ({ token: "t", expiresAt: now - 1 }), now });
  assert.equal(seen.length, 1, "no request without a live token");
  assert.match(none.reason, /no Claude Code sign-in/);
  assert.match(expired.reason, /expired/);
});

test("refusals and pauses are reported, never retried with a refreshed token", async () => {
  const token = async () => ({ token: "t", expiresAt: now + 60e3 });
  const limited = await fetchClaudeUsage({ fetchImpl: async () => answer(429, {}, { "retry-after": "120" }), readToken: token, now });
  assert.equal(limited.retryAfterMs, 120e3);
  const refused = await fetchClaudeUsage({ fetchImpl: async () => answer(401, {}), readToken: token, now });
  assert.match(refused.reason, /refused/);
  const down = await fetchClaudeUsage({ fetchImpl: async () => { throw new Error("getaddrinfo ENOTFOUND"); }, readToken: token, now });
  assert.match(down.reason, /couldn't reach Anthropic/);
});
