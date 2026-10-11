// Grok Build's files, as XenoNet reads them (core/grok.js and core/lines.js):
// each session's summary.json, the lines of its updates.jsonl, its usage
// ledger and its subagents' meta.json, all under ~/.grok/sessions (GROK_HOME
// moves it). Grok Build's user guide lists summary.json's fields and says
// updates.jsonl holds ACP session updates; the rest isn't documented, so
// every field XenoNet reads from them is one it can do without, and none of
// them carries a version (docs/design/grok-build.md).
import { vendor, open, variants, skipped, maybe, num, str, z } from "../common.js";

// A time as Grok writes it: an ISO 8601 string, or epoch seconds or
// milliseconds.
const time = z.union([str, num]);

// ---------- summary.json ----------

export const SessionSummary = vendor(
  "spacexai/grok-build/session-summary",
  1,
  open({
    info: open({
      id: maybe(str),
      session_id: maybe(str),
      sessionId: maybe(str),
      cwd: maybe(str),
    })
      .optional()
      .describe("The session's id and the folder it works in"),
    session_summary: maybe(str),
    generated_title: maybe(str).describe("The title Grok made, or yours after /rename"),
    title_is_manual: maybe(z.boolean()).describe("True once you renamed it (/rename)"),
    created_at: maybe(time),
    updated_at: maybe(time),
    last_active_at: maybe(time),
    num_messages: maybe(num),
    num_chat_messages: maybe(num),
    current_model_id: maybe(str),
    parent_session_id: maybe(str).describe("The session it was forked or restored from"),
    session_kind: maybe(str),
    hidden: maybe(z.boolean()),
    source_workspace_dir: maybe(str).describe("Where a worktree session's work came from"),
    git_root_dir: maybe(str),
    head_branch: maybe(str),
    worktree_label: maybe(str),
    agent_name: maybe(str),
    last_turn_summary: maybe(str),
    last_recap: maybe(str),
  }),
  {
    title: "A Grok Build session's summary",
    description:
      "Grok Build's ~/.grok/sessions/<encoded cwd>/<session id>/summary.json, the session's index entry: its title, model, times, folder and the session it came from. The user guide lists the fields; the file carries no version, so the newest schema reads it.",
    versions: "*",
  },
);

// ---------- a line of updates.jsonl ----------

// A content block, as ACP has them: text, or an image or a resource.
const block = open({ type: str.optional(), text: str.optional() });
const chunk = (kind) => open({ sessionUpdate: z.literal(kind), content: z.union([block, z.array(block)]).optional() });
// Where a tool call works, and what it changed (ACP's diff content).
const location = open({ path: str });
const toolContent = z.array(z.unknown());

const update = variants("sessionUpdate", [
  chunk("user_message_chunk"),
  chunk("agent_message_chunk"),
  chunk("agent_thought_chunk"),
  open({
    sessionUpdate: z.literal("tool_call"),
    toolCallId: str,
    title: maybe(str),
    kind: maybe(str).describe("ACP's kind: read, edit, delete, move, search, execute, think, fetch or other"),
    status: maybe(str),
    rawInput: z.unknown().optional(),
    locations: maybe(z.array(location)),
    content: maybe(toolContent),
  }),
  open({
    sessionUpdate: z.literal("tool_call_update"),
    toolCallId: str,
    title: maybe(str),
    kind: maybe(str),
    status: maybe(str),
    rawInput: z.unknown().optional(),
    rawOutput: z.unknown().optional(),
    locations: maybe(z.array(location)),
    content: maybe(toolContent),
  }),
  open({ sessionUpdate: z.literal("plan"), entries: z.array(open({ content: maybe(str), status: maybe(str) })) }),
  open({ sessionUpdate: z.literal("usage_update"), used: maybe(num).describe("Tokens in the context now"), size: maybe(num).describe("The context window") }),
  open({ sessionUpdate: z.literal("session_info_update"), title: maybe(str), updatedAt: maybe(time) }),
  ...skipped("sessionUpdate", "available_commands_update", "current_mode_update", "config_option_update"),
]);

export const UpdateLine = vendor(
  "spacexai/grok-build/update-line",
  1,
  open({
    update: update.optional().describe("The ACP session update the line carries"),
    timestamp: maybe(time),
    sessionId: maybe(str),
  }),
  {
    title: "A line of a Grok Build session's updates.jsonl",
    description:
      "One line of Grok Build's updates.jsonl: an ACP session update (session/update's `update`), in an envelope that may say when it was written. The live tailer, stats and search read it through core/lines.js's normalizer, which also takes a bare update. The lines carry no version, so the newest schema reads them.",
    versions: "*",
  },
);

// ---------- usage.json ----------

// Token and cost totals, as `grok usage` prints them: for the session and
// for each turn. costUsdTicks is 10^10 ticks to the dollar.
const counts = {
  inputTokens: maybe(num),
  outputTokens: maybe(num),
  totalTokens: maybe(num),
  cacheReadTokens: maybe(num),
  cacheReadInputTokens: maybe(num),
  cacheCreationTokens: maybe(num),
  reasoningTokens: maybe(num),
  modelCalls: maybe(num),
  costUsdTicks: maybe(num),
  costIsPartial: maybe(z.boolean()),
};

export const Usage = vendor(
  "spacexai/grok-build/usage",
  1,
  open({
    sessionId: maybe(str),
    updatedAt: maybe(time),
    session: open(counts).optional(),
    turns: z.array(open({ turnNumber: maybe(num), endedAt: maybe(time), model: maybe(str), ...counts })).optional(),
    modelUsage: z.record(str, open(counts)).optional().describe("By model"),
  }),
  {
    title: "A Grok Build session's usage",
    description:
      "Grok Build's usage ledger beside a session's summary.json (usage.json): its tokens and cost, for the session and each turn, in the shape `grok usage <session id>` prints. Undocumented as a file and without a version, so the newest schema reads it.",
    versions: "*",
  },
);

// ---------- a subagent's meta.json ----------

export const SubagentMeta = vendor(
  "spacexai/grok-build/subagent-meta",
  1,
  open({
    child_session_id: maybe(str).describe("The subagent's own session, in the sessions folder"),
    subagent_type: maybe(str),
    description: maybe(str),
    worktree_path: maybe(str),
    status: maybe(str),
  }),
  {
    title: "A Grok Build subagent's meta.json",
    description: "Grok Build's <session>/subagents/<subagent>/meta.json: which session a subagent runs as, and what it was asked to do. Undocumented and without a version, so the newest schema reads it.",
    versions: "*",
  },
);
