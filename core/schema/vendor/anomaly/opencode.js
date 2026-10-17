// OpenCode's data, as XenoNet's experimental OpenCode reader reads it
// (core/native/opencode.js): the session rows and the edit tools' file paths
// in OpenCode's database, opencode.db. Only those fields are declared, since
// they are the only ones read: never a prompt, a reply, a tool's output, a
// file's content, a share URL or a credential.
//
// From OpenCode's own source (github.com/anomalyco/opencode, its SQLite
// tables session and part) and checked against a database OpenCode 1.19.3
// wrote. Each session row says the OpenCode version that made it.
import { vendor, open, maybe, num, str, z } from "../common.js";

const session = open({
  id: str.describe("ses_…"),
  parent_id: maybe(str).describe("Set for a subagent's run: the session that started it"),
  directory: str.describe("Where it ran"),
  title: maybe(str),
  version: str.describe("The OpenCode version that made it"),
  time_created: num.describe("Epoch milliseconds"),
  time_updated: num.describe("Epoch milliseconds"),
  time_archived: maybe(num),
});

export const Db = vendor(
  "anomaly/opencode/db",
  1,
  open({
    session: z.array(session).describe("select id, parent_id, directory, title, version, time_created, time_updated, time_archived from session"),
    edits: z
      .array(open({ session_id: str, time_created: num, tool: str, path: maybe(str) }))
      .describe("The file each edit, write and multiedit tool part names (data.state.input.filePath), and each apply_patch part's file headers, taken from part.data; nothing else in a part is kept"),
  }),
  {
    title: "OpenCode's database",
    description:
      "The rows XenoNet reads from OpenCode's opencode.db ($XDG_DATA_HOME/opencode, ~/.local/share/opencode by default, or OPENCODE_DB), opened read only: its sessions and which files their edit tools changed. The version is the newest session's own (session.version).",
    versions: "<2.0.0",
  },
);
