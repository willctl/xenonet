// Grok Build's session updates as line events (core/lines.js's GrokLine,
// core/events.js's mapper), and through the readers: a session's stats,
// search and the live tailer, which read them as they read Claude Code's.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { grok, mapperOf } from "../core/events.js";
import { GrokLine, grokLines, linesOf } from "../core/lines.js";
import { Transcripts, createStats } from "../core/stats.js";
import { TranscriptSearch, docsFromLine } from "../core/search.js";
import { LiveTelemetry } from "../core/live.js";
import { grokHome, IDS, MAIN_UPDATES, T, up, text, jsonl, usage, turn, ledger } from "./fixtures/grok.js";

const dir = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-lines-")));
after(() => fs.rmSync(dir, { recursive: true, force: true }));
const home = grokHome(path.join(dir, "grok"));
const root = path.join(home, "sessions");
const kinds = (events) => events.map((e) => (e.kind === "broken" ? `broken:${e.at}` : e.kind));

test("a line reads its update in the envelope Grok writes, or bare", () => {
  const o = new GrokLine(MAIN_UPDATES[0]);
  assert.equal(o.kind, "user_message_chunk");
  assert.equal(o.ts, Math.floor(T / 1000) * 1000, "the envelope's time is in seconds");
  assert.equal(o.text, "Add partial refunds ");
  const bare = new GrokLine({ sessionId: IDS.main, update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "Hi" } } });
  assert.deepEqual([bare.kind, bare.text, Number.isNaN(bare.ts)], ["agent_message_chunk", "Hi", true]);
  assert.equal(new GrokLine(MAIN_UPDATES[5]).toolName, "run_terminal_cmd", "Grok's own name for its tool");
  assert.equal(new GrokLine(MAIN_UPDATES[9]).toolName, "mcp__linear__save_issue", "an MCP tool, as XenoNet names them");
  assert.deepEqual(new GrokLine(MAIN_UPDATES[7]).toolInput.file_path, "/Users/ada/atlas/src/refunds.ts", "a tool's file, from its locations");
  assert.equal(new GrokLine(MAIN_UPDATES[12]).own, true);
  assert.equal(linesOf("grok"), grokLines);
  assert.equal(mapperOf("grok"), grok);
});

test("a session's updates as events: a run of chunks is one message, whose later chunks carry it on", () => {
  const file = grok.file();
  const events = MAIN_UPDATES.map((u) => grok.events(new GrokLine(u), file));
  assert.deepEqual(events.map(kinds), [
    ["prompt"],
    ["more"],
    ["call", "said"],
    ["call", "said"],
    ["more"],
    ["call", "said", "tool"],
    ["result"],
    ["call", "said", "tool"],
    ["result"],
    ["call", "said", "tool"],
    ["result"],
    ["context"],
    [],
    ["call", "said"],
  ]);
  const [prompt] = events[0];
  assert.deepEqual([prompt.origin, prompt.text, prompt.meta, prompt.command], ["user", "Add partial refunds ", false, null]);
  const pick = (e) => [e.origin, e.of, e.text, e.whole, e.meta];
  assert.deepEqual(pick(events[1][0]), ["user", "prompt", "to the orders API.", "Add partial refunds to the orders API.", false]);
  assert.deepEqual(events[2][1].parts, [{ type: "thinking", thinking: "Look at the refund route first." }], "a thought, as Claude Code's thinking");
  assert.deepEqual(pick(events[4][0]), ["assistant", "text", " Then the route.", "I'll start with the tests. Then the route.", false]);
  assert.equal(file.run.text, "Partial refunds work. Should I open a PR?", "the run left open");
  // A run goes on past what says nothing of the conversation, and ends at
  // anything else; a prompt of another index, or an interjection's text,
  // begins another.
  const run = (...updates) => {
    const f = grok.file();
    return updates.map((u) => kinds(grok.events(new GrokLine(up(T, u)), f)).join(","));
  };
  assert.deepEqual(run(text("agent_thought_chunk", "a"), { sessionUpdate: "usage_update", used: 1, size: 2 }, text("agent_thought_chunk", "b"), { sessionUpdate: "plan", entries: [] }, text("agent_thought_chunk", "c")), ["call,said", "context", "more", "", "more"]);
  assert.deepEqual(run(text("user_message_chunk", "a", { _meta: { promptIndex: 1 } }), text("user_message_chunk", "b", { _meta: { promptIndex: 2 } }), text("user_message_chunk", "c", { _meta: { promptIndex: 2, interjection: true } })), ["prompt", "prompt", "prompt"]);
  assert.deepEqual(run(text("user_message_chunk", "a", { _meta: { interjection: true } }), text("user_message_chunk", "b", { _meta: { interjection: true } })), ["prompt", "prompt"], "each interjection's text is its own");
  assert.deepEqual(run(text("agent_message_chunk", "a"), { sessionUpdate: "turn_completed" }, text("agent_message_chunk", "b")), ["call,said", "", "call,said"]);
  const own = grok.file();
  grok.events(new GrokLine(up(T, text("agent_message_chunk", "a"))), own);
  assert.deepEqual(kinds(grok.events(new GrokLine(up(T, { sessionUpdate: "turn_completed" }, { method: "_x.ai/session/update" })), own)), []);
  assert.equal(own.run, null, "Grok's own update ends it");
  // A reader that read part of a file goes on with the run it kept.
  const kept = grok.file({ run: { kind: "assistant", key: "false||false", text: "Part one" } });
  assert.equal(grok.events(new GrokLine(up(T, text("agent_message_chunk", ", part two."))), kept)[0].whole, "Part one, part two.");
  assert.deepEqual(events[5][1].parts, [{ type: "tool_use", id: "call-1", name: "run_terminal_cmd", input: { command: "npm test -- orders" } }]);
  assert.deepEqual([events[10][0].origin, events[10][0].error], ["user", true]);
  assert.deepEqual([events[11][0].used, events[11][0].window], [41000, 500000]);
  // What Grok's host asked, or a shell command you ran from its prompt,
  // isn't a prompt of yours.
  assert.equal(grok.events(new GrokLine(up(T, text("user_message_chunk", "ls", { _meta: { hostTurn: true } }))), grok.file())[0].meta, true);
  assert.equal(grok.events(new GrokLine(up(T, { sessionUpdate: "user_message_chunk", content: { type: "text", text: "ls", _meta: { bash_command: "ls" } } })), grok.file())[0].meta, true);
  assert.deepEqual(kinds(grok.events(new GrokLine(null), grok.file())), ["broken:line"]);
});

test("a Grok Build session's stats and search, through the readers Claude Code's go through", async () => {
  const transcripts = new Transcripts({ grokRoot: root });
  assert.deepEqual(await transcripts.files(`grok:${IDS.main}`), [{ file: path.join(root, encodeURIComponent("/Users/ada/atlas"), IDS.main, "updates.jsonl"), agentId: null }]);
  assert.deepEqual(await transcripts.files(`grok:${IDS.headless}`), [], "a session with no updates has no transcript");
  const stats = await createStats({ transcripts })(`grok:${IDS.main}`);
  assert.equal(stats.app, "grok");
  assert.deepEqual(stats.turns.map((t) => t.prompt), ["Add partial refunds to the orders API."], "the whole prompt");
  assert.deepEqual(stats.tools.map((t) => [t.name, t.calls, t.errors]).sort(), [["mcp__linear__save_issue", 1, 1], ["run_terminal_cmd", 1, 0], ["search_replace", 1, 0]]);
  // Its tokens are its usage.json's: the turn's model calls, when it
  // ended, with no cost recorded, so the cost isn't known, never 0.
  assert.deepEqual([stats.totals.calls, stats.totals.input, stats.totals.cacheRead, stats.totals.output, stats.totals.tokens, stats.totals.cost], [4, 12000, 40000, 1800, 53800, null]);
  assert.deepEqual(stats.turns.map((t) => [t.tokens, t.cost, t.end]), [[53800, null, T + 14500]]);
  assert.deepEqual(stats.models.map((m) => [m.model, m.calls, m.cost, m.priced]), [["grok-build", 4, null, false]]);
  assert.deepEqual(stats.pricing.unpriced, ["grok-build"]);
  assert.deepEqual(stats.context, [{ t: Math.floor((T + 12600) / 1000) * 1000, used: 41000, window: 500000 }], "how full the context got, from its usage updates");
  // A fork's own turns, at the cost Grok recorded, not split by kind; the
  // turn it copied from its parent's ledger isn't its own.
  const fork = await createStats({ transcripts })(`grok:${IDS.fork}`);
  assert.deepEqual([fork.totals.calls, fork.totals.input, fork.totals.cost, fork.totals.unpricedCalls], [4, 9000, 0.0122, 0]);
  assert.deepEqual(Object.values(fork.kinds).map((k) => k.cost), [null, null, null, null]);
  assert.deepEqual(fork.models.map((m) => [m.model, m.cost]), [["grok-build", 0.012], ["grok-build-mini", 0.0002]]);
  assert.deepEqual(fork.turns.map((t) => t.cost), [0.0122]);
  // A subagent's own.
  const sub = await createStats({ transcripts })(`grok:${IDS.sub}`);
  assert.deepEqual([sub.totals.calls, sub.totals.tokens], [2, 9300]);

  const search = new TranscriptSearch({ dbFile: path.join(dir, "search.sqlite"), grokRoot: root });
  await search.index();
  assert.deepEqual(search.search("partial refunds").map((h) => [h.sessionId, h.app]), [[`grok:${IDS.main}`, "grok"]]);
  assert.deepEqual(search.search("feature flag").map((h) => h.sessionId), [`grok:${IDS.fork}`]);
  assert.deepEqual(search.search("Should I open").map((h) => h.snippets[0].role), ["assistant"]);
  // A message is one record, all its chunks' words in it; thinking isn't
  // indexed.
  assert.deepEqual(search.search("tests then the route").map((h) => [h.sessionId, h.hits]), [[`grok:${IDS.main}`, 1]]);
  assert.deepEqual(search.search("orders API").map((h) => h.snippets.map((x) => x.role)), [["user"]]);
  assert.deepEqual(search.search("refund route first"), []);
  assert.equal(search.sessionOf(path.join(root, "x", IDS.main, "chat_history.jsonl"), "grok"), null, "only a session's updates are read");
  assert.deepEqual(docsFromLine(MAIN_UPDATES[0], "grok").map((d) => d.role), ["user"]);
  search.close();
});

test("the live tailer follows a Grok Build session; its hooks end its turns, and a quiet one ends without them", async () => {
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: root, claudeDesktopDir: null });
  const file = path.join(root, encodeURIComponent("/Users/ada/atlas"), IDS.main, "updates.jsonl");
  assert.deepEqual(live.classify(file), { app: "grok", sessionId: `grok:${IDS.main}` });
  assert.equal(live.classify(path.join(path.dirname(file), "chat_history.jsonl")), null);
  const now = Date.now();
  const [busy, quiet] = ["019a3b5c-0000-7000-8000-0000000000a1", "019a3b5c-0000-7000-8000-0000000000a2"];
  const busyFile = path.join(root, "fresh", busy, "updates.jsonl");
  const quietFile = path.join(root, "fresh", quiet, "updates.jsonl");
  fs.mkdirSync(path.dirname(busyFile), { recursive: true });
  fs.mkdirSync(path.dirname(quietFile), { recursive: true });
  fs.writeFileSync(busyFile, jsonl([up(now - 5000, text("user_message_chunk", "Rename the refund flag")), up(now - 4000, { sessionUpdate: "tool_call", toolCallId: "c1", kind: "execute", status: "in_progress", rawInput: { command: "rg refund" } })]));
  fs.writeFileSync(quietFile, jsonl([up(now - 180e3, text("user_message_chunk", "What does reconcile do?")), up(now - 120e3, text("agent_message_chunk", "It matches the bank's lines to the ledger's."))]));
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  live.stop();
  const of = (id) => live.snapshot().sessions.find((x) => x.id === `grok:${id}`);
  let s = of(busy);
  assert.deepEqual([s.app, s.status, s.lastTool.name, s.lastTool.detail, s.context.window], ["grok", "tool", "execute", "rg refund", 256000]);
  fs.appendFileSync(busyFile, jsonl([up(now - 3000, { sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed" }), up(now - 2000, text("agent_message_chunk", "Renamed it.")), up(now - 1000, { sessionUpdate: "usage_update", used: 9000, size: 300000 })]));
  await live.track(busyFile, { prime: false });
  s = of(busy);
  assert.deepEqual([s.status, s.lastMessage, s.context.used, s.context.window], ["thinking", "Renamed it.", 9000, 300000]);
  // Its hook says the turn ended.
  live.noteHook(`grok:${busy}`, { hook_event_name: "Stop", sessionId: busy });
  assert.equal(of(busy).status, "idle");
  assert.ok(events.some((e) => e.kind === "turn-end" && e.sessionId === `grok:${busy}`));
  // Without hooks, a turn that's been quiet a while, with nothing in
  // flight, has ended when its last line was written.
  s = of(quiet);
  assert.deepEqual([s.status, s.lastTurnEndAt], ["idle", Math.floor((now - 120e3) / 1000) * 1000]);
  assert.equal(of(IDS.main), undefined, "a day-old session isn't in the snapshot");
});

test("the live tailer keeps a streamed prompt and reply whole, as their chunks come", async () => {
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: root, claudeDesktopDir: null });
  const now = Date.now();
  const id = "019a3b5c-0000-7000-8000-0000000000c1";
  const file = path.join(root, "streams", id, "updates.jsonl");
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, jsonl([up(now - 9000, text("user_message_chunk", "Make the refund ")), up(now - 9000, text("user_message_chunk", "button red."))]));
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  live.stop();
  const notes = () => live.sessionNotes(`grok:${id}`);
  assert.deepEqual(notes().prompts.map((p) => p.text), ["Make the refund button red."]);
  assert.equal(notes().firstPrompt.text, "Make the refund button red.");
  // A reply whose first chunk is blank begins with its first words, and
  // each chunk after makes it whole; a thought in between isn't kept.
  const more = [
    up(now - 8000, text("agent_thought_chunk", "The button is in refund.tsx.")),
    up(now - 7000, text("agent_message_chunk", "")),
    up(now - 6000, text("agent_message_chunk", "Done:")),
    up(now - 5000, { sessionUpdate: "usage_update", used: 5000, size: 300000 }),
    up(now - 4000, text("agent_message_chunk", " the button is red.")),
  ];
  for (const u of more) {
    fs.appendFileSync(file, jsonl([u]));
    await live.track(file, { prime: false });
  }
  const s = live.snapshot().sessions.find((x) => x.id === `grok:${id}`);
  assert.equal(s.lastMessage, "Done: the button is red.");
  assert.deepEqual(notes().messages.map((m) => m.text), ["Done: the button is red."]);
  assert.equal(s.lastEventAt, Math.floor((now - 4000) / 1000) * 1000, "each chunk is a line of the turn");
  assert.deepEqual(events.filter((e) => e.kind === "message").map((e) => e.detail), ["Done:"], "the feed names the message once, as it begins");
});

test("the live tailer counts a Grok Build session's tokens from its usage.json, a turn at a time, and a cost it didn't record as unknown", async () => {
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: root, claudeDesktopDir: null });
  const now = Date.now();
  const [main, fork, sub] = ["019a3b5c-0000-7000-8000-0000000000d1", "019a3b5c-0000-7000-8000-0000000000d2", "019a3b5c-0000-7000-8000-0000000000d3"];
  const folder = (id) => path.join(root, "usage-live", id);
  const write = (id, name, value) => fs.writeFileSync(path.join(folder(id), name), name.endsWith(".jsonl") ? jsonl(value) : JSON.stringify(value));
  for (const id of [main, fork, sub]) fs.mkdirSync(folder(id), { recursive: true });
  const summary = (more = {}) => ({ info: {}, created_at: new Date(now - 60e3).toISOString(), current_model_id: "grok-build", ...more });
  const prompt = (id) => [up(now - 50e3, text("user_message_chunk", "Add refunds"), { session: id })];
  write(main, "summary.json", summary());
  write(main, "updates.jsonl", prompt(main));
  write(main, "usage.json", ledger(main, [turn(1, now - 40e3, { "grok-build": usage({ input: 5000, output: 200, cached: 3000, calls: 2 }) })]));
  // A fork, whose first turn is its parent's, and a subagent, whose turns
  // its parent's count.
  write(fork, "summary.json", summary({ parent_session_id: main, session_kind: "fork", forked_at: new Date(now - 45e3).toISOString() }));
  write(fork, "updates.jsonl", prompt(fork));
  write(fork, "usage.json", ledger(fork, [turn(1, now - 50e3, { "grok-build": usage({ input: 1000, output: 100 }) }), turn(2, now - 30e3, { "grok-build": usage({ input: 700, output: 70 }) })]));
  write(sub, "summary.json", summary({ parent_session_id: main, session_kind: "subagent" }));
  write(sub, "updates.jsonl", prompt(sub));
  write(sub, "usage.json", ledger(sub, [turn(1, now - 35e3, { "grok-build": usage({ input: 9000, output: 900 }) })]));
  const tokens = [];
  live.on("tokens", (e) => tokens.push(e));
  await live.start();
  live.stop();
  const of = (id) => live.snapshot().sessions.find((x) => x.id === `grok:${id}`);
  assert.deepEqual([of(main).tokens, of(main).model], [{ processed: 5200, output: 200 }, "grok-build"]);
  assert.deepEqual(of(fork).tokens, { processed: 770, output: 70 }, "its own turn alone");
  assert.deepEqual(of(sub).tokens, { processed: 0, output: 0 }, "counted in its parent's turns");
  assert.deepEqual(tokens, [], "history isn't told as it's read");
  // No cost recorded: the day's cost isn't known, never 0.
  let costs = live.costs(now, []);
  assert.deepEqual([costs.today, costs.byApp.grok, costs.unpriced], [null, null, ["grok-build"]]);
  // The turn written again (a message folded into it) adds what's new, and
  // a turn on an API key brings its cost.
  write(main, "usage.json", ledger(main, [turn(1, now - 20e3, { "grok-build": usage({ input: 6000, output: 300, cached: 3000, calls: 3 }) }), turn(2, now - 10e3, { "grok-build": usage({ input: 2000, output: 100, ticks: 30000000 }) })]));
  fs.appendFileSync(path.join(folder(main), "updates.jsonl"), jsonl([up(now - 10e3, { sessionUpdate: "turn_completed" }, { session: main, method: "_x.ai/session/update" })]));
  await live.track(path.join(folder(main), "updates.jsonl"), { prime: false });
  assert.deepEqual(of(main).tokens, { processed: 8400, output: 400 });
  assert.deepEqual(tokens.map((e) => [e.app, e.n]), [["grok", 1100], ["grok", 2100]]);
  costs = live.costs(now, []);
  assert.deepEqual([costs.byApp.grok, costs.today, costs.unpriced], [0.003, 0.003, ["grok-build"]]);
  // Read again only once it's written again.
  await live.track(path.join(folder(main), "updates.jsonl"), { prime: false });
  assert.equal(tokens.length, 2);
});

test("a Grok Build session's edits count, in its own tools' names, so two sessions on one file collide", async () => {
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: root, claudeDesktopDir: null });
  const now = Date.now();
  const own = (name) => ({ _meta: { "x.ai/tool": { version: 1, name, kind: "edit" } } });
  const [a, b] = ["019a3b5c-0000-7000-8000-0000000000b1", "019a3b5c-0000-7000-8000-0000000000b2"];
  const file = "/Users/ada/atlas/src/refunds.ts";
  const write = (id, call) => {
    const f = path.join(root, "edits", id, "updates.jsonl");
    fs.mkdirSync(path.dirname(f), { recursive: true });
    fs.writeFileSync(f, jsonl([up(now - 4000, text("user_message_chunk", "Fix refunds")), up(now - 3000, { sessionUpdate: "tool_call", toolCallId: `c-${id.slice(-2)}`, kind: "edit", status: "completed", locations: [{ path: file }], ...call })]));
  };
  write(a, { rawInput: { file_path: file, old_string: "partial", new_string: "full" }, ...own("search_replace") });
  // A write names its file in locations alone.
  write(b, { rawInput: { content: "export const refunds = [];" }, ...own("write") });
  await live.start();
  live.stop();
  for (const id of [a, b]) assert.deepEqual(live.sessionNotes(`grok:${id}`).edits, [file], id);
  const [c] = live.collisions(now).filter((x) => x.a.startsWith("grok:") && x.b.startsWith("grok:"));
  assert.deepEqual([c?.a, c?.b, c?.files], [`grok:${a}`, `grok:${b}`, ["refunds.ts"]]);
  const [d] = live.collisionDiffs(`grok:${a}`, `grok:${b}`);
  assert.deepEqual([d.a[0].kind, d.a[0].diff, d.b[0].kind, d.b[0].diff], ["edit", "- partial\n+ full", "write", "+ export const refunds = [];"]);
});
