// The gatherer (core/blocks/gatherer.js): snapshots from a source when
// asked and when due, one at a time, timed, with what it serves after a
// failure.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Gatherer } from "../core/blocks/gatherer.js";
import { fakeClock } from "./harness/clock.js";

// A source that answers from a script: a value, or an Error to throw.
function source(...answers) {
  const calls = [];
  const fn = async (ctx) => {
    calls.push(ctx.now);
    const a = answers.length > 1 ? answers.shift() : answers[0];
    if (a instanceof Error) throw a;
    return typeof a === "function" ? a() : a;
  };
  fn.calls = calls;
  return fn;
}

test("on demand: every run gathers, timed, and says how it went", async () => {
  const clock = fakeClock();
  const gather = source(["s1", "s2"]);
  const g = new Gatherer({ name: "claude-sessions", gather, clock });
  assert.equal(g.state, "idle");
  assert.deepEqual(await g.run(), ["s1", "s2"]);
  await g.run();
  assert.equal(gather.calls.length, 2, "no cadence: always due");
  await g.run({ now: clock.now() - 60e3 });
  assert.equal(gather.calls.length, 3, "even when the clock steps back");
  assert.deepEqual([g.state, g.last.ok, g.last.error, g.counters.runs], ["running", true, null, 3]);
  assert.equal(g.last.at, clock.now() - 60e3);
});

test("a failure serves the fallback and fails, or with keep, the last good value and degrades", async () => {
  const plain = new Gatherer({ name: "codex-threads", gather: source(["t1"], new Error("app-server exited (2)"), ["t2"]), fallback: [] });
  await plain.run();
  assert.deepEqual(await plain.run(), [], "nothing kept: the fallback");
  assert.deepEqual([plain.state, plain.lastError.message, plain.last.ok, plain.last.error], ["failed", "app-server exited (2)", false, "app-server exited (2)"]);
  assert.deepEqual(await plain.run(), ["t2"]);
  assert.equal(plain.state, "running");

  const kept = new Gatherer({ name: "desktop-usage", gather: source([1], new Error("bad JSON")), keep: true });
  assert.deepEqual(await kept.run(), [1]);
  assert.deepEqual(await kept.run(), [1], "the last good value");
  assert.equal(kept.state, "degraded");
  const never = new Gatherer({ name: "never", gather: source(new Error("no")), keep: true, fallback: "nothing" });
  assert.equal(await never.run(), "nothing", "nothing good to keep yet");
  assert.equal(never.state, "failed");
  const sync = new Gatherer({ name: "throws-at-once", gather: () => { throw new Error("at once"); } });
  assert.equal(await sync.run(), null);
  assert.equal(sync.lastError.message, "at once");
});

test("a cadence: due again everyMs after a run, or when the source says, or after a growing backoff", async () => {
  const clock = fakeClock();
  const t0 = clock.now();
  const usage = source({ pct: 10 }, { reason: "Anthropic asked to wait (429)", retryAfterMs: 20 * 60e3 }, { pct: 12 });
  const g = new Gatherer({ name: "claude-usage", gather: usage, everyMs: 5 * 60e3, nextIn: (r) => r.retryAfterMs, clock });
  await g.run();
  assert.equal(await g.due(t0 + 60e3), false);
  assert.deepEqual(await g.run({ now: t0 + 60e3 }), { pct: 10 }, "not due: what it has");
  assert.deepEqual([usage.calls.length, g.counters.skipped], [1, 1]);
  await g.run({ now: t0 + 5 * 60e3 });
  assert.equal(g.nextAt, t0 + 25 * 60e3, "the source asked for 20 minutes");
  await g.run({ now: t0 + 15 * 60e3 });
  assert.equal(usage.calls.length, 2);
  await g.run({ now: t0 + 25 * 60e3 });
  assert.deepEqual([usage.calls.length, g.value], [3, { pct: 12 }]);
  await g.run({ force: true, now: t0 + 26 * 60e3 });
  assert.equal(usage.calls.length, 4, "forced");

  const flaky = source(new Error("1"), new Error("2"), new Error("3"), new Error("4"), "ok", new Error("5"));
  const b = new Gatherer({ name: "flaky", gather: flaky, backoff: { baseMs: 1000, maxMs: 3000 }, clock });
  const waits = [];
  for (let i = 0; i < 6; i++) {
    await b.run({ force: true });
    waits.push(b.nextAt - clock.now());
  }
  assert.deepEqual(waits, [1000, 2000, 3000, 3000, 0, 1000], "doubling to the most, and back to the start after a success");
});

test("a stamp: gathered again only when it changes, failures included", () => {
  let mtime = 1;
  let reads = 0;
  let text = '{"a":1}';
  const g = new Gatherer({ name: "claude-account", gather: () => (reads++, JSON.parse(text)), stamp: () => mtime, fallback: null });
  assert.deepEqual(g.get(), { a: 1 });
  assert.deepEqual(g.get(), { a: 1 });
  assert.equal(reads, 1);
  mtime = 2;
  text = "{ not json";
  assert.equal(g.get(), null);
  assert.equal(g.get(), null);
  assert.equal(reads, 2, "a broken file isn't read again until it changes");
  assert.equal(g.state, "failed");
  mtime = 3;
  text = '{"a":2}';
  assert.deepEqual(g.get(), { a: 2 });
  assert.equal(g.state, "running");
  const wrong = new Gatherer({ name: "async", gather: async () => 1 });
  assert.throws(() => wrong.get(), /asynchronous source: use run\(\)/);

  // With retry, a failure is tried again on the next run, stamp or not;
  // the last good value stays, with keep.
  let fails = 0;
  const repo = new Gatherer({ name: "git-identity", gather: () => (fails-- > 0 ? (() => { throw new Error("git config failed"); })() : { email: "a@example.com" }), stamp: () => "same", retry: true, keep: true });
  assert.deepEqual(repo.get(), { email: "a@example.com" });
  fails = 1;
  repo.forget();
  assert.equal(repo.get(), null, "forgotten: read again, and it failed with nothing kept");
  assert.deepEqual(repo.get(), { email: "a@example.com" }, "tried again though the stamp is the same");
  assert.equal(repo.get(), repo.value, "and then not read again");
  assert.equal(repo.counters.runs, 3);
});

test("runs at once share one gather", async () => {
  let resolve;
  let calls = 0;
  const g = new Gatherer({ name: "slow", gather: () => (calls++, new Promise((r) => (resolve = r))) });
  const a = g.run();
  const b = g.run({ force: true });
  assert.equal(a, b);
  resolve("v");
  assert.deepEqual([await a, await b, calls], ["v", "v", 1]);
});

test("started, it gathers on its cadence until stopped", async () => {
  const clock = fakeClock();
  const gather = source(1);
  const g = new Gatherer({ name: "git-status", gather, everyMs: 20e3, clock });
  await g.start();
  await clock.flush();
  assert.equal(gather.calls.length, 1, "once at start");
  await clock.advance(19e3);
  assert.equal(gather.calls.length, 1);
  await clock.advance(1e3);
  assert.equal(gather.calls.length, 2);
  await clock.advance(40e3);
  assert.equal(gather.calls.length, 4);
  g.stop();
  await clock.advance(60e3);
  assert.equal(gather.calls.length, 4, "stopped");
  assert.equal(g.state, "stopped");

  const later = new Gatherer({ name: "later", gather: source(1), everyMs: 10e3, clock });
  await later.start({ immediately: false });
  await clock.advance(9e3);
  assert.equal(later.counters.runs ?? 0, 0);
  await clock.advance(1e3);
  assert.equal(later.counters.runs, 1);
  later.stop();
});

test("stopped and started again while it gathers, it keeps one cadence, which a stop ends", async () => {
  const clock = fakeClock();
  let gathers = 0;
  // A gather that takes 60 ms, on a 100 ms cadence: at 0, 100, 200…
  const gather = () => (gathers++, new Promise((resolve) => clock.setTimeout(resolve, 60, gathers)));
  const g = new Gatherer({ name: "git-status", gather, everyMs: 100, clock });
  await g.start();
  await clock.advance(30);
  g.stop();
  await g.start();
  await clock.advance(30);
  assert.equal(gathers, 1, "the start shares the gather under way");
  await clock.advance(90);
  assert.equal(gathers, 2, "one cadence, not two");
  // Stopped between gathers, with the next one's timer set.
  await clock.advance(130);
  assert.equal(gathers, 3);
  g.stop();
  await clock.advance(1000);
  assert.deepEqual([gathers, g.state, clock.pending()], [3, "stopped", 0], "nothing after the stop");
});
