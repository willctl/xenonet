// Gemini CLI's data, as XenoNet's experimental Gemini CLI reader reads it
// (core/native/gemini-cli.js): the lines of its chat records,
// ~/.gemini/tmp/<project>/chats/session-*.jsonl. Only the fields XenoNet
// reads are declared: never a message's content, thoughts, a tool's result,
// or its arguments other than the file a write_file or replace call names.
//
// From Gemini CLI's own source (github.com/google-gemini/gemini-cli,
// packages/core/src/services/chatRecordingService.ts and
// chatRecordingTypes.ts), as Gemini CLI 0.64 writes them. The records carry
// no version, so the newest schema reads them.
import { vendor, open, maybe, str, z } from "../common.js";

const toolCall = open({
  id: str,
  name: str.describe("write_file and replace are edits"),
  args: open({ file_path: str.optional() }).optional(),
  status: str.optional().describe("success, error, cancelled…"),
  timestamp: str.optional(),
});

export const ChatRecordLine = vendor(
  "google/gemini-cli/chat-record",
  1,
  open({
    // The first line: the session's own metadata.
    sessionId: str.optional(),
    projectHash: str.optional(),
    startTime: str.optional(),
    lastUpdated: str.optional(),
    kind: maybe(str).optional().describe("main or subagent"),
    // A message.
    id: str.optional(),
    timestamp: str.optional(),
    type: str.optional().describe("user, gemini, info, error, warning"),
    toolCalls: z.array(toolCall).optional(),
    // Updates to the metadata, and rewinds.
    $set: open({ lastUpdated: str.optional(), summary: str.optional() }).optional(),
    $rewindTo: str.optional(),
  }),
  {
    title: "Gemini CLI's chat records",
    description:
      "The lines of Gemini CLI's chat records (~/.gemini/tmp/<project>/chats/session-*.jsonl, or GEMINI_CLI_HOME's): the session's metadata, its messages' ids, kinds and times, the file each write_file and replace call names, and the metadata updates. The project's folder is read from .project_root beside chats. The records carry no version.",
    versions: "*",
  },
);
