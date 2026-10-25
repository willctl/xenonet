// A made-up Grok Build home (GROK_HOME) for the tests: session folders as
// Grok Build keeps them (sessions/<URL-encoded cwd>/<session id>/), each
// with a summary.json and an updates.jsonl of session updates in the
// envelope Grok writes. The shapes follow Grok Build's guide and source
// (docs/design/grok-build.md); nothing here comes from anyone's sessions.
import fs from "node:fs";
import path from "node:path";

export const IDS = {
  main: "019a3b5c-0000-7000-8000-00000000000a",
  sub: "019a3b5c-0000-7000-8000-00000000000b",
  fork: "019a3b5c-0000-7000-8000-00000000000c",
  headless: "019a3b5c-0000-7000-8000-00000000000d",
  long: "019a3b5c-0000-7000-8000-00000000000e",
  bare: "019a3b5c-0000-7000-8000-00000000000f",
};
export const ATLAS = "/Users/ada/atlas";
export const LONG = `/Users/ada/${"deeply-nested-folder/".repeat(12)}ledger`;
export const T = Date.parse("2026-10-09T15:00:00Z");
const iso = (ms) => new Date(ms).toISOString();

// An update as updates.jsonl keeps it: in an envelope, with when it was
// written in Unix seconds.
export const up = (ms, update, { session = IDS.main, method = "session/update" } = {}) => ({ timestamp: Math.floor(ms / 1000), method, params: { sessionId: session, update } });
export const text = (kind, t, more = {}) => ({ sessionUpdate: kind, content: { type: "text", text: t }, ...more });

// The main session's conversation: a prompt in two chunks, the agent's
// thinking, a reply in two, a shell call and how it went, an edit, an MCP
// call that failed, the context's fill, one of Grok's own updates, and a
// last reply.
export const MAIN_UPDATES = [
  up(T, text("user_message_chunk", "Add partial refunds ", { _meta: { promptIndex: 0 } })),
  up(T, text("user_message_chunk", "to the orders API.", { _meta: { promptIndex: 0 } })),
  up(T + 2000, text("agent_thought_chunk", "Look at the refund route first.")),
  up(T + 3000, text("agent_message_chunk", "I'll start with the tests.")),
  up(T + 3000, text("agent_message_chunk", " Then the route.")),
  up(T + 4000, { sessionUpdate: "tool_call", toolCallId: "call-1", title: "Execute `npm test -- orders`", kind: "execute", status: "in_progress", rawInput: { command: "npm test -- orders" }, _meta: { "x.ai/tool": { version: 1, name: "run_terminal_cmd", kind: "execute" } } }),
  up(T + 9000, { sessionUpdate: "tool_call_update", toolCallId: "call-1", status: "completed", content: [{ type: "content", content: { type: "text", text: "48 passing" } }] }),
  up(T + 10000, { sessionUpdate: "tool_call", toolCallId: "call-2", title: "Edit refunds.ts", kind: "edit", status: "pending", locations: [{ path: `${ATLAS}/src/refunds.ts` }], rawInput: { old_string: "refund()", new_string: "refund(amount?: number)" }, _meta: { "x.ai/tool": { version: 1, name: "search_replace", kind: "edit" } } }),
  up(T + 11000, { sessionUpdate: "tool_call_update", toolCallId: "call-2", status: "completed", content: [{ type: "diff", path: `${ATLAS}/src/refunds.ts`, oldText: "refund()", newText: "refund(amount?: number)" }] }),
  up(T + 12000, { sessionUpdate: "tool_call", toolCallId: "call-3", title: "linear__save_issue", kind: "other", status: "pending", rawInput: { title: "Refunds" } }),
  up(T + 12500, { sessionUpdate: "tool_call_update", toolCallId: "call-3", status: "failed" }),
  up(T + 12600, { sessionUpdate: "usage_update", used: 41000, size: 500000 }),
  up(T + 12700, { sessionUpdate: "auto_compact_started", tokens_used: 41000, context_window: 500000 }, { method: "_x.ai/session/update" }),
  up(T + 14000, text("agent_message_chunk", "Partial refunds work. Should I open a PR?")),
];

const write = (file, body) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, body);
};
const json = (file, value) => write(file, JSON.stringify(value, null, 2) + "\n");
export const jsonl = (list) => list.map((o) => JSON.stringify(o)).join("\n") + "\n";

// A turn's or a session's usage, as usage.json sums it: the prompt's
// tokens include what was read from the cache.
export const usage = ({ input, output, cached = 0, created = 0, reasoning = 0, calls = 1, ticks, partial } = {}) => ({
  inputTokens: input,
  outputTokens: output,
  cachedReadTokens: cached,
  cacheCreationTokens: created,
  reasoningTokens: reasoning,
  totalTokens: input + output,
  modelCalls: calls,
  ...(ticks === undefined ? {} : { costUsdTicks: ticks }),
  ...(partial ? { costIsPartial: true } : {}),
});
// A turn of usage.json: its usage, by model, added up.
export const turn = (n, endedAt, byModel) => {
  const models = Object.entries(byModel);
  const sum = (k) => models.reduce((a, [, u]) => a + (u[k] ?? 0), 0);
  const ticks = models.some(([, u]) => u.costUsdTicks !== undefined) ? sum("costUsdTicks") : undefined;
  return { turnNumber: n, endedAt: iso(endedAt), turnId: `turn-${n}`, ...usage({ input: sum("inputTokens"), output: sum("outputTokens"), cached: sum("cachedReadTokens"), created: sum("cacheCreationTokens"), calls: sum("modelCalls"), ticks }), turnCount: 1, primaryModelId: models[0]?.[0], modelUsage: byModel };
};
// The ledger: a session's totals, and its turns.
export const ledger = (session, turns) => ({ sessionId: session, updatedAt: turns.at(-1)?.endedAt ?? "", session: { inputTokens: turns.reduce((a, t) => a + t.inputTokens, 0), outputTokens: turns.reduce((a, t) => a + t.outputTokens, 0), turnCount: turns.length }, turns });

// The main session's turn, on a SuperGrok sign-in, which records no cost;
// the fork's, on an API key, which does, after the turn it copied from
// the main session's ledger; and the subagent's own, which its parent's
// turn counts too.
export const USAGE = {
  main: ledger(IDS.main, [turn(1, T + 14500, { "grok-build": usage({ input: 52000, output: 1800, cached: 40000, reasoning: 600, calls: 4 }) })]),
  fork: ledger(IDS.fork, [
    turn(1, T - 1200e3, { "grok-build": usage({ input: 20000, output: 500, cached: 15000, calls: 2 }) }),
    turn(2, T - 60e3, { "grok-build": usage({ input: 30000, output: 900, cached: 25000, calls: 3, ticks: 120000000 }), "grok-build-mini": usage({ input: 4000, output: 100, calls: 1, ticks: 2000000 }) }),
  ]),
  sub: ledger(IDS.sub, [turn(1, T + 8000, { "grok-build": usage({ input: 9000, output: 300, cached: 6000, calls: 2 }) })]),
};

// Writes the home into `dir` (made if it isn't there) and gives it back.
export function grokHome(dir) {
  const sessions = path.join(dir, "sessions");
  const atlas = path.join(sessions, encodeURIComponent(ATLAS));
  const at = (group, id) => path.join(group, id);

  json(path.join(at(atlas, IDS.main), "summary.json"), {
    info: { id: IDS.main, cwd: ATLAS },
    session_summary: "Partial refunds for the orders API, with tests.",
    generated_title: "Add partial refunds",
    title_is_manual: false,
    created_at: iso(T - 3600e3),
    updated_at: iso(T + 14000),
    num_messages: 14,
    current_model_id: "grok-build",
    head_branch: "feat/refunds",
    last_turn_summary: "Asked whether to open a PR",
  });
  write(path.join(at(atlas, IDS.main), "updates.jsonl"), jsonl(MAIN_UPDATES));
  json(path.join(at(atlas, IDS.main), "usage.json"), USAGE.main);
  // Its other files, which XenoNet doesn't read.
  write(path.join(at(atlas, IDS.main), "chat_history.jsonl"), jsonl([{ role: "user", content: "Add partial refunds to the orders API." }]));

  // A subagent's own session, and a fork you renamed, in the same folder.
  json(path.join(at(atlas, IDS.sub), "summary.json"), { info: { id: IDS.sub, cwd: ATLAS }, session_summary: "", generated_title: "Map the refund code", created_at: iso(T + 4000), updated_at: iso(T + 8000), current_model_id: "grok-build", parent_session_id: IDS.main, session_kind: "subagent" });
  write(path.join(at(atlas, IDS.sub), "updates.jsonl"), jsonl([up(T + 4000, text("user_message_chunk", "Map the refund code"), { session: IDS.sub })]));
  json(path.join(at(atlas, IDS.sub), "usage.json"), USAGE.sub);
  // What the main session's folder says of the subagent it ran.
  json(path.join(at(atlas, IDS.main), "subagents", "sa-1", "meta.json"), {
    subagent_id: "sa-1",
    parent_session_id: IDS.main,
    child_session_id: IDS.sub,
    subagent_type: "explore",
    description: "Map the refund code",
    prompt: "Find where refunds are made and checked.",
    status: "completed",
    started_at: iso(T + 4000),
    completed_at: iso(T + 8000),
  });
  json(path.join(at(atlas, IDS.fork), "summary.json"), { info: { id: IDS.fork, cwd: ATLAS }, session_summary: "", generated_title: "Refunds, the careful version", title_is_manual: true, created_at: iso(T - 600e3), updated_at: iso(T - 60e3), current_model_id: "grok-build", parent_session_id: IDS.main, session_kind: "fork" });
  write(path.join(at(atlas, IDS.fork), "updates.jsonl"), jsonl([up(T - 600e3, text("user_message_chunk", "Try it with a feature flag."), { session: IDS.fork })]));
  json(path.join(at(atlas, IDS.fork), "usage.json"), USAGE.fork);

  // A headless run (grok -p), which is background work.
  json(path.join(at(atlas, IDS.headless), "summary.json"), { info: { id: IDS.headless, cwd: ATLAS }, session_summary: "Explain this codebase", session_kind: "headless", created_at: iso(T - 300e3), updated_at: iso(T - 290e3), current_model_id: "grok-build" });

  // A folder whose encoded name would pass 255 bytes: a slug and a hash,
  // with the folder in .cwd; its summary says no folder of its own.
  const long = path.join(sessions, "ledger-3f9a1c2b");
  write(path.join(long, ".cwd"), `${LONG}\n`);
  json(path.join(at(long, IDS.long), "summary.json"), { info: { id: IDS.long }, session_summary: "Reconcile the ledger\nand more on a second line", created_at: iso(T - 86400e3), updated_at: iso(T - 86000e3), current_model_id: "grok-build" });

  // A session whose summary isn't written yet, and a folder with nothing
  // in it.
  write(path.join(at(atlas, IDS.bare), "updates.jsonl"), jsonl([up(T + 20000, text("user_message_chunk", "Hello"), { session: IDS.bare })]));
  fs.mkdirSync(path.join(atlas, "019a3b5c-0000-7000-8000-000000000099"), { recursive: true });
  return dir;
}
