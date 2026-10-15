// The frames posters and films come in (core/frames.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import { POSTER_SIZES, posterSize, DEFAULT_SIZE, FILM_FRAMES, filmSeconds, filmPlaySeconds, fitFrame } from "../core/frames.js";

test("the four poster sizes, and the default for an unknown one", () => {
  assert.deepEqual(POSTER_SIZES.map((s) => `${s.width}x${s.height}`), ["1080x1920", "1080x1350", "1200x630", "1920x1080"]);
  assert.equal(posterSize("story").height, 1920);
  assert.equal(posterSize("nope").id, DEFAULT_SIZE);
  assert.equal(posterSize(undefined).id, DEFAULT_SIZE);
});

test("a film is 15 to 30 seconds, cards included", () => {
  assert.equal(filmSeconds(undefined), 20);
  assert.equal(filmSeconds(5), 15);
  assert.equal(filmSeconds(90), 30);
  assert.equal(filmSeconds("25"), 25);
  assert.equal(filmPlaySeconds(20), 12, "8 seconds of cards");
  assert.equal(filmPlaySeconds(15), 7);
});

test("a frame fits the screen it records on, keeping its shape", () => {
  assert.deepEqual(fitFrame(FILM_FRAMES.story, { width: 3024, height: 1964 }), { width: 1080, height: 1920 }, "a 14-inch Retina display has room");
  assert.deepEqual(fitFrame(FILM_FRAMES.wide, { width: 1440, height: 900 }), { width: 1440, height: 810 });
  // A 1080p screen under its menu bar: the frame shrinks to fit, even on
  // both sides, in its own shape.
  for (const area of [{ width: 1920, height: 1055 }, { width: 700, height: 1000 }]) {
    const { width, height } = fitFrame(FILM_FRAMES.story, area);
    assert.ok(width <= area.width && height <= area.height, `${width}x${height} fits ${area.width}x${area.height}`);
    assert.ok(width % 2 === 0 && height % 2 === 0);
    assert.ok(Math.abs(width / height - 9 / 16) < 0.01);
    assert.ok(height >= area.height - 2, "as tall as the screen allows");
  }
});
