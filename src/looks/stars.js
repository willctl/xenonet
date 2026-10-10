// The stars (theme.stars): what fills the sky around the map, with its
// tints, twinkle and shape: points of light, falling snow or drifting
// motes. Each style:
//   build(theme)                  the object to add to the scene
//   update(object, { t, dim })    each frame: the ambient clock, and how
//                                 dim the sky is at the end of the day
//   dispose(object)               frees what build made
import * as THREE from "three";
import { Additive } from "../fx.js";

// Numbers from 0 to 1 that come out the same for the same seed (mulberry32),
// so a theme draws the same sky whenever it's drawn: switching away and back
// changes nothing.
function seeded(text) {
  let a = [...text].reduce((h, c) => Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0, 2166136261);
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const howMany = (count) => Math.max(0, Math.min(20000, Math.round(count)));
const clamp01 = (x) => Math.min(1, Math.max(0, x));

// A point spread evenly over the sphere, then out into the shell around the
// map where stars are.
function inShell(random, out, i) {
  const y = random() * 2 - 1, turn = random() * Math.PI * 2, across = Math.sqrt(1 - y * y), r = 1100 + random() * 2400;
  out.set([Math.cos(turn) * across * r, y * r, Math.sin(turn) * across * r], i * 3);
}

function attributes(geo, attrs) {
  for (const [name, [array, size]] of Object.entries(attrs)) geo.setAttribute(name, new THREE.BufferAttribute(array, size));
  return geo;
}

const disposeAll = (object) =>
  object.traverse((o) => {
    o.geometry?.dispose();
    o.material?.dispose();
  });

// Snow and motes drift through a box around the map, wrapping round as
// they leave it, and fade out near the middle so the map stays clear: in
// the vertex shader, so moving them costs nothing each frame.
const SPAN = 1600, CLEAR = 420;
const DRIFTING = /* glsl */ `
  attribute float size; attribute float phase; attribute vec3 tint;
  uniform float time; uniform float pixelRatio; uniform float twinkle; uniform vec3 drift; uniform float sway;
  varying vec3 vTint; varying float vLight; varying float vSpin;
  vec3 drifted() {
    vec3 p = mod(position + drift * time + ${SPAN.toFixed(1)}, ${(2 * SPAN).toFixed(1)}) - ${SPAN.toFixed(1)};
    p.x += sin(time * 0.31 + phase) * sway;
    p.z += cos(time * 0.27 + phase * 1.7) * sway;
    p.y += sin(time * 0.23 + phase * 0.6) * sway * 0.4;
    return p;
  }
  void place(vec3 p, float most) {
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    float far = max(-mv.z, 1.0);
    gl_PointSize = clamp(size * pixelRatio * 900.0 / far, 1.0, most * pixelRatio);
    gl_Position = projectionMatrix * mv;
    // Clear of the map in the middle, and of the camera.
    vLight = smoothstep(${(CLEAR * 0.6).toFixed(1)}, ${CLEAR.toFixed(1)}, length(p)) * smoothstep(30.0, 120.0, far);
    vTint = tint;
  }
`;

function driftingMaterial(theme, { fragmentShader, flake = false }) {
  const { twinkle, drift } = theme.stars;
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    uniforms: {
      time: { value: 0 },
      pixelRatio: { value: Math.min(devicePixelRatio, 2) },
      dim: { value: 1 },
      twinkle: { value: clamp01(twinkle) },
      drift: { value: new THREE.Vector3(...drift) },
      sway: { value: flake ? 9 : 22 },
    },
    vertexShader: /* glsl */ `
      ${DRIFTING}
      void main() {
        vec3 p = drifted();
        place(p, ${flake ? "34.0" : "24.0"});
        vSpin = phase + time * (fract(phase * 3.7) - 0.5) * 0.8;
        ${
          flake
            ? "vLight *= 1.0 - twinkle * 0.6 * (0.5 + 0.5 * sin(time * (0.8 + fract(phase) * 2.0) + phase));"
            : // Motes glow on and off, like things that make their own light.
              "vLight *= 1.0 - twinkle + twinkle * pow(0.5 + 0.5 * sin(time * (0.25 + fract(phase) * 0.6) + phase), 3.0);"
        }
      }`,
    fragmentShader,
  });
}

function driftingPoints(theme, seed, sizes, material) {
  const n = howMany(theme.stars.count);
  const pos = new Float32Array(n * 3), size = new Float32Array(n), phase = new Float32Array(n), tint = new Float32Array(n * 3);
  const palette = theme.stars.tints.map((c) => new THREE.Color(c));
  const random = seeded(`${theme.id}:${seed}:${n}`);
  for (let i = 0; i < n; i++) {
    pos.set([(random() * 2 - 1) * SPAN, (random() * 2 - 1) * SPAN, (random() * 2 - 1) * SPAN], i * 3);
    size[i] = sizes(random);
    phase[i] = random() * 100;
    tint.set(palette[(random() * palette.length) | 0].toArray(), i * 3);
  }
  const geo = attributes(new THREE.BufferGeometry(), { position: [pos, 3], size: [size, 1], phase: [phase, 1], tint: [tint, 3] });
  // They move in the shader, so the bounds the CPU knows of don't hold.
  const points = new THREE.Points(geo, material);
  points.frustumCulled = false;
  return points;
}

const tick = (object, { t, dim }) => {
  object.material.uniforms.time.value = t;
  object.material.uniforms.dim.value = dim;
};

export const STARS = {
  // Twinkling points in a shell around the map, a few of them large. At a
  // twinkle of 0.5 each breathes between 10% and 100%, as they always have.
  point: {
    build(theme) {
      const { count, tints, twinkle } = theme.stars;
      const n = howMany(count);
      const pos = new Float32Array(n * 3);
      const size = new Float32Array(n);
      const phase = new Float32Array(n);
      const tint = new Float32Array(n * 3);
      const palette = tints.map((c) => new THREE.Color(c));
      const random = seeded(`${theme.id}:${n}`);
      for (let i = 0; i < n; i++) {
        inShell(random, pos, i);
        size[i] = random() < 0.03 ? 7 + random() * 5 : 1.5 + random() * 3;
        phase[i] = random() * 100;
        tint.set(palette[(random() * palette.length) | 0].toArray(), i * 3);
      }
      const geo = attributes(new THREE.BufferGeometry(), { position: [pos, 3], size: [size, 1], phase: [phase, 1], tint: [tint, 3] });
      const mat = new THREE.ShaderMaterial({
        transparent: true,
        depthWrite: false,
        blending: Additive,
        uniforms: { time: { value: 0 }, pixelRatio: { value: Math.min(devicePixelRatio, 2) }, dim: { value: 1 }, twinkle: { value: 0.9 * clamp01(twinkle) } },
        vertexShader: /* glsl */ `
          attribute float size; attribute float phase; attribute vec3 tint;
          uniform float time; uniform float pixelRatio; uniform float twinkle;
          varying vec3 vTint; varying float vTw;
          void main() {
            vec4 mv = modelViewMatrix * vec4(position, 1.0);
            vTw = max(0.0, 1.0 - twinkle + twinkle * sin(time * (0.6 + fract(phase) * 1.8) + phase));
            vTint = tint;
            gl_PointSize = size * pixelRatio * (0.8 + 0.4 * vTw);
            gl_Position = projectionMatrix * mv;
          }`,
        fragmentShader: /* glsl */ `
          uniform float dim; varying vec3 vTint; varying float vTw;
          void main() {
            vec2 c = gl_PointCoord - 0.5; float d = length(c);
            float a = smoothstep(0.5, 0.0, d); a *= a;
            gl_FragColor = vec4(vTint * a * vTw * 0.9 * dim, a * dim);
          }`,
      });
      return new THREE.Points(geo, mat);
    },
    update: tick,
    dispose: disposeAll,
  },

  // Snow falling through the dark (theme.stars.drift, units a second),
  // each flake turning as it sways; the far ones are only specks.
  flake: {
    build(theme) {
      const material = driftingMaterial(theme, {
        flake: true,
        fragmentShader: /* glsl */ `
          uniform float dim; varying vec3 vTint; varying float vLight; varying float vSpin;
          void main() {
            vec2 p = gl_PointCoord - 0.5;
            float c = cos(vSpin), s = sin(vSpin);
            p = mat2(c, s, -s, c) * p;
            float r = length(p), a = atan(p.y, p.x);
            // Six arms with a little branching, and a bright middle.
            float arm = pow(abs(cos(3.0 * a)), 24.0) * smoothstep(0.5, 0.1, r);
            float branch = pow(abs(cos(3.0 * a + 1.5708)), 10.0) * smoothstep(0.03, 0.0, abs(r - 0.26)) * 0.7;
            float k = max(arm, branch) + smoothstep(0.14, 0.0, r);
            gl_FragColor = vec4(vTint, k * vLight * dim * 0.9);
          }`,
      });
      return driftingPoints(theme, "flakes", (random) => (random() < 0.08 ? 16 + random() * 10 : 5 + random() * 7), material);
    },
    update: tick,
    dispose: disposeAll,
  },

  // Soft motes adrift (theme.stars.drift), meandering, each glowing up and
  // fading on its own slow beat: marine snow, spores, dust in a beam.
  mote: {
    build(theme) {
      const material = driftingMaterial(theme, {
        fragmentShader: /* glsl */ `
          uniform float dim; varying vec3 vTint; varying float vLight;
          void main() {
            float d = length(gl_PointCoord - 0.5);
            gl_FragColor = vec4(vTint, exp(-d * d * 18.0) * vLight * dim * 0.8);
          }`,
      });
      return driftingPoints(theme, "motes", (random) => (random() < 0.06 ? 10 + random() * 8 : 3 + random() * 5), material);
    },
    update: tick,
    dispose: disposeAll,
  },
};
