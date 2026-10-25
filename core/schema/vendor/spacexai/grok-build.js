// Grok Build's files, as XenoNet reads them: each session's summary.json
// (core/grok.js) and the lines of its updates.jsonl (core/lines.js), under
// ~/.grok/sessions/<encoded cwd>/<session id>/ (GROK_HOME moves ~/.grok). A
// subagent's session is one like any other, whose summary names its parent.
// Only the fields XenoNet reads are declared: never a tool's output.
//
// From Grok Build's user guide (17-sessions.md) and its source
// (github.com/xai-org/grok-build, crates/codegen/xai-grok-shell/src/session:
// persistence.rs's Summary, storage's SessionUpdateEnvelope), as Grok Build
// 1.0 writes them. None of them carries a version, so the newest schema
// reads them (docs/design/grok-build.md).
import { vendor, open, maybe, num, str, z } from "../common.js";

// A time as Grok writes it: summary.json's are RFC 3339 strings, an
// update's envelope's are Unix seconds.
const time = z.union([str, num]);

// ---------- summary.json ----------

export const SessionSummary = vendor(
  "spacexai/grok-build/session-summary",
  1,
  open({
    info: open({ id: maybe(str), cwd: maybe(str) })
      .optional()
      .describe("The session's id and the folder it works in"),
    session_summary: maybe(str),
    generated_title: maybe(str).describe("The title Grok made, or yours after /rename"),
    title_is_manual: maybe(z.boolean()).describe("True once you renamed it (/rename)"),
    created_at: maybe(time),
    updated_at: maybe(time),
    last_active_at: maybe(time),
    current_model_id: maybe(str),
    parent_session_id: maybe(str).describe("The session it was forked or restored from, or a subagent's parent"),
    session_kind: maybe(str).describe("worktree, fork, headless (grok -p), subagent, subagent_fork…; none for an interactive session"),
    hidden: maybe(z.boolean()).describe("Left out of Grok's own lists; a subagent's session is, unless this says"),
    source_workspace_dir: maybe(str).describe("Where a worktree session's work came from"),
    head_branch: maybe(str),
    last_turn_summary: maybe(str),
  }),
  {
    title: "A Grok Build session's summary",
    description:
      "Grok Build's ~/.grok/sessions/<encoded cwd>/<session id>/summary.json, the session's index entry: its id and folder, title, model, times, the session it came from, and what kind of session it is. The file carries no version, so the newest schema reads it.",
    versions: "*",
  },
);

// ---------- usage.json ----------

// What a stretch of work used, as Grok Build's usage ledger sums it
// (session/usage_file.rs's UsageSummary): inputTokens is the whole prompt,
// cache reads and writes included; the cost is in ticks, 1e10 to the
// dollar, and isn't there when no call said what it cost.
const tokens = {
  inputTokens: maybe(num).describe("The prompts' tokens, cache reads and writes included"),
  outputTokens: maybe(num).describe("The replies' tokens, reasoning included"),
  cachedReadTokens: maybe(num),
  cacheCreationTokens: maybe(num),
  reasoningTokens: maybe(num),
  totalTokens: maybe(num).describe("inputTokens and outputTokens"),
  modelCalls: maybe(num),
  costUsdTicks: maybe(num).describe("What it cost, in ticks of 1e10 to the dollar; not there when no call said (a SuperGrok or X Premium sign-in)"),
  costIsPartial: maybe(z.boolean()).describe("Some calls said what they cost, and some didn't"),
  usageIsIncomplete: maybe(z.boolean()).describe("Some of it may not be counted, such as a subagent's"),
};
const summary = open({ ...tokens, turnCount: maybe(num), primaryModelId: maybe(str), modelUsage: maybe(z.record(str, open(tokens))).describe("The same, by model") });

export const Usage = vendor(
  "spacexai/grok-build/usage",
  1,
  open({
    sessionId: maybe(str),
    updatedAt: maybe(str),
    session: summary.optional().describe("The whole session's, history it was resumed or forked from included"),
    turns: maybe(z.array(open({ turnNumber: maybe(num), endedAt: maybe(str).describe("When the turn ended, RFC 3339"), turnId: maybe(str), ...summary.shape }))).describe("Each turn's, when it ended"),
  }),
  {
    title: "A Grok Build session's usage",
    description:
      "Grok Build's ~/.grok/sessions/<encoded cwd>/<session id>/usage.json, the session's token and cost ledger, written at the end of each turn: the session's totals and each turn's, each by model. A fork starts with a copy of its parent's. The file carries no version, so the newest schema reads it.",
    versions: "*",
  },
);

// ---------- a line of updates.jsonl ----------

// A content block, as ACP has them: text, or an image or a resource.
const block = open({ type: str.optional(), text: str.optional() });

// An ACP session update, or one of Grok's own (the envelope's method says
// which). Its kind is `sessionUpdate`; XenoNet reads the chunks of what you
// and the agent said, tool calls and their updates, and the context's fill,
// and passes over the rest.
const update = open({
  sessionUpdate: str.describe("user_message_chunk, agent_message_chunk, tool_call, tool_call_update, usage_update…"),
  content: z.union([block, z.array(z.unknown())]).optional().describe("A chunk's content block, or a tool call's content"),
  toolCallId: maybe(str),
  title: maybe(str),
  kind: maybe(str).describe("A tool call's kind, as ACP has them: read, edit, delete, move, search, execute, think, fetch or other"),
  status: maybe(str).describe("A tool call's: pending, in_progress, completed or failed"),
  rawInput: z.unknown().optional(),
  locations: maybe(z.array(open({ path: str }))),
  used: maybe(num).describe("A usage update's tokens in the context"),
  size: maybe(num).describe("A usage update's context window"),
});

export const UpdateLine = vendor(
  "spacexai/grok-build/update-line",
  1,
  open({
    timestamp: maybe(time).describe("When it was written, in Unix seconds"),
    method: maybe(str).describe("session/update for ACP's updates, _x.ai/session/update for Grok's own"),
    params: open({ sessionId: maybe(str), update: update.optional() }).optional(),
    // A line from before the envelope: the params themselves.
    sessionId: maybe(str),
    update: update.optional(),
  }),
  {
    title: "A line of a Grok Build session's updates.jsonl",
    description:
      "One line of Grok Build's updates.jsonl: a session update (ACP's session/update notification's params, or one of Grok's own), in an envelope that says when it was written; older lines are the params alone. The live tailer, stats and search read it through core/lines.js's normalizer. The lines carry no version, so the newest schema reads them.",
    versions: "*",
  },
);
