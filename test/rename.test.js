import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanTitle, renamable, MAX_TITLE } from "../core/rename.js";

test("a session's new name is one line, without control characters, and not too long", () => {
  const lineSeparator = String.fromCharCode(0x2028);
  assert.equal(cleanTitle("  Partial refunds  "), "Partial refunds");
  assert.equal(cleanTitle(`Partial\nrefunds\r\n\tand${lineSeparator}credits`), "Partial refunds and credits");
  assert.equal(cleanTitle(`Bell${String.fromCharCode(7)} and${String.fromCharCode(0x9b)} escape`), "Bell and escape");
  assert.equal(cleanTitle("Émigré · 退款 🧾"), "Émigré · 退款 🧾", "letters and symbols stay");
  assert.equal(cleanTitle("x".repeat(500)).length, MAX_TITLE);
  assert.equal(cleanTitle(`${"x".repeat(MAX_TITLE - 1)} y`), "x".repeat(MAX_TITLE - 1), "no space left at the end");
  for (const nothing of ["", "   ", "\n\t", null, undefined, 42, {}]) assert.equal(cleanTitle(nothing), "");
});

test("only this machine's Claude Code and Codex sessions can be renamed", () => {
  assert.equal(renamable({ kind: "session", app: "claude" }), true);
  assert.equal(renamable({ kind: "session", app: "codex" }), true);
  assert.equal(renamable({ kind: "session", app: "claude", host: "studio" }), false, "another machine's are read-only");
  assert.equal(renamable({ kind: "session", app: "gemini" }), false);
  assert.equal(renamable({ kind: "project", app: "claude" }), false);
  assert.equal(renamable(null), false);
});
