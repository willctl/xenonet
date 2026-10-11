// Grok Build's sessions as XenoNet reads them (core/grok.js): a made-up
// GROK_HOME (test/fixtures/grok.js) listed into the map's sessions, a
// session's updates read as its conversation, its usage ledger, a rename,
// and which sessions a running grok has open.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { grokHome, IDS, ATLAS, LONG, T, MAIN_UPDATES } from "./fixtures/grok.js";
import { listGrokSessions, groupCwd, worktreeOrigin, readGrokSession, grokEntries, renameGrokSession, readGrokUsage, grokCounts, markRunning, grokFolder } from "../core/grok.js";
import { GrokLine } from "../core/lines.js";

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-"));
after(() => fs.rmSync(tmp, { recursive: true, force: true }));
const home = grokHome(path.join(tmp, "home"));
const byId = () => new Map(listGrokSessions({ home }).map((s) => [s.rawId, s]));

test("lists each session folder as a session: its title, folder, model, branch and times", () => {
  const all = byId();
  assert.deepEqual([...all.keys()].sort(), Object.values(IDS).sort(), "every session, and not the empty folder");
  const main = all.get(IDS.main);
  assert.equal(main.id, `grok:${IDS.main}`);
  assert.equal(main.app, "grok");
  assert.equal(main.title, "Add partial refunds");
  assert.equal(main.titleManual, false);
  assert.equal(main.cwd, ATLAS);
  assert.equal(main.worktree, null);
  assert.equal(main.model, "grok-4.6");
  assert.equal(main.branch, "feat/refunds");
  assert.equal(main.createdAt, T - 3600e3);
  assert.equal(main.updatedAt, T + 14000);
  assert.equal(main.preview, "Asked whether to open a PR");
  assert.equal(main.archived, false);
  assert.equal(main.background, false);
  assert.ok(main.sizeBytes > 0);
  assert.equal(grokFolder(IDS.main, { home }), path.join(home, "sessions", encodeURIComponent(ATLAS), IDS.main));
});

test("a fork and a subagent say whose they are", () => {
  const all = byId();
  const fork = all.get(IDS.fork);
  assert.deepEqual([fork.parentId, fork.parentKind, fork.title, fork.titleManual], [`grok:${IDS.main}`, "fork", "Refunds, the careful version", true]);
  const child = all.get(IDS.child);
  assert.deepEqual([child.parentId, child.parentKind, child.agentKind, child.agentNickname, child.background], [`grok:${IDS.main}`, "subagent", "explore", "Map the refund code", false]);
});

test("a worktree session belongs to the repo it came from, a long folder name to its .cwd, and a headless run is background", () => {
  const all = byId();
  const wt = all.get(IDS.worktree);
  assert.equal(wt.cwd, ATLAS);
  assert.equal(wt.worktree, path.join(home, "worktrees", "atlas", "wt-fix"));
  assert.equal(all.get(IDS.long).cwd, LONG);
  assert.equal(all.get(IDS.long).title, "Reconcile the ledger", "the summary's first line");
  assert.equal(all.get(IDS.long).updatedAt, Math.floor((T - 86000e3) / 1000) * 1000, "epoch seconds read as such");
  assert.equal(all.get(IDS.headless).background, true);
  const bare = all.get(IDS.bare);
  assert.deepEqual([bare.title, bare.cwd], ["(untitled session)", ATLAS]);
  assert.ok(bare.updatedAt > 0, "a session with no summary yet: when its updates were written");
});

test("a group's folder: URL-decoded, or its .cwd, or not known", () => {
  assert.equal(groupCwd(path.join(tmp, "x"), "%2FUsers%2Fada%2Fatlas"), "/Users/ada/atlas");
  assert.equal(groupCwd(path.join(tmp, "x"), "C%3A%5CUsers%5Cada"), "C:\\Users\\ada");
  assert.equal(groupCwd(path.join(tmp, "x"), "ledger-3f9a1c2b"), null);
  assert.equal(groupCwd(path.join(tmp, "x"), "%E0%A4%A"), null, "not a valid encoding");
  assert.equal(groupCwd(path.join(tmp, "x"), "relative%20name"), null, "not a path");
  assert.equal(worktreeOrigin(ATLAS, home), null, "not one of Grok's worktrees");
  assert.deepEqual(worktreeOrigin(path.join(home, "worktrees", "atlas", "gone", "src"), home), { repo: null, top: path.join(home, "worktrees", "atlas", "gone") }, "one whose .git is gone");
});

test("no sessions folder, no sessions", () => {
  assert.deepEqual(listGrokSessions({ home: path.join(tmp, "nothing") }), []);
});

test("a session's updates read as its conversation: chunks joined, each tool call with what became of it", async () => {
  const page = await readGrokSession(IDS.main, { home });
  assert.equal(page.total, 8);
  assert.equal(page.cursor, null);
  const [prompt, thought, reply, shell, edit, plan, mcp, question] = page.entries;
  assert.deepEqual([prompt.role, prompt.text], ["user", "Add partial refunds to the orders API."]);
  assert.deepEqual([thought.role, reply.role, reply.text], ["reasoning", "assistant", "I'll start with the tests."]);
  assert.deepEqual([shell.role, shell.title, shell.meta, shell.status], ["tool", "npm test -- orders", "run_terminal_command", "completed"]);
  assert.match(shell.text, /48 passing/);
  assert.deepEqual([edit.title, edit.status], ["Edit refunds.ts", "completed"]);
  assert.match(edit.text, /edit \/Users\/ada\/atlas\/src\/refunds\.ts/);
  assert.deepEqual([plan.title, plan.text], ["plan", "✓ Accept an amount\n· Test a second partial refund"]);
  assert.equal(mcp.meta, "mcp__linear__save_issue", "an MCP tool by Claude Code's name for it");
  assert.equal(question.text, "Partial refunds work. Should I open a PR?");
  // Further back, a page at a time.
  const first = await readGrokSession(IDS.main, { home, limit: 3 });
  assert.deepEqual(first.entries.map((e) => e.title ?? e.role), ["plan", "linear: save issue", "assistant"]);
  assert.equal(first.entries.at(-1).text, question.text);
  const back = await readGrokSession(IDS.main, { home, limit: 3, cursor: first.cursor });
  assert.equal(back.entries.at(-1).id, page.entries[4].id);
});

test("a session's updates grow as Grok writes them, and the reader carries on", async () => {
  const file = path.join(grokFolder(IDS.fork, { home }), "updates.jsonl");
  assert.equal((await readGrokSession(IDS.fork, { home })).total, 1);
  fs.appendFileSync(file, JSON.stringify({ timestamp: new Date(T).toISOString(), update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Done." } } }) + "\n");
  const page = await readGrokSession(IDS.fork, { home });
  assert.deepEqual(page.entries.map((e) => e.text), ["Try it with a feature flag.", "Done."]);
});

test("a line is read as an ACP update in an envelope, or bare, with its time wherever it is", () => {
  const enveloped = new GrokLine(MAIN_UPDATES[0]);
  assert.deepEqual([enveloped.kind, enveloped.ts, enveloped.text], ["user_message_chunk", T, "Add partial refunds "]);
  const bare = new GrokLine({ sessionUpdate: "agent_message_chunk", content: [{ type: "text", text: "a" }, { type: "image" }], _meta: { timestamp: T / 1000 } });
  assert.deepEqual([bare.kind, bare.ts, bare.text], ["agent_message_chunk", T, "a[image]"]);
  const notified = new GrokLine({ method: "session/update", params: { update: { sessionUpdate: "plan", entries: [] } } });
  assert.deepEqual([notified.kind, Number.isNaN(notified.ts)], ["plan", true]);
  assert.equal(new GrokLine({ update: { sessionUpdate: "tool_call", toolCallId: "x", kind: "execute" } }).toolName, "run_terminal_command", "a tool for its kind");
  assert.equal(new GrokLine({ update: { sessionUpdate: "tool_call", toolCallId: "x", kind: "nope" } }).toolName, null);
  assert.equal(grokEntries([new GrokLine({ junk: true })]).length, 0, "a line with no update says nothing");
});

test("renaming writes the summary as /rename does, and leaves the rest of it alone", async () => {
  await renameGrokSession(IDS.main, "Partial refunds, done", { home });
  const summary = JSON.parse(fs.readFileSync(path.join(grokFolder(IDS.main, { home }), "summary.json"), "utf8"));
  assert.deepEqual([summary.generated_title, summary.title_is_manual, summary.somethingNew, summary.current_model_id], ["Partial refunds, done", true, { kept: true }, "grok-4.6"]);
  assert.equal(byId().get(IDS.main).title, "Partial refunds, done");
  assert.deepEqual(fs.readdirSync(grokFolder(IDS.main, { home })).filter((f) => f.includes("xenonet")), [], "no temporary file left");
  // A summary that says it's another session's is left as it is.
  const other = path.join(grokFolder(IDS.fork, { home }), "summary.json");
  const before = fs.readFileSync(other, "utf8");
  fs.writeFileSync(other, JSON.stringify({ info: { id: "someone-else" } }));
  await assert.rejects(renameGrokSession(IDS.fork, "x", { home }), /another session/);
  fs.writeFileSync(other, before);
  await assert.rejects(renameGrokSession("019a3b5c-0000-7000-8000-0000000000ff", "x", { home }), /can't find/);
});

test("the usage ledger: totals and turns, the cache taken out of input when the total says it's in it, and Grok's own cost", () => {
  const u = readGrokUsage(grokFolder(IDS.main, { home }));
  assert.equal(u.file, "usage.json");
  assert.deepEqual(u.totals, { input: 12000, cacheRead: 40000, cacheWrite: 0, output: 3000, reasoning: 800, total: 55000, calls: 3, cost: 0.12345 });
  assert.deepEqual(u.turns.map((t) => [t.n, t.at, t.model, t.input, t.cacheRead, t.cost]), [
    [1, T + 9000, "grok-4.6", 5000, 15000, 0.04],
    [2, T + 14000, "grok-4.6", 7000, 25000, 0.08345],
  ]);
  // Input already without the cache: the parts add up to the total.
  assert.deepEqual(grokCounts({ input_tokens: 100, cache_read_input_tokens: 400, output_tokens: 50, total_tokens: 550 }), { input: 100, cacheRead: 400, cacheWrite: 0, output: 50, reasoning: 0, total: 550, calls: null, cost: null });
  // A cost Grok says is partial is no cost.
  assert.equal(grokCounts({ inputTokens: 1, costUsdTicks: 5e9, costIsPartial: true }).cost, null);
  assert.equal(grokCounts({ total_cost_usd: 0.5 }).cost, 0.5);
  assert.equal(grokCounts(null), null);
  // No ledger: signals.json, and else nothing.
  const dir = grokFolder(IDS.fork, { home });
  assert.equal(readGrokUsage(dir), null);
  fs.writeFileSync(path.join(dir, "signals.json"), JSON.stringify({ turnCount: 2, tokenUsage: { input_tokens: 10, output_tokens: 5 } }));
  assert.deepEqual(readGrokUsage(dir).totals.total, 15);
  fs.rmSync(path.join(dir, "signals.json"));
});

test("a running grok marks the session it has open in its folder", () => {
  const sessions = listGrokSessions({ home });
  markRunning(sessions, [
    { pid: 10, start: T - 60e3, cwd: ATLAS },
    { pid: 11, start: T, cwd: path.join(home, "worktrees", "atlas", "wt-fix") },
    { pid: 12, start: T, cwd: "/Users/ada/elsewhere" },
    { pid: 13, start: T, cwd: null },
  ]);
  const running = sessions.filter((s) => s.running).map((s) => s.rawId).sort();
  // In atlas, the one written to last since the process started (not the
  // subagent's, nor the fork from before it started); in the worktree, its
  // only one, written before.
  assert.deepEqual(running, [IDS.bare, IDS.worktree].sort());
  assert.ok(sessions.every((s) => typeof s.running === "boolean"));
});
