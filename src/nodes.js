// Node meshes, plus the live "rig" that session nodes grow while an agent is
// working: a context-fill gauge, a status spinner and orbiting subagents.
import * as THREE from "three";
import SpriteText from "three-spritetext";
import { COLORS, CATEGORY_COLORS, RING_COLORS, nodeColor, THEME } from "./theme.js";

// A theme color, lightened toward white by `k`.
const lighter = (hex, k) => `#${new THREE.Color(hex).lerp(new THREE.Color("#ffffff"), k).getHexString()}`;
import { Additive, glowTex, ringTex, starTex, fatEdges, fatLine, writeLine, releaseMaterial, fresnelMaterial, pillarMaterial, SHARED_TEXTURES } from "./fx.js";
import { HOST_SIZE } from "./hosts.js";
import { buildHostBody } from "./hosts-draw.js";

const trim = (s, n) => (s && s.length > n ? s.slice(0, n - 1) + "…" : s ?? "");

function remember(material) {
  material.userData.base = material.opacity;
  return material;
}

function glow(color, size, opacity) {
  const s = new THREE.Sprite(remember(new THREE.SpriteMaterial({ map: glowTex, color, transparent: true, opacity, depthWrite: false, blending: Additive })));
  s.scale.setScalar(size);
  return s;
}

function solid(geometry, color, opacity = 1) {
  return new THREE.Mesh(geometry, remember(new THREE.MeshBasicMaterial({ color, transparent: opacity < 1, opacity })));
}

function torus(radius, tube, color, opacity) {
  return new THREE.Mesh(
    new THREE.TorusGeometry(radius, tube, 12, 192),
    remember(new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthWrite: false, blending: Additive })),
  );
}

function flatRing(inner, outer, color, opacity, start = 0, length = Math.PI * 2) {
  return new THREE.Mesh(
    new THREE.RingGeometry(inner, outer, 160, 1, start, length),
    remember(new THREE.MeshBasicMaterial({ color, transparent: true, opacity, depthWrite: false, blending: Additive, side: THREE.DoubleSide, forceSinglePass: true })),
  );
}

// In the theme's display font, which is loaded by the time a node is built
// (see themeFonts in theme.js): a label is drawn once, onto a canvas.
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

// A named identity's projects stand on a thin ring in its color (see
// identities.js; identityRing is its place among the theme's ring colors).
// Only the color: no identity text is ever drawn on the map, so posters,
// replay clips and wallpaper mode can't show one.
function identityRing(body, node, r) {
  const color = RING_COLORS[node.identityRing];
  if (!color) return;
  const ring = flatRing(r * 0.82, r * 0.95, color, 0.85);
  const halo = flatRing(r * 0.95, r * 1.25, color, 0.16);
  for (const m of [ring, halo]) {
    m.rotation.x = -Math.PI / 2;
    m.position.y = -r * 1.25;
    body.add(m);
  }
}

export function sizeFor(node) {
  switch (node.kind) {
    case "core":
      return 6;
    case "host":
      return HOST_SIZE;
    case "project":
      return 3.2 + 1.6 * Math.log2(1 + (node.sessionCount ?? 0));
    case "zone":
      return 2.6 + 1.1 * Math.log2(1 + (node.sessionCount ?? 0));
    case "native":
      return 4.2;
    case "resource":
      return 3.4;
    case "wiki":
      return 4.4;
    case "page":
      return 1.6;
    default:
      return node.archived ? 1.3 : 2;
  }
}

export function buildNode(node) {
  const color = new THREE.Color(nodeColor(node));
  const group = new THREE.Group();
  const body = new THREE.Group(); // bobs gently; the group itself holds layout position
  group.add(body);
  const anim = [];
  const r = sizeFor(node);
  const phase = Math.random() * Math.PI * 2;
  let lbl = null;
  let halo = null;

  switch (node.kind) {
    case "core": {
      const shell = fatEdges(new THREE.IcosahedronGeometry(10, 1), { color: COLORS.project, width: 1.3, opacity: 0.6 });
      const shell2 = fatEdges(new THREE.IcosahedronGeometry(15.5, 0), { color: COLORS.project, width: 1, opacity: 0.25 });
      const rim = new THREE.Mesh(new THREE.SphereGeometry(8.5, 48, 32), fresnelMaterial(COLORS.project, 1.2, 2.2));
      const rings = [0, 1, 2].map((i) => {
        const m = torus(20 + i * 4.5, 0.09, COLORS.project, 0.55 - i * 0.14);
        m.rotation.set(phase + i, phase * 0.5 + i * 1.3, 0);
        return m;
      });
      halo = glow(color, 46, 0.42);
      body.add(halo, solid(new THREE.SphereGeometry(4.2, 48, 32), color), rim, shell, shell2, ...rings);
      anim.push((t, dt) => {
        shell.rotation.y += dt * 0.15;
        shell.rotation.x += dt * 0.05;
        shell2.rotation.y -= dt * 0.08;
        rings.forEach((m, i) => {
          m.rotation.x += dt * (0.08 + i * 0.05);
          m.rotation.y += dt * 0.06;
        });
      });
      // With other machines on the map, this one's name beside its folder.
      lbl = label((node.selfName ? `${node.selfName} · ${node.name}` : node.name).toUpperCase(), COLORS.label, 5);
      lbl.position.y = -30;
      break;
    }
    case "host": {
      // Another machine's XenoNet, at the center of its constellation (see
      // hosts-draw.js).
      ({ label: lbl, halo } = buildHostBody(node, body, anim, phase));
      break;
    }
    case "project": {
      if (node.group) {
        // A group (a folder of projects): a hollow frame its projects sit
        // around, lit while any of them is.
        const frame = fatEdges(new THREE.BoxGeometry(r * 1.25, r * 1.25, r * 1.25), { color, width: 1.3, opacity: 0.75 });
        const ring = torus(r * 1.6, 0.05, color, 0.4);
        ring.rotation.x = Math.PI / 2;
        halo = glow(color, r * 6, node.activeCount ? 0.45 : node.active ? 0.32 : 0.14);
        body.add(halo, frame, ring);
        anim.push((t, dt) => {
          frame.rotation.y += dt * 0.12;
          frame.rotation.x = 0.35 + 0.08 * Math.sin(t * 0.3 + phase);
        });
        identityRing(body, node, r * 1.05);
        lbl = label(`${node.name.toUpperCase()} /`, COLORS.label, 3.6);
        lbl.position.y = r + 4.5;
        break;
      }
      const shell = fatEdges(new THREE.OctahedronGeometry(r, 0), { color, width: 1.6, opacity: 0.95 });
      const inner = solid(new THREE.OctahedronGeometry(r * 0.32, 0), color);
      const rim = new THREE.Mesh(new THREE.SphereGeometry(r * 0.95, 32, 24), fresnelMaterial(color, 0.5, 3));
      halo = glow(color, r * 7, node.activeCount ? 0.6 : node.active ? 0.45 : 0.2);
      body.add(halo, rim, shell, inner);
      anim.push((t, dt) => {
        shell.rotation.y += dt * 0.3;
        inner.rotation.y -= dt * 0.7;
      });
      if (node.activeCount) {
        const orbit = torus(r * 1.9, 0.06, color, 0.75);
        orbit.rotation.x = Math.PI / 2.4;
        body.add(orbit);
        anim.push((t, dt) => (orbit.rotation.z += dt * 0.8));
      }
      // Uncommitted work builds "pressure": a gold ring that thickens and
      // pulses faster with more changed files.
      const dirty = node.repo?.dirty ?? 0;
      let tag = "";
      if (dirty) {
        const k = Math.min(1, Math.log10(1 + dirty) / 2);
        const ring = torus(r * 1.45, 0.08 + k * 0.32, COLORS.pressure, 0.35 + k * 0.4);
        ring.rotation.x = Math.PI / 2;
        const ring2 = torus(r * 1.45, 0.03, COLORS.pressure, 0.9);
        ring2.rotation.x = Math.PI / 2;
        body.add(ring, ring2);
        anim.push((t) => {
          const s = 1 + 0.06 * Math.sin(t * (1.5 + k * 3) + phase);
          ring.scale.set(s, s, 1);
          ring.material.opacity = (0.35 + k * 0.4) * (0.75 + 0.25 * Math.sin(t * (1.5 + k * 3)));
        });
        tag = `  Δ${dirty}${node.repo.ahead ? ` ↑${node.repo.ahead}` : ""}`;
      }
      identityRing(body, node, r);
      lbl = label(node.name.toUpperCase() + tag, COLORS.label, node.satelliteOf ? 3 : 4);
      lbl.position.y = r + 4.5;
      break;
    }
    case "native": {
      const orbit = torus(r, 0.1, color, 0.85);
      const orbit2 = torus(r * 0.62, 0.06, color, 0.5);
      halo = glow(color, r * 5, 0.35);
      body.add(halo, orbit, orbit2, solid(new THREE.SphereGeometry(0.7, 24, 16), color));
      anim.push((t, dt) => {
        orbit.rotation.x += dt * 0.5;
        orbit2.rotation.y += dt * 0.9;
      });
      lbl = label(trim(node.name, 32), COLORS.native, 2.3);
      lbl.position.y = r + 3.5;
      break;
    }
    case "wiki": {
      // The shared memory: a crystal inside slow rings, with unfiled inbox
      // notes orbiting as debris until the curator files them.
      const crystal = fatEdges(new THREE.OctahedronGeometry(r * 0.9, 0), { color, width: 1.6, opacity: 0.95 });
      const core = solid(new THREE.OctahedronGeometry(r * 0.35, 0), "#ffffff");
      const rim = new THREE.Mesh(new THREE.SphereGeometry(r * 1.1, 32, 24), fresnelMaterial(color, 0.9, 2.4));
      const rings = [0, 1, 2].map((i) => {
        const m = torus(r * (1.6 + i * 0.45), 0.06, color, 0.6 - i * 0.15);
        m.rotation.set(Math.PI / 2 + i * 0.35, i * 0.8, 0);
        return m;
      });
      halo = glow(color, r * 8, 0.45);
      body.add(halo, rim, crystal, core, ...rings);
      anim.push((t, dt) => {
        crystal.rotation.y += dt * 0.4;
        core.rotation.y -= dt * 0.9;
        rings.forEach((m, i) => (m.rotation.z += dt * (0.15 + i * 0.1)));
      });
      const debris = (node.inbox ?? []).slice(0, 24).map((n, i) => {
        const rock = solid(new THREE.TetrahedronGeometry(0.55 + Math.random() * 0.35, 0), lighter(THEME.colors.gold, 0.4));
        rock.userData.orbit = { radius: r * 2.8 + (i % 3) * 1.4, tilt: new THREE.Euler(Math.random() * 1.2, Math.random() * Math.PI, 0), speed: 0.35 + Math.random() * 0.3, phase: Math.random() * Math.PI * 2 };
        body.add(rock);
        return rock;
      });
      const v = new THREE.Vector3();
      anim.push((t) => {
        for (const rock of debris) {
          const o = rock.userData.orbit;
          const a = o.phase + t * o.speed;
          rock.position.copy(v.set(Math.cos(a) * o.radius, 0, Math.sin(a) * o.radius).applyEuler(o.tilt));
          rock.rotation.x += 0.02;
          rock.rotation.y += 0.03;
        }
      });
      // Its name: Xenolith, or Agent Wiki for a wiki from before it.
      const title = String(node.name || "Xenolith").toUpperCase();
      lbl = label(debris.length ? `${title} · ${debris.length} UNFILED` : title, COLORS.wiki, 3.4);
      lbl.position.y = r + 5;
      break;
    }
    case "page": {
      const fresh = (node.writes ?? []).some((w) => Date.now() - w.ts < 2 * 3600e3);
      const star = new THREE.Sprite(remember(new THREE.SpriteMaterial({ map: starTex, color, transparent: true, opacity: fresh ? 1 : 0.8, depthWrite: false, blending: Additive })));
      const size = 6 + Math.min(6, (node.writes?.length ?? 0) * 1.2);
      star.scale.setScalar(size);
      halo = glow(color, size * 1.6, fresh ? 0.5 : 0.22);
      body.add(halo, star, solid(new THREE.SphereGeometry(0.45, 16, 12), "#ffffff"));
      anim.push((t) => {
        star.material.rotation = Math.sin(t * 0.3 + phase) * 0.2;
        if (fresh) star.scale.setScalar(size * (1 + 0.12 * Math.sin(t * 3 + phase)));
      });
      lbl = label(trim(node.name, 34), color, 1.9);
      lbl.position.y = 4.2;
      break;
    }
    case "resource": {
      // A shared system agents reach through MCP: a slowly turning lattice cube.
      const cube = fatEdges(new THREE.BoxGeometry(r * 1.4, r * 1.4, r * 1.4), { color, width: 1.4, opacity: 0.85 });
      const inner = fatEdges(new THREE.OctahedronGeometry(r * 0.6, 0), { color, width: 1, opacity: 0.6 });
      halo = glow(color, r * 6, 0.32);
      body.add(halo, cube, inner);
      anim.push((t, dt) => {
        cube.rotation.y += dt * 0.25;
        cube.rotation.x += dt * 0.12;
        inner.rotation.y -= dt * 0.6;
      });
      lbl = label(node.name.toUpperCase(), COLORS.resource, 2.6);
      lbl.position.y = r + 4;
      break;
    }
    case "zone": {
      const shell = fatEdges(new THREE.DodecahedronGeometry(r, 0), { color, width: 1.1, opacity: 0.5 });
      halo = glow(color, r * 4.5, 0.18);
      body.add(halo, shell);
      anim.push((t, dt) => (shell.rotation.y -= dt * 0.2));
      lbl = label(trim(node.name, 28).toUpperCase(), lighter(COLORS.zone, 0.15), 2.4);
      lbl.position.y = r + 3.5;
      break;
    }
    default: {
      const dim = node.archived;
      const c = dim ? color.clone().lerp(new THREE.Color(COLORS.archived), 0.6) : color;
      const core = solid(new THREE.IcosahedronGeometry(r, 3), c, dim ? 0.6 : 1);
      const rim = new THREE.Mesh(new THREE.SphereGeometry(r * 1.25, 32, 24), fresnelMaterial(c, dim ? 0.3 : 0.9, 2));
      const haloBase = ({ active: 0.9, recent: 0.48, idle: 0.24 }[node.activity] ?? 0.3) * (dim ? 0.4 : 1);
      halo = glow(c, r * (node.activity === "active" ? 8 : 6), haloBase);
      body.add(halo, rim, core);
      if (node.starred) {
        const star = torus(r * 1.9, 0.05, COLORS.lineage, 0.85);
        star.rotation.x = Math.PI / 3;
        body.add(star);
        anim.push((t, dt) => (star.rotation.z += dt * 1.2));
      }
      // Breathing at rest: a gentle bob, 0.45 at XenoNet's 0.1.
      anim.push((t) => (body.position.y = Math.sin(t * 0.9 + phase) * 4.5 * THEME.motion.breathe));
      lbl = label(trim(node.title, 46), COLORS.label, 2.1);
      lbl.position.y = r + 3.4;
      lbl.visible = node.activity === "active";
      group.userData.labelSticky = lbl.visible;
    }
  }
  // Past XenoNet's own breathing (motion.breathe above 0.1) every node's glow
  // swells and sinks too, slowly and out of step with the rest. A working
  // session's rig sets its glow over this (see animateLive).
  const swell = Math.min(1, Math.max(0, (THEME.motion.breathe - 0.1) / 0.9));
  if (swell && halo) {
    const size = halo.scale.x, rate = 0.45 + Math.random() * 0.3;
    anim.push((t) => halo.scale.setScalar(size * (1 + 0.35 * swell * Math.sin(t * rate + phase))));
  }
  if (lbl) body.add(lbl);
  Object.assign(group.userData, { label: lbl, anim, radius: r, color, scaleTarget: 1, halo, haloBase: { scale: halo?.scale.x ?? 1, opacity: halo?.material.opacity ?? 0 }, body });
  return group;
}

export function disposeNode(group) {
  group.traverse((o) => {
    o.geometry?.dispose();
    if (!o.material) return;
    // A label's text is a texture of its own; glows and rings share theirs.
    if (o.material.map && !SHARED_TEXTURES.has(o.material.map)) o.material.map.dispose();
    if (o.material.isLineMaterial) releaseMaterial(o.material);
    else o.material.dispose();
  });
}

// Glows add up: a few hundred halos at 10% still fog the screen and feed the
// bloom. So additive layers dim with the square of the factor, and a dimmed
// node keeps its solid core while its halo all but goes.
export function setDim(group, factor) {
  group.traverse((o) => {
    const m = o.material;
    if (!m) return;
    const f = m.blending === THREE.AdditiveBlending ? factor * factor : factor;
    if (m.uniforms?.opacity && m.userData.fresnelBase === undefined) m.userData.fresnelBase = m.uniforms.opacity.value;
    if (m.uniforms?.opacity) m.uniforms.opacity.value = m.userData.fresnelBase * f;
    else if (m.userData?.base !== undefined) m.opacity = m.userData.base * f;
  });
}

// ---------- live rig ----------

const STATUS_SPEED = { tool: 3.2, thinking: 0.8 };

function gaugeColor(pct, app) {
  if (pct > 0.9) return THEME.colors.danger;
  if (pct > 0.7) return THEME.colors.attention;
  return COLORS[app] ?? COLORS.project;
}

function ensureRig(node) {
  const g = node.__obs;
  if (!g) return null;
  if (g.userData.rig) return g.userData.rig;
  const r = g.userData.radius;
  const R = r * 2.2 + 1.6;
  const billboard = new THREE.Group();
  const track = flatRing(R, R + 0.34, "#ffffff", 0.08);
  const spinner = new THREE.Group();
  for (let k = 0; k < 3; k++) spinner.add(flatRing(R + 0.75, R + 0.92, "#ffffff", 0.85, (k * Math.PI * 2) / 3, Math.PI * 0.42));
  spinner.visible = false;
  billboard.add(track, spinner);
  const sats = new THREE.Group();
  g.add(billboard, sats);
  const rig = { group: g, billboard, track, fill: null, spinner, sats, R, pct: -1, satellites: new Map(), tpmSmooth: 0, status: "idle", attention: null, stuck: null };
  g.userData.rig = rig;
  return rig;
}

function setGauge(rig, pct, app) {
  if (Math.abs(pct - rig.pct) < 0.004 && rig.fill) return;
  rig.pct = pct;
  if (rig.fill) {
    rig.billboard.remove(rig.fill);
    rig.fill.geometry.dispose();
    rig.fill.material.dispose();
  }
  const len = Math.max(0.02, Math.PI * 2 * Math.min(1, pct));
  rig.fill = flatRing(rig.R, rig.R + 0.34, gaugeColor(pct, app), 0.95, Math.PI / 2 - len, len);
  rig.billboard.add(rig.fill);
}

function makeSatellite(rig, sub, color, index) {
  const group = new THREE.Group();
  const core = solid(new THREE.IcosahedronGeometry(0.55, 2), new THREE.Color(color).lerp(new THREE.Color("#ffffff"), 0.35));
  group.add(glow(color, 6, 0.75), core);
  const tag = label(trim(sub.label, 34), THEME.colors.text, 1.1);
  tag.position.y = 1.6;
  tag.visible = false;
  group.add(tag);
  const tether = fatLine(1, { color, width: 1, opacity: 0.45 });
  const orbit = {
    radius: rig.R + 3 + index * 1.4,
    tilt: new THREE.Euler(Math.random() * Math.PI, Math.random() * Math.PI, 0),
    speed: 0.6 + Math.random() * 0.5,
    phase: Math.random() * Math.PI * 2,
  };
  group.scale.setScalar(0.001);
  rig.sats.add(group, tether);
  return { group, tether, tag, orbit, born: performance.now(), dying: null };
}

// Called whenever a live snapshot arrives.
export function applyLive(node, live) {
  const rig = ensureRig(node);
  if (!rig) return;
  node.__live = live;
  rig.status = live?.status ?? "idle";
  if (!live) {
    rig.billboard.visible = false;
    for (const s of rig.satellites.values()) s.dying ??= performance.now();
    return;
  }
  rig.billboard.visible = live.context.used > 0;
  setGauge(rig, live.context.window ? live.context.used / live.context.window : 0, node.app);
  const tint = live.status === "tool" ? CATEGORY_COLORS[live.lastTool?.category] ?? "#ffffff" : COLORS[node.app];
  for (const m of rig.spinner.children) m.material.color.set(tint);
  rig.spinner.visible = live.status !== "idle";

  const active = new Set(live.subagents.filter((a) => a.active).map((a) => a.id));
  let i = rig.satellites.size;
  for (const sub of live.subagents) {
    if (!active.has(sub.id) || rig.satellites.has(sub.id)) continue;
    rig.satellites.set(sub.id, makeSatellite(rig, sub, COLORS[node.app], i++));
  }
  for (const [id, s] of rig.satellites) if (!active.has(id)) s.dying ??= performance.now();
}

// ---------- attention: needs you, ready for review, stuck ----------

function buildAttention(rig, kind) {
  const g = new THREE.Group();
  const strong = kind === "question" || kind === "permission";
  const color = strong ? COLORS.attention : COLORS.review;
  const parts = { group: g, kind, strong, color };
  if (strong) {
    // A lighthouse: a shaft of light straight up and a sweeping beam.
    const h = 70;
    for (const rot of [0, Math.PI / 2]) {
      const shaft = new THREE.Mesh(new THREE.PlaneGeometry(5, h), pillarMaterial(color));
      shaft.position.y = h / 2;
      shaft.rotation.y = rot;
      g.add(shaft);
    }
    const beam = new THREE.Mesh(new THREE.ConeGeometry(7, 26, 48, 1, true), fresnelMaterial(color, 0.55, 1.2));
    beam.rotation.z = Math.PI / 2;
    beam.position.x = 13;
    const sweep = new THREE.Group();
    sweep.add(beam);
    g.add(sweep);
    parts.sweep = sweep;
    parts.shafts = g.children.filter((c) => c.material?.uniforms?.time);
  }
  const ring = new THREE.Sprite(new THREE.SpriteMaterial({ map: ringTex, color, transparent: true, depthWrite: false, blending: Additive }));
  ring.renderOrder = 14;
  g.add(ring);
  parts.ring = ring;
  rig.group.add(g);
  return parts;
}

function dropAttention(rig) {
  if (!rig.attention) return;
  rig.group.remove(rig.attention.group);
  disposeNode(rig.attention.group);
  rig.attention = null;
}

export function setAttention(node, { need = null, stuck = null } = {}) {
  const rig = ensureRig(node);
  if (!rig) return;
  const kind = need?.kind ?? null;
  if (rig.attention?.kind !== kind) {
    dropAttention(rig);
    if (kind) rig.attention = buildAttention(rig, kind);
  }
  rig.stuck = stuck;
}

const tmp = new THREE.Vector3();
const zero = new THREE.Vector3();

// Called every frame for nodes that have a rig.
export function animateLive(node, t, dt, camera, focused) {
  const g = node.__obs;
  const rig = g?.userData.rig;
  if (!rig) return;
  rig.billboard.quaternion.copy(camera.quaternion);
  if (rig.spinner.visible) rig.spinner.rotation.z -= dt * (STATUS_SPEED[rig.status] ?? 0);

  // Token flux swells the halo.
  const tpm = node.__live?.tpm ?? 0;
  rig.tpmSmooth += (tpm - rig.tpmSmooth) * Math.min(1, dt * 2);
  const halo = g.userData.halo;
  if (halo) {
    const boost = Math.min(1.6, Math.log10(1 + rig.tpmSmooth) / 4);
    const pulse = rig.status !== "idle" ? 1 + 0.15 * Math.sin(t * 5) : 1;
    halo.scale.setScalar(g.userData.haloBase.scale * (1 + boost) * pulse);
    halo.material.opacity = Math.min(1, g.userData.haloBase.opacity + boost * 0.4);
    // Stuck: the halo flickers orange and the body glitches now and then.
    if (rig.stuck) {
      // No flicker while held still (dt 0; see setStill in scene.js).
      const flick = dt && Math.random() < 0.12 ? 0.25 : 1;
      halo.material.color.set(COLORS.stuck);
      halo.material.opacity = Math.min(1, 0.8 * flick + 0.2);
      const body = g.userData.body;
      if (body) body.position.x = dt && Math.random() < 0.04 ? (Math.random() - 0.5) * 1.2 : 0;
    } else if (rig.attention?.strong) {
      // Waiting on you: the halo turns amber and breathes, visible from afar.
      halo.material.color.set(COLORS.attention);
      halo.material.opacity = 0.75 + 0.25 * Math.sin(t * 4);
      halo.scale.setScalar(g.userData.haloBase.scale * 1.8 * (1 + 0.2 * Math.sin(t * 4)));
    } else halo.material.color.copy(g.userData.color);
  }

  const att = rig.attention;
  if (att) {
    const period = att.strong ? 1.6 : 2.6;
    const p = (t % period) / period;
    att.ring.scale.setScalar(g.userData.radius * (2 + p * (att.strong ? 18 : 10)));
    att.ring.material.opacity = (1 - p) * (att.strong ? 0.95 : 0.6);
    if (att.sweep) att.sweep.rotation.y += dt * 1.6;
    for (const s of att.shafts ?? []) {
      s.material.uniforms.time.value = t;
      s.material.uniforms.opacity.value = 0.75 + 0.25 * Math.sin(t * 3);
    }
  }

  const now = performance.now();
  for (const [id, s] of rig.satellites) {
    const o = s.orbit;
    const a = o.phase + t * o.speed;
    tmp.set(Math.cos(a) * o.radius, 0, Math.sin(a) * o.radius).applyEuler(o.tilt);
    s.group.position.copy(tmp);
    const grow = Math.min(1, (now - s.born) / 600);
    const fade = s.dying ? Math.max(0, 1 - (now - s.dying) / 700) : 1;
    s.group.scale.setScalar(Math.max(0.001, grow * fade));
    s.tag.visible = focused;
    writeLine(s.tether, [zero, tmp]);
    s.tether.material.opacity = 0.45 * grow * fade;
    if (s.dying && fade <= 0) {
      rig.sats.remove(s.group, s.tether);
      disposeNode(s.group);
      s.tether.geometry.dispose();
      releaseMaterial(s.tether.material);
      rig.satellites.delete(id);
    }
  }
}
