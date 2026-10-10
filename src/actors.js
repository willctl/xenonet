// The actors layer: the person at the console, always-on agents and actors
// of your own (contracts section 3; docs/design/actors/interface.md), drawn
// on a layer of their own above the map, apart from the graph and not
// under its root, with beams into the projects and sessions they act on.
// Each takes its theme's form (looks/actors.js) in a color of the theme's
// actor palette, breathes, glows as brightly as it's present, and reacts
// when it acts: a flash, a ripple, and light running down the beam to what
// it touched. A remote actor that reaches XenoNet through a tunnel has a gate
// between it and the map (looks/gate.js), which its beams and reads pass
// through. actors-model.js has the rules for colors, presence, beams,
// gates and placement; this draws them.
//
// scene.js makes it and steps it from its frame loop. The layer keeps its
// own map of beams: setLive takes away arcs it didn't put up, and beams
// come and go with the actors' list instead. Everything here is drawn in
// the scene itself, beside the force graph's own objects, so the graph's
// picking never sees it and the layer picks for itself.
import * as THREE from "three";
import SpriteText from "three-spritetext";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import { THEME, COLORS } from "./theme.js";
import { Additive, glowTex, ringTex, lineMaterial, releaseMaterial } from "./fx.js";
import { formNamed } from "./looks/actors.js";
import { buildGate } from "./looks/gate.js";
import { charAtlas } from "./looks/chars.js";
import { allBeams, boundsOf, colorOf, formOf, presenceOf, placeActors, fractionOf, gateOf, gateSpot, BEAMS_MAX } from "./actors-model.js";

export const isActorId = (id) => typeof id === "string" && /^(@[a-z][a-z0-9-]{1,31}:)?actor:[a-z0-9][a-z0-9-]{0,31}$/.test(id);

// An agent's body radius in world units; you're a little smaller. Larger
// than a project, so they stand out from the map.
const SIZE = { agent: 8, human: 6.5 };
const SEG = 20; // pieces per beam
const SLOTS = BEAMS_MAX + 8; // beams drawn at once, those fading out included
const PULSES = 72;
const RIPPLES = 8;
// What an event makes of the actor's tie to its target, so the beam is up
// before the next list says so (core/actors does the same).
const RELATION = { spawned: "spawned", edited: "edited", messaged: "messaged", said: "messaged", ran: "works-on", status: "works-on", finished: "works-on", failed: "works-on" };

const ease = (from, to, rate, dt) => from + (to - from) * (1 - Math.exp(-rate * dt));
const WHITE = new THREE.Color("#ffffff");
const FACING = new THREE.Vector3(0, 0, 1); // a gate's own axis
// How long a beam a read lit up stays, though no list has it.
const READ_HOLD_MS = 2500;
const READ_REACH = 4;
// A snapshot of the actor's own, whose targets an event may add to.
const own = (s) => ({ ...s, targets: [...(s.targets ?? [])] });

function label(text, color) {
  const s = new SpriteText(text, 4.4, color);
  s.fontFace = THEME.fonts.display;
  s.fontWeight = "600";
  s.fontSize = 128;
  s.padding = 1;
  s.material.depthWrite = false;
  if (s.material.map) s.material.map.anisotropy = 8;
  s.renderOrder = 11;
  return s;
}

// Everything a pulse needs to be drawn as a soft dot that shrinks with
// distance: one Points object for every pulse on every beam.
function pulsePoints() {
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(new Float32Array(PULSES * 3), 3));
  geo.setAttribute("color", new THREE.BufferAttribute(new Float32Array(PULSES * 3), 3));
  geo.setAttribute("size", new THREE.BufferAttribute(new Float32Array(PULSES), 1));
  geo.setDrawRange(0, 0);
  const mat = new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    uniforms: { view: { value: 900 } },
    vertexShader: /* glsl */ `
      attribute float size; attribute vec3 color;
      uniform float view;
      varying vec3 vColor;
      void main() {
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = clamp(size * view / max(-mv.z, 1.0), 1.0, 96.0);
        gl_Position = projectionMatrix * mv;
        vColor = color;
      }`,
    fragmentShader: /* glsl */ `
      varying vec3 vColor;
      void main() {
        float d = length(gl_PointCoord - 0.5);
        float a = smoothstep(0.5, 0.0, d);
        a = a * a;
        gl_FragColor = vec4(vColor * a, a);
      }`,
  });
  const points = new THREE.Points(geo, mat);
  points.frustumCulled = false;
  points.renderOrder = 12;
  return points;
}

// `nodeOf(id)` is a drawn node, `homeOf(node)` where a gathered one lives
// when it isn't gathered (or null), `drawn(id)` whether a node is on the
// map to beam into, `nodes()` every node, and `flash(node, color, scale,
// duration)` the map's ring flash.
export function createActorLayer({ scene, camera, renderer, nodeOf, homeOf = () => null, drawn, nodes, flash }) {
  const layer = new THREE.Group();
  layer.name = "actors";
  scene.add(layer);
  const records = new Map(); // id → what's drawn for an actor
  let snapshots = []; // the latest list's actors
  let lastList = null;
  // A cast of made-up actors that stands in for the list, for the
  // benchmark and tests (see setCast); null draws the list.
  let cast = null;
  let enabled = true;
  let clock = null; // the replay's time, while it plays
  let selectedId = null, hoveredId = null;
  // Atlases of characters, by the characters, in the theme's mono font: the
  // cursor form's cells and the gates' hex digits.
  const atlases = new Map();
  const chars = (text) => {
    if (!atlases.has(text)) atlases.set(text, charAtlas(text, THEME.fonts.mono));
    return atlases.get(text);
  };
  const dropAtlases = () => {
    for (const a of atlases.values()) a.texture.dispose();
    atlases.clear();
  };
  let view = 900;
  const buffer = new THREE.Vector2();
  const stats = { stepMs: 0 };

  // ---------- beams: one line object for all of them ----------

  const beamGeo = new LineSegmentsGeometry();
  beamGeo.setPositions(new Float32Array(SLOTS * SEG * 6));
  beamGeo.setColors(new Float32Array(SLOTS * SEG * 6));
  beamGeo.instanceCount = 0;
  const beamLines = new LineSegments2(beamGeo, lineMaterial({ width: 1.4, opacity: 1, vertexColors: true }));
  beamLines.frustumCulled = false;
  beamLines.renderOrder = 6;
  layer.add(beamLines);
  const posBuf = beamGeo.attributes.instanceStart.data;
  const colBuf = beamGeo.attributes.instanceColorStart.data;
  const beams = new Map(); // key → { key, actor, node, strength, level, relation, seed, beat, jitter }

  const pulses = pulsePoints();
  layer.add(pulses);
  const live = []; // { beam, u, speed, size, color: THREE.Color, land }

  // Ripples when an actor acts: rings that spread and fade.
  const ripples = [];
  const ripplePool = [];

  // ---------- actors ----------

  function build(rec) {
    const theme = THEME;
    const s = rec.snapshot;
    const kind = s.kind === "human" ? "human" : "agent";
    rec.size = SIZE[kind];
    rec.color = new THREE.Color(colorOf(s, theme));
    rec.formName = formOf(s, theme);
    rec.seed = fractionOf(s.id, "seed");
    const make = formNamed(rec.formName);
    rec.form = make({ color: rec.color, size: rec.size, theme, params: kind === "human" ? {} : theme.actors.params ?? {}, seed: rec.seed, human: kind === "human", chars });
    const glow = theme.actors.glow ?? {};
    rec.glow = new THREE.Sprite(new THREE.SpriteMaterial({ map: glowTex, color: rec.color, transparent: true, opacity: glow.opacity ?? 0.5, depthWrite: false, blending: Additive }));
    rec.glow.material.userData.base = (glow.opacity ?? 0.5) * (rec.form.glow ?? 1);
    rec.glowSize = rec.size * (glow.scale ?? 2.4) * 3.4;
    rec.glow.scale.setScalar(rec.glowSize);
    rec.label = label(kind === "human" ? "YOU" : s.name.toUpperCase(), `#${rec.color.clone().lerp(new THREE.Color("#ffffff"), 0.35).getHexString()}`);
    // Clear of the form's top, for the tall ones.
    rec.label.position.y = rec.size * Math.max(2.7, (rec.form.top ?? 0) + 0.5);
    rec.body = new THREE.Group();
    rec.body.add(rec.form.group);
    rec.group.add(rec.glow, rec.body, rec.label);
    rec.needs = false;
  }

  function unbuild(rec) {
    dropGate(rec);
    rec.group.remove(rec.glow, rec.body, rec.label);
    rec.form.dispose();
    rec.glow.material.dispose();
    rec.label.material.map?.dispose();
    rec.label.material.dispose();
  }

  function add(s) {
    const group = new THREE.Group();
    const rec = { id: s.id, kind: "actor", snapshot: own(s), group, x: 0, y: 0, z: 0, goal: new THREE.Vector3(), placed: false, working: 0, light: 0, kick: 0, scale: 1, reach: new THREE.Vector3(), hasReach: false, gate: null, gatePos: new THREE.Vector3(), gateGoal: new THREE.Vector3(), gateDir: new THREE.Vector3(0, -1, 0), gateFace: new THREE.Vector3(0, -1, 0), gatePlaced: false };
    build(rec);
    layer.add(group);
    records.set(s.id, rec);
    return rec;
  }

  function remove(rec) {
    unbuild(rec);
    layer.remove(rec.group);
    records.delete(rec.id);
  }

  // A remote actor's tunnel, drawn as a gate (looks/gate.js) in the
  // layer's own space, apart from the actor's group, since it sits
  // between the actor and the map.
  function makeGate(rec) {
    rec.gate = buildGate({ color: rec.color, size: rec.size, style: THEME.actors.beam?.style ?? "pulse", seed: rec.seed, chars });
    layer.add(rec.gate.group, rec.gate.conduit, ...rec.gate.packets);
    if (!rec.gatePlaced) rec.gatePos.set(rec.x, rec.y, rec.z);
  }
  function dropGate(rec) {
    if (!rec.gate) return;
    layer.remove(rec.gate.group, rec.gate.conduit, ...rec.gate.packets);
    rec.gate.dispose();
    rec.gate = null;
    rec.gatePlaced = false;
  }

  // A beam runs from where its actor is to where its node is now; a
  // tunneled one's, from its gate.
  function ends(beam, a, b) {
    const rec = records.get(beam.actor), node = nodeOf(beam.node);
    if (!rec || !node || !Number.isFinite(node.x)) return false;
    b.set(node.x, node.y, node.z);
    if (rec.gate && rec.gate.shown > 0.05) return a.copy(rec.gatePos), true;
    a.set(rec.x, rec.y, rec.z);
    // From the edge of its body, not its middle.
    const d = a.distanceTo(b);
    if (d > 1) a.lerp(b, Math.min(0.45, (rec.size * 0.9 * rec.scale) / d));
    return true;
  }

  // A point along a beam: a curve bowed up over its middle, less for a
  // bolt. `out` is written and returned.
  const cm = new THREE.Vector3();
  function along(u, a, b, out) {
    const len = a.distanceTo(b);
    const bow = THEME.actors.beam?.style === "bolt" ? 0.04 : 0.12;
    cm.addVectors(a, b).multiplyScalar(0.5);
    cm.y += len * bow;
    const v = 1 - u;
    return out.set(v * v * a.x + 2 * v * u * cm.x + u * u * b.x, v * v * a.y + 2 * v * u * cm.y + u * u * b.y, v * v * a.z + 2 * v * u * cm.z + u * u * b.z);
  }

  // ---------- the list and events ----------

  function set(list) {
    lastList = list;
    apply(cast ? { actors: cast } : list);
  }

  function apply(list) {
    const seen = new Set();
    for (const s of list?.actors ?? []) {
      if (!s || !isActorId(s.id) || seen.has(s.id)) continue;
      seen.add(s.id);
      const rec = records.get(s.id);
      if (!rec) add(s);
      else {
        // A new name or kind means a new body.
        const relabel = rec.snapshot.name !== s.name || rec.snapshot.kind !== s.kind;
        rec.snapshot = { ...s, targets: mergeTouched(rec.snapshot, s) };
        if (relabel) unbuild(rec), build(rec);
      }
    }
    for (const rec of [...records.values()]) if (!seen.has(rec.id)) remove(rec);
    snapshots = [...records.values()].map((r) => r.snapshot);
    if (selectedId && !records.has(selectedId)) selectedId = null;
    currentAt = placeAt = 0;
  }

  // Targets an event touched since the list was made stay until a list
  // has them too.
  function mergeTouched(was, now) {
    const out = [...(now.targets ?? [])];
    for (const t of was.targets ?? []) if (t.touched && !out.some((o) => o.id === t.id && o.relation === t.relation) && t.lastAt > (now.lastSeen ?? 0) - 1000) out.push(t);
    return out;
  }

  // One event: the actor reacts (unless `react` is false), and if it
  // touched a node, its beam lights up there and light runs down it.
  function event(e, { still = false, react = true } = {}) {
    const rec = records.get(e?.actor);
    if (!rec) return;
    const s = rec.snapshot;
    const rel = RELATION[e.kind];
    if (e.target && rel && !isActorId(e.target)) {
      const at = e.at ?? now();
      const t = (s.targets ??= []).find((x) => x.id === e.target && x.relation === rel);
      if (t) t.lastAt = Math.max(t.lastAt ?? 0, at);
      else s.targets.push({ id: e.target, relation: rel, since: at, lastAt: at, touched: true });
    }
    if (e.kind === "presence" && typeof e.detail === "string") {
      // The list brings the new presence; until then, an event about
      // coming or going shows at once.
      if (/came online|checked in/i.test(e.detail)) s.presence = "working";
      else if (/went offline/i.test(e.detail)) s.presence = "offline";
    }
    if (still || !enabled) return;
    if (react) {
      rec.kick = 1;
      rec.form.react();
      if (THEME.actors.react?.ripple !== false) ripple(rec);
    }
    // Light down the beam to what it touched, landing with a flash.
    if (e.target && !isActorId(e.target)) {
      placeAt = 0;
      const key = `${rec.id}>${e.target}`;
      let beam = beams.get(key);
      if (!beam && drawn(e.target)) beam = newBeam(key, rec.id, e.target, 1);
      if (beam) for (let i = 0; i < 3; i++) pulse(beam, { delay: i * 0.12, size: 2.6 - i * 0.4, speed: 0.95, land: i === 0 });
    }
  }

  // A message it sent agents through XenoNet's engine (the page's
  // agent-message, which the graph's own drawing can't start from an
  // actor): it reacts once, and light runs down a beam to each session it
  // reached, as a said event draws to one.
  function message(from, to, { at, still = false } = {}) {
    if (!records.has(from)) return;
    const reached = (Array.isArray(to) ? to : []).filter((id) => typeof id === "string" && !isActorId(id) && nodeOf(id));
    if (!reached.length) return event({ actor: from, kind: "said", target: null, at }, { still });
    reached.forEach((id, i) => event({ actor: from, kind: "said", target: id, at }, { still, react: i === 0 }));
  }

  // A remote actor read through its connection (the page's remote-read):
  // light down its tunnel's conduit, if it has one, then out along beams
  // to a few of the places it read, which stay lit a moment though it
  // doesn't act on them. Reads of made-up data reach nothing on the map.
  function read(e, { still = false } = {}) {
    const rec = records.get(e?.actor);
    if (!rec || still || !enabled) return;
    const delay = rec.gate ? rec.gate.read() : 0;
    rec.kick = Math.max(rec.kick, 0.3);
    const reach = (Array.isArray(e.reach) ? e.reach : []).filter((id) => typeof id === "string" && !isActorId(id) && drawn(id)).slice(0, READ_REACH);
    const hold = performance.now() + READ_HOLD_MS;
    reach.forEach((id, i) => {
      const key = `${rec.id}>${id}`;
      const beam = beams.get(key) ?? newBeam(key, rec.id, id, 0.7);
      beam.strength = Math.max(beam.strength, 0.7);
      beam.hold = hold;
      pulse(beam, { delay: delay + i * 0.08, size: 2.3, speed: 0.9, land: true });
    });
  }

  const now = () => (clock ? clock() : Date.now());

  // ---------- beams ----------

  function newBeam(key, actor, node, strength) {
    const beam = { key, actor, node, strength, level: 0, relation: null, seed: fractionOf(key, "beam"), beat: fractionOf(key, "beat") * 2, jitter: new Float32Array(SEG + 1), jitterAt: 0 };
    beams.set(key, beam);
    return beam;
  }

  function pulse(beam, { delay = 0, size = 2, speed = 0.7, land = false } = {}) {
    if (live.length >= PULSES) return;
    live.push({ beam, u: -delay * speed, speed, size, land });
  }

  function ripple(rec) {
    if (ripples.length >= RIPPLES) return;
    const s = ripplePool.pop() ?? new THREE.Sprite(new THREE.SpriteMaterial({ map: ringTex, transparent: true, depthWrite: false, blending: Additive }));
    s.material.color.copy(rec.color);
    s.renderOrder = 14;
    s.position.set(rec.x, rec.y, rec.z);
    layer.add(s);
    ripples.push({ s, t: 0, r: rec.size, flash: THEME.actors.react?.flash ?? 1.6 });
  }

  // ---------- where they float ----------

  let bounds = { center: [0, 0, 0], radius: 300 };
  let boundsAt = 0, placeAt = 0;
  // The drawn map's middle and reach, with gathered nodes where they live
  // (boundsOf: this machine's map, not other machines').
  function measure() {
    bounds = boundsOf(nodes().filter((nd) => drawn(nd.id)), (nd) => homeOf(nd) ?? nd) ?? bounds;
  }

  function place(current) {
    const by = new Map();
    for (const b of current) {
      const node = nodeOf(b.node);
      if (!node || !Number.isFinite(node.x)) continue;
      const at = homeOf(node) ?? node;
      (by.get(b.actor) ?? by.set(b.actor, []).get(b.actor)).push([at.x, at.y, at.z, b.strength]);
    }
    const spots = placeActors(snapshots.map((s) => ({ id: s.id, kind: s.kind, presence: s.presence, targets: by.get(s.id) ?? [] })), bounds);
    for (const [id, [x, y, z]] of spots) {
      const rec = records.get(id);
      if (!rec) continue;
      rec.goal.set(x, y, z);
      if (!rec.placed) (rec.placed = true), (rec.x = x), (rec.y = y), (rec.z = z);
      // Its gate, between it and what it reads.
      if (gateOf(rec.snapshot) == null) continue;
      const g = gateSpot([x, y, z], by.get(id) ?? [], bounds.center, { gap: rec.size * 3.6 });
      rec.gateGoal.set(...g.at);
      rec.gateDir.set(...g.dir);
      if (!rec.gatePlaced) (rec.gatePlaced = true), rec.gatePos.copy(rec.gateGoal), rec.gateFace.copy(rec.gateDir);
    }
  }

  // ---------- each frame ----------

  const A = new THREE.Vector3(), B = new THREE.Vector3(), P = new THREE.Vector3(), Q = new THREE.Vector3();
  const tmpColor = new THREE.Color();
  let current = [];
  let currentAt = 0;

  // `f`: { t, dt, adt, still, lighting: { focusLevel, selectedId, branch,
  // dimmed, labelFade } }.
  function step(f) {
    if (!enabled || !records.size) {
      if (beamGeo.instanceCount) beamGeo.instanceCount = 0;
      stats.stepMs = 0;
      return;
    }
    const t0 = performance.now();
    const { t, dt, adt, still, lighting } = f;
    const time = now();
    view = renderer.getDrawingBufferSize(buffer).y / (2 * Math.tan((camera.fov * Math.PI) / 360));
    pulses.material.uniforms.view.value = view;

    if (t0 - boundsAt > 1000) (boundsAt = t0), measure();
    // Which beams to draw, a few times a second: they change with the
    // list, the clock and what's drawn, not every frame.
    if (t0 - currentAt > 250) {
      currentAt = t0;
      current = allBeams(snapshots, time, drawn);
      for (const b of current) {
        const beam = beams.get(b.key) ?? newBeam(b.key, b.actor, b.node, b.strength);
        beam.strength = b.strength;
        beam.relation = b.relation;
        beam.wanted = currentAt;
      }
      for (const beam of beams.values()) if (beam.wanted !== currentAt && !(beam.hold > t0)) beam.strength = 0;
    }
    if (t0 - placeAt > 500) (placeAt = t0), place(current);

    const look = THEME.actors;
    const period = look.breathe?.period ?? 4.5, depth = look.breathe?.depth ?? 0.12;
    const stepped = THEME.motion.ease === "stepped";
    const tt = stepped ? Math.floor(t * 4) / 4 : t;
    const related = (rec) => {
      for (const b of current) if (b.actor === rec.id && lighting.branch?.has(b.node)) return true;
      return false;
    };
    const matched = (rec) => {
      for (const b of current) if (b.actor === rec.id && lighting.dimmed.has(b.node)) return true;
      return false;
    };

    for (const rec of records.values()) {
      const s = rec.snapshot, pres = presenceOf(s);
      // Glides to where it belongs; held still, it's there.
      if (still) (rec.x = rec.goal.x), (rec.y = rec.goal.y), (rec.z = rec.goal.z);
      else (rec.x = ease(rec.x, rec.goal.x, 0.9, dt)), (rec.y = ease(rec.y, rec.goal.y, 0.9, dt)), (rec.z = ease(rec.z, rec.goal.z, 0.9, dt));
      // Bobs gently, each on its own beat.
      const bob = pres.breathe * Math.sin(tt * 0.8 + rec.seed * 6.28) * 2.2;
      rec.group.position.set(rec.x, rec.y + bob, rec.z);

      rec.working = still ? (s.presence === "working" ? 1 : 0) : ease(rec.working, s.presence === "working" ? 1 : 0, 1.5, dt);
      rec.kick = still ? 0 : Math.max(0, rec.kick - adt / 1.4);
      // Lit by its presence, and by what's in focus.
      let light = pres.light;
      const mine = lighting.selectedId === rec.id;
      if (lighting.selectedId && !mine) light *= isActorId(lighting.selectedId) ? 0.5 : related(rec) ? 1 : 0.55;
      if (!mine) light *= 1 - 0.35 * lighting.focusLevel;
      if (lighting.dimmed && !mine && !matched(rec)) light *= 0.25;
      rec.light = still ? light : ease(rec.light, light, 4, dt);

      const breathe = 1 + depth * pres.breathe * Math.sin((tt / Math.max(0.5, period)) * Math.PI * 2 + rec.seed * 6.28);
      const focus = mine ? 1.22 : hoveredId === rec.id ? 1.12 : 1;
      rec.scale = still ? pres.scale * focus : ease(rec.scale, pres.scale * focus, 6, dt);
      rec.body.scale.setScalar(rec.scale * breathe);
      const react = look.react?.flash ?? 1.6;
      rec.glow.scale.setScalar(rec.glowSize * rec.scale * (0.92 + 0.08 * breathe + 0.35 * rec.kick * (react / 1.6)));
      rec.glow.material.opacity = Math.min(1, rec.glow.material.userData.base * rec.light * rec.light * (1 + rec.kick * react * 0.5));
      // Waiting on you (needsYou): its glow turns to the theme's attention
      // color and beats, as a session's does when it asks.
      const needs = !!s.needsYou && s.presence !== "offline";
      if (needs !== rec.needs) (rec.needs = needs), rec.glow.material.color.set(needs ? COLORS.attention : rec.color);
      if (needs) {
        const beat = 0.5 + 0.5 * Math.sin(tt * 4);
        rec.glow.scale.multiplyScalar(1.35 + 0.25 * beat);
        rec.glow.material.opacity = Math.max(rec.glow.material.opacity, 0.55 + 0.35 * beat);
      }
      rec.label.material.opacity = Math.min(1, 0.25 + rec.light) * (mine ? 1 : lighting.labelFade ?? 1);

      // Which way its tendrils (or anything else) lean: toward its first beam.
      const first = current.find((b) => b.actor === rec.id);
      const node = first && nodeOf(first.node);
      rec.hasReach = !!(node && Number.isFinite(node.x));
      if (rec.hasReach) rec.reach.set(node.x - rec.x, node.y - rec.y, node.z - rec.z).normalize();
      rec.form.update(frame(rec, tt, adt));
      stepGate(rec, tt, adt, dt, still);
    }

    drawBeams(t, adt, dt, still);
    drawPulses(adt);
    for (let i = ripples.length - 1; i >= 0; i--) {
      const r = ripples[i];
      r.t += adt / 1.3;
      if (r.t >= 1 || still) {
        layer.remove(r.s);
        ripplePool.push(r.s);
        ripples.splice(i, 1);
        continue;
      }
      r.s.scale.setScalar(r.r * (2 + 9 * (1 - (1 - r.t) ** 3)) * Math.min(1.6, r.flash / 1.2));
      r.s.material.opacity = (1 - r.t) * 0.85;
    }
    stats.stepMs = performance.now() - t0;
  }

  // Its gate, if it reaches XenoNet through a tunnel: made when it first
  // does, eased to its place, turned to face the map, and let go once
  // it's faded after its tunnel went.
  const G = { t: 0, dt: 0, light: 1, want: 0, from: new THREE.Vector3(), to: null, view: 900, still: false };
  function stepGate(rec, t, adt, dt, still) {
    const want = gateOf(rec.snapshot);
    if (want != null && !rec.gate) makeGate(rec), (placeAt = 0);
    const gate = rec.gate;
    if (!gate) return;
    const p = rec.gatePos, g = rec.gateGoal;
    if (still) p.copy(g), rec.gateFace.copy(rec.gateDir);
    else (p.x = ease(p.x, g.x, 0.9, dt)), (p.y = ease(p.y, g.y, 0.9, dt)), (p.z = ease(p.z, g.z, 0.9, dt)), rec.gateFace.lerp(rec.gateDir, 1 - Math.exp(-2 * dt)).normalize();
    gate.group.position.copy(p);
    gate.group.quaternion.setFromUnitVectors(FACING, rec.gateFace);
    gate.group.scale.setScalar(rec.scale);
    // The conduit leaves from the edge of its body, toward the gate.
    G.from.subVectors(p, rec.group.position);
    const d = G.from.length();
    G.from.multiplyScalar(d > 1 ? Math.min(0.45, (rec.size * 0.9 * rec.scale) / d) : 0).add(rec.group.position);
    G.t = t + rec.seed * 10;
    G.dt = adt;
    G.light = rec.light;
    G.want = want ?? 0;
    G.to = p;
    G.view = view;
    G.still = still;
    gate.update(G);
    if (want == null && gate.shown < 0.02) dropGate(rec);
  }

  const F = { t: 0, dt: 0, light: 1, working: 0, kick: 0, camera, view: 900, reach: null };
  function frame(rec, t, dt) {
    F.t = t + rec.seed * 10;
    F.dt = dt;
    F.light = rec.light;
    F.working = rec.working;
    F.kick = rec.kick;
    F.view = view;
    F.reach = rec.hasReach ? rec.reach : null;
    return F;
  }

  function drawBeams(t, adt, dt, still) {
    const pos = posBuf.array, col = colBuf.array;
    const style = THEME.actors.beam?.style ?? "pulse";
    const speed = THEME.actors.beam?.speed ?? 1;
    // The theme's width is XenoNet's lines' (1 is a link's), a little wider
    // on screen so a beam reads over them.
    beamLines.material.linewidth = (THEME.actors.beam?.width ?? 1.4) * 1.6;
    let seg = 0;
    const nowMs = performance.now();
    for (const beam of beams.values()) {
      beam.level = still ? beam.strength : ease(beam.level, beam.strength, beam.strength > beam.level ? 3 : 1.6, dt);
      if (beam.strength === 0 && beam.level < 0.01) {
        beams.delete(beam.key);
        continue;
      }
      if (seg + SEG > SLOTS * SEG) continue;
      const rec = records.get(beam.actor), node = nodeOf(beam.node);
      if (!rec || !node || !ends(beam, A, B)) continue;
      const nodeLight = node.__obs?.userData.dim ?? 1;
      // Through a shut tunnel, what it watches shows dimmer.
      const base = beam.level * Math.min(rec.light, nodeLight) * (rec.gate ? 0.45 + 0.55 * rec.gate.level : 1);
      if (base < 0.004) continue;
      // A bolt's jags, redrawn ten times a second.
      if (style === "bolt" && adt && nowMs - beam.jitterAt > 90) {
        beam.jitterAt = nowMs;
        for (let i = 1; i < SEG; i++) beam.jitter[i] = (Math.random() - 0.5) * 2;
        beam.flicker = 0.55 + Math.random() * 0.45;
      }
      const len = A.distanceTo(B);
      along(0, A, B, P);
      for (let i = 0; i < SEG; i++) {
        const u0 = i / SEG, u1 = (i + 1) / SEG;
        along(u1, A, B, Q);
        let k0 = 1, k1 = 1;
        if (style === "stream") {
          // Light streaming from the actor down to the node.
          const w = (u) => 0.38 + 0.62 * Math.max(0, Math.sin(Math.PI * 2 * (u * 4.5 - t * speed * 0.9 + beam.seed))) ** 4;
          (k0 = w(u0)), (k1 = w(u1));
        } else if (style === "pulse") (k0 = 0.6), (k1 = 0.6);
        else if (style === "bolt") {
          const j = len * 0.025;
          if (i > 0) P.y += beam.jitter[i] * j;
          if (i + 1 < SEG) Q.y += beam.jitter[i + 1] * j;
          k0 = k1 = beam.flicker ?? 0.8;
        }
        const o = seg * 6;
        pos[o] = P.x, pos[o + 1] = P.y, pos[o + 2] = P.z;
        pos[o + 3] = Q.x, pos[o + 4] = Q.y, pos[o + 5] = Q.z;
        const c = rec.color;
        const a0 = base * k0, a1 = base * k1;
        col[o] = c.r * a0, col[o + 1] = c.g * a0, col[o + 2] = c.b * a0;
        col[o + 3] = c.r * a1, col[o + 4] = c.g * a1, col[o + 5] = c.b * a1;
        P.copy(Q);
        seg++;
      }
      // The pulse style sends light down every so often while it works.
      if (style === "pulse" && adt && beam.level > 0.3) {
        beam.beat -= adt * speed * (0.35 + 0.65 * rec.working);
        if (beam.beat <= 0) (beam.beat = 1.7), pulse(beam, { size: 1.9, speed: 0.55 * speed });
      }
    }
    beamGeo.instanceCount = seg;
    if (seg) {
      posBuf.clearUpdateRanges();
      posBuf.addUpdateRange(0, seg * 6);
      posBuf.needsUpdate = true;
      colBuf.clearUpdateRanges();
      colBuf.addUpdateRange(0, seg * 6);
      colBuf.needsUpdate = true;
    }
  }

  function drawPulses(adt) {
    const pos = pulses.geometry.attributes.position.array, col = pulses.geometry.attributes.color.array, size = pulses.geometry.attributes.size.array;
    let n = 0;
    for (let i = live.length - 1; i >= 0; i--) {
      const p = live[i];
      p.u += adt * p.speed;
      const beam = p.beam;
      const rec = records.get(beam.actor);
      if (p.u >= 1 || !rec || !beams.has(beam.key) || !ends(beam, A, B)) {
        // Light that reaches a node lands there with a flash.
        if (p.u >= 1 && p.land && rec) {
          const node = nodeOf(beam.node);
          if (node?.__obs) flash(node, `#${rec.color.getHexString()}`, 10, 1.3);
        }
        live.splice(i, 1);
        continue;
      }
      if (p.u < 0) continue;
      along(p.u, A, B, P);
      const fade = Math.sin(Math.PI * Math.min(1, p.u)) * Math.min(1, beam.level * 1.5) * Math.min(1, rec.light * 1.4);
      pos[n * 3] = P.x, pos[n * 3 + 1] = P.y, pos[n * 3 + 2] = P.z;
      tmpColor.copy(rec.color).lerp(WHITE, 0.35).multiplyScalar(fade);
      col[n * 3] = tmpColor.r, col[n * 3 + 1] = tmpColor.g, col[n * 3 + 2] = tmpColor.b;
      size[n] = p.size;
      n++;
    }
    pulses.geometry.setDrawRange(0, n);
    if (n) {
      pulses.geometry.attributes.position.needsUpdate = true;
      pulses.geometry.attributes.color.needsUpdate = true;
      pulses.geometry.attributes.size.needsUpdate = true;
    }
  }

  // ---------- picking ----------

  const sphere = new THREE.Sphere(), hit = new THREE.Vector3();
  // The nearest actor the ray passes through, with its distance, or null.
  function pick(ray) {
    let best = null;
    for (const rec of records.values()) {
      if (!rec.group.visible) continue;
      sphere.set(rec.group.position, rec.size * 1.7 * rec.scale);
      if (ray.intersectSphere(sphere, hit)) {
        const d = hit.distanceTo(ray.origin);
        if (!best || d < best.distance) best = { rec, distance: d };
      }
      // Its gate stands for it too.
      if (!rec.gate || rec.gate.shown < 0.3) continue;
      sphere.set(rec.gatePos, rec.size * 2.1 * rec.scale);
      if (!ray.intersectSphere(sphere, hit)) continue;
      const d = hit.distanceTo(ray.origin);
      if (!best || d < best.distance) best = { rec, distance: d, gate: true };
    }
    return best;
  }

  // ---------- a change of theme ----------

  // Every actor in the new theme's form and colors, and its name in its
  // font; what was drawn before is disposed. Beams and pulses on their
  // way keep going, in the new colors.
  function retheme() {
    dropAtlases();
    for (const rec of records.values()) unbuild(rec), build(rec);
  }

  function clearBursts() {
    live.length = 0;
    pulses.geometry.setDrawRange(0, 0);
    for (const r of ripples.splice(0)) layer.remove(r.s), ripplePool.push(r.s);
    for (const rec of records.values()) rec.kick = 0;
  }

  return {
    set,
    event,
    message,
    read,
    step,
    pick,
    retheme,
    clearBursts,
    get: (id) => records.get(id) ?? null,
    records: () => [...records.values()],
    // Beams drawn now, as { actor, node, relation, level }.
    beams: () => [...beams.values()].filter((b) => b.level > 0.02 && b.strength > 0).map((b) => ({ actor: b.actor, node: b.node, relation: b.relation, level: Math.round(b.level * 100) / 100 })),
    // Tunnels drawn now, as { actor, open, shown, at }: how open its gate
    // is (1 with an adapter connected, a sliver without), how much of it
    // shows, and where it is.
    tunnels: () => [...records.values()].filter((r) => r.gate).map((r) => ({ actor: r.id, open: Math.round(r.gate.level * 100) / 100, shown: Math.round(r.gate.shown * 100) / 100, at: [r.gatePos.x, r.gatePos.y, r.gatePos.z].map(Math.round) })),
    // What it's acting on now: its beams' nodes.
    targetsOf: (id) => current.filter((b) => b.actor === id).map((b) => b.node),
    positions: () => [...records.values()].filter((r) => r.placed).map((r) => new THREE.Vector3(r.x, r.y, r.z)),
    select(id) {
      selectedId = id && records.has(id) ? id : null;
    },
    hover(id) {
      hoveredId = id && records.has(id) ? id : null;
    },
    // Made-up actors (actor/snapshot/v1s) drawn instead of the list until
    // it's given null, so the benchmark measures a cast that holds still;
    // events for actors in it still reach them.
    setCast(actors) {
      cast = Array.isArray(actors) ? actors : null;
      apply(cast ? { actors: cast } : lastList);
    },
    // The replay's clock while it plays (see replay.js), or null for now.
    setClock(fn) {
      clock = fn;
      currentAt = 0;
      placeAt = 0;
    },
    // On or off, for the benchmark's comparison: off draws and steps nothing.
    setEnabled(on) {
      enabled = !!on;
      layer.visible = enabled;
      if (!enabled) clearBursts();
    },
    get enabled() {
      return enabled;
    },
    stats: () => ({ actors: records.size, beams: beamGeo.instanceCount / SEG, pulses: live.length, gates: [...records.values()].filter((r) => r.gate).length, stepMs: Math.round(stats.stepMs * 1000) / 1000 }),
    dispose() {
      for (const rec of [...records.values()]) remove(rec);
      beamGeo.dispose();
      releaseMaterial(beamLines.material);
      pulses.geometry.dispose();
      pulses.material.dispose();
      for (const s of [...ripplePool, ...ripples.map((r) => r.s)]) s.material.dispose();
      dropAtlases();
      scene.remove(layer);
    },
  };
}
