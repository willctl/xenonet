// Framing the map in the part of the screen the panels and the alert strip
// leave free (src/framing.js, for fit in scene.js): plain math on made-up
// points and screens.
import { test } from "node:test";
import assert from "node:assert/strict";
import { freeTangents, frameFree } from "../src/framing.js";

const close = (a, b, what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} ≠ ${b}`);

// Where each point lands, as tangents from the screen's middle, seen from
// the camera frameFree chose; and whether all of it is inside the free part.
function inside(points, free, { d, a, b }) {
  const eps = 1e-9;
  return points.every((p) => {
    const z = d - p.z, r = p.r ?? 0;
    return z > 0 && (p.x - a + r) / z <= free.right + eps && (p.x - a - r) / z >= -free.left - eps && (p.y - b + r) / z <= free.top + eps && (p.y - b - r) / z >= -free.bottom - eps;
  });
}

test("the free part of the screen: what the panels, the strip and a margin leave, as tangents from the middle", () => {
  const fov = 50, tan = Math.tan((fov * Math.PI) / 360);
  const whole = freeTangents({ width: 1600, height: 900, fov, margin: 0 });
  close(whole.top, tan, "a whole screen's top edge");
  close(whole.bottom, tan, "and bottom");
  close(whole.right, (tan * 1600) / 900, "and sides, by its aspect");
  // Docks of 332 and 412 px, the strip down to 250 px and the hints' 56.
  const free = freeTangents({ width: 1600, height: 900, fov, insets: { left: 332, right: 412, top: 250, bottom: 56 } });
  const px = (t) => (t / tan) * 450;
  close(800 - px(free.left), 332 + 24, "the left edge, just right of the left dock");
  close(800 + px(free.right), 1600 - 412 - 24, "the right edge, just left of the right dock");
  close(450 - px(free.top), 250 + 24, "the top edge, just under the strip");
  close(450 + px(free.bottom), 900 - 56 - 24, "the bottom edge");
  // A screen too narrow for the docks frames across all of it.
  const narrow = freeTangents({ width: 900, height: 900, fov, insets: { left: 332, right: 412 } });
  close(narrow.left, narrow.right, "no room between the docks: centered");
});

test("framing: the nearest camera that sees every point inside the free part, looking where the free part's middle is", () => {
  const tan = 0.5;
  // Symmetric points on a symmetric screen: straight at their middle.
  const square = [{ x: -10, y: -10, z: 0 }, { x: 10, y: 10, z: 0 }];
  const f = frameFree(square, { left: tan, right: tan, top: tan, bottom: tan });
  close(f.d, 20, "as near as the points allow");
  close(f.a, 0, "centered across");
  close(f.b, 0, "and up and down");
  // A free part right of the middle and below it: the camera looks left of
  // the points and above them, so they land in it.
  const free = { left: 0.2, right: 0.6, top: 0.2, bottom: 0.5 };
  const g = frameFree(square, free);
  assert.ok(g.a < 0 && g.b > 0, `looks left and up: ${JSON.stringify(g)}`);
  assert.ok(inside(square, free, g), "every point inside");
  assert.ok(!inside(square, free, { ...g, d: g.d * 0.97 }), "and no nearer would do");
  // Depth and glow count: a point near the camera, and points that reach.
  const deep = [{ x: 0, y: 30, z: 40, r: 6 }, { x: -50, y: -5, z: -30, r: 3 }, { x: 25, y: 0, z: 10, r: 14 }, { x: 0, y: -20, z: 0 }];
  const h = frameFree(deep, free, { min: 10 });
  assert.ok(inside(deep, free, h), `every point inside: ${JSON.stringify(h)}`);
  assert.ok(!inside(deep, free, { ...h, d: h.d * 0.97 }), "and no nearer would do");
  // Never nearer than asked, and nothing at all to frame is nothing.
  assert.equal(frameFree([{ x: 0, y: 0, z: 0 }], free, { min: 80 }).d, 80);
  assert.equal(frameFree([], free), null);
});
