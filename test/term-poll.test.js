// How Desktop reads its terminals' output from the service (electron/term-
// poll.js): quickly while a shell prints or is typed into, backing off
// while all are quiet, and not at all with none open.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createTermPoll, FAST_MS, SLOW_MS } from "../electron/term-poll.js";

// A poller on made-up timers: `tick()` runs the timer that's due and
// waits for its reads; `waits` are the delays it asked for, in order.
function harness({ outputs = {}, ready = () => true } = {}) {
  const timers = [];
  const waits = [];
  const reads = [];
  const delivered = [];
  const later = (fn, ms) => {
    waits.push(ms);
    const t = { fn, ms };
    timers.push(t);
    return t;
  };
  const cancel = (t) => {
    const i = timers.indexOf(t);
    if (i >= 0) timers.splice(i, 1);
  };
  const read = async (id, cursor) => {
    reads.push([id, cursor]);
    const next = outputs[id]?.shift();
    if (next instanceof Error) throw next;
    return { cursor: cursor + (next?.data?.length ?? 0), data: next?.data ?? "", exited: !!next?.exited, exitCode: next?.exitCode ?? null };
  };
  const p = createTermPoll({ read, ready, later, cancel, deliver: (id, got, opts) => delivered.push([id, got.data, got.exited, opts.replay]) });
  const tick = async () => {
    const t = timers.shift();
    assert.ok(t, "a read is scheduled");
    await t.fn();
  };
  return { p, timers, waits, reads, delivered, tick };
}

test("no terminal open: nothing is asked", () => {
  const h = harness();
  assert.equal(h.timers.length, 0);
  assert.equal(h.p.count(), 0);
});

test("quiet terminals are asked less and less often, down to every 250 ms", async () => {
  const h = harness();
  h.p.watch("t1");
  assert.deepEqual(h.waits, [FAST_MS], "at once after it's watched");
  for (let i = 0; i < 6; i++) await h.tick();
  assert.deepEqual(h.waits, [32, 64, 128, 250, 250, 250, 250]);
  assert.equal(h.waits.at(-1), SLOW_MS);
  assert.equal(h.reads.length, 6);
});

test("output brings it back to every 32 ms, and so does typing", async () => {
  const h = harness({ outputs: { t1: [null, null, null, { data: "hello" }, null] } });
  h.p.watch("t1", true);
  for (let i = 0; i < 4; i++) await h.tick();
  assert.deepEqual(h.waits, [32, 64, 128, 250, 32], "fast again once it printed");
  assert.deepEqual(h.delivered.find((d) => d[1] === "hello"), ["t1", "hello", false, false]);
  assert.equal(h.delivered[0][3], true, "the first read replays what was there");
  await h.tick();
  assert.equal(h.waits.at(-1), 64);
  h.p.soon();
  assert.equal(h.timers.length, 1, "the slow wait is replaced");
  assert.equal(h.waits.at(-1), FAST_MS);
});

test("each terminal's own cursor; an exited one is told and let go, one that fails is let go", async () => {
  const h = harness({ outputs: { a: [{ data: "abc" }, { data: "", exited: true, exitCode: 0 }], b: [new Error("gone")] } });
  h.p.watch("a");
  h.p.watch("b");
  await h.tick();
  assert.deepEqual(h.reads, [["a", 0], ["b", 0]]);
  assert.equal(h.p.count(), 1, "b is let go");
  await h.tick();
  assert.deepEqual(h.reads.at(-1), ["a", 3], "from where it got to");
  assert.deepEqual(h.delivered.at(-1), ["a", "", true, false]);
  assert.equal(h.p.count(), 0);
  assert.equal(h.timers.length, 0, "nothing left to ask about");
});

test("with no window to pass output to, nothing is read, so nothing is lost", async () => {
  let ready = false;
  const h = harness({ outputs: { t1: [{ data: "kept" }] }, ready: () => ready });
  h.p.watch("t1");
  await h.tick();
  assert.equal(h.reads.length, 0);
  ready = true;
  await h.tick();
  assert.deepEqual(h.reads, [["t1", 0]]);
  assert.equal(h.delivered[0][1], "kept");
});
