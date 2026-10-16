// How often the console draws (src/render-budget.js): the frame rate for
// each state the window can be in, which of the display's frames are
// drawn at that rate, and the page's CSS animations kept to those frames.
import { test } from "node:test";
import assert from "node:assert/strict";
import { frameRate, createPacer, createCssPacer, PACES, paceOf } from "../src/render-budget.js";

test("a recording gets every frame, whatever else holds: a film, the showcase and the X cuts record through it", () => {
  for (const pace of Object.keys(PACES))
    for (const seen of [true, false])
      for (const focused of [true, false]) assert.equal(frameRate({ pace, recording: true, seen, focused, moving: false }), Infinity, `${pace} seen=${seen} focused=${focused}`);
});

test("nothing is drawn while nobody can see the window, unless a script or agent is working it", () => {
  assert.equal(frameRate({ seen: false }), 0);
  assert.equal(frameRate({ seen: false, moving: true }), 0, "not even a layout that's still moving");
  assert.equal(frameRate({ seen: false, focused: false }), 0);
  assert.equal(frameRate({ seen: false, driven: true }), PACES.balanced.rest, "driven, it draws, so the view settles and screenshots show it");
  assert.equal(frameRate({ seen: false, driven: true, moving: true }), PACES.balanced.moving);
});

test("the balanced pace: 60 while something moves, 30 at rest, 20 at rest behind another app", () => {
  assert.equal(frameRate({ moving: true }), 60);
  assert.equal(frameRate({ moving: true, focused: false }), 60);
  assert.equal(frameRate({ focused: true }), 30);
  assert.equal(frameRate({ focused: false }), 20);
  assert.equal(frameRate({ focused: false, driven: true }), 30, "a window being driven counts as in use");
});

test("lower GPU use halves those; smooth draws every frame while it can be seen; an unknown pace is balanced", () => {
  assert.deepEqual([frameRate({ pace: "low", moving: true }), frameRate({ pace: "low" }), frameRate({ pace: "low", focused: false })], [30, 15, 10]);
  assert.equal(frameRate({ pace: "smooth" }), Infinity);
  assert.equal(frameRate({ pace: "smooth", seen: false }), 0, "smooth still draws nothing unseen");
  assert.equal(paceOf("nope"), "balanced");
  assert.equal(paceOf("__proto__"), "balanced");
  assert.equal(frameRate({ pace: "nope", focused: false }), 20);
});

// The frames a pacer draws out of `seconds` of a display at `hz`.
function drawn(hz, fps, seconds = 2) {
  const p = createPacer();
  let n = 0;
  for (let i = 0; i < hz * seconds; i++) {
    const now = 1000 + (i * 1000) / hz;
    if (p.due(now, fps)) (p.drew(now, fps), n++);
  }
  return n / seconds;
}

test("the pacer draws the rate asked of it on 60, 120 and 144 Hz displays", () => {
  assert.equal(drawn(120, 60), 60, "every other frame at 120 Hz");
  assert.equal(drawn(120, 30), 30);
  assert.equal(drawn(120, 20), 20);
  assert.equal(drawn(60, 60), 60, "every frame at 60 Hz");
  assert.equal(drawn(60, 30), 30);
  assert.equal(drawn(60, 20), 20);
  assert.equal(drawn(120, Infinity), 120);
  assert.equal(drawn(120, 0), 0);
  const at144 = drawn(144, 60);
  assert.ok(at144 >= 55 && at144 <= 60, `about 60 at 144 Hz: ${at144}`);
  assert.ok(drawn(144, 30) >= 28 && drawn(144, 30) <= 30);
});

test("the pacer says how long until the next frame, so the loop can sleep until then", () => {
  const p = createPacer();
  assert.equal(p.wait(0, 30), 0, "the first frame is due at once");
  p.drew(1000, 30);
  assert.ok(Math.abs(p.wait(1000, 30) - (1000 / 30 - 1.5)) < 1e-9);
  assert.ok(p.wait(1020, 30) > 0 && p.wait(1020, 30) < 15);
  assert.equal(p.wait(1100, 30), 0, "late is due now");
  assert.equal(p.wait(1000, 0), Infinity, "nothing to draw: no frame is due");
  assert.equal(p.wait(1000, Infinity), 0);
  // After a long pause (the window hidden), the next frame isn't owed a
  // run of catch-up frames.
  p.drew(5000, 30);
  assert.equal(p.due(5010, 30), false);
});

// A stand-in for document.getAnimations(): CSSAnimation and CSSTransition
// objects with the parts of the Web Animations API the pacer uses.
class Anim {
  constructor({ end = Infinity, target = null } = {}) {
    this.playState = "running";
    this.currentTime = 0;
    this.endTime = end;
    this.effect = { target, getComputedTiming: () => ({ endTime: this.endTime }) };
    this.calls = [];
  }
  pause() {
    this.playState = "paused";
    this.calls.push("pause");
  }
  play() {
    this.playState = "running";
    this.calls.push("play");
  }
  finish() {
    this.playState = "finished";
    this.currentTime = this.endTime;
    this.calls.push("finish");
  }
}
class CSSAnimation extends Anim {}
class CSSTransition extends Anim {}
const inLive = { closest: (sel) => (sel === "[data-live]" ? {} : null) };
const elsewhere = { closest: () => null };

test("CSS animations move on only on the map's frames; transitions answer at once, but live readouts glide at the same pace", () => {
  const clock = { t: 0 };
  const spin = new CSSAnimation();
  const fade = new CSSTransition({ end: 300, target: elsewhere });
  const gauge = new CSSTransition({ end: 900, target: inLive });
  const list = [spin, fade, gauge];
  const pacer = createCssPacer({ doc: { getAnimations: () => list.filter((a) => a.playState !== "finished") }, now: () => clock.t });
  pacer.step();
  assert.equal(spin.playState, "paused", "a CSS animation is taken over");
  assert.equal(gauge.playState, "paused", "a live readout's transition too");
  assert.equal(fade.playState, "running", "a panel's transition is left alone");
  assert.equal(pacer.held, 2);
  clock.t = 50;
  pacer.step();
  assert.equal(spin.currentTime, 50, "moved on by the time since, at each frame");
  assert.equal(gauge.currentTime, 50);
  clock.t = 1000;
  pacer.step();
  assert.equal(spin.currentTime, 1000);
  assert.equal(gauge.playState, "finished", "a finite one finishes once it's run its course");
  assert.equal(pacer.held, 1);
});

test("an animation gone from the page is let go, and release hands the rest back to the browser where they'd be", () => {
  const clock = { t: 0 };
  const a = new CSSAnimation(), b = new CSSAnimation();
  let list = [a, b];
  const pacer = createCssPacer({ doc: { getAnimations: () => list }, now: () => clock.t });
  pacer.step();
  b.playState = "idle"; // its element removed: the browser cancels it
  list = [a];
  clock.t = 40;
  pacer.step();
  assert.equal(pacer.held, 1);
  clock.t = 70;
  pacer.release();
  assert.equal(a.playState, "running");
  assert.equal(a.currentTime, 70);
  assert.equal(pacer.held, 0);
});
