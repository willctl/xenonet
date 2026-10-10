// What circles the floor (theme.sweep: its shape, opacity and speed, in
// turns of a radian a second). The opacity is how strongly it shows,
// measured against XenoNet's faint beam (0.03): the other shapes are thin or
// far off, so each draws brighter at the same value. Each style:
//   build(theme, { radius, y, color })  the object to add to the scene,
//                                        over a floor at height y
//   update(object, { t, theme })         each frame, with the ambient clock
//   dispose(object)                      frees what build made
import * as THREE from "three";
import { Additive } from "../fx.js";

const disposeAll = (object) =>
  object.traverse((o) => {
    o.geometry?.dispose();
    o.material?.map?.dispose();
    o.material?.dispose();
  });

// A little value noise for the shaders below.
const NOISE = /* glsl */ `
  float hash1(float n) { return fract(sin(n) * 43758.5453); }
  float noise1(float x) { float i = floor(x), f = fract(x); f = f * f * (3.0 - 2.0 * f); return mix(hash1(i), hash1(i + 1.0), f); }
`;

// A loop of plasma from the floor up and back down, following the rim.
class Loop extends THREE.Curve {
  constructor(radius, spread, height, y) {
    super();
    Object.assign(this, { radius, spread, height, y });
  }
  getPoint(s, out = new THREE.Vector3()) {
    const a = (s - 0.5) * 2 * this.spread;
    return out.set(Math.cos(a) * this.radius, this.y + Math.sin(Math.PI * s) * this.height, Math.sin(a) * this.radius);
  }
}

export const SWEEP = {
  // A faint wedge of light, like a radar's beam.
  beam: {
    build(theme, { radius, y, color }) {
      const sweep = new THREE.Mesh(
        new THREE.CircleGeometry(radius, 96, 0, 0.32),
        new THREE.MeshBasicMaterial({ color, transparent: true, opacity: theme.sweep.opacity, side: THREE.DoubleSide, depthWrite: false, blending: Additive }),
      );
      sweep.rotation.x = -Math.PI / 2;
      sweep.position.y = y + 1;
      return sweep;
    },
    update(sweep, { t, theme }) {
      sweep.rotation.z = -t * theme.sweep.speed;
    },
    dispose: disposeAll,
  },

  // An aurora: curtains of light far out on the horizon, folding and
  // drifting, from the theme's Codex green up into its violet.
  aurora: {
    build(theme, { y }) {
      const height = 1700;
      const material = new THREE.ShaderMaterial({
        transparent: true,
        depthWrite: false,
        blending: Additive,
        side: THREE.DoubleSide,
        uniforms: { time: { value: 0 }, opacity: { value: Math.min(1, theme.sweep.opacity * 8) }, low: { value: new THREE.Color(theme.colors.codex) }, high: { value: new THREE.Color(theme.colors.native) } },
        vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
        fragmentShader: /* glsl */ `
          uniform float time; uniform float opacity; uniform vec3 low; uniform vec3 high;
          varying vec2 vUv;
          ${NOISE}
          void main() {
            float x = vUv.x * 90.0;
            float fold = noise1(x * 0.12 + time * 0.04) * 5.0;
            float rays = pow(noise1(x * 1.3 + fold + time * 0.1), 2.0) * 0.75 + 0.25 * noise1(x * 5.0 - time * 0.25);
            float where = smoothstep(0.3, 0.8, noise1(vUv.x * 9.0 + time * 0.015));
            float h = vUv.y;
            float body = smoothstep(0.05, 0.3, h) * (1.0 - smoothstep(0.4, 0.95, h));
            vec3 col = mix(low, high, smoothstep(0.25, 0.85, h));
            gl_FragColor = vec4(col, rays * body * where * opacity);
          }`,
      });
      const curtain = new THREE.Mesh(new THREE.CylinderGeometry(2300, 2300, height, 160, 1, true), material);
      curtain.position.y = y - 900 + height / 2;
      return curtain;
    },
    update(curtain, { t, theme }) {
      curtain.material.uniforms.time.value = t * Math.max(0.2, Math.abs(theme.sweep.speed) * 4);
    },
    dispose: disposeAll,
  },

  // Loops of plasma rising from the floor's rim, like flares on the sun,
  // circling as the beam does, flaring and fading.
  flare: {
    build(theme, { radius, y }) {
      const group = new THREE.Group();
      const material = new THREE.ShaderMaterial({
        transparent: true,
        depthWrite: false,
        blending: Additive,
        side: THREE.DoubleSide,
        uniforms: { time: { value: 0 }, opacity: { value: Math.min(1, theme.sweep.opacity * 14) }, hot: { value: new THREE.Color(theme.colors.gold) }, cool: { value: new THREE.Color(theme.colors.claude) } },
        vertexShader: /* glsl */ `
          varying vec2 vUv; varying vec3 vN; varying vec3 vV;
          void main() {
            vUv = uv;
            vec4 mv = modelViewMatrix * vec4(position, 1.0);
            vN = normalize(normalMatrix * normal); vV = normalize(-mv.xyz);
            gl_Position = projectionMatrix * mv;
          }`,
        fragmentShader: /* glsl */ `
          uniform float time; uniform float opacity; uniform vec3 hot; uniform vec3 cool;
          varying vec2 vUv; varying vec3 vN; varying vec3 vV;
          ${NOISE}
          void main() {
            float flow = 0.6 * noise1(vUv.x * 24.0 - time * 1.6) + 0.4 * noise1(vUv.x * 70.0 - time * 3.2);
            float edge = pow(1.0 - abs(dot(normalize(vN), normalize(vV))), 1.4);
            float ends = smoothstep(0.0, 0.1, vUv.x) * (1.0 - smoothstep(0.9, 1.0, vUv.x));
            float pulse = 0.7 + 0.3 * sin(time * 0.7);
            gl_FragColor = vec4(mix(cool, hot, flow), (0.3 + 0.7 * flow) * (0.35 + 0.65 * edge) * ends * pulse * opacity);
          }`,
      });
      for (const [at, size] of [[0, 1], [2.6, 0.62]]) {
        const loop = new THREE.Mesh(new THREE.TubeGeometry(new Loop(radius * 0.74, 0.34 * size + 0.12, 230 * size, y), 72, 3.2 * size + 1.2, 8, false), material);
        loop.rotation.y = at;
        group.add(loop);
      }
      return group;
    },
    update(group, { t, theme }) {
      group.rotation.y = -t * theme.sweep.speed;
      group.children[0].material.uniforms.time.value = t;
    },
    dispose(group) {
      // The loops share one material.
      group.children.forEach((loop) => loop.geometry.dispose());
      group.children[0]?.material.dispose();
    },
  },

  // Nothing circles the floor.
  none: {
    build: () => new THREE.Group(),
    update() {},
    dispose() {},
  },
};
