// Cursor's data, as XenoNet's experimental Cursor reader reads it
// (core/native/cursor.js): the fields of Cursor's hook input that XenoNet's
// hook (scripts/cursor/hook.mjs) keeps, under Cursor's own names, one line
// per sessionStart, afterFileEdit, stop and sessionEnd.
//
// From Cursor's hooks reference (cursor.com/docs/agent/hooks): the common
// input every hook gets, and each of those hooks' own fields. Never the
// prompt, an edit's text, a file's content, the model or your email.
import { vendor, open, str, z } from "../common.js";

export const HookInput = vendor(
  "anysphere/cursor/hook-input",
  1,
  open({
    at: str.describe("When XenoNet's hook ran, ISO 8601"),
    hook_event_name: str.describe("sessionStart, afterFileEdit, stop or sessionEnd"),
    conversation_id: str.optional().describe("The conversation, stable across its turns"),
    session_id: str.optional().describe("sessionStart and sessionEnd: the same as conversation_id"),
    cursor_version: str.optional(),
    workspace_roots: z.array(str).optional(),
    file_path: str.optional().describe("afterFileEdit: the file, an absolute path"),
    reason: str.optional().describe("sessionEnd: completed, aborted, error, window_close or user_close"),
    status: str.optional().describe("stop: completed, aborted or error"),
  }),
  {
    title: "Cursor's hook input",
    description:
      "The fields of Cursor's hook input (cursor.com/docs/agent/hooks) that XenoNet's Cursor hook keeps, one line per sessionStart, afterFileEdit, stop and sessionEnd, in the file it's told to write. The version is Cursor's own (cursor_version).",
    versions: "*",
  },
);
