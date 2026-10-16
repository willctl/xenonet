// How often the console draws. Each frame renders the map with bloom at
// the screen's density, and the window's compositor redraws everything
// over it (the panels' blurred backgrounds among it), so a console drawing
// every frame the display offers (120 a second on a ProMotion screen) kept
// a whole GPU busy, even closed to the menu bar. Now it draws at most 60
// frames a second while something moves, fewer at rest, and none while
// nobody can see it, unless a recording needs every frame.
//
// The page's CSS animations follow the same frames (see createCssPacer):
// any one of them left running would have the compositor redraw the
// window at the display's rate anyway.

// Frames a second for each pace, the setting in the palette: while
// something moves (the layout, the camera, a gathering, your input), at
// rest in the window you're using, and at rest in a window behind
// another app's. "smooth" draws every frame the display offers.
export const PACES = {
  balanced: { moving: 60, rest: 30, background: 20 },
  low: { moving: 30, rest: 15, background: 10 },
  smooth: { moving: Infinity, rest: Infinity, background: Infinity },
};
export const DEFAULT_PACE = "balanced";
export const paceOf = (name) => (Object.hasOwn(PACES, name) ? name : DEFAULT_PACE);

// The frame rate now: 0 draws nothing, Infinity every frame.
//
//   recording  a film or a clip is being recorded: every frame
//   seen       the window can be seen (not hidden, minimized, covered, or
//              the screen locked)
//   driven     a script, test or agent is working the window through the
//              control channel, which waits for the view to settle and
//              takes screenshots of it even while it's hidden
//   focused    the window is the one you're using
//   moving     something on the map is moving, or you just touched it
export function frameRate({ pace = DEFAULT_PACE, recording = false, seen = true, driven = false, focused = true, moving = false } = {}) {
  if (recording) return Infinity;
  if (!seen && !driven) return 0;
  const p = PACES[paceOf(pace)];
  if (moving) return p.moving;
  return focused || driven ? p.rest : p.background;
}

// Which of the display's frames to draw at a given rate. Frames come on
// the display's beat, so a frame counts as due a little before its time,
// and the next one is timed from when this one was due, not when it came:
// 60 a second on a 120 Hz display is every other frame, and on a 144 Hz
// one, every second or third.
export function createPacer() {
  let last = -Infinity;
  return {
    due(now, fps) {
      if (!(fps > 0)) return false;
      if (fps === Infinity) return true;
      const gap = 1000 / fps;
      return now - last >= gap - 1.5;
    },
    // A frame was drawn at `now`, at this rate.
    drew(now, fps) {
      const gap = 1000 / fps;
      last = fps === Infinity || !Number.isFinite(last) || now - last > gap * 2 ? now : last + gap;
    },
    // How long until the next frame is due, in ms.
    wait(now, fps) {
      if (!(fps > 0)) return Infinity;
      if (fps === Infinity || !Number.isFinite(last)) return 0;
      return Math.max(0, last + 1000 / fps - 1.5 - now);
    },
  };
}

// CSS animations at the console's pace: each one running (the HUD's
// spinners, blinks and glows, a feed line's arrival) is paused and moved on
// by hand, once a frame the map draws, so the compositor redraws the window
// only then. Transitions run free, so panels and menus answer at once,
// apart from live readouts gliding to each new reading ([data-live]),
// which glide at the same pace. `release` hands them all back to the
// browser, for the smooth pace.
export function createCssPacer({ doc = globalThis.document, now = () => globalThis.performance.now() } = {}) {
  const held = new Map(); // animation → [its time, the clock] when last moved on
  const paceable = (a) => a.constructor?.name === "CSSAnimation" || (a.constructor?.name === "CSSTransition" && !!a.effect?.target?.closest?.("[data-live]"));
  return {
    step() {
      const t = now();
      for (const a of doc.getAnimations()) {
        if (!held.has(a)) {
          if (a.playState !== "running" || !paceable(a)) continue;
          a.pause();
          held.set(a, [a.currentTime ?? 0, t]);
          continue;
        }
        const [from, at] = held.get(a);
        const end = a.effect?.getComputedTiming?.().endTime ?? Infinity;
        const time = from + (t - at);
        // A finite one that has run its course finishes as it would have.
        if (time >= end) {
          held.delete(a);
          a.finish();
        } else a.currentTime = time;
      }
      // Gone from the page (its element removed, or its CSS changed).
      for (const a of held.keys()) if (a.playState === "idle" || a.playState === "finished") held.delete(a);
    },
    release() {
      for (const [a, [from, at]] of held) {
        if (a.playState !== "paused") continue;
        a.currentTime = from + (now() - at);
        a.play();
      }
      held.clear();
    },
    get held() {
      return held.size;
    },
  };
}
