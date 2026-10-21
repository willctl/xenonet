// Which experimental native reader (core/native/reader.js) reads which app:
// a row for each app the registry (core/apps.js) gives `native`, keyed by
// its id, with the reader's name as a source of the map, what it reads in
// words (for its line in the map's sources), and how it's made from
// XenoNet's settings (core/config.js). The rows are in the order the state
// lists the readers. The Rust has the same table in wire-map's observed.rs.
import { APPS } from "../apps.js";
import { OpenCodeSessions, opencodeDbPath } from "./opencode.js";
import { GeminiCliSessions, geminiRoot } from "./gemini-cli.js";
import { CopilotCliSessions, copilotRoot } from "./copilot-cli.js";
import { CursorSessions, cursorEventsFile } from "./cursor.js";

export const NATIVE_READERS = [
  { app: "opencode", source: "opencode", what: "OpenCode's database", create: ({ homeDir }) => new OpenCodeSessions({ file: opencodeDbPath({ home: homeDir }) }) },
  { app: "gemini-cli", source: "gemini-cli", what: "Gemini CLI's folder", create: ({ homeDir }) => new GeminiCliSessions({ root: geminiRoot({ home: homeDir }) }) },
  { app: "copilot", source: "copilot", what: "Copilot CLI's folder", create: ({ homeDir }) => new CopilotCliSessions({ root: copilotRoot({ home: homeDir }) }) },
  { app: "cursor", source: "cursor", what: "the Cursor hook's file", create: ({ stateDir }) => new CursorSessions({ file: cursorEventsFile({ stateDir }) }) },
];

// The readers that are on, made now: for each app the registry gives
// `native` that XENONET_READERS names (config.readers), its row with its
// `reader`, in the table's order. `apps` and `table` are a test's own.
export function nativeReaders(config, { apps = APPS, table = NATIVE_READERS } = {}) {
  const native = new Set(apps.filter((a) => a.has?.includes("native")).map((a) => a.id));
  return table.filter((row) => native.has(row.app) && config.readers?.has(row.app)).map((row) => ({ ...row, reader: row.create(config) }));
}
