#!/usr/bin/env node
// A Cursor hook that tells XenoNet's experimental Cursor reader
// (core/native/cursor.js) about Cursor's agent sessions: which conversation
// ran where, which files it edited, and when it ended. Cursor runs it for
// sessionStart, afterFileEdit and sessionEnd (docs/guide/related-work.md),
// with the hook's input on stdin; it appends one line to the file named on
// its command line, and answers {}.
//
// From each input it keeps only the fields named in KEEP, under Cursor's own
// names: never the prompt, the edits' text, a file's content, the model or
// your email. It needs Node 18 or later and nothing else, and never fails
// the agent: anything that goes wrong is passed over.
import fs from "node:fs";
import path from "node:path";

const EVENTS = new Set(["sessionStart", "afterFileEdit", "sessionEnd", "stop"]);
const KEEP = ["hook_event_name", "conversation_id", "session_id", "cursor_version", "workspace_roots", "file_path", "reason", "status"];
const LINE_MAX = 16 << 10;

async function main() {
  const out = process.argv[2];
  let text = "";
  for await (const chunk of process.stdin) {
    text += chunk;
    if (text.length > 4 << 20) break;
  }
  const input = JSON.parse(text);
  if (!out || !input || !EVENTS.has(input.hook_event_name)) return;
  const line = { at: new Date().toISOString() };
  for (const k of KEEP) if (input[k] !== undefined) line[k] = input[k];
  if (Array.isArray(line.workspace_roots)) line.workspace_roots = line.workspace_roots.filter((r) => typeof r === "string").slice(0, 8);
  const s = JSON.stringify(line);
  if (s.length > LINE_MAX) return;
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.appendFileSync(out, s + "\n", { mode: 0o600 });
}

main()
  .catch(() => {})
  .finally(() => process.stdout.write("{}\n"));
