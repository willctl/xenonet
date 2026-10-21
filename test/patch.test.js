// The one reading of an apply_patch text (core/patch.js), and what each of
// its readers keeps of it: live, the engine and the native readers all keep
// the names files move to.
import { test } from "node:test";
import assert from "node:assert/strict";
import { patchHeaders, patchPaths } from "../core/patch.js";
import { LiveTelemetry } from "../core/live.js";
import { editPaths } from "../core/engine/engine.js";
import { patchFiles as opencodeFiles } from "../core/native/opencode.js";
import { editedBy } from "../core/native/copilot-cli.js";

const PATCH = "*** Begin Patch\n*** Add File: a.txt\n+x\n*** Update File: b.txt\r\n*** Move to:  c.txt \n@@\n-old\n+new\n*** Delete File: d.txt\n*** Add File: \n*** End Patch";

test("an apply_patch text's headers, in order, each with what it does", () => {
  assert.deepEqual(patchHeaders(PATCH), [
    { kind: "add", path: "a.txt" },
    { kind: "update", path: "b.txt" },
    { kind: "move", path: "c.txt" },
    { kind: "delete", path: "d.txt" },
  ]);
  assert.deepEqual(patchPaths(PATCH), ["a.txt", "b.txt", "c.txt", "d.txt"]);
  assert.deepEqual(patchPaths(PATCH, { moves: false }), ["a.txt", "b.txt", "d.txt"]);
});

test("a header is a whole line, ended as JS ends one, and its path is trimmed", () => {
  assert.deepEqual(patchPaths("x\r*** Add File: a b *** Delete File: c"), ["a", "c"]);
  assert.deepEqual(patchPaths(" *** Add File: a\nx*** Update File: b\n*** Rename File: c"), []);
  assert.deepEqual(patchPaths("*** Add File:  "), [""]);
  assert.deepEqual(patchPaths("*** Add File:  a﻿"), ["a"]);
  assert.deepEqual(patchPaths(""), []);
});

test("each reader keeps its own record of a patch", () => {
  // The engine warns about every file named, moves too, under the folder.
  assert.deepEqual(editPaths("apply_patch", { input: PATCH }, "/r"), ["/r/a.txt", "/r/b.txt", "/r/c.txt", "/r/d.txt"]);
  // OpenCode's reader leaves out a header with no path; Copilot CLI's keeps it.
  assert.deepEqual(opencodeFiles("*** Update File: b.txt\n*** Move to: c.txt\n*** Add File:  "), ["b.txt", "c.txt"]);
  assert.deepEqual(editedBy("apply_patch", "*** Update File: b.txt\n*** Move to: c.txt\n*** Add File:  "), ["b.txt", "c.txt", ""]);
  // Live counts the files it adds, updates and deletes, and where one moved,
  // as the engine warns about them.
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, claudeDesktopDir: null });
  const cx = { app: "codex", sessionId: "codex:bbbbbbbb-0000-0000-0000-000000000002" };
  const ts = new Date(Date.now() - 30e3).toISOString();
  live.applyCodex(cx, { type: "response_item", timestamp: ts, payload: { type: "function_call", call_id: "p1", name: "apply_patch", arguments: PATCH } }, false);
  assert.deepEqual([...live.sessions.get(cx.sessionId).files.keys()], ["a.txt", "b.txt", "c.txt", "d.txt"]);
});
