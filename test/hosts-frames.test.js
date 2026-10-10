// Reading the relay's frames (core/hosts/frames.js): one JSON object per
// line, however the bytes arrive, with what a host mustn't send dropped.
import { test } from "node:test";
import assert from "node:assert/strict";
import { lineSplitter, parseFrame, encode, shallow, FROM_RELAY, FROM_CONSOLE, MAX_FRAME, MAX_DEPTH } from "../core/hosts/frames.js";

function reader(max) {
  const lines = [];
  let dropped = 0;
  const push = lineSplitter({ max, onLine: (l) => lines.push(l), onDrop: () => dropped++ });
  return { push, lines, dropped: () => dropped };
}

test("lines come out whole however the chunks fall, characters split between chunks too", () => {
  const r = reader();
  const text = encode({ t: "event", name: "live", data: { title: "Café ☕ résumé" } }) + encode({ t: "ping", now: 1 });
  const bytes = Buffer.from(text);
  // One byte at a time splits every multi-byte character.
  for (let i = 0; i < bytes.length; i++) r.push(bytes.subarray(i, i + 1));
  assert.deepEqual(r.lines.map((l) => JSON.parse(l)), [{ t: "event", name: "live", data: { title: "Café ☕ résumé" } }, { t: "ping", now: 1 }]);

  const r2 = reader();
  r2.push('{"t":"ping","now":1}\r\n\n\n{"t":"pi');
  r2.push('ng","now":2}\n{"t":"ping"');
  assert.deepEqual(r2.lines, ['{"t":"ping","now":1}', '{"t":"ping","now":2}'], "carriage returns and empty lines go; a partial line waits");
});

test("a frame over the size limit is dropped as it arrives, and the next one still reads", () => {
  const r = reader();
  const big = `{"t":"snapshot","graph":"${"x".repeat(20 << 20)}"}`;
  assert.ok(big.length > MAX_FRAME);
  const bytes = Buffer.from(`${big}\n${encode({ t: "ping", now: 3 })}`);
  for (let i = 0; i < bytes.length; i += 1 << 16) r.push(bytes.subarray(i, i + (1 << 16)));
  assert.equal(r.dropped(), 1);
  assert.deepEqual(r.lines, ['{"t":"ping","now":3}']);

  const small = reader(16);
  small.push("0123456789abcdef\n0123456789abcdefX\nok\n");
  assert.deepEqual([small.lines, small.dropped()], [["0123456789abcdef", "ok"], 1], "exactly the limit passes, one byte over doesn't");
});

test("what isn't a frame, or isn't one this side acts on, is refused", () => {
  assert.deepEqual(parseFrame('{"t":"ping","now":1}', FROM_RELAY), { frame: { t: "ping", now: 1 } });
  for (const line of ["not json", "[1,2]", "null", '"text"', '{"no":"t"}', '{"t":7}']) assert.ok(parseFrame(line).error, line);
  assert.match(parseFrame('{"t":"exec","cmd":"rm -rf ~"}', FROM_RELAY).error, /doesn't know \(exec\)/);
  assert.match(parseFrame('{"t":"post","path":"/api/open"}', FROM_CONSOLE).error, /doesn't know/);
  assert.match(parseFrame('{"t":"hello"}', FROM_CONSOLE).error, /doesn't know/, "a console doesn't say hello");
  assert.equal(parseFrame(`{"t":"${"x".repeat(500)}"}`, FROM_RELAY).error.length < 100, true, "an odd kind isn't echoed whole");
});

test("a frame nested deeper than anything XenoNet sends is caught before it can overflow the stack", () => {
  const deep = JSON.parse(`{"t":"event","name":"live","data":${"[".repeat(200000)}${"]".repeat(200000)}}`);
  assert.throws(() => JSON.stringify(deep), RangeError, "what it would do to the console");
  assert.equal(shallow(deep), false);
  const nest = (n) => (n ? { x: [nest(n - 1)] } : 1);
  assert.equal(shallow(nest(MAX_DEPTH / 2 - 1)), true);
  assert.equal(shallow(nest(MAX_DEPTH / 2 + 1)), false);
  assert.equal(shallow({ t: "snapshot", graph: { nodes: [{ id: "core", identities: [{ detail: { plan: "Max" } }] }] } }), true);
});
