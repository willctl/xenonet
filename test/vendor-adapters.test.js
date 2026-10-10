// The adapters that read other apps' files and answers, read leniently
// (core/vendor.js): what differs from the schemas is kept as drift, as
// shapes, and what fits is still used. Made-up data only.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { drift } from "../core/drift.js";
import { chatgptAccount, claudeAccount, ghHostsShape, parseGhHosts } from "../core/identities.js";
import { fetchClaudeUsage, desktopUsageGatherer } from "../core/claude-usage.js";
import { reader, jsonErrorAt, parseJson } from "../core/vendor.js";
import { tempTree, cleanupAll } from "./harness/files.js";

after(cleanupAll);

const found = (source) => drift.list().filter((r) => r.source === source).map((r) => [r.kind, r.path, r.shape]);
const jwt = (claims) => `x.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.y`;

test("Codex's ID token: claims of another type are drift, and the rest is still read", () => {
  const account = chatgptAccount({ auth_mode: "chatgpt", tokens: { id_token: jwt({ email: "ada@example.com", auth_time: "yesterday", "https://api.openai.com/auth": { chatgpt_account_id: "acct-1", chatgpt_plan_type: "plus" } }) } });
  assert.deepEqual([account.id, account.handle, account.detail.plan, account.since], ["chatgpt:acct-1", "ada@example.com", "plus", null], "the time that isn't a number isn't read");
  assert.deepEqual(found("openai/codex/auth-claims"), [["type-change", "auth_time", "string"]]);
  assert.ok(!JSON.stringify(drift.list()).includes("ada@example.com"), "never a value");
});

test("~/.claude.json: an account id of another type is drift, and there's no account", () => {
  const config = reader("anthropic/claude-code/config").check({ oauthAccount: { accountUuid: 42, organizationUuid: "4f1c0000-0000-4000-8000-000000000002" } });
  assert.equal(claudeAccount(config), null);
  assert.deepEqual(found("anthropic/claude-code/config"), [["type-change", "oauthAccount.accountUuid", "number"]]);
});

test("gh's hosts.yml as keys only: what the schema checks, with no token or value in it", () => {
  const text = "github.com:\n    git_protocol: https\n    users:\n        ada-dev:\n            oauth_token: gho_secret\n        orbitworks:\n    user: ada-dev\nghe.example.com:\n    oauth_token: gho_other\n    user: ada\n";
  const shape = ghHostsShape(text);
  assert.deepEqual(shape, { "github.com": { git_protocol: "", users: { "ada-dev": { oauth_token: "" }, orbitworks: null }, user: "" }, "ghe.example.com": { oauth_token: "", user: "" } });
  assert.ok(!JSON.stringify(shape).includes("gho_"));
  assert.deepEqual(parseGhHosts(text).map((h) => [h.host, h.users, h.active]), [["github.com", ["ada-dev", "orbitworks"], "ada-dev"], ["ghe.example.com", ["ada"], "ada"]]);
  const hosts = reader("github/gh/hosts");
  hosts.check(shape);
  assert.deepEqual(found("github/gh/hosts"), []);
  // gh writing its users as a list one day.
  hosts.check(ghHostsShape("github.com:\n    users:\n        - ada-dev\n    user: ada-dev\n"));
  assert.deepEqual(found("github/gh/hosts"), [["type-change", "*.users", "array<>"]]);
});

test("Anthropic's usage answer: a window of another sort is drift, and an answer that isn't JSON is too", async () => {
  const answer = (body) => async () => ({ status: 200, ok: true, headers: new Map(), json: async () => (typeof body === "function" ? body() : body) });
  const readToken = async () => ({ token: "t", expiresAt: null });
  const odd = await fetchClaudeUsage({ fetchImpl: answer({ five_hour: { utilization: "10%", resets_at: null }, seven_day: { utilization: 4, resets_at: null } }), readToken });
  assert.deepEqual([odd.usage.fiveHour, odd.usage.sevenDay], [null, 4], "the window that fits is read");
  const broken = await fetchClaudeUsage({ fetchImpl: answer(() => JSON.parse("<html>")), readToken });
  assert.equal(broken.reason, "Anthropic's answer had no usage windows");
  assert.deepEqual(found("anthropic/oauth/usage"), [["type-change", "five_hour.utilization", "string"], ["unparsable", null, null]]);
});

test("Claude desktop's samples: a key the samples didn't have is drift, and the file's version picks the schema", async () => {
  const tree = tempTree();
  tree.write("plan-usage-history.json", JSON.stringify({ version: 2, samples: [{ t: 1791230000000, u: { fh: 12, sd: 40, op: 3 } }] }));
  const samples = await desktopUsageGatherer({ dir: tree.root }).run();
  assert.deepEqual(samples.map((s) => [s.fiveHour, s.sevenDay]), [[12, 40]]);
  tree.write("plan-usage-history.json", JSON.stringify({ version: 3, samples: [] }));
  await desktopUsageGatherer({ dir: tree.root }).run();
  assert.deepEqual(
    drift.list().filter((r) => r.source === "anthropic/claude-desktop/plan-usage").map((r) => [r.kind, r.path, r.vendorVersion]),
    [["new-field", "samples[].u.op", "2"], ["new-version", null, "3"]],
  );
});

test("logins, tokens and ids as keys never reach drift: inside gh's records no key is kept, and a key that reads as data is *", () => {
  drift.clear();
  const token = "ghp_" + "Zx9kQ2mN7pL4vB8cR1tY6wE3uI5oA0sDfGhJ"; // made up
  const hosts = reader("github/gh/hosts");
  // gh writing its active account as a map one day (with a token where a
  // login was, the next), and its users as a list with their settings.
  hosts.check({ "ghe.example.com": { user: { [token]: "" } } });
  hosts.check(ghHostsShape("github.com:\n    user:\n        ada-dev:\n    users:\n        orbitworks:\n"));
  hosts.check({ "ghe.example.com": { users: [{ "ada-dev": { oauth_token: "" } }] } });
  assert.deepEqual(found("github/gh/hosts"), [["type-change", "*.user", "record<string>"], ["type-change", "*.users", "array<record<record<string>>>"]]);
  assert.equal(drift.list().find((r) => r.path === "*.user").count, 2, "the same drift, counted");

  // Anywhere else, a key that reads as data is * in a new field's path, and
  // its value keeps no key's name either.
  const lines = reader("anthropic/claude-code/transcript-line");
  const usage = { input_tokens: 1, output_tokens: 2, [token]: { scopes: "repo" }, "3f9a2c1": 3 };
  lines.check({ type: "assistant", timestamp: "2026-10-05T12:00:00Z", version: "2.1.288", message: { id: "msg_1", model: "claude-made-up", content: [], usage, stop_reason: null } });
  assert.deepEqual(found("anthropic/claude-code/transcript-line"), [["new-field", "message.usage.*", "record<string>"]]);
  const kept = JSON.stringify(drift.list());
  for (const data of ["ada-dev", "orbitworks", "ghp_", "3f9a2c1", "ghe.example", "github.com"]) assert.ok(!kept.includes(data), data);
});

test("a sign-in file that isn't JSON says where it breaks, never its text", () => {
  // Made-up files, broken the ways V8 quotes the text around.
  const account = '{"oauthAccount": {"emailAddress": "ada@example.com", "accountUuid": tru}}';
  const auth = '{"tokens": {"id_token": "eyJmadeUp.eyJmadeUp.sig",, "refresh_token": "made-up"}}';
  const key = '{\n  "OPENAI_API_KEY": "sk-proj-madeUpForTests"\n  "auth_mode": "apikey"\n}';
  const cases = [
    [account, 1, account.indexOf("tru") + 1],
    [auth, 1, auth.indexOf(",,") + 2],
    [key, 3, 3],
    ["", 1, 1],
  ];
  for (const [text, line, column] of cases) {
    assert.match(String((() => { try { JSON.parse(text); } catch (e) { return e.message; } })()), text ? /JSON/ : /end/);
    assert.deepEqual(jsonErrorAt(text), { line, column }, text);
    assert.throws(() => parseJson(text), (err) => err instanceof SyntaxError && err.message === `isn't valid JSON (line ${line}, column ${column})`, text);
  }
  // Where V8 says where, they agree.
  for (const text of ['{"a":1,}', '{"a":"b', '{"a":1} x', '{"a":"\u0001"}', '[1, 2', '{"a" 1}']) {
    const v8 = (() => { try { JSON.parse(text); } catch (e) { return /line (\d+) column (\d+)/.exec(e.message)?.slice(1).map(Number); } })();
    assert.deepEqual([jsonErrorAt(text).line, jsonErrorAt(text).column], v8, text);
  }
  assert.equal(jsonErrorAt(JSON.stringify({ a: [1, { b: null, c: true }], d: -1.5e3, e: "\né\"" })), null);
  assert.deepEqual(parseJson('{"a": [1]}'), { a: [1] });
  assert.deepEqual(jsonErrorAt("[".repeat(100000)), { line: 1, column: 100001 }, "deep nesting is no trouble");
});
