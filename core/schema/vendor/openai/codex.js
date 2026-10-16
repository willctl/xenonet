// Codex's data, as XenoNet reads it: a rollout's lines (~/.codex/sessions/**,
// which the live tailer, stats and search read through core/lines.js), the
// codex app-server's answers (core/codex.js), the state database's
// projects, and the claims in auth.json's ID token (core/identities.js).
import { vendor, open, closed, variants, skipped, maybe, num, str, z } from "../common.js";

// ---------- a rollout's line ----------

// Token counts, as token_count events carry them, for the whole thread and
// for the last call. Cached and written input are part of input_tokens.
const tokens = closed({
  input_tokens: num.optional(),
  cached_input_tokens: num.optional(),
  cache_write_input_tokens: num.optional(),
  output_tokens: num.optional(),
  reasoning_output_tokens: num.optional(),
  total_tokens: num,
});

// A rate limit window. resets_at is in epoch seconds; XenoNet's API sends it
// as milliseconds (live/snapshot's limits.codex).
const window = closed(
  {
    used_percent: num,
    window_minutes: maybe(num),
    resets_at: maybe(num).describe("Epoch seconds"),
  },
  { known: ["resets_in_seconds"] },
);

const text = open({ text: str.optional() });

// The closed objects below list every key Codex's protocol gives them (the
// types codex app-server generate-ts writes), the ones XenoNet doesn't read
// in `known`.

// A file's change in a FileChange item: added or deleted (its content), or
// updated (its diff, and where it moved).
const change = closed({ type: str.optional(), unified_diff: str.optional(), content: str.optional() }, { known: ["move_path"] });

// What a web search did: searched (its query, or queries), opened a page or
// looked in one.
const searchAction = closed({ query: str.optional() }, { known: ["type", "queries", "url", "pattern"] });

// What a finished item says, as the live tailer reads it (event_msg's
// item_completed), and a thread shown from its rollout.
const item = variants("type", [
  open({ type: z.literal("UserMessage"), id: str.optional(), content: z.array(text) }),
  open({ type: z.literal("AgentMessage"), id: str.optional(), content: z.array(text) }),
  open({ type: z.literal("Reasoning"), id: str.optional(), summary_text: z.array(z.unknown()).optional(), raw_content: z.array(z.unknown()).optional() }),
  open({ type: z.literal("CommandExecution"), id: str.optional(), command: z.union([z.array(str), str]), exit_code: maybe(num), status: str.optional(), aggregated_output: maybe(str) }),
  open({ type: z.literal("McpToolCall"), id: str.optional(), server: str, tool: str, status: str.optional(), error: z.unknown().optional(), arguments: z.unknown().optional(), result: z.unknown().optional() }),
  open({ type: z.literal("FileChange"), id: str.optional(), changes: z.record(str, change).describe("By path"), status: str.optional() }),
  open({ type: z.literal("WebSearch"), id: str.optional(), query: str.optional(), action: searchAction.optional() }),
  open({ type: z.literal("ImageView"), id: str.optional(), path: str }),
  open({ type: z.literal("ContextCompaction"), id: str.optional() }),
  ...["CollabAgentToolCall", "SubAgentActivity"].map((type) =>
    open({
      type: z.literal(type),
      id: str.optional(),
      kind: str.optional().describe("What happened to a subagent: started, interacted, interrupted or completed"),
      agent_thread_id: str.optional(),
      receiver_thread_ids: z.array(str).optional(),
      agent_path: str.optional(),
      prompt: str.optional(),
      agentThreadId: str.optional().describe("Read as well as agent_thread_id"),
      receiverThreadIds: z.array(str).optional(),
      agentPath: str.optional(),
    }).describe("A subagent: the thread it runs in, its path or prompt for a label, and what happened to it"),
  ),
]);

const event = variants("type", [
  open({ type: z.literal("task_started"), model_context_window: maybe(num) }),
  open({ type: z.literal("task_complete"), last_agent_message: maybe(str), duration_ms: maybe(num) }),
  open({
    type: z.literal("token_count"),
    info: closed({ total_token_usage: tokens.optional(), last_token_usage: tokens.optional(), model_context_window: maybe(num) }).nullable(),
    rate_limits: open({ primary: maybe(window), secondary: maybe(window) }).nullable().optional(),
  }),
  open({ type: z.literal("item_completed"), item, started_at_ms: maybe(num), completed_at_ms: maybe(num) }),
  open({ type: z.literal("user_message"), message: str }),
  open({ type: z.literal("agent_message"), message: str }),
  open({
    type: z.literal("thread_settings_applied"),
    thread_settings: open({ service_tier: maybe(str).describe("priority, or default; not said is default") }).describe("All of the thread's settings, as of the turn after it"),
  }),
  open({
    type: z.literal("turn_aborted"),
    reason: maybe(str).describe("interrupted (you stopped it), replaced (the next turn took its place) or review_ended; older rollouts leave it out"),
  }).describe("A turn that ended without finishing: one you stopped (interrupted, or no reason) ends there, and the live view says so"),
  // Kinds XenoNet passes over; newer Codex writes the last.
  ...skipped("type", "agent_reasoning", "mcp_tool_call_end"),
]);

// A tool call and its output, as response_item lines have them. Every
// response item can carry the passthrough key.
const PASSTHROUGH = "internal_chat_message_metadata_passthrough";
const callShape = (type) => ({ type: z.literal(type), call_id: str.optional(), id: str.optional(), name: str.optional(), input: z.unknown().optional(), arguments: z.unknown().optional(), action: z.unknown().optional() });
const outputShape = (type) => ({ type: z.literal(type), call_id: str.optional(), id: str.optional(), output: z.unknown().optional() });
const call = (type, known) => closed(callShape(type), { known: [...known, PASSTHROUGH] });
const output = (type, known) => closed(outputShape(type), { known: [...known, PASSTHROUGH] });

const response = variants("type", [
  open({ type: z.literal("message"), role: str, content: z.union([z.array(text), str]) }),
  call("function_call", ["namespace", "encrypted_function_args", "encrypted_content", "status"]),
  call("custom_tool_call", ["status", "namespace", "encrypted_content"]),
  call("local_shell_call", ["status", "encrypted_content"]),
  output("function_call_output", ["name", "namespace"]),
  output("custom_tool_call_output", ["name"]),
  // Not in the protocol's types any more; older rollouts may have it.
  open(outputShape("local_shell_call_output")),
  // Kinds XenoNet passes over: newer Codex writes the last nine.
  ...skipped("type", "reasoning", "web_search_call", "agent_message", "tool_search_call", "tool_search_output", "image_generation_call", "compaction", "configuration_update", "compaction_trigger", "context_compaction", "additional_tools"),
]);

// Newer Codex numbers each line (ordinal) and tags response items with
// metadata; XenoNet reads neither.
const envelope = (type, payload) => closed({ timestamp: str.describe("ISO 8601"), type: z.literal(type), payload }, { known: ["ordinal", "metadata"] });

export const RolloutLine = vendor(
  "openai/codex/rollout-line",
  1,
  variants("type", [
    envelope("session_meta", open({ cli_version: str.describe("The Codex that wrote the rollout"), model_provider: str.optional() })),
    envelope("turn_context", open({ model: str, effort: maybe(str), service_tier: maybe(str) })),
    envelope("event_msg", event),
    envelope("response_item", response),
    envelope("compacted", open({})),
    // Kinds of line newer Codex writes, which XenoNet passes over.
    ...skipped("type", "world_state", "token_usage_record"),
  ]),
  {
    title: "A line of a Codex rollout",
    description:
      "A line of ~/.codex/sessions/<yyyy>/<mm>/<dd>/rollout-<time>-<thread>.jsonl (and of archived, zstd-compressed ones), as the live tailer, a session's stats and search read it, and a thread shown from its rollout when the app-server can't list its items. The version is the rollout's first line's, session_meta's cli_version.",
    versions: "<1.0.0",
  },
);

// ---------- the app-server ----------

const when = z.union([num, str]).nullable().optional().describe("Epoch seconds or milliseconds, or ISO 8601");

const thread = open({
  id: str,
  name: maybe(str),
  preview: maybe(str),
  cwd: maybe(str),
  createdAt: when,
  updatedAt: when,
  recencyAt: when,
  status: z.union([str, open({ type: str })]).optional(),
  model: maybe(str),
  modelProvider: maybe(str),
  reasoningEffort: maybe(str),
  gitInfo: maybe(closed({ branch: maybe(str), originUrl: maybe(str), origin_url: maybe(str) }, { known: ["sha"] })),
  source: z
    .union([
      str,
      open({
        subAgent: z.union([str, open({ thread_spawn: open({ parent_thread_id: maybe(str), agent_nickname: maybe(str) }).optional(), other: str.optional() })]).optional(),
      }),
    ])
    .optional(),
  originator: maybe(str),
  projectId: maybe(str),
  parentThreadId: maybe(str),
  forkedFromId: maybe(str),
  agentNickname: maybe(str),
});

const threadItem = variants("type", [
  open({ type: z.literal("userMessage"), content: z.array(open({ type: str.optional(), text: str.optional() })).optional() }),
  open({ type: z.literal("agentMessage"), text: maybe(str) }),
  open({ type: z.literal("reasoning"), summary: z.array(z.unknown()).optional(), content: z.array(z.unknown()).optional() }),
  open({ type: z.literal("plan"), text: maybe(str) }),
  open({ type: z.literal("commandExecution"), command: z.unknown().optional(), status: maybe(str), exitCode: maybe(num), aggregatedOutput: maybe(str) }),
  open({ type: z.literal("mcpToolCall"), server: str, tool: str, status: maybe(str), error: z.unknown().optional(), result: z.unknown().optional(), arguments: z.unknown().optional() }),
  open({ type: z.literal("dynamicToolCall"), tool: str, status: maybe(str), arguments: z.unknown().optional() }),
  open({ type: z.literal("fileChange"), changes: z.array(closed({ path: str, kind: z.union([str, closed({ type: str }, { known: ["move_path"] })]).optional() }, { known: ["diff"] })).optional(), status: maybe(str) }),
  open({ type: z.literal("webSearch"), query: maybe(str) }),
  open({ type: z.literal("contextCompaction") }),
  // Kinds newer Codex lists, which XenoNet passes over.
  ...skipped("type", "hookPrompt", "functionCallOutput", "collabAgentToolCall", "subAgentActivity", "imageView", "sleep", "imageGeneration", "enteredReviewMode", "exitedReviewMode"),
]);

export const AppServer = vendor(
  "openai/codex/app-server",
  1,
  open({
    initialize: open({ userAgent: str.describe("<client>/<codex version> (…)") }).optional(),
    "thread/list": open({ data: z.array(thread), nextCursor: maybe(str) }).optional(),
    "thread/items/list": open({
      data: z.array(open({ item: threadItem, startedAtMs: maybe(num), completedAtMs: maybe(num) })),
      nextCursor: maybe(str),
    }).optional(),
  }),
  {
    title: "The codex app-server's answers",
    description:
      "The results of the codex app-server methods XenoNet calls over stdio JSON-RPC, by method: initialize (its userAgent), thread/list (the map's Codex threads) and thread/items/list (a thread in the console). XenoNet sends initialize with its name and version, and thread/list with archived, limit, cursor, sourceKinds and useStateDbOnly. Renaming a thread in XenoNet sends thread/name/set with threadId and name, whose answer ({}) isn't read. The version is Codex's, from the userAgent.",
    versions: "<1.0.0",
  },
);

// ---------- the state database ----------

export const StateDb = vendor(
  "openai/codex/state-db",
  1,
  open({
    projects: z.array(open({ id: str, name: str })).describe("select id, name from projects"),
    project_roots: z.array(open({ project_id: str, path: str })).describe("select project_id, path from project_roots order by position"),
  }),
  {
    title: "Codex's state database",
    description: "The tables and columns XenoNet queries in ~/.codex/state_5.sqlite, opened read only, for Codex desktop's projects. The version is the file's number (state_5); a state database of another number is drift.",
    versions: "5",
  },
);

// ---------- auth.json's ID token ----------

export const AuthClaims = vendor(
  "openai/codex/auth-claims",
  1,
  open({
    email: str.optional(),
    auth_time: num.optional().describe("When you signed in, in epoch seconds"),
    "https://api.openai.com/auth": open({ chatgpt_account_id: str.optional(), chatgpt_plan_type: str.optional() }).optional(),
  }),
  {
    title: "The claims XenoNet reads from Codex's ID token",
    description:
      "The claims in the ID token of ~/.codex/auth.json, decoded without verifying: who is signed in, the workspace, the plan and when. XenoNet reads auth_mode, whether there's an API key, and the token's claims, and never keeps a token. The claims carry no version, so the newest schema reads them.",
    versions: "*",
  },
);
