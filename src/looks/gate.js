// A remote actor's tunnel, drawn as a gate (docs/guide/cloud-assistants.md):
// an actor that reaches XenoNet through a tunnel, such as a dot through
// OpenAI's Secure MCP Tunnel, isn't on this computer, so it doesn't touch
// the map directly. A conduit runs from it down to a ring between it and
// the map, with hex digits round its rim, and its beams go out through the
// ring. The ring's iris is open while an adapter is connected through the
// tunnel, a sliver while none is, and each read sends light down the
// conduit that flares the iris on its way through and changes some of the
// digits. The actors layer (src/actors.js) places it, and passes what it
// needs each frame:
//
//   buildGate({ color, size, style, seed, chars })
//       color is the actor's THREE.Color, size its body's radius, style the
//       theme's beam style (pulse, stream or bolt: a stream's iris spirals
//       faster, a bolt's crackles), and chars(text) the layer's atlas of
//       those characters (looks/chars.js).
//   group      the ring and iris, which the layer places at the gate and
//              turns to face the map (its +Z axis).
//   conduit    a line in the layer's own space, from the actor to the gate.
//   update(f)  each frame, with f = { t, dt, light, want, from, to, view,
//              still }: the clock, frame time, how bright its actor is, how
//              open it should be (1 open, a sliver for standby, 0 shut), and
//              where the actor and the gate are. Allocates nothing.
//   read()     light down the conduit; returns how long it takes to reach
//              the gate, in seconds, so beams light up after it.
//   level      how open it is now, eased (0 to 1); `shown` how much of it
//              shows at all, which fades out when its tunnel goes.
//   dispose()  frees what it made.
import * as THREE from "three";
import { Additive, fatLine, writeLine, releaseMaterial, glowTex } from "../fx.js";
import { charPoints } from "./actors.js";

const WHITE = new THREE.Color("#ffffff");
const CONDUIT = 18; // pieces of the conduit
const PACKETS = 4;
const TRAVEL = 0.5; // seconds down the conduit
const HEX = "0123456789ABCDEF";

const ease = (from, to, rate, dt) => from + (to - from) * (1 - Math.exp(-rate * dt));

function irisMaterial(color, style, seed) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    side: THREE.DoubleSide,
    uniforms: {
      color: { value: color.clone() },
      time: { value: 0 },
      open: { value: 0 },
      flare: { value: 0 },
      opacity: { value: 1 },
      seed: { value: seed * 40 },
      arms: { value: style === "stream" ? 5 : 3 },
      spin: { value: style === "stream" ? 1.8 : 0.8 },
      crackle: { value: style === "bolt" ? 1 : 0 },
    },
    vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform vec3 color; uniform float time; uniform float open; uniform float flare; uniform float opacity;
      uniform float seed; uniform float arms; uniform float spin; uniform float crackle;
      varying vec2 vUv;
      float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
      float noise(vec2 p) {
        vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
        return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
      }
      void main() {
        vec2 p = (vUv - 0.5) * 2.0;
        float r = length(p);
        float a = atan(p.y, p.x);
        // The iris reaches out to the rim as it opens.
        float rad = mix(0.12, 0.9, open);
        float inside = smoothstep(rad, rad - 0.07, r);
        if (inside <= 0.0) discard;
        float s = r / max(rad, 0.01);
        // A vortex: arms twisting in toward the middle, turning.
        float swirl = pow(0.5 + 0.5 * sin(arms * a + 7.0 * s - time * spin + seed), 3.0);
        float vortex = 0.16 + 0.6 * swirl * smoothstep(0.0, 0.8, s);
        // A bolt's crackle: sparks that jump round the iris.
        float crack = crackle * step(0.8, noise(vec2(a * 5.0 + seed, time * 9.0))) * smoothstep(0.35, 1.0, s) * 0.9;
        float core = exp(-s * s * 6.0) * (0.35 + 0.9 * flare);
        float k = (vortex + crack + core) * inside;
        vec3 c = mix(color, vec3(1.0), clamp(core * 0.7 + flare * 0.2, 0.0, 1.0));
        float alpha = clamp(k * opacity * (0.4 + 0.6 * open + 0.6 * flare), 0.0, 1.0);
        gl_FragColor = vec4(c * alpha, alpha);
      }`,
  });
}

export function buildGate({ color, size, style = "pulse", seed = 0, chars }) {
  const group = new THREE.Group();
  group.name = "gate";
  const R = size * 2;
  const rimColor = color.clone().lerp(WHITE, 0.2);
  const rim = new THREE.Mesh(new THREE.TorusGeometry(R, size * 0.055, 8, 120), new THREE.MeshBasicMaterial({ color: rimColor, transparent: true, opacity: 0.9, depthWrite: false, blending: Additive }));
  const inner = new THREE.Mesh(new THREE.TorusGeometry(R * 0.86, size * 0.02, 6, 96), new THREE.MeshBasicMaterial({ color, transparent: true, opacity: 0.45, depthWrite: false, blending: Additive }));
  rim.rotation.z = seed * 6.28;
  // Hex digits round the rim, like a readout of what passes through: some
  // change with each read.
  const atlas = chars(HEX);
  const digits = charPoints({ count: 12, radius: R * 1.24, size: size * 0.95, color: color.clone().lerp(WHITE, 0.3), atlas });
  digits.rotation.x = Math.PI / 2;
  const iris = new THREE.Mesh(new THREE.PlaneGeometry(R * 2, R * 2), irisMaterial(color, style, seed));
  iris.renderOrder = 7;
  group.add(iris, inner, rim, digits);

  const conduit = fatLine(CONDUIT, { color: color.clone().lerp(WHITE, 0.15), width: 1.1, opacity: 0.7 });
  conduit.renderOrder = 6;
  const points = Array.from({ length: CONDUIT + 1 }, () => new THREE.Vector3());

  // Light on its way down the conduit, as glows that run along it.
  const packets = [];
  for (let i = 0; i < PACKETS; i++) {
    const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: glowTex, color: color.clone().lerp(WHITE, 0.45), transparent: true, opacity: 0, depthWrite: false, blending: Additive }));
    s.visible = false;
    s.renderOrder = 12;
    s.userData.u = 1;
    packets.push(s);
  }
  const cells = digits.geometry.attributes.cell;
  const redraw = (all) => {
    for (let i = 0; i < cells.count; i++) if (all || Math.random() < 0.3) cells.array[i] = Math.floor(Math.random() * atlas.count);
    cells.needsUpdate = true;
  };
  redraw(true);

  let level = 0, shown = 0, flare = 0, flicker = 1, flickerAt = 0;
  const mid = new THREE.Vector3(), up = new THREE.Vector3();
  const gate = {
    group,
    conduit,
    packets,
    get level() {
      return level;
    },
    get shown() {
      return shown;
    },
    update({ t, dt, light, want, from, to, view, still }) {
      level = still ? want : ease(level, want, want > level ? 2.2 : 1.2, dt);
      shown = still ? (want > 0 ? 1 : 0) : ease(shown, want > 0 ? 1 : 0, 1.5, dt);
      flare = still ? 0 : Math.max(0, flare - dt / 0.9);
      const lit = light * shown;
      if (style === "bolt" && dt && t - flickerAt > 0.09) (flickerAt = t), (flicker = 0.6 + Math.random() * 0.4);
      const glow = (style === "bolt" ? flicker : 1) * lit;
      rim.material.opacity = 0.9 * glow * (0.45 + 0.55 * level + 0.5 * flare);
      inner.material.opacity = 0.45 * lit * (0.3 + 0.7 * level);
      digits.material.uniforms.opacity.value = lit * (0.35 + 0.65 * level + 0.4 * flare);
      digits.material.uniforms.view.value = view;
      const u = iris.material.uniforms;
      u.time.value = t;
      u.open.value = level;
      u.flare.value = flare;
      u.opacity.value = lit;
      // Turns slowly, faster while it's open; held still, it doesn't.
      if (dt) {
        rim.rotation.z += dt * (0.08 + 0.35 * level + 1.5 * flare);
        inner.rotation.z -= dt * (0.05 + 0.2 * level);
        digits.rotation.y += dt * (0.04 + 0.12 * level + 0.6 * flare);
      }
      // The conduit: from the actor's edge down to the gate, bowed a
      // little, as light falls.
      mid.addVectors(from, to).multiplyScalar(0.5);
      const len = from.distanceTo(to);
      up.set(0, len * 0.08, 0);
      mid.add(up);
      for (let i = 0; i <= CONDUIT; i++) {
        const s = i / CONDUIT, v = 1 - s;
        points[i].set(v * v * from.x + 2 * v * s * mid.x + s * s * to.x, v * v * from.y + 2 * v * s * mid.y + s * s * to.y, v * v * from.z + 2 * v * s * mid.z + s * s * to.z);
      }
      writeLine(conduit, points);
      conduit.material.opacity = 0.7 * lit * (0.3 + 0.7 * level + 0.6 * flare);
      for (const p of packets) {
        if (p.userData.u >= 1) {
          p.visible = false;
          continue;
        }
        p.userData.u += dt / TRAVEL;
        const s = Math.max(0, Math.min(1, p.userData.u)), v = 1 - s;
        p.position.set(v * v * from.x + 2 * v * s * mid.x + s * s * to.x, v * v * from.y + 2 * v * s * mid.y + s * s * to.y, v * v * from.z + 2 * v * s * mid.z + s * s * to.z);
        p.visible = p.userData.u >= 0;
        p.scale.setScalar(size * (1.1 + 0.6 * Math.sin(Math.PI * s)));
        p.material.opacity = lit * Math.sin(Math.PI * Math.min(1, s * 1.05));
        // Through the gate: the iris flares as light reaches it.
        if (p.userData.u >= 1) (flare = 1), redraw(false);
      }
    },
    read() {
      // Three glows, one after another.
      for (let i = 0; i < 3; i++) {
        const p = packets.find((x) => x.userData.u >= 1);
        if (!p) break;
        p.userData.u = -i * 0.18;
      }
      return TRAVEL;
    },
    dispose() {
      for (const o of [rim, inner, iris, digits]) o.geometry.dispose(), o.material.dispose();
      conduit.geometry.dispose();
      releaseMaterial(conduit.material);
      for (const p of packets) p.material.dispose();
    },
  };
  return gate;
}
