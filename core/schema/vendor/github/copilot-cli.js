// GitHub Copilot CLI's data, as XenoNet's experimental Copilot CLI reader
// reads it (core/native/copilot-cli.js): the lines of a session's event log,
// ~/.copilot/session-state/<session id>/events.jsonl. Only the fields
// XenoNet reads are declared: never a message, a tool's result, or a tool's
// arguments other than the file an edit tool names.
//
// From GitHub's docs (docs.github.com, Copilot CLI best practices: where
// sessions are stored) and the event schema the official Copilot SDK
// publishes (@github/copilot-sdk, generated/session-events.d.ts). Each log
// starts with session.start, which says the format's version.
import { vendor, open, str, num, z } from "../common.js";

export const EventLine = vendor(
  "github/copilot-cli/session-event",
  1,
  open({
    id: str,
    timestamp: str,
    parentId: z.union([str, z.null()]).optional(),
    type: str.describe("session.start, session.context_changed, session.title_changed, session.shutdown and tool.execution_start are read; the rest (over a hundred kinds) are passed over"),
    data: open({
      sessionId: str.optional().describe("session.start"),
      version: num.optional().describe("session.start: the format's version"),
      startTime: str.optional(),
      context: open({ cwd: str, branch: str.optional() }).optional().describe("session.start and session.resume: the working folder"),
      cwd: str.optional().describe("session.context_changed"),
      title: str.optional().describe("session.title_changed"),
      codeChanges: open({ filesModified: z.array(str) }).optional().describe("session.shutdown"),
      toolName: str.optional().describe("tool.execution_start"),
      arguments: z.unknown().optional().describe("tool.execution_start: only an edit tool's path is read"),
    }).optional(),
  }),
  {
    title: "Copilot CLI's session events",
    description:
      "The lines of Copilot CLI's session event logs (~/.copilot/session-state/<id>/events.jsonl, or <id>.jsonl before 1.0.11): session.start (id, version, start, working folder and branch), resume and context changes, title changes, shutdown (the files it changed), and tool.execution_start for the file an edit tool names. Other kinds of event are passed over. The version is session.start's own (version).",
    versions: "1",
  },
);
