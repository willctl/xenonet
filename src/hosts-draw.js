// The three.js half of other machines on the map (hosts.js has the rest):
// the host node at the center of each host's constellation, with a ring in
// the color of how it stands, and the dashed bridges between checkouts of
// one repo on different machines. scene.js keeps one layer, hands it the
// map before the force graph gets it, and steps it every frame.
import * as THREE from "three";
import SpriteText from "three-spritetext";
import { LineSegments2 } from "three/addons/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/addons/lines/LineSegmentsGeometry.js";
import { Additive, glowTex, fatEdges, lineMaterial, releaseMaterial, fresnelMaterial } from "./fx.js";
import { COLORS, THEME } from "./theme.js";
import { HOST_SIZE, labelOf, hostLayout, installHostLayout } from "./hosts.js";

// How a host stands, in the colors the page already uses for it: live as
// the status light's "connected", stale in amber, offline greyed, and a
// host that needs you as a problem.
const STATE_ROLE = { live: "codex", connecting: "muted", stale: "attention", offline: "muted", error: "danger" };
export const stateColor = (state) => COLORS[STATE_ROLE[state] ?? "muted"] ?? "#8899aa";

const remember = (m) => ((m.userData.base = m.opacity), m);

function glow(color, size, opacity) {
  const s = new THREE.Sprite(remember(new THREE.SpriteMaterial({ map: glowTex, color, transparent: true, opacity, depthWrite: false, blending: Additive })));
  s.scale.setScalar(size);
  return s;
}

function torus(radius, tube, color, opacity, arc = Math.PI * 2) {
  return new THREE.Mesh(new THREE.TorusGeometry(radius, tube, 10, 160, arc), remember(new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthWrite: false, blending: Additive })));
}

function label(text, color, height) {
  const s = new SpriteText(text, height, color);
  s.fontFace = THEME.fonts.display;
  s.fontWeight = "600";
  s.fontSize = 128;
  s.padding = 1;
  s.material.depthWrite = false;
  if (s.material.map) s.material.map.anisotropy = 8;
  remember(s.material);
  s.renderOrder = 10;
  return s;
}

// The host node, built into `body` as nodes.js builds the others: a smaller
// core of its own, a tilted ring in the color of its state with three arcs
// that turn while it's live (and pulse while it connects), its name under
// it and how it stands under that. Returns its label and halo.
export function buildHostBody(node, body, anim, phase = 0) {
  const r = HOST_SIZE;
  const live = node.state === "live";
  const tint = stateColor(node.state);
  const shell = fatEdges(new THREE.IcosahedronGeometry(r * 1.45, 1), { color: COLORS.project, width: 1.1, opacity: live ? 0.6 : 0.32 });
  const inner = new THREE.Mesh(new THREE.SphereGeometry(r * 0.55, 32, 20), remember(new THREE.MeshBasicMaterial({ color: COLORS.core, transparent: true, opacity: live ? 1 : 0.55 })));
  const rim = new THREE.Mesh(new THREE.SphereGeometry(r * 1.15, 32, 24), fresnelMaterial(COLORS.core, live ? 0.9 : 0.4, 2.2));
  const ring = new THREE.Group();
  ring.rotation.set(Math.PI / 2 - 0.42, 0, phase);
  const band = torus(r * 2.6, 0.07, tint, live ? 0.75 : 0.5);
  const arcs = new THREE.Group();
  for (let k = 0; k < 3; k++) {
    const arc = torus(r * 3.05, 0.16, tint, 0.9, Math.PI * 0.42);
    arc.rotation.z = (k * Math.PI * 2) / 3;
    arcs.add(arc);
  }
  ring.add(band, arcs);
  const halo = glow(COLORS.core, r * 9, live ? 0.36 : 0.14);
  body.add(halo, rim, inner, shell, ring);
  anim.push((t, dt) => {
    shell.rotation.y += dt * 0.12;
    if (live) arcs.rotation.z += dt * 0.6;
    if (node.state === "connecting") for (const a of arcs.children) a.material.opacity = 0.35 + 0.45 * (0.5 + 0.5 * Math.sin(t * 3 + phase));
  });
  const [name, how] = labelOf(node).split("\n");
  const lbl = label(name, COLORS.label, 4.2);
  lbl.position.y = -(r * 3.4 + 4);
  const sub = label(how, tint, 2.3);
  sub.position.y = lbl.position.y - 4.6;
  body.add(sub);
  return { label: lbl, halo };
}

// ---------- bridges: checkouts of one repo on different machines ----------

const DASHES = 30;
const DUTY = 0.55;
const FADE_MS = 700;
// Faint at rest, so a few shared repos don't net the map over; lit while
// either end is selected, hovered or in the selection's branch.
const BRIDGE_REST = 0.2;
const BRIDGE_LIT = 0.8;
const a = new THREE.Vector3(), b = new THREE.Vector3(), c = new THREE.Vector3();

function bridgeLine() {
  const geo = new LineSegmentsGeometry();
  geo.setPositions(new Float32Array(DASHES * 6));
  const line = new LineSegments2(geo, lineMaterial({ color: COLORS.project, width: 1.25, opacity: 0 }));
  line.frustumCulled = false;
  line.renderOrder = 6;
  return line;
}

// Dashes along a curve that rises a little over the gap between the two
// machines, as a bridge does.
function placeBridge(line, from, to) {
  a.set(from.x, from.y, from.z);
  b.set(to.x, to.y, to.z);
  c.addVectors(a, b).multiplyScalar(0.5);
  c.y += a.distanceTo(b) * 0.08 + 8;
  const arr = line.geometry.attributes.instanceStart.data.array;
  const at = (t, o) => {
    const u = 1 - t;
    arr[o] = u * u * a.x + 2 * u * t * c.x + t * t * b.x;
    arr[o + 1] = u * u * a.y + 2 * u * t * c.y + t * t * b.y;
    arr[o + 2] = u * u * a.z + 2 * u * t * c.z + t * t * b.z;
  };
  for (let i = 0; i < DASHES; i++) {
    at(i / DASHES, i * 6);
    at((i + DUTY) / DASHES, i * 6 + 3);
  }
  line.geometry.attributes.instanceStart.data.needsUpdate = true;
}

// The layer scene.js keeps: the layout (hosts.js) put into the force graph
// the first time a host is on the map, and the bridges. `shown(id)` says
// whether the filters let a node through, `lightOf(a, b)` how lit a line
// between two nodes is, and `lit(n)` whether a node is selected, hovered or
// in the selection's branch.
export function createHostLayer({ graph, scene, shown = () => true, lightOf = () => 1, lit = () => false }) {
  const layout = hostLayout({ radial: () => !!graph.dagMode() });
  let installed = false;
  const bridges = new Map(); // key → { line, a, b, born, dying, at: where its ends were placed }

  function drop(key, br) {
    scene.remove(br.line);
    br.line.geometry.dispose();
    releaseMaterial(br.line.material);
    bridges.delete(key);
  }

  return {
    // Before the force graph gets the map. Says whether a host is on it.
    prepare(nodes, links) {
      const any = layout.prepare(nodes, links);
      if (any && !installed) {
        installHostLayout(graph, layout);
        installed = true;
      }
      return any;
    },

    // The map's same-repo relations, as drawn nodes.
    relations(relations, nodeIndex) {
      const seen = new Set();
      for (const r of relations) {
        if (r.kind !== "same-repo") continue;
        const na = nodeIndex.get(r.a), nb = nodeIndex.get(r.b);
        if (!na || !nb) continue;
        const key = `${r.a}|${r.b}`;
        seen.add(key);
        let br = bridges.get(key);
        if (!br) {
          br = { line: bridgeLine(), born: performance.now(), dying: null, at: new Float64Array(6).fill(NaN) };
          scene.add(br.line);
          bridges.set(key, br);
        }
        Object.assign(br, { a: na, b: nb, dying: null });
      }
      for (const [key, br] of bridges) if (!seen.has(key)) br.dying ??= performance.now();
    },

    // Every frame: each bridge follows its ends, and fades with them.
    step() {
      const now = performance.now();
      for (const [key, br] of bridges) {
        const fade = br.dying ? Math.max(0, 1 - (now - br.dying) / FADE_MS) : 1;
        if (br.dying && fade <= 0) {
          drop(key, br);
          continue;
        }
        const ok = br.a.__obs && br.b.__obs && shown(br.a.id) && shown(br.b.id) && Number.isFinite(br.a.x) && Number.isFinite(br.b.x);
        br.line.visible = !!ok;
        if (!ok) continue;
        // Placed again only when an end has moved: nothing allocated per frame.
        const at = br.at;
        if (at[0] !== br.a.x || at[1] !== br.a.y || at[2] !== br.a.z || at[3] !== br.b.x || at[4] !== br.b.y || at[5] !== br.b.z) {
          placeBridge(br.line, br.a, br.b);
          at.set([br.a.x, br.a.y, br.a.z, br.b.x, br.b.y, br.b.z]);
        }
        const grow = Math.min(1, (now - br.born) / FADE_MS);
        const stale = br.a.stale || br.b.stale ? 0.5 : 1;
        const on = lit(br.a) || lit(br.b);
        br.line.material.opacity = (on ? BRIDGE_LIT : BRIDGE_REST * lightOf(br.a, br.b)) * grow * fade * stale;
      }
    },

    retheme() {
      for (const br of bridges.values()) br.line.material.color.set(COLORS.project);
    },

    // For the driver: where each constellation is, and the bridges drawn.
    state() {
      const round = (v) => (Number.isFinite(v) ? Math.round(v) : null);
      return {
        constellations: layout.systems.map((s) => ({ host: s.host, anchor: [s.anchor.x, s.anchor.y, s.anchor.z], at: [round(s.node.x), round(s.node.y), round(s.node.z)], radius: Math.round(s.radius), nodes: s.members.length + 1 })),
        bridges: { total: [...bridges.values()].filter((br) => !br.dying).length, drawn: [...bridges.values()].filter((br) => !br.dying && br.line.visible).length },
      };
    },
  };
}
