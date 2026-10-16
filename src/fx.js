// Rendering building blocks: procedural textures, anti-aliased "fat" lines,
// the nebula sky and the final lens pass. The stars, floor, sweep, sonar
// and lines are in looks/, by style.
import * as THREE from "three";
import { Line2 } from "three/addons/lines/Line2.js";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineGeometry } from "three/addons/lines/LineGeometry.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import { LineMaterial } from "three/addons/lines/LineMaterial.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { THEME } from "./theme.js";

export const Additive = THREE.AdditiveBlending;

// ---------- textures ----------

function canvasTexture(size, draw) {
  const c = document.createElement("canvas");
  c.width = c.height = size;
  draw(c.getContext("2d"), size);
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 4;
  return t;
}

export const glowTex = canvasTexture(256, (g, s) => {
  const grd = g.createRadialGradient(s / 2, s / 2, 0, s / 2, s / 2, s / 2);
  grd.addColorStop(0, "rgba(255,255,255,1)");
  grd.addColorStop(0.08, "rgba(255,255,255,0.85)");
  grd.addColorStop(0.22, "rgba(255,255,255,0.32)");
  grd.addColorStop(0.5, "rgba(255,255,255,0.07)");
  grd.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = grd;
  g.fillRect(0, 0, s, s);
});

export const sparkTex = canvasTexture(64, (g, s) => {
  const grd = g.createRadialGradient(s / 2, s / 2, 0, s / 2, s / 2, s / 2);
  grd.addColorStop(0, "rgba(255,255,255,1)");
  grd.addColorStop(0.3, "rgba(255,255,255,0.5)");
  grd.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = grd;
  g.fillRect(0, 0, s, s);
});

export const ringTex = canvasTexture(512, (g, s) => {
  g.strokeStyle = "#fff";
  g.shadowColor = "#fff";
  g.shadowBlur = 18;
  g.lineWidth = 5;
  g.beginPath();
  g.arc(s / 2, s / 2, s / 2 - 24, 0, Math.PI * 2);
  g.stroke();
});

export const reticleTex = canvasTexture(1024, (g, s) => {
  const c = s / 2;
  g.strokeStyle = "#fff";
  g.lineCap = "round";
  g.shadowColor = "#fff";
  g.shadowBlur = 8;
  g.lineWidth = 7;
  for (let i = 0; i < 4; i++) {
    g.beginPath();
    g.arc(c, c, s * 0.36, i * (Math.PI / 2) + 0.2, (i + 1) * (Math.PI / 2) - 0.2);
    g.stroke();
  }
  g.lineWidth = 2.5;
  g.globalAlpha = 0.5;
  g.beginPath();
  g.arc(c, c, s * 0.3, 0, Math.PI * 2);
  g.stroke();
  g.globalAlpha = 1;
  for (let i = 0; i < 72; i++) {
    const a = (i / 72) * Math.PI * 2;
    const major = i % 6 === 0;
    const r0 = s * (major ? 0.395 : 0.415);
    g.lineWidth = major ? 5 : 2;
    g.beginPath();
    g.moveTo(c + Math.cos(a) * r0, c + Math.sin(a) * r0);
    g.lineTo(c + Math.cos(a) * s * 0.44, c + Math.sin(a) * s * 0.44);
    g.stroke();
  }
  g.lineWidth = 7;
  const b = s * 0.47, l = s * 0.07;
  for (const [sx, sy] of [[-1, -1], [1, -1], [1, 1], [-1, 1]]) {
    g.beginPath();
    g.moveTo(c + sx * b, c + sy * (b - l));
    g.lineTo(c + sx * b, c + sy * b);
    g.lineTo(c + sx * (b - l), c + sy * b);
    g.stroke();
  }
});

// ---------- fat lines ----------
// GL lines are 1px and alias badly; LineMaterial draws screen-space quads that
// stay smooth under MSAA at any width.

const lineMaterials = new Set();
const resolution = new THREE.Vector2(innerWidth, innerHeight);

export function lineMaterial({ color = 0xffffff, width = 1.4, opacity = 1, vertexColors = false, additive = true } = {}) {
  const m = new LineMaterial({
    color,
    linewidth: width,
    transparent: true,
    opacity,
    vertexColors,
    depthWrite: false,
    blending: additive ? Additive : THREE.NormalBlending,
    worldUnits: false,
  });
  m.resolution.copy(resolution);
  m.userData.base = opacity;
  lineMaterials.add(m);
  // The force graph disposes the lines it drops itself; let them go here too.
  m.addEventListener("dispose", () => lineMaterials.delete(m));
  return m;
}

export function releaseMaterial(m) {
  lineMaterials.delete(m);
  m.dispose();
}

export function setLineResolution(w, h) {
  resolution.set(w, h);
  for (const m of lineMaterials) m.resolution.set(w, h);
}

export function fatEdges(geometry, opts) {
  // The line's geometry takes the edges' points, so the geometry that found
  // them is done with at once; `geometry` stays its caller's, which may
  // draw it too.
  const edges = new THREE.EdgesGeometry(geometry);
  const geo = new LineSegmentsGeometry().fromEdgesGeometry(edges);
  edges.dispose();
  const line = new LineSegments2(geo, lineMaterial(opts));
  line.computeLineDistances();
  return line;
}

// A polyline with `segments` pieces whose points are rewritten in place.
export function fatLine(segments, opts, colors) {
  const geo = new LineGeometry();
  geo.setPositions(new Float32Array((segments + 1) * 3));
  if (colors) geo.setColors(colors);
  const line = new Line2(geo, lineMaterial({ ...opts, vertexColors: !!colors }));
  line.userData.segments = segments;
  line.frustumCulled = false;
  return line;
}

export function writeLine(line, points) {
  const attr = line.geometry.attributes.instanceStart.data;
  const arr = attr.array;
  for (let i = 0; i < points.length - 1; i++) {
    const a = points[i], b = points[i + 1], o = i * 6;
    arr[o] = a.x; arr[o + 1] = a.y; arr[o + 2] = a.z;
    arr[o + 3] = b.x; arr[o + 4] = b.y; arr[o + 5] = b.z;
  }
  attr.needsUpdate = true;
}

export function gradientColors(segments, from, to) {
  const a = new THREE.Color(from), b = new THREE.Color(to), c = new THREE.Color();
  const out = new Float32Array((segments + 1) * 3);
  for (let i = 0; i <= segments; i++) {
    c.copy(a).lerp(b, i / segments);
    out.set([c.r, c.g, c.b], i * 3);
  }
  return out;
}

// Quadratic arc between two points, bowed away from the scene centre.
export function arcPoints(a, b, out, bow = 0.25) {
  const mid = new THREE.Vector3().addVectors(a, b).multiplyScalar(0.5);
  const len = a.distanceTo(b);
  const lift = mid.clone();
  if (lift.lengthSq() < 1) lift.set(0, 1, 0);
  lift.normalize().multiplyScalar(len * bow);
  const ctrl = mid.add(lift);
  const n = out.length - 1;
  for (let i = 0; i <= n; i++) {
    const t = i / n, u = 1 - t;
    out[i].set(u * u * a.x + 2 * u * t * ctrl.x + t * t * b.x, u * u * a.y + 2 * u * t * ctrl.y + t * t * b.y, u * u * a.z + 2 * u * t * ctrl.z + t * t * b.z);
  }
  return out;
}

export const pointsArray = (n) => Array.from({ length: n + 1 }, () => new THREE.Vector3());

// ---------- environment ----------

const NOISE = /* glsl */ `
  float hash(vec3 p) { p = fract(p * 0.3183099 + 0.1); p *= 17.0; return fract(p.x * p.y * p.z * (p.x + p.y + p.z)); }
  float noise(vec3 x) {
    vec3 i = floor(x); vec3 f = fract(x); f = f * f * (3.0 - 2.0 * f);
    return mix(mix(mix(hash(i), hash(i + vec3(1,0,0)), f.x), mix(hash(i + vec3(0,1,0)), hash(i + vec3(1,1,0)), f.x), f.y),
               mix(mix(hash(i + vec3(0,0,1)), hash(i + vec3(1,0,1)), f.x), mix(hash(i + vec3(0,1,1)), hash(i + vec3(1,1,1)), f.x), f.y), f.z);
  }
  float fbm(vec3 p) { float v = 0.0, a = 0.5; for (int i = 0; i < 5; i++) { v += a * noise(p); p *= 2.03; a *= 0.5; } return v; }
`;

// Sky palettes through the day, from the theme in use. Each is [deep,
// primary cloud, secondary cloud, band].
let skyOf = null, SKY = null;
function skyStops() {
  if (skyOf === THEME.sky) return SKY;
  const { night, dawn, day, dusk } = (skyOf = THEME.sky);
  return (SKY = [
    [0, night],
    [5.5, night],
    [7, dawn],
    [9, day],
    [16.5, day],
    [18.5, dusk],
    [21, night],
    [24, night],
  ]);
}

export function skyColors(hour) {
  const stops = skyStops();
  const h = ((hour % 24) + 24) % 24;
  let i = 0;
  while (i < stops.length - 2 && stops[i + 1][0] <= h) i++;
  const [h0, c0] = stops[i], [h1, c1] = stops[i + 1];
  const t = h1 > h0 ? (h - h0) / (h1 - h0) : 0;
  const k = t * t * (3 - 2 * t);
  return c0.map((c, j) => new THREE.Color(c).lerp(new THREE.Color(c1[j]), k));
}

export function nebula() {
  const [deep, a, b, band] = skyColors(new Date().getHours() + new Date().getMinutes() / 60);
  const mat = new THREE.ShaderMaterial({
    side: THREE.BackSide,
    depthWrite: false,
    fog: false,
    uniforms: { time: { value: 0 }, environment: { value: 0 }, accent: { value: new THREE.Color(THEME.colors.accent) }, deep: { value: deep }, cloudA: { value: a }, cloudB: { value: b }, bandColor: { value: band }, rest: { value: 0 } },
    vertexShader: /* glsl */ `varying vec3 vDir; void main() { vDir = normalize(position); gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform float time; uniform float environment; uniform vec3 accent; uniform vec3 deep; uniform vec3 cloudA; uniform vec3 cloudB; uniform vec3 bandColor; uniform float rest;
      varying vec3 vDir;
      ${NOISE}
      void main() {
        vec3 d = normalize(vDir);
        if (environment > 0.5) {
          float t = time * 0.035;
          float az = atan(d.z, d.x);
          vec3 col = deep * 0.25;
          vec3 primary = cloudA * 1.4 + accent * 0.07;
          vec3 secondary = cloudB * 1.4 + accent * 0.02;
          vec3 edge = bandColor * 1.3 + accent * 0.06;
          if (environment < 1.5) {
            float horizon = -0.20 + 0.12 * sin(az * 3.0 + t) + 0.06 * sin(az * 7.0 - t * 0.6);
            float height = d.y - horizon;
            float curtain = exp(-max(height, 0.0) * 5.0) * smoothstep(-0.08, 0.025, height);
            float threads = 0.45 + 0.55 * pow(0.5 + 0.5 * sin(az * 90.0 + sin(az * 9.0 + t) * 5.0), 2.0);
            float veil = 0.55 + 0.45 * sin(az * 4.0 - t);
            col += mix(primary, edge, 0.5 + 0.5 * sin(az * 2.0 + t)) * curtain * threads * veil * 0.9;
            col += secondary * exp(-pow(height - 0.12, 2.0) * 35.0) * 0.2;
          } else if (environment < 2.5) {
            vec2 p = d.xz / (abs(d.y) + 0.35);
            float wave = sin(p.x * 9.0 + sin(p.y * 7.0 + t)) + sin(p.y * 11.0 - cos(p.x * 6.0 - t));
            float caustic = pow(max(0.0, 1.0 - abs(wave) * 2.5), 4.0);
            float above = smoothstep(-0.6, 0.4, d.y);
            float shafts = pow(0.5 + 0.5 * sin(az * 13.0 + t * 0.3), 18.0) * pow(max(d.y + 0.45, 0.0), 0.7);
            col += primary * (0.12 + caustic * 0.45) * above;
            col += edge * shafts * 0.32;
            col += secondary * exp(-pow(d.y + 0.12, 2.0) * 6.0) * 0.12;
          } else {
            // A cylindrical arcade: vertical piers meet curved ceiling ribs.
            float bay = abs(sin(az * 8.0));
            float arch = d.y - (-0.08 + 0.30 * sqrt(max(0.0, 1.0 - bay * bay)));
            float ribs = exp(-abs(arch) * 65.0);
            float piers = exp(-bay * 35.0) * (1.0 - smoothstep(0.0, 0.3, d.y));
            float inner = exp(-abs(arch + 0.07) * 65.0) * 0.3;
            float ceiling = pow(max(d.y, 0.0), 3.0);
            col += primary * (ribs + piers + inner) * 0.65;
            col += secondary * (0.12 * ceiling + 0.06 * smoothstep(-0.8, 0.1, arch));
            col += edge * exp(-abs(d.y + 0.5) * 65.0) * 0.14;
          }
          gl_FragColor = vec4(col * 2.5 * (1.0 - 0.45 * rest), 1.0);
          return;
        }
        float n = fbm(d * 2.2 + vec3(0.0, time * 0.004, 0.0));
        float m = fbm(d * 4.7 - vec3(time * 0.003));
        float bx = d.y * 3.2 + 0.25 * sin(d.x * 3.0);
        float band = exp(-bx * bx);
        vec3 col = deep * 0.35 + cloudA * smoothstep(0.45, 0.85, n) * (0.5 + band) + cloudB * smoothstep(0.5, 0.9, m);
        col += bandColor * band * smoothstep(0.35, 0.8, n);
        col *= 1.0 - 0.45 * rest;
        gl_FragColor = vec4(col, 1.0);
      }`,
  });
  return new THREE.Mesh(new THREE.SphereGeometry(4200, 64, 32), mat);
}

// Four-point star for wiki pages.
export const starTex = canvasTexture(256, (g, s) => {
  const c = s / 2;
  const grd = g.createRadialGradient(c, c, 0, c, c, c);
  grd.addColorStop(0, "rgba(255,255,255,1)");
  grd.addColorStop(0.12, "rgba(255,255,255,0.7)");
  grd.addColorStop(0.4, "rgba(255,255,255,0.08)");
  grd.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = grd;
  g.fillRect(0, 0, s, s);
  g.globalCompositeOperation = "lighter";
  for (const [w, h] of [[s * 0.5, 3], [3, s * 0.5]]) {
    const lg = w > h ? g.createLinearGradient(c - w, c, c + w, c) : g.createLinearGradient(c, c - h, c, c + h);
    lg.addColorStop(0, "rgba(255,255,255,0)");
    lg.addColorStop(0.5, "rgba(255,255,255,0.95)");
    lg.addColorStop(1, "rgba(255,255,255,0)");
    g.fillStyle = lg;
    g.fillRect(c - w, c - h / 2, w * 2, h);
  }
});

// The textures above, shared by every glow, spark, ring and star: whatever
// disposes a node's own (a label's text) leaves these alone.
export const SHARED_TEXTURES = new Set([glowTex, sparkTex, ringTex, reticleTex, starTex]);

// A vertical shaft of light, brightest at its base, for the "needs you" beacon.
export function pillarMaterial(color) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    side: THREE.DoubleSide,
    uniforms: { color: { value: new THREE.Color(color) }, opacity: { value: 1 }, time: { value: 0 } },
    vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
    fragmentShader: /* glsl */ `
      uniform vec3 color; uniform float opacity; uniform float time; varying vec2 vUv;
      void main() {
        // Clamp before pow(): interpolated uv can dip just below 0, and pow() of
        // a negative is NaN, which bloom would smear across the whole frame.
        float edge = clamp(1.0 - abs(vUv.x - 0.5) * 2.0, 0.0, 1.0);
        float fall = pow(clamp(1.0 - vUv.y, 0.0, 1.0), 1.6);
        float shimmer = 0.75 + 0.25 * sin(vUv.y * 40.0 - time * 6.0);
        float a = pow(edge, 1.8) * fall * shimmer * opacity;
        gl_FragColor = vec4(color * a * 1.8, a);
      }`,
  });
}

// Fresnel shell for sonar waves and the core's glow.
export function fresnelMaterial(color, opacity = 1, power = 2.5) {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    blending: Additive,
    side: THREE.DoubleSide,
    uniforms: { color: { value: new THREE.Color(color) }, opacity: { value: opacity }, power: { value: power } },
    vertexShader: /* glsl */ `
      varying vec3 vN; varying vec3 vV;
      void main() { vec4 mv = modelViewMatrix * vec4(position, 1.0); vN = normalize(normalMatrix * normal); vV = normalize(-mv.xyz); gl_Position = projectionMatrix * mv; }`,
    fragmentShader: /* glsl */ `
      uniform vec3 color; uniform float opacity; uniform float power; varying vec3 vN; varying vec3 vV;
      void main() { float f = pow(clamp(1.0 - abs(dot(normalize(vN), normalize(vV))), 0.0, 1.0), power); gl_FragColor = vec4(color * f * opacity, f * opacity); }`,
  });
}

// Final pass: soft chromatic aberration toward the edges, vignette, fine grain.
export const LensShader = {
  uniforms: {
    tDiffuse: { value: null },
    time: { value: 0 },
    aberration: { value: 0.004 },
    vignette: { value: 0.55 },
    grain: { value: 0.022 },
  },
  vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
  fragmentShader: /* glsl */ `
    uniform sampler2D tDiffuse; uniform float time; uniform float aberration; uniform float vignette; uniform float grain;
    varying vec2 vUv;
    float rand(vec2 co) { return fract(sin(dot(co, vec2(12.9898, 78.233))) * 43758.5453); }
    void main() {
      vec2 d = vUv - 0.5; float r2 = dot(d, d);
      vec2 off = d * r2 * aberration * 4.0;
      vec4 base = texture2D(tDiffuse, vUv);
      vec3 col = vec3(texture2D(tDiffuse, vUv + off).r, base.g, texture2D(tDiffuse, vUv - off).b);
      col *= 1.0 - vignette * smoothstep(0.08, 0.55, r2);
      col += (rand(vUv * 1024.0 + fract(time)) - 0.5) * grain;
      gl_FragColor = vec4(max(col, 0.0), base.a);
    }`,
};

// The lens and the output pass (tone mapping and the screen's color
// space) in one pass over the screen instead of two: the same picture,
// with one fewer full-screen read and write each frame. Built on three's
// OutputPass, whose shader it adds the lens to; null if that shader isn't
// as expected, for the two passes instead.
export function lensOutputPass() {
  const pass = new OutputPass();
  const lens = /void main\(\) \{([\s\S]*)gl_FragColor = vec4\(max\(col, 0\.0\), base\.a\);\s*\}/.exec(LensShader.fragmentShader)?.[1];
  const read = "gl_FragColor = texture2D( tDiffuse, vUv );";
  const shader = pass.material.fragmentShader;
  if (!lens || !shader.includes(read) || !shader.includes("varying vec2 vUv;")) {
    pass.dispose();
    return null;
  }
  for (const k of ["time", "aberration", "vignette", "grain"]) pass.uniforms[k] = { value: LensShader.uniforms[k].value };
  pass.material.fragmentShader = shader
    .replace("varying vec2 vUv;", "varying vec2 vUv;\nuniform float time; uniform float aberration; uniform float vignette; uniform float grain;\nfloat rand(vec2 co) { return fract(sin(dot(co, vec2(12.9898, 78.233))) * 43758.5453); }")
    .replace(read, `${lens}gl_FragColor = vec4(max(col, 0.0), base.a);`);
  return pass;
}
