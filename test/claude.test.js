import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// Claude Code's folder and Claude desktop's, made up, before anything reads
// the config: the Agent SDK reads CLAUDE_CONFIG_DIR, XenoNet the desktop folder.
const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-claude-")));
after(() => fs.rmSync(root, { recursive: true, force: true }));
process.env.CLAUDE_CONFIG_DIR = path.join(root, "claude");
process.env.XENONET_CLAUDE_DESKTOP_DIR = path.join(root, "desktop");
process.env.XENONET_WIKI_DIR = path.join(root, "no-wiki");
const { listClaudeSessions, readClaudeSession, claudeSessionsGatherer, renameClaudeSession, renameDesktopRecord } = await import("../core/claude.js");

const ID = (n) => `0190d3e0-0000-7000-8000-${String(n).padStart(12, "0")}`;
const ACCOUNT = "0190d3e0-0000-7000-a000-000000000001";
const ORG = "0190d3e0-0000-7000-b000-000000000001";
const line = (o) => JSON.stringify(o) + "\n";
const LATER = Date.now() + 3600e3;
const ts = (min) => new Date(Date.parse("2026-10-03T12:00:00Z") + min * 60e3).toISOString();

function transcript(n, cwd, lines) {
  const dir = path.join(root, "claude", "projects", cwd.replace(/[^a-zA-Z0-9]/g, "-"));
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, `${ID(n)}.jsonl`), lines.map((o) => line({ sessionId: ID(n), cwd, ...o })).join(""));
}

function desktop(parts, name, data) {
  const dir = path.join(root, "desktop", "claude-code-sessions", ...parts);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, name), typeof data === "string" ? data : JSON.stringify(data));
}

const cwd = path.join(root, "Projects", "atlas");
transcript(1, cwd, [
  { type: "user", uuid: "u1", parentUuid: null, gitBranch: "main", timestamp: ts(0), message: { role: "user", content: "Add partial refunds" } },
  { type: "assistant", uuid: "a1", parentUuid: "u1", timestamp: ts(1), message: { id: "m1", role: "assistant", model: "claude-x", content: [{ type: "thinking", thinking: "Where are refunds?" }, { type: "text", text: "On it." }, { type: "tool_use", id: "t1", name: "Bash", input: { command: "npm test" } }] } },
  { type: "user", uuid: "u2", parentUuid: "a1", timestamp: ts(2), message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", is_error: true, content: [{ type: "text", text: "1 failed" }, { type: "image" }] }] } },
  { type: "assistant", uuid: "a2", parentUuid: "u2", timestamp: ts(3), message: { id: "m2", role: "assistant", model: "claude-x", content: [{ type: "tool_use", id: "t2", name: "Write", input: { content: "x".repeat(5000) } }, { type: "tool_result", tool_use_id: "t2", content: "y".repeat(5000) }] } },
]);
transcript(2, cwd, [{ type: "user", uuid: "u1", parentUuid: null, timestamp: ts(5), message: { role: "user", content: "Review the refunds change" } }]);
transcript(3, path.join(root, "elsewhere"), [{ type: "user", uuid: "u1", parentUuid: null, timestamp: ts(9), message: { role: "user", content: "Explain this regex" } }]);

// Claude desktop files each session under <account>/<organization>/.
desktop([ACCOUNT, ORG], "local_1.json", { sessionId: "local_1", cliSessionId: ID(1), title: "Partial refunds", originCwd: cwd, worktreePath: path.join(cwd, ".claude", "worktrees", "refunds"), lastActivityAt: LATER, isArchived: false, isStarred: true, model: "claude-opus", effort: "high", permissionMode: "acceptEdits", branch: "feat/refunds" });
desktop([ACCOUNT, ORG], "local_2.json", { sessionId: "local_2", cliSessionId: ID(2), title: "", isArchived: true, spawnedFrom: { sessionId: "local_1" } });
desktop([ACCOUNT, ORG], "local_broken.json", "{ not json");
desktop([ACCOUNT, ORG], "notes.txt", "not a session");
desktop(["deleted_old", ORG], "local_9.json", { sessionId: "local_9", cliSessionId: ID(3), title: "Deleted" });
desktop(["not-an-account"], "local_3.json", { sessionId: "local_3", cliSessionId: ID(3), title: "" });

test("Claude Code's sessions, with what Claude desktop knows about them", async () => {
  const sessions = await listClaudeSessions();
  const byId = new Map(sessions.map((s) => [s.rawId, s]));
  assert.equal(sessions.length, 3);

  const one = byId.get(ID(1));
  assert.equal(one.id, `claude:${ID(1)}`);
  assert.equal(one.title, "Partial refunds", "desktop's title first");
  assert.equal(one.desktopId, "local_1");
  assert.equal(one.inDesktop, true);
  assert.equal(one.cwd, cwd);
  assert.match(one.worktree, /worktrees[\\/]refunds$/);
  assert.equal(one.updatedAt, LATER, "the later of the file and desktop's activity");
  assert.deepEqual([one.starred, one.archived, one.model, one.effort, one.permissionMode, one.branch], [true, false, "claude-opus", "high", "acceptEdits", "feat/refunds"]);
  assert.deepEqual(one.claudeAccount, { account: ACCOUNT, org: ORG }, "from the folders it's in");
  assert.equal(one.preview, "Add partial refunds");
  assert.ok(one.sizeBytes > 0 && one.createdAt);

  const two = byId.get(ID(2));
  assert.equal(two.title, "Review the refunds change", "the first prompt, when desktop has no title");
  assert.deepEqual([two.archived, two.parentId, two.parentKind], [true, `claude:${ID(1)}`, "spawn"]);

  const three = byId.get(ID(3));
  assert.equal(three.inDesktop, true, "a session under an unknown folder still counts");
  assert.equal(three.claudeAccount, null, "but names no account");
  assert.equal(three.title, "Explain this regex");
  assert.equal(three.branch, null);
});

test("the listing as a gatherer: each run lists again, and a failure leaves nothing to show", async () => {
  const g = claudeSessionsGatherer();
  assert.equal((await g.run()).length, 3);
  assert.deepEqual([g.name, g.kind, g.state, g.last.ok], ["claude-sessions", "gatherer", "running", true]);
  let fail = true;
  const flaky = claudeSessionsGatherer({ gather: async () => (fail ? Promise.reject(new Error("the SDK broke")) : listClaudeSessions()) });
  assert.deepEqual(await flaky.run(), []);
  assert.deepEqual([flaky.state, flaky.last.error], ["failed", "the SDK broke"]);
  fail = false;
  assert.equal((await flaky.run()).length, 3);
  assert.equal(flaky.state, "running");
});

test("a transcript, newest page first, as the console shows it", async () => {
  const all = await readClaudeSession(ID(1));
  assert.equal(all.total, 4);
  assert.equal(all.cursor, null);
  assert.deepEqual(all.entries.map((e) => [e.role, e.kind ?? null]), [
    ["user", null],
    ["reasoning", null],
    ["assistant", null],
    ["tool", "tool_use"],
    ["tool", "tool_result"],
    ["tool", "tool_use"],
    ["tool", "tool_result"],
  ]);
  const [prompt, , , call, result, write, written] = all.entries;
  assert.deepEqual([prompt.id, prompt.ts, prompt.text], ["u1", Date.parse(ts(0)), "Add partial refunds"]);
  assert.deepEqual([call.title, JSON.parse(call.text)], ["Bash", { command: "npm test" }]);
  assert.deepEqual([result.status, result.text], ["failed", "1 failed\n[image]"]);
  assert.match(write.text, /… \(\d+ more chars\)$/, "long inputs are clipped");
  assert.match(written.text, /^y{4000}\n… \(1000 more chars\)$/);

  const page = await readClaudeSession(ID(1), { limit: 2 });
  assert.deepEqual(page.entries.map((e) => e.id), ["u2:0", "a2:0", "a2:1"]);
  assert.equal(page.cursor, "2");
  const older = await readClaudeSession(ID(1), { cursor: page.cursor, limit: 2 });
  assert.equal(older.entries[0].id, "u1");
  assert.equal(older.cursor, null);
});

test("renaming a session: a custom title in Claude Code's transcript, and the title in Claude desktop's record", async () => {
  transcript(4, cwd, [{ type: "user", uuid: "u1", parentUuid: null, timestamp: ts(12), message: { role: "user", content: "Tidy the ledger" } }]);
  const record = { sessionId: "local_4", cliSessionId: ID(4), title: "Ledger tidy", titleSource: "auto", isStarred: true, spawnSeed: { prompt: "x" } };
  desktop([ACCOUNT, ORG], "local_4.json", record);
  // The listing finds where the desktop's records are.
  assert.equal((await listClaudeSessions()).find((s) => s.rawId === ID(4)).title, "Ledger tidy");

  assert.deepEqual(await renameClaudeSession({ rawId: ID(4), desktopId: "local_4" }, "Ledger: tidy the 2025 entries"), { claude: true, desktop: true });
  const file = path.join(root, "claude", "projects", cwd.replace(/[^a-zA-Z0-9]/g, "-"), `${ID(4)}.jsonl`);
  const last = JSON.parse(fs.readFileSync(file, "utf8").trim().split("\n").at(-1));
  assert.deepEqual([last.type, last.customTitle, last.sessionId], ["custom-title", "Ledger: tidy the 2025 entries", ID(4)]);
  const saved = JSON.parse(fs.readFileSync(path.join(root, "desktop", "claude-code-sessions", ACCOUNT, ORG, "local_4.json"), "utf8"));
  assert.deepEqual(saved, { ...record, title: "Ledger: tidy the 2025 entries", titleSource: "user" }, "only the title changes, marked as yours");
  assert.equal((await listClaudeSessions()).find((s) => s.rawId === ID(4)).title, "Ledger: tidy the 2025 entries");
  assert.deepEqual(fs.readdirSync(path.join(root, "desktop", "claude-code-sessions", ACCOUNT, ORG)).filter((f) => f.endsWith(".tmp")), [], "no temporary file left");

  // A session Claude desktop doesn't have: Claude Code's name alone.
  assert.deepEqual(await renameClaudeSession({ rawId: ID(4), desktopId: null }, "Ledger"), { claude: true, desktop: false });
  // A record that isn't there, or is another session's, is left alone.
  const missing = await renameClaudeSession({ rawId: ID(4), desktopId: "local_404" }, "Ledger");
  assert.deepEqual([missing.claude, missing.desktop], [true, false]);
  assert.match(missing.desktopError, /hasn't found/);
  await assert.rejects(renameDesktopRecord("local_4", ID(1), "Not this one"), /another session/);
  assert.equal(JSON.parse(fs.readFileSync(path.join(root, "desktop", "claude-code-sessions", ACCOUNT, ORG, "local_4.json"), "utf8")).title, "Ledger: tidy the 2025 entries");
  // The SDK refuses an id that isn't a session's.
  await assert.rejects(renameClaudeSession({ rawId: "not-a-uuid", desktopId: null }, "x"));
});

test("without Claude desktop, sessions come from Claude Code alone", async () => {
  fs.rmSync(path.join(root, "desktop"), { recursive: true, force: true });
  const sessions = await listClaudeSessions();
  assert.equal(sessions.find((s) => s.rawId === ID(1)).title, "Add partial refunds");
  assert.ok(sessions.every((s) => !s.inDesktop && s.claudeAccount === null));
});
