// A made-up Grok Build home (GROK_HOME) for the adapter's tests: session
// folders laid out as Grok Build's user guide describes them
// (sessions/<URL-encoded cwd>/<session id>/), with a summary.json of the
// fields the guide lists, updates.jsonl lines that are ACP session updates,
// a usage ledger in `grok usage`'s shape, a subagent's meta.json, a
// worktree, and a group whose folder name was too long to encode. The
// formats beyond the guide are XenoNet's best reading of them
// (docs/design/grok-build.md). Nothing here comes from anyone's real
// sessions.
import fs from "node:fs";
import path from "node:path";

export const IDS = {
  main: "019a3b5c-0000-7000-8000-00000000000a",
  fork: "019a3b5c-0000-7000-8000-00000000000b",
  child: "019a3b5c-0000-7000-8000-00000000000c",
  worktree: "019a3b5c-0000-7000-8000-00000000000d",
  long: "019a3b5c-0000-7000-8000-00000000000e",
  headless: "019a3b5c-0000-7000-8000-00000000000f",
  bare: "019a3b5c-0000-7000-8000-000000000010",
};
export const ATLAS = "/Users/ada/atlas";
export const LONG = `/Users/ada/${"deeply-nested-folder/".repeat(12)}ledger`;
export const T = Date.parse("2026-10-09T15:00:00Z");
const iso = (ms) => new Date(ms).toISOString();

const write = (file, text) => {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, text);
};
const json = (file, value) => write(file, JSON.stringify(value, null, 2) + "\n");
const lines = (file, list) => write(file, list.map((o) => JSON.stringify(o)).join("\n") + "\n");
// An update as updates.jsonl keeps it: ACP's session/update params, with
// when it was written.
const up = (ms, update) => ({ timestamp: iso(ms), sessionId: IDS.main, update });
const text = (kind, t) => ({ sessionUpdate: kind, content: { type: "text", text: t } });

// The main session's conversation: a prompt in two chunks, a reply, a
// shell call, an edit with its diff, a plan, the context's fill, an MCP
// call, and a question for you.
export const MAIN_UPDATES = [
  up(T, text("user_message_chunk", "Add partial refunds ")),
  up(T, text("user_message_chunk", "to the orders API.")),
  up(T + 2000, text("agent_thought_chunk", "Look at the refund route first.")),
  up(T + 3000, text("agent_message_chunk", "I'll start ")),
  up(T + 3000, text("agent_message_chunk", "with the tests.")),
  up(T + 4000, { sessionUpdate: "tool_call", toolCallId: "call-1", title: "npm test -- orders", kind: "execute", status: "pending", rawInput: { command: "npm test -- orders" } }),
  up(T + 9000, { sessionUpdate: "tool_call_update", toolCallId: "call-1", status: "completed", content: [{ type: "content", content: { type: "text", text: "48 passing" } }] }),
  up(T + 10000, { sessionUpdate: "tool_call", toolCallId: "call-2", title: "Edit refunds.ts", kind: "edit", status: "in_progress", locations: [{ path: `${ATLAS}/src/refunds.ts` }], rawInput: { path: `${ATLAS}/src/refunds.ts` } }),
  up(T + 11000, { sessionUpdate: "tool_call_update", toolCallId: "call-2", status: "completed", content: [{ type: "diff", path: `${ATLAS}/src/refunds.ts`, oldText: "refund()", newText: "refund(amount?: number)" }] }),
  up(T + 12000, { sessionUpdate: "plan", entries: [{ content: "Accept an amount", status: "completed" }, { content: "Test a second partial refund", status: "pending" }] }),
  up(T + 12500, { sessionUpdate: "usage_update", used: 41000, size: 500000 }),
  up(T + 13000, { sessionUpdate: "tool_call", toolCallId: "call-3", title: "linear: save issue", kind: "other", status: "completed", rawInput: { title: "Refunds" }, _meta: { toolName: "linear__save_issue" } }),
  up(T + 14000, text("agent_message_chunk", "Partial refunds work. Should I open a PR?")),
  { sessionUpdate: "available_commands_update", availableCommands: [] },
];

// The usage ledger, in `grok usage`'s shape: input here is the whole
// prompt, cache reads included (the total says so).
export const MAIN_USAGE = {
  sessionId: IDS.main,
  updatedAt: iso(T + 14000),
  session: { inputTokens: 52000, cacheReadTokens: 40000, cacheCreationTokens: 0, outputTokens: 3000, reasoningTokens: 800, totalTokens: 55000, modelCalls: 3, costUsdTicks: 1234500000, costIsPartial: false },
  turns: [
    { turnNumber: 1, endedAt: iso(T + 9000), model: "grok-4.6", inputTokens: 20000, cacheReadTokens: 15000, outputTokens: 1000, totalTokens: 21000, modelCalls: 1, costUsdTicks: 400000000 },
    { turnNumber: 2, endedAt: iso(T + 14000), model: "grok-4.6", inputTokens: 32000, cacheReadTokens: 25000, outputTokens: 2000, totalTokens: 34000, modelCalls: 2, costUsdTicks: 834500000 },
  ],
};

// Writes the home into `dir` and returns it.
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
    num_chat_messages: 6,
    current_model_id: "grok-4.6",
    head_branch: "feat/refunds",
    git_root_dir: ATLAS,
    last_turn_summary: "Asked whether to open a PR",
    somethingNew: { kept: true },
  });
  lines(path.join(at(atlas, IDS.main), "updates.jsonl"), MAIN_UPDATES);
  json(path.join(at(atlas, IDS.main), "usage.json"), MAIN_USAGE);
  json(path.join(at(atlas, IDS.main), "subagents", "explore-1", "meta.json"), { child_session_id: IDS.child, subagent_type: "explore", description: "Map the refund code", status: "completed" });

  // A fork you renamed, and the subagent's own session, in the same folder.
  json(path.join(at(atlas, IDS.fork), "summary.json"), { info: { id: IDS.fork, cwd: ATLAS }, generated_title: "Refunds, the careful version", title_is_manual: true, created_at: iso(T - 600e3), updated_at: iso(T - 60e3), current_model_id: "grok-build-0.1", parent_session_id: IDS.main });
  lines(path.join(at(atlas, IDS.fork), "updates.jsonl"), [up(T - 600e3, text("user_message_chunk", "Try it with a feature flag."))]);
  json(path.join(at(atlas, IDS.child), "summary.json"), { info: { id: IDS.child, cwd: ATLAS }, generated_title: "Map the refund code", created_at: iso(T + 4000), updated_at: iso(T + 8000), current_model_id: "grok-4.6", session_kind: "subagent" });

  // A session Grok isolated in a worktree of atlas.
  const wt = path.join(dir, "worktrees", "atlas", "wt-fix");
  write(path.join(wt, ".git"), `gitdir: ${ATLAS}/.git/worktrees/wt-fix\n`);
  json(path.join(at(path.join(sessions, encodeURIComponent(wt)), IDS.worktree), "summary.json"), { info: { id: IDS.worktree, cwd: wt }, generated_title: "Fix the rounding", created_at: iso(T - 7200e3), updated_at: iso(T - 7000e3), worktree_label: "wt-fix", head_branch: "fix/rounding" });

  // A folder whose encoded name would pass 255 bytes: a slug and a hash,
  // with the folder in .cwd.
  const long = path.join(sessions, "ledger-3f9a1c2b");
  write(path.join(long, ".cwd"), `${LONG}\n`);
  json(path.join(at(long, IDS.long), "summary.json"), { info: { id: IDS.long }, session_summary: "Reconcile the ledger\nand more on a second line", created_at: Math.floor((T - 86400e3) / 1000), updated_at: Math.floor((T - 86000e3) / 1000) });

  // A headless run (grok -p), which is background work.
  json(path.join(at(atlas, IDS.headless), "summary.json"), { info: { id: IDS.headless, cwd: ATLAS }, generated_title: "Explain this codebase", session_kind: "headless", created_at: iso(T - 300e3), updated_at: iso(T - 290e3) });

  // A session whose summary isn't written yet, and a folder with nothing in it.
  lines(path.join(at(atlas, IDS.bare), "updates.jsonl"), [up(T + 20000, text("user_message_chunk", "Hello"))]);
  fs.mkdirSync(path.join(atlas, "019a3b5c-0000-7000-8000-000000000099"), { recursive: true });
  return dir;
}
