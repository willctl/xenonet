// A turn you stopped, as the page says it (src/live-words.js): stopped in
// the live status and the active sessions, a stopped turn in the feed, and
// no chime for it. Sessions and events are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { stoppedWords, turnEndWords, chimes } from "../src/live-words.js";

const T = 1791230000000;
const MIN = 60e3;

test("an idle session whose last turn you stopped says when, until it's at work again", () => {
  assert.equal(stoppedWords({ status: "idle", interruptedAt: T - 2 * MIN }, T), "stopped 2m ago");
  assert.equal(stoppedWords({ status: "idle", interruptedAt: T - 10e3 }, T), "stopped just now");
  assert.equal(stoppedWords({ status: "idle", interruptedAt: null }, T), null, "a turn that ended by itself");
  assert.equal(stoppedWords({ status: "idle" }, T), null, "a host from before interruptedAt");
  for (const status of ["thinking", "tool"]) assert.equal(stoppedWords({ status, interruptedAt: T - MIN }, T), null, `${status}: what it's doing instead`);
  // What isn't a time, as a hostile host might send, says nothing.
  for (const at of ["2m ago", 0, -1, Infinity]) assert.equal(stoppedWords({ status: "idle", interruptedAt: at }, T), null, String(at));
  assert.equal(stoppedWords(null, T), null);
});

test("a turn-end in the feed: finished, or stopped for one you stopped, with how long it ran", () => {
  assert.equal(turnEndWords({ kind: "turn-end", duration: 42000 }), "turn finished in 42s");
  assert.equal(turnEndWords({ kind: "turn-end" }), "turn finished");
  assert.equal(turnEndWords({ kind: "turn-end", duration: 12400, interrupted: true }), "turn stopped after 12s");
  assert.equal(turnEndWords({ kind: "turn-end", interrupted: true }), "turn stopped");
  assert.equal(turnEndWords({ kind: "turn-end", duration: "long" }), "turn finished", "a duration that isn't a number says nothing");
});

test("a turn that ends by itself chimes, and so does a subagent starting; a turn you stopped doesn't", () => {
  assert.equal(chimes({ kind: "turn-end", duration: 42000 }), true);
  assert.equal(chimes({ kind: "subagent-start" }), true);
  assert.equal(chimes({ kind: "turn-end", interrupted: true }), false);
  for (const kind of ["tool", "prompt", "turn-start", "needs-you"]) assert.equal(chimes({ kind }), false, kind);
  assert.equal(chimes(null), false);
});
