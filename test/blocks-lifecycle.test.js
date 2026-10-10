// What every building block has (core/blocks/lifecycle.js): start and stop,
// and a health report that says how it's doing and since when.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Block, Blocks } from "../core/blocks/lifecycle.js";
import { fakeClock } from "./harness/clock.js";

class Thing extends Block {
  constructor(opts) {
    super({ kind: "thing", ...opts });
    this.calls = [];
  }
  async onStart() {
    this.calls.push("start");
    if (this.refuse) throw new Error("no way in");
  }
  onStop() {
    this.calls.push("stop");
  }
}

test("a block starts idle, runs once started, and says when each state began", async () => {
  const clock = fakeClock();
  const t0 = clock.now();
  const b = new Thing({ name: "a", clock });
  assert.deepEqual(b.health(), { state: "idle", since: t0, lastOk: null, lastError: null, counters: { errors: 0 }, budget: null });
  await clock.advance(1000);
  const states = [];
  b.on("state", (s) => states.push(`${s.from}→${s.to}`));
  await b.start();
  await b.start();
  assert.deepEqual(b.calls, ["start"], "starting twice starts once");
  assert.deepEqual([b.health().state, b.health().since], ["running", t0 + 1000]);

  await clock.advance(1000);
  b.problem(new Error("a folder it couldn't read"));
  assert.deepEqual([b.state, b.lastError, b.counters.errors], ["degraded", { at: t0 + 2000, message: "a folder it couldn't read" }, 1]);
  b.ok();
  assert.deepEqual([b.state, b.lastOk], ["running", t0 + 2000]);
  assert.equal(b.lastError.message, "a folder it couldn't read", "the last error stays, as history");
  b.fail("gone");
  assert.equal(b.state, "failed");
  b.count("lines", 40);
  b.count("lines");
  assert.equal(b.health().counters.lines, 41);
  b.stop();
  assert.deepEqual(b.calls, ["start", "stop"]);
  b.ok();
  b.problem("after it stopped");
  assert.equal(b.state, "stopped", "a stopped block stays stopped");
  assert.deepEqual(states, ["idle→running", "running→degraded", "degraded→running", "running→failed", "failed→stopped"]);
});

test("a block used without a start counts as running from its first success; one that can't start has failed", async () => {
  const b = new Thing({ name: "on-demand" });
  b.ok();
  assert.equal(b.state, "running");
  const c = new Thing({ name: "refusing" });
  c.refuse = true;
  await assert.rejects(c.start(), /no way in/);
  assert.deepEqual([c.state, c.lastError.message], ["failed", "no way in"]);
  assert.throws(() => new Block({ name: "nameless" }), /a name and a kind/);
});

// A block whose start takes a while: it waits for the test to let it go
// on, then does what the test set (throws, or reports a problem).
class Slow extends Block {
  constructor(opts) {
    super({ kind: "thing", ...opts });
    this.gates = [];
    this.outcome = null;
  }
  onStart() {
    const outcome = this.outcome;
    return new Promise((resolve) => this.gates.push(resolve)).then(() => {
      if (outcome === "throw") throw new Error("couldn't start");
      if (outcome === "problem") this.problem(new Error("a folder it couldn't read"));
    });
  }
  go() {
    this.gates.shift()();
  }
}

test("a block stopped while it starts stays stopped, whatever its start does after", async () => {
  const b = new Slow({ name: "a" });
  const starting = b.start();
  b.stop();
  b.go();
  await starting;
  assert.deepEqual([b.state, b.started], ["stopped", false], "no flip back to running");

  const c = new Slow({ name: "b" });
  c.outcome = "throw";
  const failing = c.start();
  c.stop();
  c.go();
  await assert.rejects(failing, /couldn't start/);
  assert.deepEqual([c.state, c.lastError], ["stopped", null], "a start that was stopped isn't the block's failure");
});

test("a block started again after a stop: a start that throws fails it, one with a problem degrades it", async () => {
  const b = new Slow({ name: "a" });
  let starting = b.start();
  b.go();
  await starting;
  b.stop();
  b.outcome = "throw";
  starting = b.start();
  assert.equal(b.state, "idle", "idle again while it starts");
  b.go();
  await assert.rejects(starting, /couldn't start/);
  assert.deepEqual([b.state, b.lastError.message], ["failed", "couldn't start"]);

  b.stop();
  b.outcome = "problem";
  starting = b.start();
  b.go();
  await starting;
  assert.deepEqual([b.state, b.lastError.message], ["degraded", "a folder it couldn't read"], "not running");

  b.stop();
  b.outcome = null;
  starting = b.start();
  b.go();
  await starting;
  assert.equal(b.state, "running");
});

test("a start overtaken by a stop and another start doesn't speak for the new one", async () => {
  const b = new Slow({ name: "a" });
  b.outcome = "throw";
  const first = b.start();
  b.stop();
  b.outcome = null;
  const second = b.start();
  // The first start's end comes while the second is still starting.
  b.go();
  await assert.rejects(first, /couldn't start/);
  assert.deepEqual([b.state, b.lastError], ["idle", null], "neither failed nor running yet");
  b.go();
  await second;
  assert.equal(b.state, "running");
});

test("a list of blocks reports each one's health with its name and kind", () => {
  const blocks = new Blocks();
  const a = blocks.add(new Thing({ name: "a" }));
  blocks.add(new Thing({ name: "b" }));
  blocks.add(a);
  assert.throws(() => blocks.add(new Thing({ name: "a" })), /already a block called a/);
  a.budget = { folders: 10, last: { read: 3 } };
  const h = blocks.health();
  assert.deepEqual(h.map((x) => [x.name, x.kind, x.state]), [["a", "thing", "idle"], ["b", "thing", "idle"]]);
  assert.deepEqual(h[0].budget, { folders: 10, last: { read: 3 } });
  h[0].budget.last.read = 99;
  assert.equal(a.budget.last.read, 3, "a copy");
  assert.equal(blocks.get("b").name, "b");
  blocks.remove("b");
  assert.deepEqual([...blocks].map((x) => x.name), ["a"]);

  // A block that reads for one of the map's sources says which.
  blocks.add(new Thing({ name: "c" }), { source: "claude" });
  assert.deepEqual(blocks.health().map((x) => [x.name, x.source]), [["a", undefined], ["c", "claude"]]);
  assert.ok(!("source" in blocks.health()[0]), "and only one that does");
  blocks.remove("c");
  blocks.add(new Thing({ name: "c" }));
  assert.equal(blocks.health()[1].source, undefined, "forgotten with its block");
});
