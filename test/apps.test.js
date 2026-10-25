// The app registry (core/apps.js): each entry fits app/v1, it is what the
// session ids and the Rust's copy say, its products are the provider drift
// check's, and its values are the ones the code it feeds reads.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { APPS, APP, TAILED, ASKED, appsWith, tailedApp, tailedOr, askedOr, appRefusal, untailed, hookWrittenBy } from "../core/apps.js";
import { SESSION_APPS, appOfSession } from "../core/schema/ids.js";
import { check, explain } from "../core/schema/index.js";
import { CAPABILITIES } from "../core/schema/app.js";
import { LEFT_OUT, readRecord } from "../core/numbat.js";
import { PROVIDERS } from "../scripts/provider-drift/registry.mjs";
import { event } from "./fixtures/numbat.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => fs.readFileSync(path.join(root, rel), "utf8");

test("every entry fits app/v1, strictly", () => {
  for (const app of APPS) {
    const r = check("app/v1", JSON.parse(JSON.stringify(app)), { strict: true });
    assert.ok(r.ok, `${app.id} doesn't fit app/v1:\n${explain(r)}`);
  }
  for (const bad of [{ id: "x", name: "X" }, { id: "X", name: "X", numbat: [] }, { id: "x", name: "X", numbat: [], has: ["teleport"] }, { id: "x", name: "X", numbat: [], connect: { format: "claude-settings", events: [["A", "b", "c"]] } }]) {
    assert.equal(check("app/v1", bad).ok, false, JSON.stringify(bad));
  }
});

test("the registry is what session ids say, in their order", () => {
  assert.deepEqual(Object.entries(SESSION_APPS), APPS.map((a) => [a.id, a.name]));
  assert.equal(new Set(APPS.map((a) => a.id)).size, APPS.length, "each id once");
  for (const a of APPS) assert.equal(appOfSession(`${a.id}:0123456789`), a.id);
  assert.deepEqual(Object.keys(APP), APPS.map((a) => a.id));
});

test("the Rust's copy of the registry is current: node scripts/apps.mjs writes it", () => {
  execFileSync(process.execPath, ["scripts/apps.mjs", "--check"], { cwd: root, stdio: "pipe" });
});

test("every product an entry names is the provider drift check's, and every source the drift log names belongs to one", () => {
  const providers = new Set(PROVIDERS.map((p) => p.id));
  for (const a of APPS) for (const p of a.products ?? []) assert.ok(providers.has(p), `${a.id}'s ${p} is in scripts/provider-drift/registry.mjs`);
  const owners = (source) => PROVIDERS.filter((p) => p.sources.includes(source)).map((p) => p.id);
  const js = read("core/drift.js");
  const jsSources = [...js.slice(js.indexOf("const SOURCES = {"), js.indexOf("};", js.indexOf("const SOURCES = {"))).matchAll(/^\s+"([^"]+)": /gm)].map((m) => m[1]);
  const rs = read("crates/xenonet-schema/src/drift.rs");
  const rsSources = [...rs.slice(rs.indexOf("pub const SOURCES"), rs.indexOf("];", rs.indexOf("pub const SOURCES"))).matchAll(/^\s+\("([^"]+)", /gm)].map((m) => m[1]);
  assert.ok(jsSources.length > 20 && rsSources.length > 20, "the tables were found");
  assert.deepEqual(rsSources, jsSources, "drift.rs names the sources drift.js does");
  for (const s of jsSources) assert.deepEqual(owners(s).length, 1, `${s} belongs to one provider (${owners(s)})`);
  // A tailed or natively read app's sources are its own products'.
  for (const a of APPS.filter((a) => a.has?.includes("lines") || a.has?.includes("native"))) {
    const sources = PROVIDERS.filter((p) => a.products.includes(p.id)).flatMap((p) => p.sources);
    assert.ok(sources.length, `${a.id}'s products have sources`);
  }
});

test("numbat leaves out the apps XenoNet tails, and reads every other app under its own id", () => {
  for (const a of APPS) {
    if (a.has?.includes("lines")) for (const n of a.numbat) assert.ok(LEFT_OUT.has(n), `numbat leaves out ${n}`);
    else {
      assert.deepEqual(a.numbat, [a.id]);
      const r = readRecord(JSON.stringify(event({ source_agent: a.id, session_id: "s-0000001" })));
      assert.equal(r.event?.app, a.id, `numbat reads ${a.id}'s runs`);
    }
  }
});

test("what the code reads from the registry", async () => {
  assert.deepEqual(TAILED, ["claude", "codex", "grok"]);
  assert.deepEqual(appsWith("native").map((a) => a.id), ["gemini-cli", "cursor", "copilot", "opencode"]);
  for (const a of APPS) for (const c of a.has ?? []) assert.ok(CAPABILITIES.includes(c));
  const [{ EDIT_TOOLS: gemini }, { EDIT_TOOLS: copilot }, { EDIT_TOOLS: opencode }] = await Promise.all([import("../core/native/gemini-cli.js"), import("../core/native/copilot-cli.js"), import("../core/native/opencode.js")]);
  assert.deepEqual([...gemini], ["write_file", "replace"]);
  assert.deepEqual([...copilot], ["str_replace_editor", "edit", "create", "write", "apply_patch"]);
  assert.deepEqual(opencode, ["edit", "write", "multiedit"]);
  const { LiveTelemetry } = await import("../core/live.js");
  const live = new LiveTelemetry({ claudeRoot: path.join(root, "no-such-folder"), codexRoot: path.join(root, "no-such-folder"), claudeDesktopDir: null });
  assert.deepEqual(["claude", "codex", "grok", "opencode"].map((app, i) => live.session(`${app}:x${i}`, app).context.window), [200000, 258400, 256000, 200000], "a session's context window, until a line says");
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
