// The floor under the map (theme.grid). Each style:
//   build(theme, { radius, y })   the object to add to the scene, a floor of
//                                 that radius at height y
//   update(object, { t })         each frame, with the ambient clock
//   dispose(object)               frees what build made
import * as THREE from "three";
import { Additive } from "../fx.js";

const disposeAll = (object) =>
  object.traverse((o) => {
    o.geometry?.dispose();
    o.material?.dispose();
  });

export const GRID = {
  // A polar grid: 24 spokes and 10 rings, brighter at the center.
  polar: {
    build(theme, { radius, y }) {
      const { center, lines, opacity } = theme.grid;
      const grid = new THREE.PolarGridHelper(radius, 24, 10, 192, center, lines);
      grid.material.transparent = true;
      grid.material.opacity = opacity;
      grid.material.depthWrite = false;
      grid.position.y = y;
      return grid;
    },
    update() {},
    dispose: disposeAll,
  },

  // A lattice of hexagons inside a ring, like frost on glass: brightest at
  // the center, in the lines' color toward the rim.
  hex: {
    build(theme, { radius, y }) {
      const { center, lines, opacity } = theme.grid;
      const side = 22, h = (Math.sqrt(3) / 2) * side;
      const near = new THREE.Color(center), far = new THREE.Color(lines), c = new THREE.Color();
      const pos = [], col = [];
      const seen = new Set();
      const shade = (x, z) => c.copy(near).lerp(far, Math.min(1, Math.hypot(x, z) / (radius * 0.55)));
      const edge = (x1, z1, x2, z2) => {
        // Neighbors share edges; draw each once.
        const key = [x1, z1, x2, z2].map((v) => Math.round(v * 10)).join(",");
        const back = [x2, z2, x1, z1].map((v) => Math.round(v * 10)).join(",");
        if (seen.has(key) || seen.has(back)) return;
        seen.add(key);
        pos.push(x1, 0, z1, x2, 0, z2);
        col.push(...shade(x1, z1).toArray(), ...shade(x2, z2).toArray());
      };
      // Flat-topped hexagons on axial coordinates, kept whole inside the ring.
      const n = Math.ceil(radius / (1.5 * side)) + 1;
      for (let q = -n; q <= n; q++)
        for (let r = -2 * n; r <= 2 * n; r++) {
          const x = 1.5 * side * q, z = 2 * h * (r + q / 2);
          if (Math.hypot(x, z) > radius - side) continue;
          for (let k = 0; k < 6; k++) {
            const a = (Math.PI / 3) * k, b = (Math.PI / 3) * (k + 1);
            edge(x + side * Math.cos(a), z + side * Math.sin(a), x + side * Math.cos(b), z + side * Math.sin(b));
          }
        }
      // The ring around them.
      for (let k = 0; k < 128; k++) {
        const a = (k / 128) * Math.PI * 2, b = ((k + 1) / 128) * Math.PI * 2;
        pos.push(radius * Math.cos(a), 0, radius * Math.sin(a), radius * Math.cos(b), 0, radius * Math.sin(b));
        col.push(...far.toArray(), ...far.toArray());
      }
      const geo = new THREE.BufferGeometry();
      geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
      geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 3));
      // Untouched by tone mapping, as the polar grid is.
      const floor = new THREE.LineSegments(geo, new THREE.LineBasicMaterial({ vertexColors: true, toneMapped: false, transparent: true, opacity, depthWrite: false }));
      floor.position.y = y;
      return floor;
    },
    update() {},
    dispose: disposeAll,
  },

  // Cells that churn slowly, bright in the middle with dark lanes between,
  // like the granulation of the sun's surface.
  cells: {
    build(theme, { radius, y }) {
      const { center, lines, opacity } = theme.grid;
      const material = new THREE.ShaderMaterial({
        transparent: true,
        depthWrite: false,
        blending: Additive,
        uniforms: { time: { value: 0 }, opacity: { value: opacity }, radius: { value: radius }, bright: { value: new THREE.Color(center) }, lanes: { value: new THREE.Color(lines) } },
        vertexShader: /* glsl */ `varying vec2 vAt; void main() { vAt = position.xy; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
        fragmentShader: /* glsl */ `
          uniform float time; uniform float opacity; uniform float radius; uniform vec3 bright; uniform vec3 lanes;
          varying vec2 vAt;
          vec2 hash2(vec2 p) { return fract(sin(vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)))) * 43758.5453); }
          void main() {
            vec2 p = vAt / 24.0, i = floor(p), f = fract(p);
            float d1 = 8.0, d2 = 8.0;
            for (int y = -1; y <= 1; y++)
              for (int x = -1; x <= 1; x++) {
                vec2 g = vec2(float(x), float(y));
                vec2 o = 0.5 + 0.38 * sin(time * 0.3 + 6.2831 * hash2(i + g));
                float d = length(g + o - f);
                if (d < d1) { d2 = d1; d1 = d; } else if (d < d2) d2 = d;
              }
            float lane = smoothstep(0.02, 0.16, d2 - d1);
            float glow = 1.0 - smoothstep(0.0, 0.75, d1);
            float rim = 1.0 - smoothstep(0.72, 1.0, length(vAt) / radius);
            vec3 col = mix(lanes, bright, glow) * (0.25 + 0.75 * lane);
            gl_FragColor = vec4(col, opacity * rim);
          }`,
      });
      const floor = new THREE.Mesh(new THREE.CircleGeometry(radius, 96), material);
      floor.rotation.x = -Math.PI / 2;
      floor.position.y = y;
      return floor;
    },
    update(floor, { t }) {
      floor.material.uniforms.time.value = t;
    },
    dispose: disposeAll,
  },

  // No floor: the map floats in the dark.
  none: {
    build: () => new THREE.Group(),
    update() {},
    dispose() {},
  },
};
