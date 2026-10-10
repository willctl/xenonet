// Generative ambient score and UI sound effects, synthesized live with Web
// Audio (no audio files).
//
// The music listens to your agents: token flow opens the pad filter, the
// arpeggio thins out when nobody is working, and when everything has been idle
// for a minute it resolves to a held tonic chord. Each project has its own
// three-note motif, played when an agent works in it, so you can hear which
// project is busy without looking.
//
// Browsers only start audio after a click or key press, so a fresh load waits
// ("armed") until the first gesture. Mute is remembered.
//
// The music is the theme's, and a change of theme changes it as it plays.
//
// What happens on the map sounds from where it happens (sound-space.js): a
// tool call from its session, a turn's end, an alert from what needs you,
// a message from its sender to whoever it reached, an actor's act and a
// remote actor's read from the actor. Nearer is louder, it comes from its
// side, and far off is duller and further back in the room. The music
// itself (pads, arpeggio, drone, air) and the window's own clicks stay
// where they are. `xenonet.audio.space` keeps the mode: speakers (the
// default), headphones (placed in 3D), or flat (as before).
import { THEME, onTheme } from "./theme.js";
import { hear, placement, MODES } from "./sound-space.js";

const STORAGE_KEY = "xenonet.audio.muted";
const SPACE_KEY = "xenonet.audio.space";
// Placed sounds at once, at most: a busy map's quieter ones are let go.
const PLACED_MAX = 10;
const PLACED_MS = 2500;
const VOLUME = 0.75;
const CHORD_SECONDS = 16;
const IDLE_RESOLVE_MS = 60e3;

const hz = (midi) => 440 * 2 ** ((midi - 69) / 12);

function hash(s) {
  let h = 2166136261;
  for (const c of String(s)) h = Math.imul(h ^ c.charCodeAt(0), 16777619);
  return h >>> 0;
}

function readMuted() {
  try {
    return localStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function writeMuted(muted) {
  try {
    localStorage.setItem(STORAGE_KEY, muted ? "1" : "0");
  } catch {}
}

function readSpace() {
  try {
    const v = localStorage.getItem(SPACE_KEY);
    return MODES.includes(v) ? v : "speakers";
  } catch {
    return "speakers";
  }
}

function impulse(ctx, seconds, decay) {
  const len = Math.floor(ctx.sampleRate * seconds);
  const buf = ctx.createBuffer(2, len, ctx.sampleRate);
  for (let ch = 0; ch < 2; ch++) {
    const data = buf.getChannelData(ch);
    for (let i = 0; i < len; i++) data[i] = (Math.random() * 2 - 1) * (1 - i / len) ** decay;
  }
  return buf;
}

function noiseBuffer(ctx, seconds, brown = true) {
  const len = Math.floor(ctx.sampleRate * seconds);
  const buf = ctx.createBuffer(1, len, ctx.sampleRate);
  const data = buf.getChannelData(0);
  let last = 0;
  for (let i = 0; i < len; i++) {
    if (brown) {
      last = (last + 0.02 * (Math.random() * 2 - 1)) / 1.02;
      data[i] = last * 3.5;
    } else data[i] = Math.random() * 2 - 1;
  }
  return buf;
}

export function createAmbient() {
  // The theme's music (core/themes.js): a chord cycle and a scale in MIDI
  // notes, the tempo and the voices. XenoNet's own is a D lydian cycle,
  // Dmaj9 → Bm11 → Gmaj7#11 → Aadd9, at 84 BPM.
  let music = THEME.sound;
  let step = 60 / music.bpm / 2; // eighth notes
  // The music's notes that are sounding or due (from `at`), so a change of
  // theme can fade them out.
  const voices = new Set();
  const voice = (osc, env, at) => {
    const v = { osc, env, at };
    voices.add(v);
    osc.onended = () => voices.delete(v);
  };
  let muted = readMuted();
  let ctx = null;
  let master = null;
  let sfxBus = null;
  let bus = null;
  let chordTimer = null;
  let suspendTimer = null;
  let chordIndex = 0;
  let lastChime = 0;
  let lastHover = 0;
  let density = 1;
  let resolved = false;
  let idleSince = Date.now();
  let whiteNoise = null;
  // Whether XenoNet's music has stepped aside for Spotify (spotify-ui.js):
  // never mixed with it, so the music fades while Spotify plays and comes
  // back when it stops. Sound effects stay.
  let yielding = false;
  const motifAt = new Map();
  // Where sounds come from: the mode, the camera as an ear and where a
  // node or actor is (attach), the placed sounds of the last moments, and
  // the last few, for the driver.
  let mode = readSpace();
  let ear = () => null;
  let where = () => null;
  const placed = [];
  const heardLog = [];
  const cueAt = new Map();

  // Where a sound of `kind` made at the first of `ids` that's on the map
  // is heard, as the mode places it, or null to let it go: a busy moment
  // keeps its loudest. Noted for the driver either way, sound or none.
  function spot(kind, ids, { pan = 0, boost = 1, floor } = {}) {
    const id = (Array.isArray(ids) ? ids : [ids]).find((x) => x && where(x));
    const at = id ? where(id) : null;
    const e = mode === "flat" || !at ? null : ear();
    const heard = e ? hear(e, at, { boost, ...(floor != null ? { floor } : {}) }) : null;
    const place = placement(mode, heard, { pan });
    const now = performance.now();
    while (placed.length && now - placed[0].t > PLACED_MS) placed.shift();
    let kept = true;
    if (placed.length >= PLACED_MAX) {
      const quietest = placed.reduce((q, p) => (p.gain < q.gain ? p : q));
      if (quietest.gain >= place.gain) kept = false;
      else placed.splice(placed.indexOf(quietest), 1);
    }
    if (kept) placed.push({ t: now, gain: place.gain });
    heardLog.push({ kind, id: id ?? null, mode, gain: Math.round(place.gain * 100) / 100, pan: Math.round(place.pan * 100) / 100, behind: !!heard?.behind, played: kept && ready(), at: Date.now() });
    if (heardLog.length > 32) heardLog.shift();
    return kept ? place : null;
  }

  // The end of a placed sound's chain: a lowpass for distance (and from
  // behind), then a stereo pan, or a 3D panner for headphones, with some
  // of it sent to the room.
  function placeOut(node, place, fallbackPan) {
    let tail = node;
    if (place?.cutoff) {
      const f = ctx.createBiquadFilter();
      f.type = "lowpass";
      f.frequency.value = place.cutoff;
      tail.connect(f);
      tail = f;
    }
    let p;
    if (place?.dir) {
      p = ctx.createPanner();
      p.panningModel = "HRTF";
      p.distanceModel = "linear";
      p.rolloffFactor = 0;
      p.positionX.value = place.dir[0];
      p.positionY.value = place.dir[1];
      p.positionZ.value = place.dir[2];
    } else {
      p = ctx.createStereoPanner();
      p.pan.value = place ? place.pan : fallbackPan;
    }
    tail.connect(p);
    if (place?.wet != null) {
      const w = ctx.createGain();
      w.gain.value = place.wet * 0.5;
      p.connect(w).connect(bus.reverb);
    }
    return p;
  }
  const listeners = new Set();

  const state = () => (muted ? "muted" : ctx && ctx.state === "running" ? "playing" : "armed");
  const notify = () => listeners.forEach((fn) => fn(state()));
  const ready = () => ctx && !muted && ctx.state === "running";

  function build() {
    ctx = new AudioContext();
    master = ctx.createGain();
    master.gain.value = 0;
    const comp = ctx.createDynamicsCompressor();
    comp.threshold.value = -18;
    comp.ratio.value = 3;
    master.connect(comp).connect(ctx.destination);
    // Effects bypass the music's slow fade so they're crisp, but share the mute.
    sfxBus = ctx.createGain();
    sfxBus.gain.value = 0;
    sfxBus.connect(comp);

    const reverb = ctx.createConvolver();
    reverb.buffer = impulse(ctx, 6, 2.6);
    const wet = ctx.createGain();
    wet.gain.value = 0.7;
    reverb.connect(wet).connect(master);
    const dry = ctx.createGain();
    dry.gain.value = 0.55;
    dry.connect(master);
    const sfxVerb = ctx.createGain();
    sfxVerb.gain.value = 0.35;
    sfxVerb.connect(reverb);

    // Dotted-eighth feedback delay for the arpeggio.
    const delay = ctx.createDelay(2);
    delay.delayTime.value = step * 1.5;
    const feedback = ctx.createGain();
    feedback.gain.value = 0.42;
    const delayTone = ctx.createBiquadFilter();
    delayTone.type = "lowpass";
    delayTone.frequency.value = 2400;
    delay.connect(delayTone).connect(feedback).connect(delay);
    delayTone.connect(reverb);
    delayTone.connect(dry);

    // Pads breathe through a slowly swept lowpass whose centre follows token flow.
    const padFilter = ctx.createBiquadFilter();
    padFilter.type = "lowpass";
    padFilter.frequency.value = 800;
    padFilter.Q.value = 0.8;
    const lfo = ctx.createOscillator();
    lfo.frequency.value = 0.04;
    const lfoDepth = ctx.createGain();
    lfoDepth.gain.value = 420;
    lfo.connect(lfoDepth).connect(padFilter.frequency);
    lfo.start();
    const pads = ctx.createGain();
    pads.gain.value = 0.15;
    pads.connect(padFilter);
    padFilter.connect(dry);
    padFilter.connect(reverb);

    // Sub drone on D with a slow swell.
    const sub = ctx.createOscillator();
    sub.type = "sine";
    sub.frequency.value = hz(music.drone);
    const subGain = ctx.createGain();
    subGain.gain.value = 0.05;
    const swell = ctx.createOscillator();
    swell.frequency.value = 0.07;
    const swellDepth = ctx.createGain();
    swellDepth.gain.value = 0.025;
    swell.connect(swellDepth).connect(subGain.gain);
    sub.connect(subGain).connect(dry);
    sub.start();
    swell.start();

    // Filtered "air" like a ventilation hum in a lab.
    const air = ctx.createBufferSource();
    air.buffer = noiseBuffer(ctx, 4);
    air.loop = true;
    const airFilter = ctx.createBiquadFilter();
    airFilter.type = "bandpass";
    airFilter.frequency.value = 520;
    airFilter.Q.value = 0.6;
    const airLfo = ctx.createOscillator();
    airLfo.frequency.value = 0.03;
    const airLfoDepth = ctx.createGain();
    airLfoDepth.gain.value = 260;
    airLfo.connect(airLfoDepth).connect(airFilter.frequency);
    const airGain = ctx.createGain();
    airGain.gain.value = 0.05;
    air.connect(airFilter).connect(airGain).connect(reverb);
    air.start();
    airLfo.start();

    whiteNoise = noiseBuffer(ctx, 1.5, false);
    bus = { pads, padFilter, delay, reverb, dry, sfxVerb, drone: sub };
  }

  function padChord(notes, at, length = CHORD_SECONDS) {
    for (const midi of notes) {
      for (const detune of [-7, 6]) {
        const osc = ctx.createOscillator();
        osc.type = music.pad;
        osc.frequency.value = hz(midi);
        osc.detune.value = detune;
        const env = ctx.createGain();
        const peak = resolved ? 0.035 : 0.05;
        env.gain.setValueAtTime(0, at);
        env.gain.linearRampToValueAtTime(peak, at + 5);
        env.gain.setValueAtTime(peak, at + length - 1);
        env.gain.linearRampToValueAtTime(0, at + length + 6);
        osc.connect(env).connect(bus.pads);
        osc.start(at);
        osc.stop(at + length + 6.2);
        voice(osc, env, at);
      }
    }
  }

  function tone(midi, at, { type = music.lead, velocity = 1, decay = 1.6, dest = bus.delay, pan = Math.random() * 1.4 - 0.7, dry = true, place = null } = {}) {
    const osc = ctx.createOscillator();
    osc.type = type;
    osc.frequency.value = hz(midi);
    const env = ctx.createGain();
    env.gain.setValueAtTime(0, at);
    env.gain.linearRampToValueAtTime(0.06 * velocity * (place?.gain ?? 1), at + 0.006);
    env.gain.exponentialRampToValueAtTime(0.0001, at + decay);
    osc.connect(env);
    const p = placeOut(env, place, pan);
    p.connect(dest);
    if (dry) p.connect(bus.dry);
    osc.start(at);
    osc.stop(at + decay + 0.1);
    voice(osc, env, at);
  }

  function ping(at, place = null) {
    for (const [mult, peak] of [[1, 0.05], [2.01, 0.015]]) {
      const gain = peak * (place?.gain ?? 1);
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.value = hz(86) * mult;
      const env = ctx.createGain();
      env.gain.setValueAtTime(0, at);
      env.gain.linearRampToValueAtTime(gain, at + 0.01);
      env.gain.exponentialRampToValueAtTime(0.0001, at + 2.5);
      osc.connect(env).connect(bus.reverb);
      osc.start(at);
      osc.stop(at + 2.6);
    }
  }

  // Coarse scheduler: plan each chord's notes ahead in audio time. When the
  // agents have all been idle a while, hold the tonic chord and go quiet.
  function scheduleChord() {
    const at = ctx.currentTime + 0.1;
    const chord = resolved ? music.chords[0] : music.chords[chordIndex % music.chords.length];
    if (!resolved) chordIndex++;
    padChord(chord, at);
    const tones = chord.slice(1).flatMap((n) => [n + 12, n + 24]).filter((n) => n <= 90);
    const steps = Math.floor(CHORD_SECONDS / step);
    for (let i = 0; i < steps; i++) {
      const p = (i % 8 === 0 ? 0.55 : i % 2 === 0 ? 0.28 : 0.12) * density;
      if (Math.random() < p) tone(tones[Math.floor(Math.random() * tones.length)], at + i * step, { velocity: 0.5 + Math.random() * 0.5 });
    }
    chordTimer = setTimeout(scheduleChord, CHORD_SECONDS * 1000);
  }

  async function start() {
    if (muted) return;
    clearTimeout(suspendTimer);
    if (!ctx) build();
    if (ctx.state !== "running") await ctx.resume().catch(() => {});
    if (ctx.state !== "running") return notify();
    // Scheduling only while the clock runs; a suspended context would bunch notes up.
    if (!chordTimer) scheduleChord();
    master.gain.cancelScheduledValues(ctx.currentTime);
    master.gain.setTargetAtTime(yielding ? 0 : VOLUME, ctx.currentTime, 1.6);
    sfxBus.gain.cancelScheduledValues(ctx.currentTime);
    sfxBus.gain.setTargetAtTime(1, ctx.currentTime, 0.05);
    notify();
  }

  function stop() {
    if (!ctx) return notify();
    master.gain.cancelScheduledValues(ctx.currentTime);
    master.gain.setTargetAtTime(0, ctx.currentTime, 0.25);
    sfxBus.gain.setTargetAtTime(0, ctx.currentTime, 0.05);
    clearTimeout(chordTimer);
    chordTimer = null;
    clearTimeout(suspendTimer);
    suspendTimer = setTimeout(() => {
      if (muted) ctx.suspend().then(notify);
    }, 1500);
    notify();
  }

  function setMuted(next) {
    muted = next;
    writeMuted(muted);
    if (muted) stop();
    else start();
  }

  // While armed, the toggle starts audio rather than muting it.
  const toggle = () => (state() === "armed" ? start() : setMuted(!muted));
  const isMuteKey = (e) => (e.key === "m" || e.key === "M") && !e.target.matches?.("input, textarea, .xterm-helper-textarea") && !e.ctrlKey && !e.metaKey && !e.altKey;

  // Autoplay rules: the first click or key press anywhere starts audio. The
  // mute button and the M key handle themselves.
  const onGesture = (e) => {
    if (e.target.closest?.("[data-audio]") || (e.type === "keydown" && isMuteKey(e))) return;
    removeEventListener("pointerdown", onGesture, true);
    removeEventListener("keydown", onGesture, true);
    start();
  };
  addEventListener("pointerdown", onGesture, true);
  addEventListener("keydown", onGesture, true);
  addEventListener("keydown", (e) => isMuteKey(e) && toggle());

  // A new theme's music takes over at once: what's sounding fades out over
  // a second and a half and what's still due never starts, the echo and the
  // drone move to the new tempo and key, and the new chord cycle starts from
  // its first chord.
  onTheme((theme) => {
    music = theme.sound;
    step = 60 / music.bpm / 2;
    chordIndex = 0;
    if (!ctx) return;
    const now = ctx.currentTime;
    for (const { osc, env, at } of voices) {
      try {
        if (at > now) {
          osc.stop(now);
          continue;
        }
        env.gain.cancelAndHoldAtTime(now);
        env.gain.linearRampToValueAtTime(0, now + 1.5);
        osc.stop(now + 1.6);
      } catch {}
    }
    bus.delay.delayTime.setTargetAtTime(step * 1.5, now, 0.3);
    bus.drone.frequency.setTargetAtTime(hz(music.drone), now, 0.8);
    if (chordTimer) {
      clearTimeout(chordTimer);
      scheduleChord();
    }
  });

  // ---------- UI sound effects ----------

  function sweep(from, to, dur, { gain = 0.08, q = 6, at = ctx.currentTime } = {}) {
    const src = ctx.createBufferSource();
    src.buffer = whiteNoise;
    const f = ctx.createBiquadFilter();
    f.type = "bandpass";
    f.Q.value = q;
    f.frequency.setValueAtTime(from, at);
    f.frequency.exponentialRampToValueAtTime(to, at + dur);
    const env = ctx.createGain();
    env.gain.setValueAtTime(0, at);
    env.gain.linearRampToValueAtTime(gain, at + dur * 0.35);
    env.gain.exponentialRampToValueAtTime(0.0001, at + dur);
    src.connect(f).connect(env);
    env.connect(sfxBus);
    env.connect(bus.sfxVerb);
    src.start(at);
    src.stop(at + dur + 0.05);
  }

  function blipTone(freq, dur, { type = "sine", gain = 0.05, at = ctx.currentTime, glide = null, verb = true, place = null } = {}) {
    const osc = ctx.createOscillator();
    osc.type = type;
    osc.frequency.setValueAtTime(freq, at);
    if (glide) osc.frequency.exponentialRampToValueAtTime(glide, at + dur * 0.8);
    const env = ctx.createGain();
    env.gain.setValueAtTime(0, at);
    env.gain.linearRampToValueAtTime(gain * (place?.gain ?? 1), at + 0.005);
    env.gain.exponentialRampToValueAtTime(0.0001, at + dur);
    osc.connect(env);
    const out = place ? placeOut(env, place, 0) : env;
    out.connect(sfxBus);
    if (verb) out.connect(bus.sfxVerb);
    osc.start(at);
    osc.stop(at + dur + 0.05);
  }

  const SFX = {
    // Camera locks onto a node: rising whoosh, then a clean "lock" ping.
    focus: (t) => {
      sweep(320, 2600, 0.42, { gain: 0.07, at: t });
      blipTone(1318.5, 0.5, { gain: 0.045, at: t + 0.32 });
      blipTone(1975.5, 0.35, { gain: 0.02, at: t + 0.36 });
    },
    // Released: falling whoosh and a soft low thump.
    unfocus: (t) => {
      sweep(2400, 260, 0.38, { gain: 0.06, at: t });
      blipTone(147, 0.3, { gain: 0.06, at: t + 0.04, glide: 98, verb: false });
    },
    hover: (t) => blipTone(2637, 0.04, { gain: 0.01, at: t, verb: false }),
    open: (t) => {
      blipTone(880, 0.18, { gain: 0.035, at: t, glide: 1320 });
      blipTone(1760, 0.25, { gain: 0.015, at: t + 0.05 });
    },
    close: (t) => blipTone(1320, 0.18, { gain: 0.03, at: t, glide: 660 }),
    palette: (t) => {
      blipTone(880, 0.12, { gain: 0.035, at: t });
      blipTone(1318.5, 0.2, { gain: 0.035, at: t + 0.07 });
    },
    select: (t) => {
      blipTone(1800, 0.05, { gain: 0.03, at: t, verb: false, type: "triangle" });
      sweep(5000, 3000, 0.04, { gain: 0.02, q: 2, at: t });
    },
    toggle: (t) => blipTone(1200, 0.06, { gain: 0.03, at: t, type: "triangle", verb: false }),
    // Someone needs you: a gentle three-note bell, from where they wait.
    alert: (t, place) => [74, 78, 81].forEach((m, i) => {
      blipTone(hz(m + 12), 1.4, { gain: 0.05, at: t + i * 0.16, place });
      blipTone(hz(m + 24) * 1.002, 0.8, { gain: 0.012, at: t + i * 0.16, place });
    }),
    review: (t, place) => blipTone(hz(81 + 12), 1.2, { gain: 0.03, at: t, place }),
    // Two agents on one file: a soft minor-second rub, a note from each.
    collision: (t, place, other) => {
      blipTone(hz(76), 0.6, { gain: 0.04, at: t, type: "triangle", place });
      blipTone(hz(77), 0.6, { gain: 0.04, at: t + 0.09, type: "triangle", place: other ?? place });
    },
    stuck: (t, place) => {
      blipTone(220, 0.5, { gain: 0.04, at: t, glide: 180, type: "triangle", place });
      blipTone(208, 0.5, { gain: 0.03, at: t + 0.18, glide: 170, type: "triangle", place });
    },
    success: (t) => {
      blipTone(hz(81), 0.5, { gain: 0.04, at: t });
      blipTone(hz(85), 0.7, { gain: 0.04, at: t + 0.1 });
    },
    error: (t) => blipTone(185, 0.3, { gain: 0.05, at: t, type: "square", verb: false, glide: 150 }),
    shutter: (t) => {
      sweep(6000, 2000, 0.06, { gain: 0.12, q: 1, at: t });
      sweep(5000, 1800, 0.08, { gain: 0.09, q: 1, at: t + 0.09 });
      blipTone(3000, 0.03, { gain: 0.02, at: t, verb: false });
    },
    rewind: (t) => {
      sweep(3800, 300, 0.9, { gain: 0.06, q: 3, at: t });
      for (let i = 0; i < 6; i++) blipTone(1600 - i * 180, 0.06, { gain: 0.02, at: t + i * 0.12, verb: false });
    },
    pin: (t) => {
      blipTone(660, 0.08, { gain: 0.04, at: t, type: "triangle", verb: false });
      blipTone(990, 0.2, { gain: 0.03, at: t + 0.06 });
    },
    terminal: (t) => {
      sweep(200, 1400, 0.25, { gain: 0.05, at: t });
      blipTone(523, 0.12, { gain: 0.03, at: t + 0.15, type: "square", verb: false });
    },
  };

  // A sound effect; the map's own (alert, review, collision, stuck) from
  // where it happened when given `id` (for a collision, `id` and `to`).
  function sfx(name, { id = null, to = null } = {}) {
    if (!SFX[name]) return;
    const placedHere = id ? spot(name, id, { floor: 0.5 }) : null;
    if (id && !placedHere) return;
    if (!ready()) return;
    const now = ctx.currentTime;
    if (name === "hover") {
      if (now - lastHover < 0.09) return;
      lastHover = now;
    }
    SFX[name](now + 0.005, placedHere, to ? spot(name, to, { floor: 0.5 }) : null);
  }

  return {
    toggle,
    get state() {
      return state();
    },
    onChange(fn) {
      listeners.add(fn);
      fn(state());
    },
    sfx,
    // The sonar ping, played when the scene emits a sonar wave, from the
    // map's middle.
    ping() {
      const place = spot("ping", "core", { floor: 0.4 });
      if (ready()) ping(ctx.currentTime + 0.02, place);
    },
    // Where sounds come from: the camera as an ear ({ pos, right, up, fwd,
    // orbit, focus }) and where a node or an actor is, by id.
    attach({ ear: e = null, where: w = null } = {}) {
      if (e) ear = e;
      if (w) where = w;
    },
    get mode() {
      return mode;
    },
    // Spotify playing (true) or not: the music fades out, or back.
    yieldTo(on) {
      if (yielding === !!on) return;
      yielding = !!on;
      if (!ready()) return;
      master.gain.cancelScheduledValues(ctx.currentTime);
      master.gain.setTargetAtTime(yielding ? 0 : VOLUME, ctx.currentTime, 1.2);
    },
    get yielding() {
      return yielding;
    },
    setMode(next) {
      if (!MODES.includes(next)) return mode;
      mode = next;
      try {
        localStorage.setItem(SPACE_KEY, next);
      } catch {}
      return mode;
    },
    // The map's own quiet cues: a message from its sender to each it
    // reached, an actor acting, a remote actor's read through its tunnel.
    cue(kind, { id = null, to = [] } = {}) {
      const gap = { message: 0.15, actor: 0.2, read: 0.3 }[kind];
      if (gap == null) return;
      const now = ctx?.currentTime ?? performance.now() / 1000;
      if (now - (cueAt.get(kind) ?? -10) < gap) return;
      cueAt.set(kind, now);
      const from = spot(kind, id, { floor: 0.12 });
      if (!from || !ready()) return;
      const t = ctx.currentTime + 0.02;
      const chord = music.chords[(chordIndex - 1 + music.chords.length) % music.chords.length];
      if (kind === "message") {
        tone(chord[3] + 24, t, { velocity: 0.35, decay: 0.7, dest: bus.reverb, place: from });
        for (const [i, target] of (to ?? []).slice(0, 3).entries()) {
          const there = spot(kind, target, { floor: 0.12 });
          if (there) tone(chord[4] + 24, t + 0.18 + i * 0.06, { velocity: 0.28, decay: 0.8, dest: bus.reverb, place: there });
        }
      } else if (kind === "actor") tone(chord[1] + 24, t, { type: "sine", velocity: 0.3, decay: 0.9, dest: bus.reverb, place: from });
      else blipTone(hz(chord[2] + 36), 0.25, { gain: 0.02, at: t, place: from });
    },
    // What was placed lately, newest last: each sound's kind, where (by
    // id), and how loud and from which side, played or not.
    heard: () => heardLog.map((h) => ({ ...h })),
    // Music follows the agents: brighter with token flow, sparser when idle,
    // resolving to a held chord after a minute of quiet.
    setIntensity(tpm, working) {
      if (working > 0) idleSince = Date.now();
      resolved = working === 0 && Date.now() - idleSince > IDLE_RESOLVE_MS;
      density = working ? 0.65 + 0.35 * Math.min(1, working / 3) : 0.15;
      if (!ready()) return;
      const k = Math.min(1, Math.log10(1 + tpm) / 7);
      bus.padFilter.frequency.setTargetAtTime(650 + 1700 * k, ctx.currentTime, 3);
    },
    // Each project's motif: three notes from the scale, fixed per project.
    // Each project's motif: three notes from the scale, fixed per project,
    // from the session that worked (`id`) or the project.
    motif(projectId, { id = null } = {}) {
      if (!projectId) return;
      const now = ctx?.currentTime ?? 0;
      if (ctx && now - (motifAt.get(projectId) ?? -10) < 2.5) return;
      const h = hash(projectId);
      const place = spot("motif", [id, projectId], { pan: ((h % 200) / 100 - 1) * 0.7 });
      if (!ready() || !place) return;
      motifAt.set(projectId, now);
      const notes = [0, 1, 2].map((i) => music.scale[(h >>> (i * 4)) % music.scale.length] + (i === 2 && h & 1 ? 12 : 0));
      const type = ["sine", "triangle", "sine"][h % 3];
      notes.forEach((m, i) => tone(m + 12, now + 0.02 + i * step * (h & 2 ? 0.5 : 1), { type, velocity: 0.42, decay: 1.2, place }));
    },
    // A soft two-note chime when a turn completes or a subagent launches,
    // from its session.
    chime({ id = null } = {}) {
      const now = ctx?.currentTime ?? 0;
      if (ctx && now - lastChime < 3) return;
      const place = spot("chime", id, { floor: 0.3 });
      if (!ready() || !place) return;
      lastChime = now;
      const chord = music.chords[(chordIndex - 1 + music.chords.length) % music.chords.length];
      tone(chord[2] + 24, now + 0.02, { velocity: 0.9, dest: bus.reverb, place });
      tone(chord[4] + 24, now + 0.2, { velocity: 0.7, dest: bus.reverb, place });
    },
    get debug() {
      return { state: state(), density, resolved, ctxState: ctx?.state ?? null };
    },
  };
}
