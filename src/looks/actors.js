// How an actor is drawn, by form (theme.actors.form, and actors.human.form
// for you; contracts section 4): an orb, a block cursor in a ring of memory
// cells, a crystal that grows facets, a small star whose corona flares, a
// swarm that gathers and scatters, or a jellyfish trailing tendrils. The
// actors layer (src/actors.js) adds the glow round each one and its name,
// and moves, lights and disposes what a form builds:
//
//   build({ color, size, theme, params, seed, human, chars })
//       a form for one actor: { group, update(f), react(), dispose() }.
//       color is a THREE.Color, size the body's radius in world units,
//       params the theme's knobs for the form (actors.params), seed a
//       number from the actor's id so no two move in step, and chars(text)
//       the layer's atlas of those characters (looks/chars.js), for the
//       forms that draw them.
//   update(f)  each frame, with f = { t, dt, light, working, kick, camera,
//              view, reach }: the ambient clock and frame time (0 while
//              held still), how bright it is (0 to 1), how much it's at
//              work (0 to 1, eased), how much of a reaction is left (1 when
//              it acts, falling to 0), the camera, the drawing buffer's
//              pixels per world unit at a distance of 1 (for sizing point
//              sprites), and the direction to what it acts on in its own
//              frame, or null. Allocates nothing.
//   react()    it just did something: a form may flip or reshuffle.
//   dispose()  frees what build made.
//   glow       how much of the layer's glow to draw round it (1 when left
//              out); a form with a glow of its own, like the flame's
//              corona, wants less.
//   top        how far up it reaches, in sizes, when that's further than
//              its name usually sits (2.2).
//
// The first form draws any form this XenoNet doesn't know. A knob that turns
// something on (flip, grow, flares, pulse) is on unless a theme sets it to
// false.
import * as THREE from "three";
import { Additive, fatEdges, fatLine, writeLine, releaseMaterial, fresnelMaterial } from "../fx.js";
import { CHAR_SPRITE } from "./chars.js";

const WHITE = new THREE.Color("#ffffff");
const lighter = (color, k) => color.clone().lerp(WHITE, k);

// Materials that dim with the actor, each from its own opacity. Glows add
// up, so additive ones dim with the square, as nodes do (setDim).
function lights() {
  const list = [];
  const add = (m, base = m.uniforms?.opacity?.value ?? m.opacity) => {
    m.userData.base = base;
    list.push(m);
    return m;
  };
  let last = -1;
  const set = (light) => {
    if (Math.abs(light - last) < 0.004) return;
    last = light;
    for (const m of list) {
      const f = m.blending === Additive ? light * light : light;
      if (m.uniforms?.opacity) m.uniforms.opacity.value = m.userData.base * f;
      else m.opacity = m.userData.base * f;
    }
  };
  return { add, set };
}

const basic = (color, opacity = 1, additive = false) => new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthWrite: !additive, blending: additive ? Additive : THREE.NormalBlending });

function disposeAll(group) {
  group.traverse((o) => {
    o.geometry?.dispose();
    if (!o.material) return;
    if (o.material.isLineMaterial) releaseMaterial(o.material);
    else o.material.dispose();
  });
}

// ---------- orb ----------
// A luminous core in a fresnel shell, ringed. With params.eclipse, a black
// disc in a white corona; params.rings sets how many rings (1).
function orb({ color, size, params = {}, seed, human }) {
  const group = new THREE.Group();
  const l = lights();
  const eclipse = !!params.eclipse;
  const core = new THREE.Mesh(new THREE.SphereGeometry(size * (eclipse ? 0.82 : 0.5), 32, 20), eclipse ? new THREE.MeshBasicMaterial({ color: "#000000" }) : l.add(basic(lighter(color, 0.35), 1)));
  const shell = new THREE.Mesh(new THREE.SphereGeometry(size * (eclipse ? 0.95 : 0.86), 32, 20), l.add(fresnelMaterial(eclipse ? WHITE : color, eclipse ? 1.6 : 0.95, eclipse ? 1.4 : 2.2)));
  group.add(core, shell);
  const count = Math.max(0, Math.min(4, Math.round(params.rings ?? (eclipse ? 0 : 1))));
  const rings = [];
  for (let i = 0; i < count; i++) {
    const ring = new THREE.Mesh(new THREE.TorusGeometry(size * (1.55 + i * 0.32), size * (human ? 0.03 : 0.045), 8, 120), l.add(basic(lighter(color, 0.15), human ? 0.55 : 0.8, true)));
    ring.rotation.set(1.15 + i * 0.55 + seed, seed * 3 + i * 0.9, 0);
    rings.push(ring);
    group.add(ring);
  }
  return {
    group,
    update({ dt, light, working, kick }) {
      l.set(light * (1 + 0.5 * kick));
      for (let i = 0; i < rings.length; i++) {
        rings[i].rotation.z += dt * (0.25 + 0.6 * working + 2.5 * kick) * (i % 2 ? -1 : 1);
        rings[i].rotation.x += dt * 0.05;
      }
      if (eclipse) shell.material.uniforms.power.value = 1.4 - 0.5 * kick;
    },
    react() {},
    dispose: () => disposeAll(group),
  };
}

// ---------- cursor ----------
// A block cursor at its heart, blinking, faster while it works, in a
// turning ring of memory cells, full or empty, which flip while it works
// (params.flip) and all at once when it acts; params.cells is how many (8).
// A tunnel's gate (gate.js) rings itself with characters the same way.
const CELLS = "▮▯"; // a full cell, which is also the cursor, and an empty one
export function charPoints({ count, radius, size, color, atlas, cursor }) {
  const pos = new Float32Array(count * 3), cell = new Float32Array(count), scale = new Float32Array(count);
  // With a cursor, it's the first, at the heart, and the rest ring it.
  const ring = cursor ? count - 1 : count;
  for (let i = 0; i < count; i++) {
    const a = ((cursor ? i - 1 : i) / ring) * Math.PI * 2;
    pos.set(cursor && i === 0 ? [0, 0, 0] : [Math.cos(a) * radius, 0, Math.sin(a) * radius], i * 3);
    scale[i] = cursor && i === 0 ? 2.3 : 1;
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geo.setAttribute("cell", new THREE.BufferAttribute(cell, 1));
  geo.setAttribute("scale", new THREE.BufferAttribute(scale, 1));
  const mat = new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    uniforms: { atlas: { value: atlas.texture }, grid: { value: atlas.grid }, color: { value: color }, opacity: { value: 1 }, size: { value: size }, view: { value: 900 }, blink: { value: 1 } },
    vertexShader: /* glsl */ `
      attribute float cell; attribute float scale;
      uniform float size; uniform float view; uniform float blink;
      varying float vCell; varying float vOn;
      void main() {
        vec4 mv = modelViewMatrix * vec4(position, 1.0);
        gl_PointSize = clamp(size * scale * view / max(-mv.z, 1.0), 1.0, 256.0);
        gl_Position = projectionMatrix * mv;
        vCell = cell;
        vOn = scale > 1.5 ? blink : 1.0;
      }`,
    fragmentShader: /* glsl */ `
      uniform vec3 color; uniform float opacity;
      varying float vCell; varying float vOn;
      ${CHAR_SPRITE}
      void main() {
        float a = charAt(vCell) * opacity * vOn;
        if (a < 0.01) discard;
        gl_FragColor = vec4(color * a, a);
      }`,
  });
  const points = new THREE.Points(geo, mat);
  points.frustumCulled = false;
  return points;
}

function cursor({ color, size, params = {}, seed, chars }) {
  const group = new THREE.Group();
  const l = lights();
  const atlas = chars(CELLS);
  const count = Math.max(3, Math.min(16, Math.round(params.cells ?? 8))) + 1;
  const core = new THREE.Mesh(new THREE.SphereGeometry(size * 0.22, 20, 14), l.add(basic(lighter(color, 0.5), 1)));
  const ring = charPoints({ count, radius: size * 1.55, size: size * 1.25, color: lighter(color, 0.25), atlas, cursor: true });
  l.add(ring.material);
  // A track through the cells, and a fainter one inside it.
  const track = new THREE.Mesh(new THREE.TorusGeometry(size * 1.55, size * 0.025, 6, 140), l.add(basic(color, 0.7, true)));
  const inner = new THREE.Mesh(new THREE.TorusGeometry(size * 0.95, size * 0.018, 6, 100), l.add(basic(color, 0.35, true)));
  track.rotation.x = inner.rotation.x = Math.PI / 2;
  const turn = new THREE.Group();
  turn.rotation.set(0.5 + seed * 0.6, 0, 0.2);
  turn.add(ring, track, inner);
  group.add(core, turn);
  const cells = ring.geometry.attributes.cell;
  // The cursor is always the full cell; the rest are full or empty.
  const pick = (i) => (i === 0 ? 0 : Math.floor(Math.random() * atlas.count));
  const flip = (all) => {
    for (let i = 0; i < count; i++) if (all || Math.random() < 0.34) cells.array[i] = pick(i);
    cells.needsUpdate = true;
  };
  flip(true);
  let since = 0;
  return {
    group,
    update({ t, dt, light, working, kick, view }) {
      l.set(light * (1 + 0.4 * kick));
      turn.rotation.y += dt * (0.18 + 0.5 * working + 3 * kick);
      ring.material.uniforms.view.value = view;
      // Flips a few cells at a time while it works.
      since += dt;
      if (params.flip !== false && working > 0.5 && since > 1.4 - working * 0.6) (since = 0), flip(false);
      // On two thirds of the time, faster while it works; on while held still.
      ring.material.uniforms.blink.value = !dt || Math.sin(t * (working > 0.5 ? 9 : 3.5) + seed * 6) > -0.5 ? 1 : 0.06;
    },
    react: () => flip(true),
    dispose: () => disposeAll(group),
  };
}

// ---------- crystal ----------
// A bipyramid of params.facets sides (4) with lit faces and bright edges,
// and an outer shell of twice the facets that grows round it while it works
// (params.grow).
function bipyramid(n, w, h) {
  const pos = [];
  const ring = Array.from({ length: n }, (_, i) => [Math.cos((i / n) * Math.PI * 2) * w, 0, Math.sin((i / n) * Math.PI * 2) * w]);
  for (let i = 0; i < n; i++) {
    const a = ring[i], b = ring[(i + 1) % n];
    pos.push(0, h, 0, ...b, ...a, 0, -h, 0, ...a, ...b);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  return geo;
}

function crystal({ color, size, params = {}, seed }) {
  const group = new THREE.Group();
  const l = lights();
  const n = Math.max(3, Math.min(12, Math.round(params.facets ?? 4)));
  const shape = bipyramid(n, size * 0.9, size * 1.45);
  const faces = new THREE.Mesh(shape, l.add(new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.18, depthWrite: false, blending: Additive, side: THREE.DoubleSide, forceSinglePass: true })));
  const edges = fatEdges(shape, { color: lighter(color, 0.3), width: 1.5, opacity: 0.95 });
  l.add(edges.material);
  const body = new THREE.Group();
  body.add(faces, edges);
  group.add(body);
  let outer = null;
  if (params.grow !== false) {
    const shellShape = bipyramid(n * 2, size * 1.35, size * 2.1);
    outer = fatEdges(shellShape, { color, width: 1, opacity: 0.5 });
    shellShape.dispose();
    l.add(outer.material);
    outer.scale.setScalar(0.6);
    group.add(outer);
  }
  let grown = 0.6;
  return {
    group,
    glow: 0.7,
    top: params.grow !== false ? 2.9 : 1.6,
    update({ dt, light, working, kick }) {
      l.set(light * (1 + 0.6 * kick));
      body.rotation.y += dt * (0.3 + 0.5 * working);
      body.rotation.x = 0.12 * Math.sin(seed * 6 + body.rotation.y * 0.5);
      if (outer) {
        grown += ((0.62 + 0.55 * working + 0.35 * kick) - grown) * Math.min(1, dt * 1.5);
        outer.scale.setScalar(grown);
        outer.rotation.y -= dt * (0.12 + 0.3 * working);
        outer.material.opacity = outer.material.userData.base * light * Math.min(1, Math.max(0, (grown - 0.62) / 0.4) + 0.15);
      }
    },
    react() {},
    dispose: () => disposeAll(group),
  };
}

// ---------- flame ----------
// A small star in a corona of flames that waver, params.corona times its
// size (1.4), flaring when it acts (params.flares).
function flame({ color, size, params = {}, seed }) {
  const group = new THREE.Group();
  const l = lights();
  const core = new THREE.Mesh(new THREE.SphereGeometry(size * 0.42, 24, 16), l.add(basic(lighter(color, 0.3), 1)));
  const corona = new THREE.Mesh(
    new THREE.PlaneGeometry(1, 1),
    l.add(
      new THREE.ShaderMaterial({
        transparent: true,
        depthWrite: false,
        blending: Additive,
        uniforms: { color: { value: color.clone() }, time: { value: 0 }, opacity: { value: 1 }, flare: { value: 0 }, seed: { value: seed * 40 } },
        vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
        fragmentShader: /* glsl */ `
          uniform vec3 color; uniform float time; uniform float opacity; uniform float flare; uniform float seed;
          varying vec2 vUv;
          float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
          float noise(vec2 p) {
            vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
            return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
          }
          void main() {
            vec2 p = vUv - 0.5;
            float r = length(p) * 2.0;
            float a = atan(p.y, p.x);
            // Flames: a reach that wavers round the rim, on two scales, cut
            // into tongues and rays that flicker.
            float n = 0.62 * noise(vec2(a * 2.5 + seed, time * 0.8)) + 0.38 * noise(vec2(a * 6.0 - seed, time * 1.9));
            float reach = 0.3 + 0.32 * n + 0.3 * flare;
            float rays = pow(noise(vec2(a * 15.0 + seed * 3.0, time * 1.3)), 1.5);
            float flames = smoothstep(reach, reach * 0.5, r) * (0.35 + 0.8 * rays);
            float glow = exp(-r * 5.5) * 0.7;
            float k = clamp(flames * 0.6 + glow, 0.0, 1.0) * smoothstep(1.0, 0.85, r);
            vec3 c = mix(color, vec3(1.0), smoothstep(0.22, 0.0, r) * 0.8);
            float alpha = k * opacity;
            gl_FragColor = vec4(c * alpha, alpha);
          }`,
      }),
    ),
  );
  const reach = size * 4.4 * (params.corona ?? 1.4);
  corona.scale.setScalar(reach);
  corona.renderOrder = 7;
  group.add(core, corona);
  let flare = 0;
  return {
    group,
    glow: 0.45,
    update({ t, dt, light, working, kick, camera }) {
      l.set(light);
      const u = corona.material.uniforms;
      u.time.value = t * (0.7 + 0.8 * working);
      flare += ((params.flares === false ? 0 : kick) + 0.12 * working - flare) * Math.min(1, dt * 6);
      u.flare.value = flare;
      if (camera) corona.quaternion.copy(camera.quaternion);
      corona.scale.setScalar(reach * (1 + 0.25 * flare));
    },
    react() {},
    dispose: () => disposeAll(group),
  };
}

// ---------- swarm ----------
// A cloud of motes swirling up a funnel, like a dust devil, which gathers
// while it works and scatters when idle or when it acts. They move in the
// vertex shader, so a swarm costs nothing each frame.
const MOTES = 90;
function swarm({ color, size, params = {}, seed }) {
  const group = new THREE.Group();
  const l = lights();
  const n = Math.max(12, Math.min(160, Math.round(params.motes ?? MOTES)));
  const pos = new Float32Array(n * 3), seeds = new Float32Array(n * 4);
  let x = Math.floor(seed * 1e6) || 7;
  const rnd = () => ((x = (x * 1103515245 + 12345) % 2147483648) / 2147483648);
  for (let i = 0; i < n; i++) seeds.set([rnd() * Math.PI * 2, rnd(), 0.4 + rnd() * 0.6, rnd()], i * 4);
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geo.setAttribute("seed", new THREE.BufferAttribute(seeds, 4));
  const mat = l.add(
    new THREE.ShaderMaterial({
      transparent: true,
      depthWrite: false,
      blending: Additive,
      uniforms: { time: { value: 0 }, gather: { value: 0.5 }, scale: { value: size }, color: { value: lighter(color, 0.2) }, opacity: { value: 1 }, view: { value: 900 } },
      vertexShader: /* glsl */ `
        attribute vec4 seed;
        uniform float time; uniform float gather; uniform float scale; uniform float view;
        varying float vA;
        void main() {
          float h = fract(seed.y + time * (0.05 + 0.05 * seed.w));
          float turn = seed.x + time * (0.7 + seed.w * 0.8) * (1.5 - h);
          // A funnel, narrow at its foot and wide at its top, that draws in
          // as it gathers.
          float r = scale * mix(2.6, 1.15, gather) * seed.z * (0.25 + 1.1 * h);
          vec3 p = vec3(cos(turn) * r, (h - 0.45) * scale * mix(4.2, 3.2, gather), sin(turn) * r);
          vec4 mv = modelViewMatrix * vec4(p, 1.0);
          gl_PointSize = clamp(scale * (0.42 + 0.4 * seed.w) * view / max(-mv.z, 1.0), 1.0, 64.0);
          gl_Position = projectionMatrix * mv;
          vA = (0.55 + 0.45 * sin(time * 2.3 + seed.x * 7.0)) * smoothstep(0.0, 0.12, h) * smoothstep(1.0, 0.8, h);
        }`,
      fragmentShader: /* glsl */ `
        uniform vec3 color; uniform float opacity;
        varying float vA;
        void main() {
          float d = length(gl_PointCoord - 0.5);
          float a = smoothstep(0.5, 0.0, d) * vA * opacity;
          gl_FragColor = vec4(color * a, a);
        }`,
    }),
  );
  const motes = new THREE.Points(geo, mat);
  motes.frustumCulled = false;
  const core = new THREE.Mesh(new THREE.SphereGeometry(size * 0.24, 16, 12), l.add(basic(lighter(color, 0.5), 0.9)));
  group.add(core, motes);
  let gather = 0.5;
  return {
    group,
    update({ t, dt, light, working, kick, view }) {
      l.set(light);
      const u = mat.uniforms;
      u.time.value = t;
      gather += ((0.25 + 0.75 * working) * (1 - kick) - gather) * Math.min(1, dt * 1.8);
      u.gather.value = gather;
      u.view.value = view;
    },
    react() {},
    dispose: () => disposeAll(group),
  };
}

// ---------- jelly ----------
// A bell whose rim pulses as it breathes, trailing params.tendrils tendrils
// (5) that drift toward what it acts on.
const SEGS = 12;
function jelly({ color, size, params = {}, seed }) {
  const group = new THREE.Group();
  const l = lights();
  const bell = new THREE.Group();
  const dome = new THREE.Mesh(new THREE.SphereGeometry(size, 32, 12, 0, Math.PI * 2, 0, Math.PI / 2), l.add(fresnelMaterial(color, 1.15, 1.5)));
  const inner = new THREE.Mesh(new THREE.SphereGeometry(size * 0.62, 24, 10, 0, Math.PI * 2, 0, Math.PI / 2), l.add(basic(lighter(color, 0.4), 0.42, true)));
  bell.add(dome, inner);
  const count = Math.max(1, Math.min(12, Math.round(params.tendrils ?? 5)));
  const strands = new THREE.Group();
  const lines = [];
  const pts = Array.from({ length: SEGS + 1 }, () => new THREE.Vector3());
  for (let i = 0; i < count; i++) {
    const line = fatLine(SEGS, { color, width: 1.1, opacity: 0.7 });
    l.add(line.material);
    lines.push(line);
    strands.add(line);
  }
  group.add(bell, strands);
  const down = new THREE.Vector3(0, -1, 0), toward = new THREE.Vector3();
  return {
    group,
    update({ t, light, kick, reach }) {
      l.set(light);
      // The bell contracts and springs back: a pulse each breath, a strong
      // one when it acts.
      const pulse = Math.max(0, Math.sin(t * 1.6 + seed * 6)) ** 2 * (params.pulse === false ? 0 : 1) + kick * 0.8;
      bell.scale.set(1 + 0.1 * pulse, 1 - 0.2 * pulse, 1 + 0.1 * pulse);
      toward.copy(reach ?? down).lerp(down, reach ? 0.55 : 0).normalize();
      const len = size * 3.2;
      for (let i = 0; i < lines.length; i++) {
        const a = (i / lines.length) * Math.PI * 2 + seed;
        const bx = Math.cos(a) * size * 0.72, bz = Math.sin(a) * size * 0.72;
        for (let k = 0; k <= SEGS; k++) {
          const s = k / SEGS, sway = Math.sin(t * 1.3 + i * 1.7 + s * 3.4) * s * size * 0.45;
          pts[k].set(bx * (1 - s * 0.35) + toward.x * len * s + Math.cos(a + 1.6) * sway, -size * 0.1 + toward.y * len * s, bz * (1 - s * 0.35) + toward.z * len * s + Math.sin(a + 1.6) * sway);
        }
        writeLine(lines[i], pts);
      }
    },
    react() {},
    dispose: () => disposeAll(group),
  };
}

export const FORMS = { orb, cursor, crystal, flame, swarm, jelly };
export const formNamed = (name) => FORMS[name] ?? FORMS.orb;
