import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";

// A Codex home of the test's own, and a stand-in for `codex app-server`:
// it answers thread/list and thread/items/list, takes a thread's new name
// (keeping it in named.json), asks XenoNet something of its own (which XenoNet
// declines), and fails or exits when told to.
const root = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-codex-"));
process.env.CODEX_HOME = path.join(root, "codex-home");
process.env.XENONET_WIKI_DIR = path.join(root, "no-wiki");
const bin = path.join(root, "codex");
const declined = path.join(root, "declined.json");
const named = path.join(root, "named.json");
fs.writeFileSync(
  bin,
  `#!${process.execPath}
const fs = require("fs");
if (process.env.FAKE_APP_SERVER === "exit") { process.stderr.write("codex: not signed in\\n"); process.exit(2); }
const send = (m) => process.stdout.write(JSON.stringify(m) + "\\n");
const thread = (id, extra = {}) => ({ id, name: "Thread " + id, cwd: "/p/" + id, createdAt: 1790000000, updatedAt: 1790000100, status: { type: "idle" }, ...extra });
const pages = { "false:": { data: [thread("a"), thread("b")], nextCursor: "p2" }, "false:p2": { data: [thread("c")], nextCursor: null }, "true:": { data: [thread("d")], nextCursor: null } };
let buf = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buf += chunk;
  for (let nl; (nl = buf.indexOf("\\n")) >= 0; ) {
    const line = buf.slice(0, nl);
    buf = buf.slice(nl + 1);
    const m = JSON.parse(line);
    if (m.method === undefined) { fs.writeFileSync(${JSON.stringify(declined)}, line); continue; }
    if (m.method === "initialize") {
      process.stdout.write("not json\\n");
      send({ jsonrpc: "2.0", method: "thread/started", params: {} });
      send({ jsonrpc: "2.0", id: "ask-1", method: "item/commandExecution/requestApproval", params: {} });
      send({ jsonrpc: "2.0", id: m.id, result: { userAgent: "fake" } });
    } else if (m.method === "initialized") {
    } else if (m.method === "thread/list") {
      send({ jsonrpc: "2.0", id: m.id, result: pages[m.params.archived + ":" + (m.params.cursor ?? "")] });
    } else if (m.method === "thread/items/list" && m.params.threadId.startsWith("019a")) {
      // As this Mac's Codex answers.
      send({ jsonrpc: "2.0", id: m.id, error: { code: -32603, message: "thread/items/list is not supported yet" } });
    } else if (m.method === "thread/items/list") {
      const items = [
        { item: { id: "i8", type: "mystery", detail: 1 } },
        { item: { id: "i7", type: "contextCompaction" } },
        { item: { id: "i6", type: "webSearch", query: "zod v4 coerce" } },
        { item: { id: "i5", type: "fileChange", status: "completed", changes: [{ path: "a.py", kind: { type: "update" } }, { path: "b.py", kind: "add" }] } },
        { item: { id: "i4", type: "dynamicToolCall", tool: "lookup", status: "completed", arguments: { q: 1 } } },
        { item: { id: "i3", type: "mcpToolCall", server: "wiki", tool: "log", status: "failed", error: { message: "nope" } } },
        { item: { id: "i2", type: "commandExecution", command: "pytest", status: "completed", exitCode: 0, aggregatedOutput: "ok" }, startedAtMs: 20 },
        { item: { id: "i1b", type: "plan", text: "1. read" } },
        { item: { id: "i1a", type: "reasoning", summary: ["thinking", { text: "more" }], content: [{ text: "deep" }] } },
        { item: { id: "i1", type: "agentMessage", text: "Hello" }, completedAtMs: 15 },
        { item: { id: "i0", type: "userMessage", content: [{ type: "text", text: "Hi" }, { type: "localImage" }] }, startedAtMs: 10 },
      ];
      send({ jsonrpc: "2.0", id: m.id, result: { data: items.slice(0, m.params.limit), nextCursor: "older" } });
    } else if (m.method === "thread/name/set" && !m.params.name.trim()) {
      send({ jsonrpc: "2.0", id: m.id, error: { code: -32600, message: "thread name must not be empty" } });
    } else if (m.method === "thread/name/set") {
      fs.writeFileSync(${JSON.stringify(named)}, JSON.stringify(m.params));
      send({ jsonrpc: "2.0", id: m.id, result: {} });
    } else send({ jsonrpc: "2.0", id: m.id, error: { code: -32601, message: "unknown method " + m.method } });
  }
});
`,
  { mode: 0o755 },
);
process.env.XENONET_CODEX_BIN = bin;
const { threadToSession, subAgentOf, listCodexThreads, readCodexThread, readRolloutPage, listCodexNativeProjects, codexClient, renameCodexThread } = await import("../core/codex.js");
const { drift } = await import("../core/drift.js");
const found = (source) => drift.list().filter((r) => r.source === source).map((r) => [r.kind, r.path, r.shape ?? r.vendorVersion]);
const unix = process.platform !== "win32";

after(() => {
  codexClient.stop();
  fs.rmSync(root, { recursive: true, force: true });
});

test("a thread becomes a session, with what kind of agent ran it", () => {
  const s = threadToSession({ id: "t1", preview: "Fix the build\nand more", cwd: "/p", createdAt: 1790000000, recencyAt: "2026-10-03T12:00:00Z", status: "active", model: "gpt-5", modelProvider: "openai", reasoningEffort: "high", gitInfo: { branch: "main", origin_url: "git@github.com:o/r.git" }, source: "cli", originator: "codex_cli", projectId: "np1" }, false);
  assert.deepEqual(
    [s.id, s.title, s.createdAt, s.updatedAt, s.status, s.branch, s.origin, s.source, s.nativeProjectId, s.parentId, s.background],
    ["codex:t1", "Fix the build", 1790000000000, Date.parse("2026-10-03T12:00:00Z"), "active", "main", "git@github.com:o/r.git", "cli", "np1", null, false],
  );
  const spawned = threadToSession({ id: "t2", name: "Benchmark", source: { subAgent: { thread_spawn: { parent_thread_id: "t1", agent_nickname: "bench" } } }, updatedAt: 1790000000123 }, true);
  assert.deepEqual([spawned.title, spawned.parentId, spawned.parentKind, spawned.agentNickname, spawned.agentKind, spawned.source, spawned.archived, spawned.updatedAt, spawned.background], ["Benchmark", "codex:t1", "subagent", "bench", "spawned", "subAgent", true, 1790000000123, false]);
  const fork = threadToSession({ id: "t3", forkedFromId: "t1", status: { type: "idle" } }, false);
  assert.deepEqual([fork.title, fork.parentId, fork.parentKind, fork.status, fork.createdAt], ["(untitled thread)", "codex:t1", "fork", "idle", null]);
  assert.equal(threadToSession({ id: "t4", source: "exec" }, false).background, true, "codex exec runs on its own");
  assert.equal(threadToSession({ id: "t5", source: { subAgent: "review" } }, false).background, true);
  assert.deepEqual(subAgentOf({ source: { subAgent: { other: "guardian" } } }), { parent: null, nickname: null, kind: "guardian" });
  assert.deepEqual(subAgentOf({ source: { subAgent: { compact: {} } } }).kind, "compact");
  assert.deepEqual(subAgentOf({ source: "vscode" }), { parent: null, nickname: null, kind: null });
});

test("threads are listed page by page, archived ones too", { skip: !unix }, async () => {
  const sessions = await listCodexThreads();
  assert.deepEqual(sessions.map((s) => [s.rawId, s.archived]), [["a", false], ["b", false], ["c", false], ["d", true]]);
  // The app-server asked to run a command; XenoNet only reads, so it said no.
  const answer = JSON.parse(fs.readFileSync(declined, "utf8"));
  assert.deepEqual([answer.id, answer.error.code], ["ask-1", -32601]);
});

test("a thread is renamed through the app-server, which says what it won't take", { skip: !unix }, async () => {
  await renameCodexThread("b", "Benchmarks, take two");
  assert.deepEqual(JSON.parse(fs.readFileSync(named, "utf8")), { threadId: "b", name: "Benchmarks, take two" });
  await assert.rejects(renameCodexThread("b", " "), /must not be empty/);
});

test("a thread's items read oldest first, as the console shows them", { skip: !unix }, async () => {
  const page = await readCodexThread("a", { limit: 20 });
  assert.equal(page.cursor, "older");
  assert.deepEqual(page.entries.map((e) => [e.id, e.role]), [["i0", "user"], ["i1", "assistant"], ["i1a", "reasoning"], ["i1b", "assistant"], ["i2", "tool"], ["i3", "tool"], ["i4", "tool"], ["i5", "tool"], ["i6", "tool"], ["i7", "system"], ["i8", "system"]]);
  const by = Object.fromEntries(page.entries.map((e) => [e.id, e]));
  assert.deepEqual([by.i0.text, by.i0.ts], ["Hi\n[image]", 10]);
  assert.deepEqual([by.i1.text, by.i1.ts], ["Hello", 15]);
  assert.equal(by.i1a.text, "thinking\nmore\ndeep");
  assert.equal(by.i1b.title, "plan");
  assert.deepEqual([by.i2.title, by.i2.meta, by.i2.text], ["pytest", "exit 0", "ok"]);
  assert.deepEqual([by.i3.title, by.i3.status, JSON.parse(by.i3.text)], ["wiki.log", "failed", { message: "nope" }]);
  assert.equal(by.i4.title, "lookup");
  assert.deepEqual([by.i5.title, by.i5.text], ["edit 2 file(s)", "update a.py\nadd b.py"]);
  assert.equal(by.i6.title, "search: zod v4 coerce");
  assert.equal(by.i7.text, "context compacted");
  assert.equal(by.i8.title, "mystery");
  assert.equal((await readCodexThread("a", { limit: 2 })).entries.length, 2);
  // An item type the schema doesn't list is drift, and still shown.
  assert.deepEqual(found("openai/codex/app-server"), [["new-variant", '["thread/items/list"].data[].item.type', '"mystery"']]);
});

test("an error answer, and an app-server that exits, fail the request; the next one starts it again", { skip: !unix }, async () => {
  await assert.rejects(codexClient.request("thread/unsubscribe", {}), /unknown method thread\/unsubscribe/);
  codexClient.stop();
  await new Promise((r) => setTimeout(r, 200));
  const timers = () => process.getActiveResourcesInfo().filter((r) => r === "Timeout").length;
  const before = timers();
  process.env.FAKE_APP_SERVER = "exit";
  try {
    await assert.rejects(listCodexThreads(), /codex app-server exited \(2\): codex: not signed in/);
  } finally {
    delete process.env.FAKE_APP_SERVER;
  }
  assert.equal(timers(), before, "no request's time limit outlives it");
  assert.equal((await listCodexThreads()).length, 4);
});

test("Codex desktop's projects, from its state database", () => {
  assert.deepEqual(listCodexNativeProjects(), [], "no database, no projects");
  fs.mkdirSync(process.env.CODEX_HOME, { recursive: true });
  const file = path.join(process.env.CODEX_HOME, "state_5.sqlite");
  const db = new DatabaseSync(file);
  db.exec("create table projects(id text, name text); create table project_roots(project_id text, path text, position integer);");
  db.exec("insert into projects values ('p1', 'Atlas'), ('p2', 'Empty'); insert into project_roots values ('p1', '/p/atlas-web', 2), ('p1', '/p/atlas-api', 1);");
  db.close();
  assert.deepEqual(listCodexNativeProjects(), [{ id: "p1", name: "Atlas", roots: ["/p/atlas-api", "/p/atlas-web"] }, { id: "p2", name: "Empty", roots: [] }]);
  fs.writeFileSync(file, "not a database");
  assert.deepEqual(listCodexNativeProjects(), [], "an unreadable one only drops the grouping");
  assert.deepEqual(found("openai/codex/state-db"), [], "and isn't drift");

  // A column it reads gone, and a state database of another number: drift.
  fs.rmSync(file);
  const moved = new DatabaseSync(file);
  moved.exec("create table projects(id text, title text); create table project_roots(project_id text, path text, position integer);");
  moved.close();
  fs.writeFileSync(path.join(process.env.CODEX_HOME, "state_6.sqlite"), "");
  assert.deepEqual(listCodexNativeProjects(), []);
  assert.deepEqual(found("openai/codex/state-db"), [["new-version", null, "6"], ["missing-field", "projects[].name", "5"]]);
});

// ---------- a thread from its rollout ----------

test("a thread the app-server can't list is read from its rollout, a page at a time", { skip: !unix }, async () => {
  const { Transcripts } = await import("../core/stats.js");
  const { drift: log } = await import("../core/drift.js");
  const sessions = path.join(root, "rollouts", "sessions");
  const day = path.join(sessions, "2026", "10", "05");
  fs.mkdirSync(day, { recursive: true });
  const id = "019a0000-0000-7000-8000-00000000c0de";
  const at = (s) => new Date(Date.parse("2026-10-05T12:00:00Z") + s * 1000).toISOString();
  const ms = (s) => Date.parse(at(s));
  const done = (s, item, extra = {}) => ({ type: "event_msg", timestamp: at(s), payload: { type: "item_completed", thread_id: id, turn_id: "u1", started_at_ms: ms(s), completed_at_ms: ms(s + 1), item, ...extra } });
  const lines = [
    { type: "session_meta", timestamp: at(0), payload: { id, cwd: "/p/ledger", cli_version: "0.155.0-alpha.9.2", model_provider: "openai" } },
    { type: "turn_context", timestamp: at(0), payload: { model: "gpt-6-sol", effort: "high" } },
    { type: "response_item", timestamp: at(0), payload: { type: "message", role: "user", content: [{ type: "input_text", text: "<environment_context>…</environment_context>" }] } },
    done(1, { type: "UserMessage", id: "i1", client_id: "c1", content: [{ type: "text", text: "Why is the build slow?", text_elements: [] }, { type: "local_image", path: "/p/ledger/flame.png" }] }),
    { type: "event_msg", timestamp: at(1), payload: { type: "user_message", message: "Why is the build slow?" } },
    done(2, { type: "Reasoning", id: "i2", summary_text: ["Looking at the Makefile"], raw_content: [] }),
    // The first command's output is of another type: read without it.
    done(3, { type: "CommandExecution", id: "i3", command: ["/bin/zsh", "-lc", "time make"], exit_code: 0, status: "completed", aggregated_output: 42 }),
    done(4, { type: "CommandExecution", id: "i4", command: ["/bin/zsh", "-lc", "make -j8"], exit_code: 2, status: "failed", aggregated_output: "make: *** [all] Error 2" }),
    "not json",
    done(5, { type: "McpToolCall", id: "i5", server: "wiki", tool: "read", arguments: { page: "ledger" }, status: "completed", result: { content: [{ type: "text", text: "notes" }] } }),
    done(6, { type: "FileChange", id: "i6", changes: { "/p/ledger/Makefile": { type: "update", unified_diff: "@@" }, "/p/ledger/ci.yml": { type: "add", content: "" } }, status: "completed" }),
    done(7, { type: "WebSearch", id: "i7", query: "", action: { type: "search", query: "make -j" } }),
    done(8, { type: "ContextCompaction", id: "i8" }),
    done(9, { type: "SubAgentActivity", id: "i9", kind: "started", agent_thread_id: "t2", agent_path: "/root/profiler" }),
    { type: "event_msg", timestamp: at(10), payload: { type: "item_completed", completed_at_ms: ms(10), item: { type: "AgentMessage", id: "i10", content: [{ type: "output_text", text: "Make runs serially." }, { type: "output_text", text: "Add -j8." }] } } },
  ];
  fs.writeFileSync(path.join(day, `rollout-2026-10-05T12-00-00-${id}.jsonl`), lines.map((l) => (typeof l === "string" ? l : JSON.stringify(l))).join("\n") + "\n");
  // The rollout is found as XenoNetState finds it, by the thread's id.
  const transcripts = new Transcripts({ codexRoots: [sessions] });
  const rollout = (thread) => async () => (await transcripts.files(`codex:${thread}`))[0]?.file ?? null;

  // An app-server that doesn't start at all: the rollout says it all.
  codexClient.stop();
  process.env.FAKE_APP_SERVER = "exit";
  let page;
  try {
    page = await readCodexThread(id, { rollout: rollout(id) });
  } finally {
    delete process.env.FAKE_APP_SERVER;
  }
  assert.deepEqual(page.entries.map((e) => [e.id, e.role, e.ts]), [["i1", "user", ms(1)], ["i2", "reasoning", ms(2)], ["i3", "tool", ms(3)], ["i4", "tool", ms(4)], ["i5", "tool", ms(5)], ["i6", "tool", ms(6)], ["i7", "tool", ms(7)], ["i8", "system", ms(8)], ["i9", "system", ms(9)], ["i10", "assistant", ms(10)]]);
  assert.deepEqual([page.cursor, page.total], [null, 10]);
  const by = Object.fromEntries(page.entries.map((e) => [e.id, e]));
  assert.equal(by.i1.text, "Why is the build slow?\n[image]");
  assert.equal(by.i2.text, "Looking at the Makefile");
  assert.deepEqual([by.i3.title, by.i3.meta, by.i3.text], ["time make", "exit 0", ""]);
  assert.deepEqual([by.i4.title, by.i4.status, by.i4.meta, by.i4.text], ["make -j8", "failed", "exit 2", "make: *** [all] Error 2"]);
  assert.deepEqual([by.i5.title, JSON.parse(by.i5.text)], ["wiki.read", [{ type: "text", text: "notes" }]]);
  assert.deepEqual([by.i6.title, by.i6.text], ["edit 2 file(s)", "update /p/ledger/Makefile\nadd /p/ledger/ci.yml"]);
  assert.equal(by.i7.title, "search: make -j");
  assert.equal(by.i8.text, "context compacted");
  assert.deepEqual([by.i9.title, JSON.parse(by.i9.text).agent_path], ["SubAgentActivity", "/root/profiler"]);
  assert.equal(by.i10.text, "Make runs serially.\nAdd -j8.");
  assert.ok(log.list().some((r) => r.source === "openai/codex/rollout-line" && r.kind === "type-change" && r.path === "payload.item.aggregated_output"), "read leniently: the odd field is drift");

  // An app-server that says it can't: the rollout, and it isn't asked again.
  const asked = () => codexClient.counters.requests;
  const before = asked();
  const last = await readCodexThread(id, { limit: 4, rollout: rollout(id) });
  assert.equal(asked(), before + 2, "initialize, and thread/items/list once");
  assert.deepEqual([last.entries.map((e) => e.id), last.cursor, last.total], [["i7", "i8", "i9", "i10"], "rollout:4", 10]);
  const earlier = await readCodexThread(id, { limit: 4, cursor: last.cursor, rollout: rollout(id) });
  assert.deepEqual([earlier.entries.map((e) => e.id), earlier.cursor], [["i3", "i4", "i5", "i6"], "rollout:8"]);
  const first = await readCodexThread(id, { limit: 4, cursor: earlier.cursor, rollout: rollout(id) });
  assert.deepEqual([first.entries.map((e) => e.id), first.cursor], [["i1", "i2"], null]);
  assert.equal(asked(), before + 2, "pages further back, and this Codex's other threads, come from their rollouts");

  // Without a rollout, the app-server's answer stands.
  await assert.rejects(readCodexThread("019a0000-0000-7000-8000-00000000dead"), /not supported yet/);
  await assert.rejects(readCodexThread("019a0000-0000-7000-8000-00000000dead", { rollout: rollout("019a0000-0000-7000-8000-00000000dead") }), /can't list a thread's items, and XenoNet can't find this thread's rollout/);
  // And one it can list is read from it, as before.
  assert.equal((await readCodexThread("a", { limit: 3 })).cursor, "older");
});

test("a rollout without finished items, as codex exec writes them, shows what was said and each call", async () => {
  const file = path.join(root, "rollouts", "rollout-2026-10-05T13-00-00-019a0000-0000-7000-8000-0000000e0e0e.jsonl");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const at = (s) => new Date(Date.parse("2026-10-05T13:00:00Z") + s * 1000).toISOString();
  const lines = [
    { type: "session_meta", timestamp: at(0), payload: { id: "e1", cwd: "/p/ledger", cli_version: "0.146.0", source: "exec", originator: "codex_exec" } },
    { type: "response_item", timestamp: at(0), payload: { type: "message", role: "developer", content: [{ type: "input_text", text: "<permissions>…</permissions>" }] } },
    { type: "response_item", timestamp: at(0), payload: { type: "message", role: "user", content: [{ type: "input_text", text: "Summarize the day" }] } },
    { type: "event_msg", timestamp: at(0), payload: { type: "user_message", message: "Summarize the day", images: [] } },
    { type: "response_item", timestamp: at(1), payload: { type: "custom_tool_call", id: "ct1", call_id: "call_1", name: "exec_command", input: '{"cmd":"git log -3"}', status: "completed" } },
    { type: "response_item", timestamp: at(2), payload: { type: "custom_tool_call_output", id: "ct1o", call_id: "call_1", output: [{ type: "input_text", text: "3 commits" }] } },
    { type: "event_msg", timestamp: at(3), payload: { type: "agent_message", message: "Three commits today.", phase: "final" } },
    { type: "response_item", timestamp: at(3), payload: { type: "message", role: "assistant", content: [{ type: "output_text", text: "Three commits today." }] } },
    { type: "event_msg", timestamp: at(4), payload: { type: "task_complete", last_agent_message: "Three commits today." } },
  ];
  fs.writeFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  const page = await readRolloutPage(file);
  assert.deepEqual(page.entries.map((e) => [e.role, e.title ?? null, e.ts]), [["user", null, Date.parse(at(0))], ["tool", "exec_command", Date.parse(at(1))], ["tool", "result", Date.parse(at(2))], ["assistant", null, Date.parse(at(3))]]);
  assert.deepEqual(page.entries.map((e) => e.text), ["Summarize the day", '{"cmd":"git log -3"}', JSON.stringify([{ type: "input_text", text: "3 commits" }], null, 2), "Three commits today."]);
});

test("a rollout's plan reads as the app-server's, and an extension's web search as a web search", async () => {
  const file = path.join(root, "rollouts", "rollout-2026-10-05T15-00-00-019a0000-0000-7000-8000-0000000f1a77.jsonl");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const at = (s) => new Date(Date.parse("2026-10-05T15:00:00Z") + s * 1000).toISOString();
  const done = (s, item) => ({ type: "event_msg", timestamp: at(s), payload: { type: "item_completed", thread_id: "f1", turn_id: "u1", completed_at_ms: Date.parse(at(s)), item } });
  const lines = [
    { type: "session_meta", timestamp: at(0), payload: { id: "f1", cwd: "/p/ledger", cli_version: "0.150.0" } },
    done(1, { type: "Plan", id: "p1", text: "1. Read the Makefile\n2. Add -j8" }),
    done(2, { type: "Extension", id: "x1", kind: "web.search", query: "", action: { type: "search", query: null, queries: ["make -j", "make jobs"] }, results: [] }),
    done(3, { type: "Extension", id: "x2", kind: "web.search", query: "gnu make jobserver", action: { type: "search", query: "gnu make jobserver", queries: null } }),
    done(4, { type: "Extension", id: "x3", kind: "image_gen.generation", status: "completed", revisedPrompt: "a ledger", savedPath: "/p/ledger/ledger.png", failure: null }),
  ];
  fs.writeFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
  const page = await readRolloutPage(file);
  assert.deepEqual(
    page.entries.map((e) => [e.id, e.role, e.title ?? null]),
    [
      ["p1", "assistant", "plan"],
      ["x1", "tool", "search: "],
      ["x2", "tool", "search: gnu make jobserver"],
      ["x3", "system", "Extension"],
    ],
  );
  assert.equal(page.entries[0].text, "1. Read the Makefile\n2. Add -j8");
  assert.equal(JSON.parse(page.entries[3].text).kind, "image_gen.generation", "an image made shows as any other item");
});

test("a rollout that grows is read on from where its last read ended, not whole again", async () => {
  const { codexLines } = await import("../core/lines.js");
  const file = path.join(root, "rollouts", "rollout-2026-10-05T14-00-00-019a0000-0000-7000-8000-00000000abcd.jsonl");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const at = (s) => new Date(Date.parse("2026-10-05T14:00:00Z") + s * 1000).toISOString();
  const done = (s, item) => JSON.stringify({ type: "event_msg", timestamp: at(s), payload: { type: "item_completed", item: { id: `i${s}`, ...item } } }) + "\n";
  const said = (s, text) => done(s, { type: "AgentMessage", content: [{ type: "output_text", text }] });
  const records = () => codexLines.counters.records;
  fs.writeFileSync(file, JSON.stringify({ type: "session_meta", timestamp: at(0), payload: { id: "g1", cli_version: "0.155.0" } }) + "\n" + done(1, { type: "UserMessage", content: [{ type: "text", text: "Profile the build" }] }) + said(2, "Looking.").trimEnd());
  let before = records();
  let page = await readRolloutPage(file);
  assert.deepEqual([page.entries.map((e) => e.id), page.total, records() - before], [["i1", "i2"], 2, 3], "a last line with no newline yet is read when it's whole");

  // Lines added, the last one half written, split inside a character.
  const next = Buffer.from("\n" + said(3, "Make runs serially ✓") + said(4, "Add -j8."));
  const cut = next.indexOf(Buffer.from("✓")) + 1;
  fs.appendFileSync(file, next.subarray(0, cut));
  before = records();
  page = await readRolloutPage(file);
  assert.deepEqual([page.entries.map((e) => e.id), records() - before], [["i1", "i2"], 0], "only what was added is read, and half a line waits");
  fs.appendFileSync(file, next.subarray(cut));
  before = records();
  page = await readRolloutPage(file);
  assert.deepEqual([page.entries.map((e) => e.id), page.entries.at(-2).text, records() - before], [["i1", "i2", "i3", "i4"], "Make runs serially ✓", 2]);
  before = records();
  assert.equal((await readRolloutPage(file, { limit: 2 })).cursor, "rollout:2");
  assert.equal(records(), before, "paged through without reading it again");

  // Two pages asked at once take each line once.
  fs.appendFileSync(file, said(5, "Done."));
  const [a, b] = await Promise.all([readRolloutPage(file), readRolloutPage(file)]);
  assert.deepEqual([a.total, b.total], [5, 5]);

  // Written again from the start, shorter: read from the start.
  fs.writeFileSync(file, said(9, "A new start."));
  page = await readRolloutPage(file);
  assert.deepEqual(page.entries.map((e) => e.id), ["i9"]);
});
