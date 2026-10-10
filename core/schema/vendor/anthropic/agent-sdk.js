// What the Claude Agent SDK's session functions return, as core/claude.js
// reads it: listSessions() for the map's Claude sessions, and
// getSessionMessages() for a session's transcript in the console. Both are
// the SDK's public types (sdk.d.ts), which name every field.
import { vendor, open, closed, variants, skipped, num, str, z } from "../common.js";

export const SessionListing = vendor(
  "anthropic/agent-sdk/session-listing",
  1,
  closed({
    sessionId: str,
    summary: str.describe("A title: the custom one, a summary, or the first prompt"),
    lastModified: num.describe("Epoch milliseconds"),
    fileSize: num.optional(),
    customTitle: str.optional(),
    firstPrompt: str.optional(),
    gitBranch: str.optional(),
    cwd: str.optional(),
    tag: str.optional(),
    createdAt: num.optional().describe("Epoch milliseconds"),
  }),
  {
    title: "A session as the Agent SDK lists it",
    description: "One of the sessions listSessions() returns (SDKSessionInfo). The version is the installed SDK's own (its package.json).",
    versions: ">=0.3.0 <0.4.0",
  },
);

// A message's content blocks, as the console shows them: the Messages
// API's, whose own types (the Anthropic SDK's) give every key, the ones
// XenoNet doesn't read in `known`, a reply's and what you sent alike.
const block = variants("type", [
  closed({ type: z.literal("text"), text: str }, { known: ["citations", "cache_control"] }),
  closed({ type: z.literal("thinking"), thinking: str.optional() }, { known: ["signature"] }),
  closed({ type: z.literal("tool_use"), name: str, input: z.unknown() }, { known: ["id", "caller", "toolset_name"] }),
  closed({ type: z.literal("tool_result"), is_error: z.boolean().optional(), content: z.unknown() }, { known: ["tool_use_id", "cache_control", "toolset_name"] }),
  ...skipped("type", "image", "redacted_thinking", "document"),
]);

const message = (type) =>
  closed(
    {
      type: z.literal(type),
      uuid: str,
      timestamp: str.optional().describe("ISO 8601; the SDK sends it, though its types don't declare it"),
      message: open({ content: z.union([str, z.array(block)]).optional() }),
    },
    { known: ["session_id", "parent_tool_use_id", "parent_agent_id"] },
  );

export const SessionMessage = vendor("anthropic/agent-sdk/session-message", 1, variants("type", [message("user"), message("assistant"), message("system")]), {
  title: "A message as the Agent SDK reads it from a transcript",
  description: "One of the messages getSessionMessages() returns (SessionMessage), as the console's transcript view reads it. The version is the installed SDK's own.",
  versions: ">=0.3.0 <0.4.0",
});

