// OSC 52 in the terminal drawer (src/osc52.js): a program may set the
// clipboard, and asking what's on it is never answered, since the answer
// would be typed into the shell. Made-up clipboard text throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import xterm from "@xterm/xterm";
import { clipboardWrite, osc52Handler } from "../src/osc52.js";

const b64 = (text) => Buffer.from(text, "utf8").toString("base64");

test("clipboardWrite: what a sequence sets the clipboard to, and nothing for a question", () => {
  assert.equal(clipboardWrite(`c;${b64("git log --oneline")}`), "git log --oneline");
  assert.equal(clipboardWrite(`c;${b64("naïve ✓ 漢字")}`), "naïve ✓ 漢字", "UTF-8 text");
  assert.equal(clipboardWrite("c;"), "", "empty clears it");
  assert.equal(clipboardWrite("c;?"), null, "a question is never answered");
  assert.equal(clipboardWrite(`p;${b64("x")}`), null, "the primary selection isn't the clipboard");
  assert.equal(clipboardWrite(`s0;${b64("x")}`), null);
  assert.equal(clipboardWrite(`;${b64("x")}`), null);
  for (const bad of ["c;not base64!", "c;aGk", "c;aG=k", `c;${Buffer.from([0xff, 0xfe]).toString("base64")}`, "c", "", null, undefined]) assert.equal(clipboardWrite(bad), null, String(bad));
});

test("on xterm's own parser: a question types nothing into the shell, and a set reaches the clipboard", async () => {
  const term = new xterm.Terminal({ allowProposedApi: true });
  const typed = [];
  const set = [];
  term.onData((d) => typed.push(d));
  term.parser.registerOscHandler(52, osc52Handler((text) => set.push(text)));
  // A file that asks for the clipboard as it's printed, then one that sets it.
  await new Promise((resolve) => term.write(`notes.txt\r\n\x1b]52;c;?\x07more\r\n\x1b]52;c;${b64("echo made-up")}\x07`, resolve));
  assert.deepEqual(typed, [], "nothing goes to the shell as if typed");
  assert.deepEqual(set, ["echo made-up"]);
  term.dispose();
});
