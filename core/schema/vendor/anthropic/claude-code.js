// Claude Code's data, as XenoNet reads it: a transcript's lines
// (~/.claude/projects/**/*.jsonl, which the live tailer, stats and search
// read through core/lines.js) and the parts of ~/.claude.json the
// identities and connecting read.
import { vendor, open, closed, variants, skipped, maybe, num, str, z } from "../common.js";

// ---------- a transcript's line ----------

// What every line XenoNet reads carries. `version` is Claude Code's, on each
// line it writes, and picks the schema.
const line = {
  timestamp: str.describe("ISO 8601; the readers each take a time they can't read their own way"),
  isSidechain: z.boolean().optional().describe("A subagent's line"),
  isMeta: z.boolean().optional().describe("Claude Code's own note, not something you typed"),
  version: str.optional().describe("The Claude Code that wrote the line"),
};

// The Messages API's token counts, as Claude Code keeps them: the
// usage-based readers price each kind. The closed objects here list every
// key the API's own types give them (the Anthropic SDK's BetaUsage,
// BetaMessage and its blocks), the ones XenoNet doesn't read in `known`.
const usage = closed(
  {
    input_tokens: num,
    cache_creation_input_tokens: num.optional(),
    cache_read_input_tokens: num.optional(),
    cache_creation: closed({ ephemeral_5m_input_tokens: num.optional(), ephemeral_1h_input_tokens: num.optional() }).optional(),
    output_tokens: num,
    speed: maybe(str).describe('"fast" in fast mode'),
  },
  { known: ["service_tier", "server_tool_use", "inference_geo", "iterations", "output_tokens_details", "fallback_credit"] },
);

const assistantBlock = variants("type", [
  closed({ type: z.literal("text"), text: str }, { known: ["citations"] }),
  closed({ type: z.literal("tool_use"), id: str, name: str, input: z.unknown().describe("The tool's input, as the model wrote it") }, { known: ["caller", "toolset_name"] }),
  ...skipped("type", "thinking", "redacted_thinking", "server_tool_use", "web_search_tool_result"),
]);

const userBlock = variants("type", [
  closed({ type: z.literal("text"), text: str }, { known: ["cache_control", "citations"] }),
  closed({ type: z.literal("tool_result"), tool_use_id: str, is_error: z.boolean().optional() }, { known: ["content", "cache_control", "toolset_name"] }),
  ...skipped("type", "image", "document"),
]);

export const TranscriptLine = vendor(
  "anthropic/claude-code/transcript-line",
  1,
  variants("type", [
    open({
      type: z.literal("user"),
      ...line,
      isCompactSummary: z.boolean().optional().describe("The summary a compacted conversation goes on from"),
      message: closed({ content: z.union([str, z.array(userBlock)]) }, { known: ["role"] }),
    }).describe("Something you typed, a tool's result, or Claude Code's own note"),
    open({
      type: z.literal("assistant"),
      ...line,
      effort: str.optional(),
      message: closed(
        {
          id: str.describe("The message's id: Claude Code writes a line per content block, each with the message's usage"),
          model: str,
          content: z.array(assistantBlock),
          usage,
          stop_reason: z.string().nullable(),
        },
        { known: ["type", "role", "stop_sequence", "container", "context_management", "stop_details", "diagnostics", "input_transformations"] },
      ),
    }).describe("A reply, or a part of one"),
    // Kinds XenoNet passes over: Claude Code's notes and summaries, and what
    // newer versions write about a session around them (its titles and
    // mode, queued prompts, file history, links and costs).
    ...skipped(
      "type",
      "system",
      "summary",
      "queue-operation",
      "last-prompt",
      "attachment",
      "file-history-snapshot",
      "file-history-delta",
      "agent-name",
      "custom-title",
      "ai-title",
      "atis-latch",
      "cost-state",
      "frame-link",
      "mode",
      "pr-link",
      "relocated",
      "permission-mode",
    ),
  ]),
  {
    title: "A line of a Claude Code transcript",
    description:
      "A line of ~/.claude/projects/<project>/<session>.jsonl, or of a subagent's <session>/subagents/agent-<id>.jsonl, as the live tailer, a session's stats and search read it. The version is the line's own `version`, Claude Code's.",
    versions: ">=1.0.0 <3.0.0",
  },
);

// ---------- ~/.claude.json ----------

export const Config = vendor(
  "anthropic/claude-code/config",
  1,
  open({
    oauthAccount: open({
      accountUuid: str,
      organizationUuid: str,
      emailAddress: maybe(str),
      organizationType: maybe(str).describe("claude_max, claude_pro, claude_team or claude_enterprise"),
      organizationName: maybe(str).describe("Read for team and enterprise organizations only"),
    })
      .optional()
      .describe("The account Claude Code is signed in to; never its name fields"),
    mcpServers: z.record(str, open({ command: str.optional(), args: z.array(str).optional(), env: z.record(str, str).optional() })).optional().describe("XenoNet's own entry, which connecting writes and checks"),
  }),
  {
    title: "Claude Code's ~/.claude.json",
    description: "The parts of Claude Code's ~/.claude.json (or $CLAUDE_CONFIG_DIR/.claude.json) XenoNet reads: the signed-in account, and its own MCP server entry. The file carries no version, so the newest schema reads it.",
    versions: "*",
  },
);
