// Where a sound comes from, as you'd hear it from the camera (ambient.js):
// a sound made at a place on the map is louder the nearer it is, comes
// from its side, is duller and more distant in the room the further off
// it is, and muffled from behind. Plain functions of plain numbers, so the
// rules can be tested without audio.
//
// Distance is measured against how far the camera is from what it looks
// at (`orbit`), not in world units: looking at the whole map, everything
// is about as near as everything else and the mix stays near even; flown
// in close (`focus`, 0 to 1, the scene's focus level), what's beside you
// stands out and the rest falls back. Nothing falls silent: far off is a
// quiet bed (`floor`), never nothing.
//
//   ear  { pos, right, up, fwd: [x, y, z] (unit vectors), orbit, focus }
//   at   [x, y, z], where the sound is
//
// hear(ear, at) → { gain, pan, behind, wet, cutoff, dist, dir }
//   gain    0 to 1.25, what to scale its loudness by
//   pan     -1 (left) to 1 (right)
//   behind  whether it's behind the camera
//   wet     0 to 1, how much of it goes to the reverb
//   cutoff  a lowpass's frequency, in Hz
//   dir     [right, up, -forward]: its direction in the camera's own
//           space, for a panner that places it in 3D (headphones)

export const MODES = ["flat", "speakers", "headphones"];

const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

export function hear(ear, at, { floor = 0.15, boost = 1, width = 0.85 } = {}) {
  const d = [at[0] - ear.pos[0], at[1] - ear.pos[1], at[2] - ear.pos[2]];
  const dist = Math.hypot(d[0], d[1], d[2]);
  const dir = dist > 1e-6 ? d.map((v) => v / dist) : [0, 0, -1];
  const orbit = Math.max(1, ear.orbit ?? dist);
  const focus = clamp(ear.focus ?? 0, 0, 1);
  const rel = dist / orbit;
  // Within the orbit, as loud as can be, a little louder still when
  // nearer than what the camera looks at; beyond it, falling away, faster
  // the closer in you've flown.
  const atten = 1 / (1 + (0.5 + 1.5 * focus) * Math.max(0, rel - 1));
  const near = rel < 1 ? 1 + 0.25 * (1 - rel) : 1;
  const gain = clamp(Math.max(floor, atten * near) * boost, 0, 1.25);
  const ahead = dot(dir, ear.fwd);
  const behind = ahead < -0.2;
  const pan = clamp(dot(dir, ear.right) * width, -1, 1);
  const wet = clamp(0.2 + 0.6 * (1 - atten), 0, 1);
  const cutoff = Math.round(Math.min(2500 + 9500 * atten, behind ? 3000 : Infinity));
  return { gain, pan, behind, wet, cutoff, dist, dir: [dot(dir, ear.right), dot(dir, ear.up), -ahead] };
}

// A sound that travels from one place to another (a message, an arc):
// where it is heard at its start, middle and end, for a pan and gain that
// move with it.
export function path(ear, from, to, opts) {
  const mid = [(from[0] + to[0]) / 2, (from[1] + to[1]) / 2 + Math.hypot(to[0] - from[0], to[2] - from[2]) * 0.12, (from[2] + to[2]) / 2];
  return [hear(ear, from, opts), hear(ear, mid, opts), hear(ear, to, opts)];
}

// How a sound is placed in each mode: flat leaves it as it was (`pan` its
// own), speakers pans and fades it, headphones places it in 3D. Null
// `heard` (no place known) is flat.
export function placement(mode, heard, { pan = 0 } = {}) {
  if (mode === "flat" || !heard) return { gain: 1, pan, wet: null, cutoff: null, dir: null };
  return { gain: heard.gain, pan: heard.pan, wet: heard.wet, cutoff: heard.cutoff, dir: mode === "headphones" ? heard.dir : null };
}
