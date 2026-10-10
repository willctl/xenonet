// Where a sound on the map is heard from the camera (src/sound-space.js):
// from its side, louder when nearer, duller and further back in the room
// when far, muffled from behind, near even when you look at the whole map,
// and never silent.
import { test } from "node:test";
import assert from "node:assert/strict";
import { hear, path, placement, MODES } from "../src/sound-space.js";

// Looking down -z from z = 100, orbiting a point at the origin.
const ear = (focus = 1, orbit = 100) => ({ pos: [0, 0, 100], right: [1, 0, 0], up: [0, 1, 0], fwd: [0, 0, -1], orbit, focus });

test("from its side: ahead is the middle, right is right, left is left", () => {
  assert.equal(hear(ear(), [0, 0, 0]).pan, 0);
  assert.ok(hear(ear(), [60, 0, 0]).pan > 0.3);
  assert.ok(hear(ear(), [-60, 0, 0]).pan < -0.3);
  assert.deepEqual(hear(ear(), [0, 0, 0]).dir.map((v) => Math.round(v * 100) / 100), [0, 0, -1], "ahead is -z in the camera's own space");
  assert.equal(hear(ear(), [0, 0, 0]).behind, false);
  const behind = hear(ear(), [0, 0, 300]);
  assert.equal(behind.behind, true);
  assert.ok(behind.cutoff <= 3000, "muffled from behind");
});

test("nearer is louder, far is quieter, duller and wetter, but never silent", () => {
  const near = hear(ear(), [0, 0, 60]);
  const mid = hear(ear(), [0, 0, 0]);
  const far = hear(ear(), [0, 0, -900]);
  assert.ok(near.gain > mid.gain && mid.gain > far.gain, `${near.gain} > ${mid.gain} > ${far.gain}`);
  assert.equal(mid.gain, 1, "at the orbit, as loud as can be");
  assert.ok(near.gain <= 1.25);
  assert.ok(far.gain >= 0.15, "the floor");
  assert.equal(hear(ear(), [0, 0, -100000], { floor: 0.5 }).gain, 0.5, "a floor of its own, for alerts");
  assert.ok(far.cutoff < mid.cutoff && far.wet > mid.wet);
  assert.ok(hear(ear(), [0, 0, 0], { boost: 1.3 }).gain > 1, "a boost for what you've selected");
});

test("looking at the whole map, the mix stays near even; flown in close, distance tells", () => {
  // At the overview the camera is far out, orbiting the map's middle; a
  // node across the map is about as far as one at its middle.
  const overview = { ...ear(0, 600), pos: [0, 0, 600] };
  const middle = hear(overview, [0, 0, 0]).gain;
  const across = hear(overview, [0, 0, -300]).gain;
  assert.ok(across > 0.7 && middle - across < 0.3, `near even: ${middle}, ${across}`);
  // Flown in to a session, one across the map falls back.
  const close = hear(ear(1, 100), [0, 0, -300]).gain;
  assert.ok(close < 0.5, `${close}`);
  assert.ok(hear(ear(0, 100), [0, 0, -300]).gain > close, "focus is what makes distance tell");
});

test("a sound that travels is heard from its start, its middle and its end, and each mode places it its own way", () => {
  const [a, , c] = path(ear(), [-80, 0, 0], [80, 0, 0]);
  assert.ok(a.pan < 0 && c.pan > 0, "it moves from left to right");
  assert.deepEqual(MODES, ["flat", "speakers", "headphones"]);
  const h = hear(ear(), [60, 0, 0]);
  assert.deepEqual(placement("flat", h, { pan: -0.3 }), { gain: 1, pan: -0.3, wet: null, cutoff: null, dir: null }, "flat: as before");
  assert.equal(placement("speakers", h).dir, null);
  assert.deepEqual(placement("headphones", h).dir, h.dir, "headphones: in 3D");
  assert.equal(placement("speakers", null, { pan: 0.2 }).pan, 0.2, "nowhere known: flat");
  for (const v of [...Object.values(hear(ear(), [1, 2, 3])).filter((x) => typeof x === "number"), ...hear(ear(), [0, 0, 100]).dir]) assert.ok(Number.isFinite(v), "at the ear itself, still numbers");
});
