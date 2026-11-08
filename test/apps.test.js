// The app registry (src/shared/apps.js): it is what the session ids and the
// Rust's copy say, and its fallbacks and refusals are the ones the code it
// feeds reads.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APPS, APP, TAILED, ASKED, appsWith, tailedApp, tailedOr, askedOr, appRefusal, untailed, hookWrittenBy } from "../src/shared/apps.js";
import { SESSION_APPS, appOfSession } from "../src/shared/ids.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => fs.readFileSync(path.join(root, rel), "utf8");

test("the registry is what session ids say, in their order", () => {
  assert.deepEqual(Object.entries(SESSION_APPS), APPS.map((a) => [a.id, a.name]));
  assert.equal(new Set(APPS.map((a) => a.id)).size, APPS.length, "each id once");
  for (const a of APPS) assert.equal(appOfSession(`${a.id}:0123456789`), a.id);
  assert.deepEqual(Object.keys(APP), APPS.map((a) => a.id));
});

test("the Rust's copy of the registry is current: node scripts/apps.mjs writes it", () => {
  execFileSync(process.execPath, ["scripts/apps.mjs", "--check"], { cwd: root, stdio: "pipe" });
});

test("where an app is asked for, one XenoNet doesn't tail is refused, and none given is the default", () => {
  for (const [given, fallback, read] of [
    [undefined, "codex", "codex"],
    [null, "codex", "codex"],
    ["", "claude", "claude"],
    ["claude", "codex", "claude"],
    ["codex", null, "codex"],
    ["grok", "codex", "grok"],
    ["gemini-cli", "codex", null],
    ["Codex", "codex", null],
    ["toString", "codex", null],
    ["__proto__", "codex", null],
    [5, "codex", null],
    [{}, "codex", null],
    [undefined, null, null],
  ]) assert.equal(tailedOr(given, fallback), read, String(given));
  for (const a of APPS) assert.equal(tailedOr(a.id, "codex"), TAILED.includes(a.id) ? a.id : null, a.id);
  assert.equal(appRefusal("--app", "gemini-cli"), '--app takes claude, codex or grok, not "gemini-cli"');
  assert.equal(appRefusal("app", 5), "app takes claude, codex or grok, not 5");
  assert.equal(untailed("opencode"), "XenoNet doesn't read OpenCode's transcripts");
  assert.equal(untailed("nobody"), 'XenoNet doesn\'t read "nobody"\'s transcripts');
  // Asking and handing off take Claude Code and Codex alone: Grok Build is
  // tailed, but XenoNet doesn't ask it or hand a session to it.
  for (const [given, fallback, read] of [
    [undefined, "codex", "codex"],
    ["", "claude", "claude"],
    ["claude", "codex", "claude"],
    ["codex", "claude", "codex"],
    ["grok", "codex", null],
    ["toString", "codex", null],
    [5, "claude", null],
  ]) assert.equal(askedOr(given, fallback), read, String(given));
  assert.equal(appRefusal("target", "grok", ASKED), 'target takes claude or codex, not "grok"');
});

test("the fallbacks the live tailer and the normalizers keep: an app they don't tail reads as one they do", () => {
  const inputs = ["claude", "codex", ...APPS.map((a) => a.id), "Codex", "", "toString", "__proto__", undefined, null, 1, {}];
  for (const x of inputs) {
    const tailed = x === "claude" || x === "codex" || x === "grok";
    assert.equal(tailedApp(x, "claude"), tailed ? x : "claude", String(x));
    assert.equal(tailedApp(x, "codex"), tailed ? x : "codex", String(x));
  }
});

test("a hook's input says when Cursor or Grok Build wrote it, not Claude Code", () => {
  const sid = "0190d3e0-0000-7000-8000-0000000000aa";
  for (const [body, by] of [
    [{ session_id: sid, hook_event_name: "Stop", stop_hook_active: false }, null],
    [{ session_id: sid, hook_event_name: "SessionStart", cursor_version: "3.19.7", conversation_id: "c1" }, "cursor"],
    [{ conversation_id: "c1", hook_event_name: "sessionStart", cursor_version: "3.19.7" }, "cursor"],
    [{ sessionId: sid, hookEventName: "stop", hook_event_name: "Stop" }, "grok"],
    [{ sessionId: sid, hook_event_name: "Stop" }, "grok"],
    [{ session_id: sid, sessionId: sid, hook_event_name: "Stop" }, null],
    [{ session_id: sid, cursor_version: 3 }, null],
    [null, null],
    [{}, null],
  ]) assert.equal(hookWrittenBy(body), by, JSON.stringify(body));
});
