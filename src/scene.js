import ForceGraph3D from "3d-force-graph";
import * as THREE from "three";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import { ShaderPass } from "three/addons/postprocessing/ShaderPass.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { COLORS, CATEGORY_COLORS, nodeColor, THEME, onTheme } from "./theme.js";
import {
  Additive, glowTex, sparkTex, ringTex, reticleTex, fatEdges, fatLine, writeLine, arcPoints, pointsArray,
  releaseMaterial, setLineResolution, nebula, LensShader, skyColors,
} from "./fx.js";
import { STARS, GRID, SWEEP, SONAR, LINES, pick } from "./looks/index.js";
import { sonarGap } from "../core/themes.js";
import { buildNode, disposeNode, setDim, sizeFor, applyLive, animateLive, setAttention } from "./nodes.js";
import { createActorLayer, isActorId } from "./actors.js";
import { pointerTakes } from "./actors-model.js";
import { freeTangents, frameFree } from "./framing.js";
import { hostVisual, STALE_LIGHT, sameHost } from "./hosts.js";
import { createHostLayer } from "./hosts-draw.js";
import { onEnvironment, environmentState } from "./environment.js";
import { environmentOf } from "../core/environments.js";

const HIERARCHY = new Set(["contains", "nested", "satellite", "native", "zone", "session", "lineage", "resource", "wiki", "page"]);
const idOf = (end) => (typeof end === "object" ? end.id : end);
export const linkKey = (l) => `${idOf(l.source)}>${idOf(l.target)}:${l.kind}`;
const posOf = (n, v = new THREE.Vector3()) => v.set(n?.x ?? 0, n?.y ?? 0, n?.z ?? 0);

const LINK_STYLE = {
  contains: { width: 1.5, opacity: 0.5 },
  nested: { width: 1.3, opacity: 0.5 },
  satellite: { width: 1.3, opacity: 0.5 },
  native: { width: 1.3, opacity: 0.55 },
  zone: { width: 1.1, opacity: 0.35 },
  resource: { width: 1.2, opacity: 0.4 },
  session: { width: 1.1, opacity: 0.38 },
  lineage: { width: 1.9, opacity: 0.85 },
  "lineage-cross": { width: 1.6, opacity: 0.7 },
  wiki: { width: 1.5, opacity: 0.5 },
  page: { width: 0.9, opacity: 0.3 },
  // Another machine's host node hangs from this one's core only to keep the
  // map one tree; it sits beside it (see hosts.js), so the tie is faint.
  host: { width: 0.8, opacity: 0.14 },
};

// Arcs are drawn in a role's color (see COLORS), looked up as they're made
// and again when the theme changes.
const RELATION_STYLE = {
  wikilink: { role: "wiki", width: 0.9, opacity: 0.22, bow: 0.22 },
  mentions: { role: "project", width: 0.8, opacity: 0.16, bow: 0.3 },
};

// The floor: its height and radius.
const FLOOR = -230, FLOOR_RADIUS = 440;

export function createScene(el, { onSelect, onHover, onPreview, onSonar, onPin, onGather, insets }) {
  const nodeIndex = new Map();
  let visible = null;
  let dimmed = null;
  let selected = null;
  let hovered = null;
  let lastClick = { id: null, t: 0 };
  let lastPointer = null;
  let liveData = null;
  let wallpaper = null;
  let labelFade = 1;

  // ---------- links: in the theme's style of line (see looks/lines.js) ----------

  // After a change of theme the force graph makes every link's line again,
  // but places lines only while the layout runs; the frame loop places the
  // new ones (see relink).
  let relinking = null; // { lines, since }

  function makeLink(l) {
    const src = nodeIndex.get(idOf(l.source));
    const dst = nodeIndex.get(idOf(l.target));
    const from = l.kind.startsWith("lineage") ? COLORS.lineage : src ? nodeColor(src) : COLORS.project;
    const to = l.kind.startsWith("lineage") ? COLORS.lineage : dst ? nodeColor(dst) : COLORS.project;
    const style = LINK_STYLE[l.kind] ?? LINK_STYLE.session;
    const opacity = dst?.archived ? style.opacity * 0.4 : style.opacity;
    const look = pick(LINES, THEME.lines.style);
    const width = style.width * THEME.lines.width;
    // Lineage across projects always bows, so it doesn't run through them.
    const bow = l.kind === "lineage-cross" ? Math.max(0.3, THEME.lines.curvature) : THEME.lines.curvature;
    const line = look.build({ from, to, width, opacity, bow });
    Object.assign(line.userData, { link: l, look, style, width, baseOpacity: opacity, glow: 0 });
    relinking?.lines.push(line);
    return line;
  }

  const placeLink = (line, start, end) => line.userData.look.place(line, start, end);

  const graph = new ForceGraph3D(el, { controlType: "orbit", rendererConfig: { antialias: true, powerPreference: "high-performance" } })
    .backgroundColor(THEME.colors.bg)
    .showNavInfo(false)
    .nodeLabel(() => "")
    .nodeThreeObject((n) => (n.__obs = buildNode(n)))
    .nodeThreeObjectExtend(false)
    .nodeVisibility((n) => !visible || visible.has(n.id))
    .linkVisibility((l) => !visible || (visible.has(idOf(l.source)) && visible.has(idOf(l.target))))
    .linkThreeObject(makeLink)
    .linkThreeObjectExtend(false)
    .linkPositionUpdate((line, { start, end }) => {
      placeLink(line, start, end);
      return true;
    })
    .linkDirectionalParticles(0)
    .dagMode("radialout")
    .dagLevelDistance(72)
    .onDagError(() => {})
    .d3VelocityDecay(0.32)
    .warmupTicks(160)
    .cooldownTime(3500)
    // Moving nodes is Alt-drag (see below), so a drag that starts on a node
    // still turns the view.
    .enableNodeDrag(false)
    .onNodeHover((node) => hoverNode(node && lastPointer ? (nodeBodyAt(lastPointer) ?? node) : node))
    .onNodeClick((node, ev) => {
      // A foreground glow or label must not steal another node's body.
      node = nodeBodyAt(ev) ?? node;
      // The graph doesn't know actors (see actors.js), and its hits count a
      // node's glow: an actor under the pointer takes the click when it's
      // drawn in front of the node's own body, or no body is under it.
      const actor = actorAt(ev);
      if (actorInFront(actor, node)) return onSelect?.(actor.rec);
      const now = performance.now();
      const dbl = lastClick.id === node.id && now - lastClick.t < 400;
      lastClick = { id: node.id, t: now };
      if (dbl && node.kind === "session") onPreview?.(node);
      else onSelect?.(node);
    })
    .onBackgroundClick((ev) => onSelect?.(actorAt(ev)?.rec ?? null));

  function hoverNode(node) {
    if (hovered && hovered !== selected) unhighlight(hovered);
    hovered = node;
    if (node) highlight(node);
    el.style.cursor = node || hoveredActor ? "pointer" : "grab";
    onHover?.(hoveredActor ?? node);
  }

  // An actor under the pointer is hovered over whatever's behind it.
  let hoveredActor = null;
  function hoverActor(rec) {
    if (rec === hoveredActor) return;
    hoveredActor = rec;
    actors.hover(rec?.id ?? null);
    el.style.cursor = rec || hovered ? "pointer" : "grab";
    onHover?.(rec ?? hovered);
  }

  // Whether the force layout is still placing nodes, for `motion` below. It
  // runs for `cooldownTime` after the data changes or the layout is reheated.
  let engineRunning = false;
  graph.onEngineTick(() => (engineRunning = true));

  graph.d3Force("charge").strength(-120);
  graph.d3Force("link").distance((l) => ({ contains: 70, nested: 34, satellite: 34, native: 26, zone: 70, resource: 80, session: 30, lineage: 18, wiki: 80, page: 24 })[l.kind] ?? 40);

  // ---------- renderer & post-processing ----------

  const renderer = graph.renderer();
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = THEME.glow.exposure;
  const composer = graph.postProcessingComposer();
  // The composer renders off-screen, where the canvas's own antialiasing does
  // not apply; a multisampled half-float target brings MSAA back and avoids banding.
  const size = renderer.getDrawingBufferSize(new THREE.Vector2());
  composer.reset(new THREE.WebGLRenderTarget(size.x, size.y, { samples: 4, type: THREE.HalfFloatType }));
  composer.setPixelRatio(renderer.getPixelRatio());
  const bloom = new UnrealBloomPass(new THREE.Vector2(innerWidth, innerHeight), THEME.glow.bloom, THEME.glow.radius, THEME.glow.threshold);
  const lens = new ShaderPass(LensShader);
  composer.addPass(bloom);
  composer.addPass(lens);
  composer.addPass(new OutputPass());

  const scene = graph.scene();
  const camera = graph.camera();
  // Other machines (hosts-draw.js): each a constellation of its own beside
  // this one's map, with bridges between checkouts of one repo. Nothing
  // changes until a host is on the map.
  const hosts = createHostLayer({ graph, scene, shown: (id) => !visible || visible.has(id), lightOf: (a, b) => lightOf(a, b), lit: (n) => n === hovered || branch.has(n.id) });

  // ---------- environment ----------
  //
  // The sky, the stars, the lattice around the map, the floor, the sweep
  // and the dust. The stars, floor and sweep are drawn in the theme's style
  // of each (see looks/), and built again when the theme changes; the rest
  // take the new theme's colors in place.

  scene.fog = new THREE.FogExp2(THEME.colors.bg, THEME.glow.fog);
  const sky = nebula();
  // A part drawn in a style: { look, object }, the old one taken away.
  function restyle(part, registry, name, build) {
    if (part) {
      scene.remove(part.object);
      part.look.dispose(part.object);
    }
    const look = pick(registry, name);
    const object = build(look);
    scene.add(object);
    return { look, object };
  }
  const starsOf = (theme, old) => restyle(old, STARS, theme.stars.shape, (look) => look.build(theme));
  const gridOf = (theme, old) => restyle(old, GRID, theme.grid.style, (look) => look.build(theme, { radius: FLOOR_RADIUS, y: FLOOR }));
  const sweepOf = (theme, old) => restyle(old, SWEEP, theme.sweep.shape, (look) => look.build(theme, { radius: FLOOR_RADIUS, y: FLOOR, color: COLORS.project }));
  scene.add(sky);
  let stars = starsOf(THEME);
  const lattice = fatEdges(new THREE.IcosahedronGeometry(900, 2), { color: COLORS.project, width: 0.8, opacity: THEME.lattice.opacity });
  scene.add(lattice);
  let grid = gridOf(THEME);
  let sweep = sweepOf(THEME);
  const dustGeo = new THREE.BufferGeometry();
  const dustPos = new Float32Array(700 * 3);
  for (let i = 0; i < 700; i++) dustPos.set(new THREE.Vector3().randomDirection().multiplyScalar(Math.random() * 520).toArray(), i * 3);
  dustGeo.setAttribute("position", new THREE.BufferAttribute(dustPos, 3));
  const dust = new THREE.Points(dustGeo, new THREE.PointsMaterial({ map: sparkTex, color: THEME.dust.color, size: THEME.dust.size, transparent: true, opacity: THEME.dust.opacity, depthWrite: false, blending: Additive }));
  scene.add(dust);
  function setEnvironment(environment) {
    sky.material.uniforms.environment.value = environment.index;
    stars.object.visible = environment.id === "nebula";
    lattice.visible = environment.id === "nebula";
  }
  setEnvironment(environmentOf(environmentState().id));
  onEnvironment(setEnvironment);

  // Sky follows the clock; `rest` dims it once everyone has stopped for the day.
  let restLevel = 0, restTarget = 0;
  let skyHour = new Date().getHours() + new Date().getMinutes() / 60;
  function setSky(hour) {
    skyHour = hour;
    const [deep, a, b, band] = skyColors(hour);
    const u = sky.material.uniforms;
    u.accent.value.set(THEME.colors.accent);
    u.deep.value.copy(deep);
    u.cloudA.value.copy(a);
    u.cloudB.value.copy(b);
    u.bandColor.value.copy(band);
  }
  setSky(skyHour);

  // ---------- controls ----------

  const controls = graph.controls();
  controls.autoRotate = true;
  controls.autoRotateSpeed = 0.28;
  controls.enableDamping = true;
  controls.dampingFactor = 0.06;
  let idleTimer = null;
  controls.addEventListener("start", () => {
    controls.autoRotate = false;
    clearTimeout(idleTimer);
  });
  // Auto-rotation resumes after 20 s idle, except over a gathering, which
  // faces the camera, and while the view is held still (see setStill).
  const idle = () => {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (controls.autoRotate = !fan && !still), 20000);
  };
  controls.addEventListener("end", idle);

  // ---------- moving nodes ----------
  //
  // A plain drag always turns the view, even when it starts on a node: close
  // in, nodes cover most of the screen, and a drag that grabbed one instead
  // moved and pinned it by accident. Alt-drag (⌥ on a Mac) moves the node
  // under the pointer across the screen's plane and pins it where it's
  // dropped (saved server-side). Only that node moves; the rest of the map
  // stays put. A gathered node dropped elsewhere leaves the gathering, and a
  // gathered root takes its gathering along.

  const ray = new THREE.Raycaster(), ndc = new THREE.Vector2(), dragPlane = new THREE.Plane();
  const dragHit = new THREE.Vector3(), dragOffset = new THREE.Vector3();
  let dragging = null;
  function aim(e) {
    const r = renderer.domElement.getBoundingClientRect();
    ndc.set(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
    ray.setFromCamera(ndc, camera);
  }
  const pointerOnPlane = (e) => (aim(e), ray.ray.intersectPlane(dragPlane, dragHit));
  // The actor under the pointer, nearest first: { rec, distance }, or null.
  const actorAt = (e) => (e && Number.isFinite(e.clientX) ? (aim(e), actors.pick(ray.ray)) : null);
  // How far along the pointer's ray (as aim last set it) a node's own body
  // starts, not just its glow; Infinity where the ray misses the body.
  const bodySphere = new THREE.Sphere(), bodyHit = new THREE.Vector3();
  function bodyDistance(node) {
    if (!node) return Infinity;
    bodySphere.set(posOf(node, bodySphere.center), sizeFor(node) * (node.__obs?.scale.x ?? 1) * 1.3);
    return ray.ray.intersectSphere(bodySphere, bodyHit) ? bodyHit.distanceTo(ray.ray.origin) : Infinity;
  }
  function nodeBodyAt(e) {
    if (!e || !Number.isFinite(e.clientX)) return null;
    aim(e);
    let nearest = null, distance = Infinity;
    for (const n of graph.graphData().nodes) if (n.__obs?.visible && (!visible || visible.has(n.id))) {
      const d = bodyDistance(n);
      if (d < distance) { distance = d; nearest = n; }
    }
    return nearest;
  }
  el.addEventListener("pointermove", (e) => { lastPointer = { clientX: e.clientX, clientY: e.clientY }; });
  // Whether an actor under the pointer (`hit`, from actorAt) is drawn in
  // front of the node there (pointerTakes), for a click and for hovering
  // alike, so what's hovered is what a click takes.
  const actorInFront = (hit, node) => !!hit && pointerTakes(hit.distance, bodyDistance(node)) === "actor";
  // The drawn node under the pointer, nearest first.
  function nodeAt(e) {
    aim(e);
    const owner = new Map();
    for (const n of graph.graphData().nodes) if (n.__obs?.visible && (!visible || visible.has(n.id))) owner.set(n.__obs, n);
    for (const hit of ray.intersectObjects([...owner.keys()], true))
      for (let o = hit.object; o; o = o.parent) if (owner.has(o)) return owner.get(o);
    return null;
  }
  function placeNode(n, p) {
    Object.assign(n, { x: p.x, y: p.y, z: p.z, vx: 0, vy: 0, vz: 0, fx: p.x, fy: p.y, fz: p.z });
    n.__obs?.position.copy(p);
    for (const l of graph.graphData().links) if (l.__lineObj?.userData.look && (l.source === n || l.target === n)) placeLink(l.__lineObj, l.source, l.target);
  }
  el.addEventListener(
    "pointerdown",
    (e) => {
      if (!e.altKey || e.button !== 0) return;
      const n = nodeAt(e);
      if (!n) return;
      e.stopPropagation(); // the camera controls never see it
      e.preventDefault();
      dragPlane.setFromNormalAndCoplanarPoint(camera.getWorldDirection(dragPlane.normal), posOf(n));
      if (!pointerOnPlane(e)) return;
      dragging = n;
      moves.delete(dragging);
      dragOffset.copy(posOf(dragging)).sub(dragHit);
      el.style.cursor = "move";
    },
    true,
  );
  addEventListener("pointermove", (e) => {
    if (dragging && pointerOnPlane(e)) placeNode(dragging, dragHit.add(dragOffset));
  });
  addEventListener("pointerup", () => {
    if (!dragging) return;
    const n = dragging;
    dragging = null;
    el.style.cursor = hovered ? "pointer" : "grab";
    n.pinned = true;
    dropped(n);
    // Another machine's host node takes its constellation along (hosts.js).
    if (n.kind === "host") {
      engineRunning = true;
      graph.d3ReheatSimulation();
    }
    onPin?.(n);
  });

  // ---------- selection reticle ----------

  const reticle = new THREE.Sprite(new THREE.SpriteMaterial({ map: reticleTex, color: "#fff", transparent: true, opacity: 0.9, depthTest: false, depthWrite: false, blending: Additive }));
  reticle.renderOrder = 20;
  reticle.visible = false;
  scene.add(reticle);

  // `baseScale` shrinks a node on display (a gathering's root) without
  // touching its size anywhere else.
  function highlight(node) {
    const g = node.__obs;
    if (!g) return;
    g.userData.scaleTarget = 1.4 * (g.userData.baseScale ?? 1);
    if (g.userData.label) g.userData.label.visible = true;
  }
  function unhighlight(node) {
    const g = node.__obs;
    if (!g) return;
    g.userData.scaleTarget = g.userData.baseScale ?? 1;
    if (g.userData.label && node.kind === "session") g.userData.label.visible = !!g.userData.labelSticky || !!fan?.items.has(node);
  }

  // ---------- packets: glowing data moving along links and arcs ----------

  const packetPool = [];
  const packets = [];
  function packet({ from, to, color, size = 2.4, duration = 1.1, bow = 0 }) {
    if (packets.length > 600) return;
    const s = packetPool.pop() ?? new THREE.Sprite(new THREE.SpriteMaterial({ map: glowTex, transparent: true, depthWrite: false, blending: Additive }));
    s.material.color.set(color);
    s.renderOrder = 12;
    scene.add(s);
    packets.push({ s, from, to, size, duration, bow, t: 0, pts: bow ? pointsArray(1) : null });
  }
  const pa = new THREE.Vector3(), pb = new THREE.Vector3(), pc = new THREE.Vector3();
  function stepPackets(dt) {
    for (let i = packets.length - 1; i >= 0; i--) {
      const p = packets[i];
      p.t += dt / p.duration;
      if (p.t >= 1) {
        scene.remove(p.s);
        packetPool.push(p.s);
        packets.splice(i, 1);
        continue;
      }
      posOf(p.from, pa);
      posOf(p.to, pb);
      const e = p.t < 0.5 ? 2 * p.t * p.t : 1 - (-2 * p.t + 2) ** 2 / 2;
      if (p.bow) {
        const mid = pc.addVectors(pa, pb).multiplyScalar(0.5);
        const len = pa.distanceTo(pb);
        const lift = mid.clone().normalize().multiplyScalar(len * p.bow);
        const ctrl = mid.add(lift);
        const u = 1 - e;
        p.s.position.set(u * u * pa.x + 2 * u * e * ctrl.x + e * e * pb.x, u * u * pa.y + 2 * u * e * ctrl.y + e * e * pb.y, u * u * pa.z + 2 * u * e * ctrl.z + e * e * pb.z);
      } else p.s.position.lerpVectors(pa, pb, e);
      p.s.scale.setScalar(p.size * (0.6 + 0.6 * Math.sin(Math.PI * p.t)));
      p.s.material.opacity = Math.sin(Math.PI * p.t) * lightOf(p.from, p.to);
    }
  }

  // ---------- sparks: a burst when an agent calls a tool ----------

  const sparks = [];
  function spark(node, color, count = 22) {
    const geo = new THREE.BufferGeometry();
    const pos = new Float32Array(count * 3);
    const vel = [];
    const c = posOf(node);
    for (let i = 0; i < count; i++) {
      pos.set([c.x, c.y, c.z], i * 3);
      vel.push(new THREE.Vector3().randomDirection().multiplyScalar(7 + Math.random() * 12));
    }
    geo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
    const pts = new THREE.Points(geo, new THREE.PointsMaterial({ map: sparkTex, color, size: 1.8, transparent: true, depthWrite: false, blending: Additive }));
    pts.renderOrder = 13;
    scene.add(pts);
    sparks.push({ pts, vel, t: 0 });
  }
  function stepSparks(dt) {
    for (let i = sparks.length - 1; i >= 0; i--) {
      const s = sparks[i];
      s.t += dt / 0.9;
      if (s.t >= 1) {
        scene.remove(s.pts);
        s.pts.geometry.dispose();
        s.pts.material.dispose();
        sparks.splice(i, 1);
        continue;
      }
      const arr = s.pts.geometry.attributes.position.array;
      const drag = Math.max(0, 1 - s.t * 1.2);
      s.vel.forEach((v, j) => {
        arr[j * 3] += v.x * dt * drag;
        arr[j * 3 + 1] += v.y * dt * drag;
        arr[j * 3 + 2] += v.z * dt * drag;
      });
      s.pts.geometry.attributes.position.needsUpdate = true;
      s.pts.material.opacity = 1 - s.t;
    }
  }

  // ---------- ring flashes and sonar ----------

  const flashes = [];
  function flash(node, color, scale = 14, duration = 1.6) {
    const s = new THREE.Sprite(new THREE.SpriteMaterial({ map: ringTex, color: color ?? node.__obs?.userData.color ?? "#fff", transparent: true, depthWrite: false, blending: Additive }));
    posOf(node, s.position);
    s.renderOrder = 15;
    scene.add(s);
    flashes.push({ s, t0: performance.now(), r: sizeFor(node), scale, duration });
  }

  // In the theme's style of sonar (see looks/sonar.js); each wave keeps the
  // style it started in.
  const waves = [];
  function sonar() {
    const look = pick(SONAR, THEME.sonar.style);
    const core = nodeIndex.get("core");
    const wave = look.start(THEME, { center: core ? posOf(core) : null, floorY: FLOOR, color: COLORS.project });
    wave.look = look;
    scene.add(wave.object);
    waves.push(wave);
    onSonar?.();
  }
  function dropWave(wave) {
    scene.remove(wave.object);
    wave.look.dispose(wave);
  }
  // Time since the last wave, for the theme's sonar.every (sonarGap in
  // core/themes.js: at most one a second, and none for 0 or less).
  let sonarClock = 0;

  // ---------- arcs: collaboration, shared resources, synchronized work ----------

  const arcs = new Map();
  function upsertArc(key, a, b, { role, width, opacity, bow, flow }) {
    const color = COLORS[role];
    let arc = arcs.get(key);
    if (!arc) {
      const line = fatLine(40, { color, width, opacity: 0 });
      line.renderOrder = 6;
      scene.add(line);
      arc = { line, pts: pointsArray(40), born: performance.now() };
      arcs.set(key, arc);
    }
    Object.assign(arc, { a, b, opacity, bow, flow, role, color, dying: null, seen: true });
    return arc;
  }
  function stepArcs(t, dt) {
    const now = performance.now();
    for (const [key, arc] of arcs) {
      if (!arc.a.__obs || !arc.b.__obs) arc.dying ??= now;
      const show = !visible || (visible.has(arc.a.id) && visible.has(arc.b.id));
      arcPoints(posOf(arc.a), posOf(arc.b), arc.pts, arc.bow);
      writeLine(arc.line, arc.pts);
      const grow = Math.min(1, (now - arc.born) / 800);
      const fade = arc.dying ? Math.max(0, 1 - (now - arc.dying) / 800) : 1;
      arc.line.visible = show;
      arc.line.material.opacity = arc.opacity * grow * fade * (0.75 + 0.25 * Math.sin(t * 2.4 + key.length));
      if (show && arc.flow && !arc.dying && Math.random() < arc.flow * dt) {
        const fwd = Math.random() < 0.5;
        packet({ from: fwd ? arc.a : arc.b, to: fwd ? arc.b : arc.a, color: arc.color, size: 2.2, duration: 1.6, bow: arc.bow });
      }
      if (arc.dying && fade <= 0) {
        scene.remove(arc.line);
        arc.line.geometry.dispose();
        releaseMaterial(arc.line.material);
        arcs.delete(key);
      }
    }
  }

  // Which project (or zone) a node ultimately lives in.
  let parentOf = new Map();
  let childrenOf = new Map();
  function homeOf(id) {
    let cur = id;
    for (let i = 0; i < 6 && cur; i++) {
      const n = nodeIndex.get(cur);
      if (n && n.kind !== "session") return cur;
      cur = parentOf.get(cur);
    }
    return null;
  }

  // ---------- link flow driven by live activity ----------

  const flowRate = new Map(); // target node id → packets per second
  function recomputeFlow() {
    flowRate.clear();
    if (!liveData) return;
    const byId = new Map(liveData.sessions.map((s) => [s.id, s]));
    for (const n of nodeIndex.values()) {
      if (n.kind !== "session" || n.stale) continue;
      const live = byId.get(n.id);
      const rate = live?.status === "tool" ? 2.6 : live?.status === "thinking" ? 1.2 : n.activity === "active" ? 0.12 : 0;
      if (!rate) continue;
      flowRate.set(n.id, rate);
      let up = parentOf.get(n.id);
      let carry = rate * 0.4;
      while (up) {
        flowRate.set(up, Math.min(2.2, (flowRate.get(up) ?? 0) + carry));
        carry *= 0.6;
        up = parentOf.get(up);
      }
    }
    for (const r of liveData.resources ?? []) if (Date.now() - r.ts < 120e3) flowRate.set(`resource:${r.server}`, 0.6);
  }

  function stepLinks(t, dt) {
    for (const l of graph.graphData().links) {
      const line = l.__lineObj;
      if (!line?.userData?.look) continue;
      const tgt = typeof l.target === "object" ? l.target : nodeIndex.get(l.target);
      const src = typeof l.source === "object" ? l.source : nodeIndex.get(l.source);
      // A host's tie to this core carries nothing: its work is drawn in its
      // own constellation (hosts.js).
      const rate = l.kind === "host" ? 0 : (flowRate.get(tgt?.id) ?? 0);
      const target = rate > 0 ? 1 : 0;
      line.userData.glow = still ? target : line.userData.glow + (target - line.userData.glow) * Math.min(1, dt * 3);
      const g = line.userData.glow;
      const dimF = lightOf(src, tgt);
      line.userData.look.update(line, Math.min(1, line.userData.baseOpacity * (1 + g * (1.3 + 0.3 * Math.sin(t * 6))) * dimF), line.userData.width + g * 0.9, t);
      if (rate && !still && line.visible && src && tgt && Math.random() < rate * dt) packet({ from: src, to: tgt, color: nodeColor(tgt), size: tgt.kind === "session" ? 2.6 : 3.2, duration: 0.9 + Math.random() * 0.5 });
    }
  }

  // ---------- lighting: search dimming and focus ----------
  //
  // Close in on a node or one corner of the map and the rest of it dims with
  // distance from where you're looking, so far-off clusters and the core's
  // glow don't wash out what's in front of you through the bloom. Nodes that
  // drift between the camera and what you're looking at fade too, instead of
  // blooming into large discs, and the bloom itself eases off. With a node
  // selected, its own branch stays fully lit and its neighbours step back a
  // little. Sessions that need you or look stuck never fade far. Pulled back
  // to the overview, everything is lit as before.

  const ease = (x, a, b) => {
    const k = Math.min(1, Math.max(0, (x - a) / (b - a)));
    return k * k * (3 - 2 * k);
  };
  let extent = 400; // the drawn map's radius, refreshed every second
  let extentAt = 0;
  let focusLevel = 0; // 0 at the overview, 1 close in
  let focusLighting = true;
  let branch = new Set(); // the selection, what it hangs from and what hangs from it
  const lp = new THREE.Vector3();

  const lightOf = (a, b) => Math.min(a?.__obs?.userData.dim ?? 1, b?.__obs?.userData.dim ?? 1);

  // Projects, Codex projects and folders where nothing ran lately (`active`
  // from the server, or a session working there right now) step back, so the
  // ones in use stand out. Unlike the filters, nothing is hidden, and a
  // quiet place in focus lights up as usual.
  const PLACES = new Set(["project", "native", "zone"]);
  const QUIET_PLACE = 0.38;
  let busy = new Set(); // places with a session working now
  const quiet = (n) => PLACES.has(n.kind) && !n.active && !busy.has(n.id);

  function branchOf(node) {
    const out = new Set();
    if (!node) return out;
    for (let cur = node.id; cur; cur = parentOf.get(cur)) out.add(cur);
    const down = [node.id];
    while (down.length) for (const c of childrenOf.get(down.pop()) ?? []) if (!out.has(c)) out.add(c), down.push(c);
    return out;
  }

  // What stays fully lit: the selection's branch and, while it's part of a
  // gathering, everything gathered, so its neighbours stay easy to pick. The
  // core is everything's ancestor, and close in its flare washes out the
  // view, so it dims like any bystander unless it's the selection.
  function litFor(node) {
    if (node?.kind === "actor") return actorBranch(node);
    const out = branchOf(node);
    if (holds(node)) for (const id of branchOf(fan.root)) out.add(id);
    if (node?.kind !== "core") out.delete("core");
    return out;
  }

  function stepLighting(dt) {
    const nodes = graph.graphData().nodes;
    const now = performance.now();
    if (now - extentAt > 1000) {
      extentAt = now;
      const drawn = nodes.filter((n) => (!visible || visible.has(n.id)) && Number.isFinite(n.x));
      const c = drawn.reduce((a, n) => a.add(posOf(n, lp)), new THREE.Vector3()).divideScalar(Math.max(1, drawn.length));
      extent = Math.max(120, ...drawn.map((n) => posOf(n, lp).distanceTo(c)));
    }
    const d = camera.position.distanceTo(controls.target);
    const want = wallpaper || !focusLighting ? 0 : 1 - ease(d, extent * 0.6, extent * 2);
    focusLevel += (want - focusLevel) * Math.min(1, dt * 3);
    // Lit around the point you orbit, out to about the camera's distance.
    const inner = Math.max(40, d * (selected ? 0.6 : 0.8)), outer = inner * 2.8;
    for (const n of nodes) {
      const g = n.__obs;
      if (!g) continue;
      let f = 1;
      if (focusLevel > 0.005 && !branch.has(n.id)) {
        posOf(n, lp);
        const far = ease(lp.distanceTo(controls.target), inner, outer);
        const near = 1 - ease(lp.distanceTo(camera.position), d * 0.3, d * 0.8);
        const floor = g.userData.rig?.attention || g.userData.rig?.stuck ? 0.6 : 0.1;
        f = 1 - focusLevel * Math.max(far, near) * (1 - floor);
        if (selected) f = Math.min(f, 1 - focusLevel * 0.7);
      }
      if (quiet(n) && !branch.has(n.id)) f *= QUIET_PLACE;
      // A host that isn't live: its constellation dims (hosts.js).
      if (n.stale) f *= STALE_LIGHT;
      if (dimmed && !dimmed.has(n.id)) f *= 0.12;
      if (Math.abs((g.userData.dim ?? 1) - f) > 0.01 || (f === 1 && g.userData.dim !== undefined && g.userData.dim !== 1)) {
        setDim(g, f);
        g.userData.dim = f;
      }
    }
  }

  // ---------- choosing where to look from ----------
  //
  // The direction to view something from, picked so as little as possible
  // covers it on screen, or glows right behind it. What's viewed is a box in
  // the plane facing the camera: (right, up) bounds around `center`, framed
  // from `dist` away with the camera aimed at (ox, oy) in that plane. About a
  // hundred directions spread evenly over the sphere (a Fibonacci lattice)
  // are scored by every drawn node whose glow, projected into that plane,
  // overlaps the box: each counts by how large it looks from the camera (its
  // size, glow included, over its depth) and how deep into the box it falls,
  // in full in front and half behind. Swinging far from the current view
  // costs a little, as does looking from straight above or below. That's
  // about a hundred cheap passes over the drawn nodes.

  const VIEWS = (() => {
    const out = [], n = 96, turn = Math.PI * (3 - Math.sqrt(5));
    for (let i = 0; i < n; i++) {
      const y = 1 - (2 * (i + 0.5)) / n, r = Math.sqrt(1 - y * y);
      out.push(new THREE.Vector3(Math.cos(turn * i) * r, y, Math.sin(turn * i) * r));
    }
    return out;
  })();

  function clearestView(center, { x0, x1, y0, y1, ox = (x0 + x1) / 2, oy = (y0 + y1) / 2 }, dist, skip = new Set()) {
    const others = [];
    for (const n of graph.graphData().nodes) {
      if (skip.has(n) || (visible && !visible.has(n.id)) || !Number.isFinite(n.x)) continue;
      const rel = posOf(n).sub(center);
      // The core's flare reaches far beyond its size.
      if (rel.lengthSq() < 9e6) others.push({ rel, glow: sizeFor(n) * (n.kind === "core" ? 10 : n.kind === "session" ? 3 : 4) });
    }
    const now = camera.position.clone().sub(controls.target).normalize();
    const u = new THREE.Vector3(), v = new THREE.Vector3();
    let best = VIEWS[0], least = Infinity;
    for (const w of VIEWS) {
      facing(w, u, v);
      let cost = 1.2 * (1 - w.dot(now)) + 6 * Math.max(0, Math.abs(w.y) - 0.6);
      for (const { rel, glow } of others) {
        const along = rel.dot(w);
        const depth = dist - along; // from the camera, along the view
        if (depth < 4) continue; // beside or behind the camera
        const scale = dist / depth; // into the box's plane
        const sx = ox + (rel.dot(u) - ox) * scale, sy = oy + (rel.dot(v) - oy) * scale;
        const g = glow * scale;
        const out = Math.hypot(Math.max(x0 - sx, 0, sx - x1), Math.max(y0 - sy, 0, sy - y1));
        if (out >= g) continue;
        const k = 1 - out / g;
        cost += (along > 0 ? 1 : 0.5) * (glow / depth) * k * k * 20;
      }
      if (cost < least) (least = cost), (best = w);
    }
    return best.clone();
  }

  // ---------- gathering: a project's sessions brought to it ----------
  //
  // In the radial layout a project's sessions sit a ring further out, mixed
  // in with every other project's and often between the camera and the
  // project, so focusing it left them hard to reach. Focusing a project, a
  // Codex project or a folder outside ~/Projects now gathers them for as long
  // as it's in focus: pinned in arcs beside it, facing the camera, most
  // recent first with the sessions they spawned right after them, each
  // labelled to its right, and the camera frames the lot. Selecting one of
  // them keeps the gathering. Letting go (Esc, a click on empty space, any
  // other node) eases everything back where it was, pins included.
  //
  // Gathered nodes move by hand rather than through the simulation, which
  // would re-run the whole layout: their fixed position and their links are
  // set each frame until they arrive.

  const GATHER_KINDS = new Set(["project", "native", "zone"]);
  const GATHER_MAX = 60;
  const GATHER_ROOT_SIZE = 9; // a root drawn larger (thousands of sessions) shows at this size while gathered
  const ARC_START = 16, ARC_GAP = 78, ARC_ROW = 7.5, ARC_SPREAD = (150 * Math.PI) / 180, LABEL_ROOM = 62;
  const UP = new THREE.Vector3(0, 1, 0);
  let fan = null; // { root, c, u, v, w, r0, order: [node], items: Map(node → target), sessions, total }
  const homes = new Map(); // node → { home, fixed }: where to put it back
  const moves = new Map(); // node → { from, to, t0, ms, done }
  const mv = new THREE.Vector3();
  const fixedOf = (n) => ({ fx: n.fx, fy: n.fy, fz: n.fz });
  const fixAt = (n, p) => Object.assign(n, { fx: p.x, fy: p.y, fz: p.z });
  const holds = (n) => !!fan && !!n && (n === fan.root || fan.items.has(n));

  // The plane facing a camera that looks along -w, upright: (right, up).
  function facing(w, u = new THREE.Vector3(), v = new THREE.Vector3()) {
    u.crossVectors(UP, w);
    if (u.lengthSq() < 1e-6) u.set(1, 0, 0);
    u.normalize();
    v.crossVectors(w, u);
    return { u, v };
  }

  function moveTo(n, to, { ms = 800, delay = 0, done } = {}) {
    moves.set(n, { from: posOf(n), to: to.clone(), t0: performance.now() + delay, ms, done });
  }

  const touched = new Set();
  function stepMoves() {
    if (!moves.size) return;
    const now = performance.now();
    touched.clear();
    for (const [n, m] of moves) {
      const k = Math.min(1, Math.max(0, (now - m.t0) / m.ms));
      mv.lerpVectors(m.from, m.to, 1 - (1 - k) ** 3);
      Object.assign(n, { x: mv.x, y: mv.y, z: mv.z, vx: 0, vy: 0, vz: 0 });
      fixAt(n, mv);
      n.__obs?.position.copy(mv);
      touched.add(n);
      if (k >= 1) {
        moves.delete(n);
        m.done?.();
      }
    }
    for (const l of graph.graphData().links) if (l.__lineObj?.userData.look && (touched.has(l.source) || touched.has(l.target))) placeLink(l.__lineObj, l.source, l.target);
  }

  // Everything under the root that's drawn, newest first, each followed by
  // what it spawned.
  function membersOf(root) {
    const out = [];
    const when = (n) => n.updatedAt ?? n.lastActivity ?? 0;
    (function walk(id) {
      const kids = (childrenOf.get(id) ?? []).map((k) => nodeIndex.get(k)).filter((n) => n?.__obs && (!visible || visible.has(n.id)));
      for (const n of kids.sort((a, b) => when(b) - when(a))) out.push(n), walk(n.id);
    })(root.id);
    return out;
  }

  // Slot i on the arcs, as (right, up) from the root: each arc holds as many
  // rows as fit, centred on the root, and the next arc starts further out.
  function slots(count) {
    const out = [];
    for (let arc = 0, i = 0; i < count; arc++) {
      const r = fan.r0 + arc * ARC_GAP;
      const k = Math.min(count - i, Math.floor((r * ARC_SPREAD) / ARC_ROW) + 1);
      for (let j = 0; j < k; j++, i++) {
        const a = (((k - 1) / 2 - j) * ARC_ROW) / r;
        out.push([r * Math.cos(a), r * Math.sin(a)]);
      }
    }
    return out;
  }

  function fanLabel(n, on) {
    const g = n.__obs, lbl = g?.userData.label;
    if (!lbl) return;
    lbl.userData.home ??= lbl.position.clone();
    if (on) {
      lbl.center.set(0, 0.5);
      lbl.position.set((g.userData.radius ?? 2) * 2 + 3, 0, 0); // clear of its glow
      lbl.visible = true;
    } else {
      lbl.center.set(0.5, 0.5);
      lbl.position.copy(lbl.userData.home);
      lbl.visible = n.kind !== "session" || !!g.userData.labelSticky || n === selected || n === hovered;
    }
  }

  function enlist(n) {
    if (!homes.has(n)) homes.set(n, { home: Number.isFinite(n.x) ? posOf(n) : fan.c.clone(), fixed: fixedOf(n) });
    if (!Number.isFinite(n.x)) Object.assign(n, { x: fan.c.x, y: fan.c.y, z: fan.c.z });
    fan.items.set(n, null);
    fanLabel(n, true);
  }

  function place({ stagger = true } = {}) {
    const at = slots(fan.order.length);
    fan.order.forEach((n, i) => {
      const to = fan.c.clone().addScaledVector(fan.u, at[i][0]).addScaledVector(fan.v, at[i][1]);
      const was = fan.items.get(n);
      if (was && was.distanceTo(to) < 0.01) return;
      fan.items.set(n, to);
      moveTo(n, to, { delay: stagger ? i * 14 : 0 });
    });
  }

  function announce() {
    const sessions = fan.order.filter((n) => n.kind === "session").length;
    onGather?.({ root: fan.root, sessions, total: fan.sessions });
  }

  // The root and its arcs, labels included, as (right, up) bounds, and the
  // camera distance and offset that frame them in the space the HUD leaves
  // (`insets`, in pixels from each edge).
  function framing() {
    // The root with its reticle and its label above.
    const r = fan.rs;
    let x0 = -r * 3 - 10, x1 = r * 3 + 10, y0 = -r * 3 - 6, y1 = Math.max(r * 3, r + 10) + 6;
    for (const [x, y] of slots(fan.order.length)) {
      x0 = Math.min(x0, x - 4);
      x1 = Math.max(x1, x + LABEL_ROOM);
      y0 = Math.min(y0, y - 4);
      y1 = Math.max(y1, y + 5);
    }
    const ins = { left: 0, right: 0, top: 0, bottom: 0, ...insets?.() };
    const W = innerWidth, H = innerHeight;
    const wPx = Math.max(240, W - ins.left - ins.right - 48), hPx = Math.max(240, H - ins.top - ins.bottom - 48);
    const tan = Math.tan((camera.fov * Math.PI) / 360);
    const d = Math.max(((x1 - x0) * H) / (2 * tan * wPx), ((y1 - y0) * H) / (2 * tan * hPx), 80);
    const upp = (2 * d * tan) / H; // world units per pixel at that distance
    return { d, x0, x1, y0, y1, ox: (x0 + x1) / 2 - ((ins.left - ins.right) / 2) * upp, oy: (y0 + y1) / 2 + ((ins.top - ins.bottom) / 2) * upp };
  }

  function frame(ms = 1300) {
    const { d, ox, oy } = framing();
    const target = fan.c.clone().addScaledVector(fan.u, ox).addScaledVector(fan.v, oy);
    const pos = target.clone().addScaledVector(fan.w, d);
    fly(pos, target, ms);
  }

  function gather(root) {
    const all = membersOf(root);
    if (!all.length) return false;
    // A root still easing back from an earlier gathering settles first.
    if (moves.has(root)) {
      moves.delete(root);
      const h = homes.get(root).home;
      Object.assign(root, { x: h.x, y: h.y, z: h.z });
      root.__obs?.position.copy(h);
    }
    if (!homes.has(root)) homes.set(root, { home: posOf(root), fixed: fixedOf(root) });
    const c = posOf(root);
    // The root shows at a modest size, and the arcs start clear of its
    // reticle and rings (3 × that size).
    const rs = Math.min(sizeFor(root), GATHER_ROOT_SIZE);
    fan = { root, c, rs, r0: rs * 3 + ARC_START, order: all.slice(0, GATHER_MAX), items: new Map(), sessions: all.filter((n) => n.kind === "session").length };
    shrink(root, rs / sizeFor(root));
    // The arcs face whichever way has the least in the way; they don't count,
    // since they're about to move.
    const box = framing();
    const w = clearestView(c, box, box.d, new Set([root, ...fan.order]));
    Object.assign(fan, { w, ...facing(w) });
    fixAt(root, c);
    for (const n of fan.order) enlist(n);
    place();
    controls.autoRotate = false;
    clearTimeout(idleTimer);
    frame();
    announce();
    return true;
  }

  function release() {
    if (!fan) return;
    const { root, order } = fan;
    fan = null;
    order.forEach((n, i) => {
      fanLabel(n, false);
      const rest = homes.get(n);
      if (!rest) return;
      moveTo(n, rest.home, { ms: 650, delay: i * 6, done: () => (Object.assign(n, rest.fixed), homes.delete(n)) });
    });
    const rest = homes.get(root);
    if (rest) Object.assign(root, rest.fixed), homes.delete(root);
    shrink(root, 1);
    idle();
    onGather?.(null);
  }

  function shrink(n, k) {
    const g = n.__obs;
    if (!g) return;
    g.userData.baseScale = k;
    g.userData.scaleTarget = k * (n === hovered || (n === selected && fan?.root === n) ? 1.4 : 1);
  }

  // Dropped after a drag: it stays there, out of the gathering; a dragged
  // root takes its gathering with it.
  function dropped(n) {
    if (!fan) return;
    if (n === fan.root) {
      homes.get(n).fixed = fixedOf(n);
      fan.c = posOf(n);
      return place({ stagger: false });
    }
    if (!fan.items.has(n)) return;
    fan.items.delete(n);
    fan.order = fan.order.filter((m) => m !== n);
    homes.delete(n);
    fanLabel(n, false);
    announce();
  }

  // A graph refresh resets fixed positions to the saved layout. Gathered and
  // moving nodes keep theirs, and remember the saved one for later.
  function hold() {
    for (const [n, rest] of homes) {
      rest.fixed = fixedOf(n);
      const at = moves.has(n) ? posOf(n) : n === fan?.root ? fan.c : fan?.items.get(n);
      if (at) fixAt(n, at);
    }
  }

  // After a refresh: forget nodes that left, gather new arrivals.
  function regather() {
    for (const n of [...homes.keys()]) if (nodeIndex.get(n.id) !== n) homes.delete(n), moves.delete(n), fan?.items.delete(n);
    if (!fan) return;
    if (nodeIndex.get(fan.root.id) !== fan.root) return release();
    const all = membersOf(fan.root);
    fan.sessions = all.filter((n) => n.kind === "session").length;
    const order = fan.order.filter((n) => fan.items.has(n));
    for (const n of all) if (order.length < GATHER_MAX && !fan.items.has(n)) order.push(n), enlist(n);
    fan.order = order;
    place({ stagger: false });
    announce();
  }

  // ---------- camera moves and settling ----------
  //
  // Scripts and tests need to know when the view has come to rest: the
  // layout cooled, camera moves finished and gathered nodes arrived. Camera
  // moves go through `fly`, which notes when they end, and the frame loop
  // notes when the camera last moved for any other reason (a drag easing out).
  // Auto-rotation and wallpaper mode's drift are ambient and don't count.

  let cameraUntil = 0;
  let cameraMovedAt = 0;
  const lastCamera = new THREE.Vector3(), lastTarget = new THREE.Vector3();
  function fly(pos, look, ms) {
    cameraUntil = Math.max(cameraUntil, performance.now() + ms);
    graph.cameraPosition({ x: pos.x, y: pos.y, z: pos.z }, { x: look.x, y: look.y, z: look.z }, ms);
  }
  function motion() {
    const now = performance.now();
    const drifting = !still && (controls.autoRotate || !!wallpaper);
    const cameraMoving = now < cameraUntil || (!drifting && now - cameraMovedAt < 150);
    // A change of theme counts until its new lines are placed.
    return { layout: engineRunning, camera: cameraMoving, moves: moves.size, theme: !!relinking, settled: !engineRunning && !cameraMoving && !moves.size && !relinking };
  }

  // ---------- holding still ----------
  //
  // Still mode shows one fixed moment of every ambient animation (sky, stars,
  // sweep, spinners, pulses) and stops the sonar, auto-rotation and the
  // bursts that live activity sets off, so two screenshots of the same state
  // match. Layout, camera moves and gathering still run in real time.

  const STILL_T = 30; // the moment, in seconds of animation time
  let still = false;
  function clearBursts() {
    for (const f of flashes.splice(0)) scene.remove(f.s), f.s.material.dispose();
    for (const w of waves.splice(0)) dropWave(w);
    for (const p of packets.splice(0)) scene.remove(p.s), packetPool.push(p.s);
    for (const s of sparks.splice(0)) scene.remove(s.pts), s.pts.geometry.dispose(), s.pts.material.dispose();
    actors.clearBursts();
  }
  function setStill(on) {
    if (!!on === still) return;
    still = !!on;
    if (still) {
      clearTimeout(idleTimer);
      controls.autoRotate = false;
      clearBursts();
    } else idle();
  }

  // ---------- actors: a layer of their own, above the map ----------
  //
  // See actors.js. Lit with the map: what's in focus, what's selected and
  // what a search dims, read each frame. Selecting an actor frames it with
  // what it acts on and lights that, as a node's branch is lit.

  const drawnNode = (id) => {
    const n = nodeIndex.get(id);
    return !!n?.__obs && Number.isFinite(n.x) && (!visible || visible.has(id));
  };
  const actors = createActorLayer({
    scene,
    camera,
    renderer,
    nodeOf: (id) => nodeIndex.get(id),
    // Where a gathered node lives when it isn't gathered, so actors stay
    // put while a gathering comes and goes.
    homeOf: (node) => homes.get(node)?.home ?? null,
    drawn: drawnNode,
    nodes: () => graph.graphData().nodes,
    flash: (node, color, scale, duration) => !still && flash(node, color, scale, duration),
  });
  const actorLighting = { focusLevel: 0, selectedId: null, branch: null, dimmed: null, labelFade: 1 };
  let actorBranchAt = 0;
  function actorBranch(rec) {
    const out = new Set();
    for (const id of actors.targetsOf(rec.id)) {
      const n = nodeIndex.get(id);
      if (n) for (const x of branchOf(n)) out.add(x);
    }
    out.delete("core");
    return out;
  }
  // The actor and what it acts on, framed from about where the camera is
  // and a little from above, in the space the panels leave (`insets`), as
  // a gathering is (see framing).
  function focusActor(rec) {
    const c = rec.group.position.clone();
    const pts = [c.clone()];
    for (const id of actors.targetsOf(rec.id)) {
      const n = nodeIndex.get(id);
      if (n && Number.isFinite(n.x)) pts.push(posOf(n));
    }
    const w = camera.position.clone().sub(controls.target).normalize();
    w.y = Math.max(w.y, 0.22);
    w.normalize();
    const { u, v } = facing(w);
    const pad = rec.size * 2.5;
    let x0 = -pad, x1 = pad, y0 = -pad, y1 = pad + rec.size * 2;
    for (const p of pts) {
      const rel = p.clone().sub(c), x = rel.dot(u), y = rel.dot(v);
      (x0 = Math.min(x0, x - pad)), (x1 = Math.max(x1, x + pad)), (y0 = Math.min(y0, y - pad)), (y1 = Math.max(y1, y + pad));
    }
    const ins = { left: 0, right: 0, top: 0, bottom: 0, ...insets?.() };
    const W = innerWidth, H = innerHeight;
    const wPx = Math.max(240, W - ins.left - ins.right - 48), hPx = Math.max(240, H - ins.top - ins.bottom - 48);
    const tan = Math.tan((camera.fov * Math.PI) / 360);
    const d = Math.max(((x1 - x0) * H) / (2 * tan * wPx), ((y1 - y0) * H) / (2 * tan * hPx), 90) * 1.1;
    const upp = (2 * d * tan) / H;
    const target = c.clone().addScaledVector(u, (x0 + x1) / 2 - ((ins.left - ins.right) / 2) * upp).addScaledVector(v, (y0 + y1) / 2 + ((ins.top - ins.bottom) / 2) * upp);
    fly(target.clone().addScaledVector(w, d), target, 1500);
  }
  // The pointer's last move, looked at once a frame.
  let pointerMove = null;
  el.addEventListener("pointermove", (e) => (pointerMove = e.buttons ? null : e), { passive: true });
  el.addEventListener("pointerleave", () => ((pointerMove = null), hoverActor(null)));

  // ---------- frame loop ----------

  const timer = new THREE.Timer();
  let ambientClock = 0;
  (function loop() {
    requestAnimationFrame(loop);
    timer.update();
    const dt = Math.min(timer.getDelta(), 0.05);
    // Ambient animation runs on its own clock, at the theme's pace, and
    // stops while still.
    ambientClock += dt * THEME.motion.pace;
    const t = still ? STILL_T : ambientClock;
    const adt = still ? 0 : dt;
    if (camera.position.distanceToSquared(lastCamera) > 1e-6 || controls.target.distanceToSquared(lastTarget) > 1e-6) cameraMovedAt = performance.now();
    lastCamera.copy(camera.position);
    lastTarget.copy(controls.target);
    sky.material.uniforms.time.value = t;
    lens.uniforms.time.value = t;
    restLevel += (restTarget - restLevel) * Math.min(1, dt * 0.8);
    sky.material.uniforms.rest.value = restLevel;
    stars.look.update(stars.object, { t, dim: 1 - 0.55 * restLevel });
    bloom.strength = THEME.glow.bloom * (1 - 0.3 * restLevel) * (1 - 0.35 * focusLevel);
    if (wallpaper && !still) {
      // Slow breathing dolly around whatever the camera is orbiting.
      const target = controls.target;
      const off = camera.position.clone().sub(target);
      const want = wallpaper.distance * (1 + 0.07 * Math.sin(t * 0.07));
      off.setLength(off.length() + (want - off.length()) * Math.min(1, dt * 0.5));
      camera.position.copy(target).add(off);
    }
    dust.rotation.y = -t * 0.012;
    lattice.rotation.y = t * 0.006;
    grid.look.update(grid.object, { t });
    sweep.look.update(sweep.object, { t, theme: THEME });
    for (const n of graph.graphData().nodes) {
      const g = n.__obs;
      if (!g) continue;
      if (!n.stale) for (const f of g.userData.anim) f(t, adt);
      const k = g.scale.x + (g.userData.scaleTarget - g.scale.x) * Math.min(1, dt * 10);
      g.scale.setScalar(k);
      if (g.userData.rig) animateLive(n, t, adt, camera, n === selected || n === hovered);
      const lbl = g.userData.label;
      if (lbl) {
        const fade = n === selected || n === hovered ? 1 : labelFade;
        lbl.material.opacity = (lbl.material.userData.base ?? 1) * (g.userData.dim ?? 1) * fade;
      }
    }
    if (selected && reticle.visible) {
      const actor = selected.kind === "actor";
      if (actor) reticle.position.copy(selected.group.position);
      else posOf(selected, reticle.position);
      reticle.material.rotation += adt * 0.5;
      const size = actor ? selected.size * selected.scale * 4.4 : sizeFor(selected) * (selected.__obs?.userData.baseScale ?? 1) * (selected.kind === "session" ? 10 : 6);
      reticle.scale.setScalar(size * (1 + 0.04 * Math.sin(t * 4)));
    }
    if (relinking) relink();
    stepMoves();
    stepLighting(dt);
    if (selected?.kind === "actor" && performance.now() - actorBranchAt > 500) (actorBranchAt = performance.now()), (branch = actorBranch(selected));
    Object.assign(actorLighting, { focusLevel, selectedId: selected?.id ?? null, branch, dimmed, labelFade });
    actors.step({ t, dt, adt, still, lighting: actorLighting });
    if (pointerMove) {
      if (!dragging) {
        const hit = actorAt(pointerMove);
        hoverActor(actorInFront(hit, hovered) ? hit.rec : null);
      }
      pointerMove = null;
    }
    stepLinks(t, dt);
    stepArcs(t, adt);
    hosts.step();
    stepPackets(dt);
    stepSparks(dt);
    const now = performance.now();
    for (let i = flashes.length - 1; i >= 0; i--) {
      const f = flashes[i];
      const p = (now - f.t0) / (f.duration * 1000);
      if (p >= 1) {
        scene.remove(f.s);
        f.s.material.dispose();
        flashes.splice(i, 1);
        continue;
      }
      f.s.scale.setScalar(f.r * (2 + f.scale * (1 - (1 - p) ** 3)));
      f.s.material.opacity = (1 - p) * 0.9;
    }
    const gap = sonarGap(THEME.sonar.every);
    sonarClock = gap == null ? 0 : sonarClock + adt;
    if (gap != null && sonarClock > gap) {
      sonarClock = 0;
      sonar();
    }
    for (let i = waves.length - 1; i >= 0; i--) {
      if (waves[i].look.step(waves[i], dt)) continue;
      dropWave(waves[i]);
      waves.splice(i, 1);
    }
  })();

  addEventListener("resize", () => {
    graph.width(innerWidth).height(innerHeight);
    bloom.setSize(innerWidth, innerHeight);
    setLineResolution(innerWidth, innerHeight);
  });

  // ---------- camera framing ----------

  // The whole map and the actors over it, from a little above and to the
  // side, framed in the space the panels, the alert strip and the terminal
  // leave (`insets`; see framing.js), so actors floating near the top
  // aren't hidden behind the strip. Each node counts with its glow.
  const OVERVIEW = new THREE.Vector3(0.42, 0.36, 1).normalize();
  const ACTOR_REACH = 14;
  function fit(ms = 1400) {
    const ns = graph.graphData().nodes.filter((n) => (!visible || visible.has(n.id)) && Number.isFinite(n.x));
    if (!ns.length) return;
    const pts = [...ns.map((n) => [posOf(n), sizeFor(n) * 2]), ...actors.positions().map((p) => [p, ACTOR_REACH])];
    const c = pts.reduce((sum, [p]) => sum.add(p), new THREE.Vector3()).divideScalar(pts.length);
    const { u, v } = facing(OVERVIEW);
    const rel = new THREE.Vector3();
    const view = pts.map(([p, r]) => (rel.subVectors(p, c), { x: rel.dot(u), y: rel.dot(v), z: rel.dot(OVERVIEW), r }));
    const free = freeTangents({ width: innerWidth, height: innerHeight, fov: camera.fov, insets: insets?.() });
    const { d, a, b } = frameFree(view, free, { min: 80 });
    const target = c.clone().addScaledVector(u, a).addScaledVector(v, b);
    fly(target.clone().addScaledVector(OVERVIEW, d), target, ms);
  }
  // A fit `delay` ms from now; the view counts as moving until it's done.
  function fitLater(delay, ms = 1400) {
    cameraUntil = Math.max(cameraUntil, performance.now() + delay + ms);
    setTimeout(() => fit(ms), delay);
  }

  let fitted = false;
  // The first time the layout settles, frame it, unless something was
  // selected in the meantime.
  graph.onEngineStop(() => {
    engineRunning = false;
    if (!fitted) {
      fitted = true;
      if (!selected) fit(1800);
    }
  });

  const visualKey = (n) =>
    [n.activity, n.active, n.archived, n.starred, n.sessionCount, n.activeCount, n.title, n.name, n.group, n.satelliteOf, n.repo?.dirty, n.repo?.ahead, n.inbox?.length, n.writes?.length, n.updated, n.identityRing, hostVisual(n)].join("|");

  function rebuildInPlace(n) {
    const g = n.__obs;
    if (!g) return;
    const fresh = buildNode(n);
    for (const child of [...g.children]) {
      g.remove(child);
      disposeNode(child);
    }
    g.add(...fresh.children);
    g.userData = fresh.userData;
    if (n.__live !== undefined) applyLive(n, n.__live);
    if (n === selected || n === hovered) highlight(n);
    if (fan?.items.has(n)) fanLabel(n, true);
    if (fan?.root === n) shrink(n, fan.rs / sizeFor(n));
  }

  // ---------- a change of theme ----------
  //
  // Everything takes the new theme where it is: the environment is built
  // again, each drawn node is built again in place (keeping where it is,
  // its live rig, its highlight and its place in a gathering), the force
  // graph makes every link's line again, and arcs and the reticle take
  // their new colors. Sparks, packets and waves already on their way finish
  // in the old ones. What's replaced is disposed.
  function retheme(theme) {
    stars = starsOf(theme, stars);
    setEnvironment(environmentOf(environmentState().id));
    grid = gridOf(theme, grid);
    sweep = sweepOf(theme, sweep);
    lattice.material.color.set(COLORS.project);
    lattice.material.opacity = lattice.material.userData.base = theme.lattice.opacity;
    dust.material.color.set(theme.dust.color);
    Object.assign(dust.material, { size: theme.dust.size, opacity: theme.dust.opacity });
    scene.fog.color.set(theme.colors.bg);
    scene.fog.density = theme.glow.fog;
    renderer.toneMappingExposure = theme.glow.exposure;
    Object.assign(bloom, { radius: theme.glow.radius, threshold: theme.glow.threshold });
    graph.backgroundColor(theme.colors.bg);
    setSky(skyHour);
    for (const n of nodeIndex.values()) if (n.__obs?.parent) rebuildInPlace(n);
    relinking = { lines: [], since: performance.now() };
    graph.linkThreeObject((l) => makeLink(l));
    for (const arc of arcs.values()) {
      arc.color = COLORS[arc.role];
      arc.line.material.color.set(arc.color);
    }
    actors.retheme();
    hosts.retheme();
    if (selected?.kind === "actor") reticle.material.color.copy(selected.color);
    else if (selected?.__obs) reticle.material.color.set(selected.__obs.userData.color);
    // Attention beacons come back with the live state, as after a refresh.
    if (liveData) api.setLive(liveData);
  }

  // The force graph makes the new lines a moment after a change of theme,
  // all at once, but places lines only while the layout runs: the frame
  // loop places them here. With no links drawn there's nothing to wait for.
  function relink() {
    const { lines, since } = relinking;
    if (!lines.length) {
      if (performance.now() - since > 500) relinking = null;
      return;
    }
    for (const line of lines) {
      const { link } = line.userData;
      if (Number.isFinite(link.source?.x) && Number.isFinite(link.target?.x)) placeLink(line, link.source, link.target);
    }
    relinking = null;
  }

  const api = {
    graph,
    setData(data) {
      const current = graph.graphData();
      const oldNodes = new Map(current.nodes.map((n) => [n.id, n]));
      const changed = [];
      let unpinned = false;
      const nodes = data.nodes.map((n) => {
        const o = oldNodes.get(n.id);
        if (!o) return n;
        if (visualKey(o) !== visualKey(n)) changed.push(o);
        if (o.pinned && !n.pinned) unpinned = true;
        return Object.assign(o, n);
      });
      nodeIndex.clear();
      for (const n of nodes) nodeIndex.set(n.id, n);
      parentOf = new Map();
      for (const l of data.links) if (HIERARCHY.has(l.kind)) parentOf.set(idOf(l.target), idOf(l.source));
      childrenOf = new Map();
      for (const [child, parent] of parentOf) (childrenOf.get(parent) ?? childrenOf.set(parent, []).get(parent)).push(child);
      for (const n of changed) rebuildInPlace(n);
      hold();
      const oldLinks = new Map(current.links.map((l) => [linkKey(l), l]));
      const links = data.links.map((l) => oldLinks.get(linkKey(l)) ?? l);
      hosts.prepare(nodes, links);
      const structural = nodes.length !== current.nodes.length || links.length !== current.links.length || links.some((l) => !oldLinks.has(linkKey(l)));
      if (structural || !current.nodes.length) {
        engineRunning = true;
        graph.graphData({ nodes, links });
      } else if (unpinned) {
        engineRunning = true;
        graph.d3ReheatSimulation();
      }
      regather();
      if (selected) selected = selected.kind === "actor" ? actors.get(selected.id) : (nodeIndex.get(selected.id) ?? null);
      branch = litFor(selected);
      if (dimmed) this.dim(dimmed);

      // Standing relations: wiki pages linking each other and the projects they cover.
      const relKeys = new Set();
      for (const r of data.relations ?? []) {
        const a = nodeIndex.get(r.a), b = nodeIndex.get(r.b);
        const style = RELATION_STYLE[r.kind];
        if (!a || !b || !style) continue;
        const key = `rel:${r.kind}:${r.a}|${r.b}`;
        relKeys.add(key);
        upsertArc(key, a, b, { ...style, flow: 0 });
      }
      for (const [key, arc] of arcs) if (key.startsWith("rel:") && !relKeys.has(key)) arc.dying ??= performance.now();
      hosts.relations(data.relations ?? [], nodeIndex);

      if (liveData) this.setLive(liveData);
      return { ...data, nodes, links };
    },

    // Live telemetry snapshot: per-session rigs, link flow and arcs.
    setLive(snap) {
      liveData = snap;
      const byId = new Map(snap.sessions.map((s) => [s.id, s]));
      for (const n of nodeIndex.values()) {
        if (n.kind !== "session" || !(byId.has(n.id) || n.__live)) continue;
        const s = byId.get(n.id) ?? null;
        applyLive(n, s);
        setAttention(n, { need: s?.stale ? null : (s?.needsYou ?? null), stuck: s?.stuck ?? null });
      }
      recomputeFlow();
      busy = new Set();
      for (const s of snap.sessions) if (s.status !== "idle") for (let up = parentOf.get(s.id); up; up = parentOf.get(up)) busy.add(up);

      for (const [key, arc] of arcs) if (!key.startsWith("rel:")) arc.seen = false;
      // Two sessions editing the same file: a collision in the making.
      for (const c of snap.collisions ?? []) {
        const a = nodeIndex.get(c.a), b = nodeIndex.get(c.b);
        if (a && b) upsertArc(`collision:${c.a}|${c.b}`, a, b, { role: "collision", width: 2.6, opacity: 1, bow: 0.42, flow: 2.4 });
      }
      // Sessions editing the same files.
      for (const e of snap.collab ?? []) {
        const a = nodeIndex.get(e.a), b = nodeIndex.get(e.b);
        if (a && b) upsertArc(`collab:${e.a}|${e.b}`, a, b, { role: "collab", width: 2, opacity: 0.85, bow: 0.35, flow: 1.2 });
      }
      // Sessions working at the same time in the same project.
      const working = snap.sessions.filter((s) => s.status !== "idle" && nodeIndex.has(s.id));
      for (let i = 0; i < working.length; i++)
        for (let j = i + 1; j < working.length; j++) {
          const a = working[i].id, b = working[j].id;
          if (homeOf(a) && homeOf(a) === homeOf(b)) upsertArc(`sync:${a}|${b}`, nodeIndex.get(a), nodeIndex.get(b), { role: "sync", width: 1.3, opacity: 0.5, bow: 0.5, flow: 0.5 });
        }
      // Sessions that reached a shared system (MCP server) in the last 10 minutes.
      const now = Date.now();
      for (const s of snap.sessions) {
        const a = nodeIndex.get(s.id);
        if (!a) continue;
        for (const m of s.mcp) {
          const b = nodeIndex.get(sameHost(a, `resource:${m.server}`));
          if (b && now - m.ts < 10 * 60e3) upsertArc(`res:${s.id}|${m.server}`, a, b, { role: "resource", width: 1.1, opacity: 0.32, bow: 0.18, flow: 0 });
        }
      }
      for (const [key, arc] of arcs) if (!key.startsWith("rel:") && !arc.seen) arc.dying ??= performance.now();
    },

    // One live event: a tool call, prompt, subagent, turn boundary.
    activity(ev) {
      const node = nodeIndex.get(ev.sessionId);
      if (still || !node?.__obs || (visible && !visible.has(node.id))) return;
      const color = ev.kind === "tool" ? CATEGORY_COLORS[ev.category] ?? "#fff" : nodeColor(node);
      if (ev.kind === "tool") {
        spark(node, color, ev.category === "edit" ? 30 : 20);
        if (ev.server) {
          const res = nodeIndex.get(sameHost(node, `resource:${ev.server}`));
          const color2 = res?.kind === "wiki" ? COLORS.wiki : COLORS.resource;
          if (res) for (let i = 0; i < 3; i++) setTimeout(() => packet({ from: node, to: res, color: color2, size: 3, duration: 1.3, bow: 0.18 }), i * 140);
          // Wiki writes continue on to the pages they touched.
          for (const slug of ev.pages ?? []) {
            const page = nodeIndex.get(sameHost(node, `wiki:${slug}`));
            if (res && page) {
              setTimeout(() => packet({ from: res, to: page, color: COLORS.wiki, size: 3.2, duration: 0.9 }), 1300);
              setTimeout(() => flash(page, COLORS.wiki, 12, 1.4), 2150);
            }
          }
        }
      } else if (ev.kind === "prompt") {
        flash(node, "#ffffff", 10, 1.2);
        const up = nodeIndex.get(parentOf.get(node.id));
        if (up) for (let i = 0; i < 4; i++) setTimeout(() => packet({ from: up, to: node, color: "#ffffff", size: 3, duration: 0.9 }), i * 90);
      } else if (ev.kind === "turn-end") {
        flash(node, color, 18, 2);
      } else if (ev.kind === "subagent-start") {
        flash(node, "#ffffff", 22, 1.8);
        spark(node, "#ffffff", 36);
      } else if (ev.kind === "tool-error") {
        flash(node, COLORS.danger, 8, 1);
      } else if (ev.kind === "needs-you") {
        flash(node, COLORS.attention, 26, 2.4);
        setTimeout(() => flash(node, COLORS.attention, 18, 2), 500);
      } else if (ev.kind === "ship-it") {
        const project = nodeIndex.get(ev.projectId);
        if (project?.__obs) flash(project, COLORS.pressure, 16, 2.2);
      }
    },

    // Tokens processed: packets stream from the session back to its parent.
    tokens(ev) {
      const node = nodeIndex.get(ev.sessionId);
      const up = node && nodeIndex.get(parentOf.get(node.id));
      if (still || !up || (visible && !visible.has(node.id))) return;
      const n = Math.min(8, Math.max(1, Math.round(Math.log10(1 + ev.n) * 1.6)));
      for (let i = 0; i < n; i++) setTimeout(() => packet({ from: node, to: up, color: nodeColor(node), size: 2 + Math.log10(1 + ev.n) * 0.3, duration: 1.2 }), i * 70);
    },

    setVisible(set) {
      visible = set;
      graph.nodeVisibility((n) => !visible || visible.has(n.id));
      graph.linkVisibility((l) => !visible || (visible.has(idOf(l.source)) && visible.has(idOf(l.target))));
    },
    // Search dimming; the frame loop applies it along with the focus lighting.
    dim(ids) {
      dimmed = ids;
    },
    setFocusLighting(on) {
      focusLighting = !!on;
    },
    // Selecting a project gathers its sessions (see gather); selecting one
    // of them keeps the gathering, and anything else lets it go.
    select(node) {
      if (node && isActorId(node.id)) node = actors.get(node.id);
      else if (node) node = nodeIndex.get(node.id) ?? node;
      if (selected && selected !== node) unhighlight(selected);
      const stay = holds(node);
      if (!stay) release();
      selected = node;
      actors.select(node?.kind === "actor" ? node.id : null);
      reticle.visible = !!node && !wallpaper;
      if (node?.kind === "actor") {
        reticle.material.color.copy(node.color);
        focusActor(node);
      } else if (node) {
        reticle.material.color.set(node.__obs?.userData.color ?? "#fff");
        highlight(node);
        if (stay) node === fan.root && frame();
        else if (!(GATHER_KINDS.has(node.kind) && gather(node))) this.focus(node);
      }
      branch = litFor(node);
    },
    // The root of the current gathering, and the gathered node `by` places
    // from `node` (wrapping), for stepping through with the arrow keys.
    gatheredRoot: () => fan?.root ?? null,
    stepGathered(node, by) {
      if (!fan?.order.length) return null;
      const list = fan.order, i = list.indexOf(node);
      return list[i < 0 ? (by > 0 ? 0 : list.length - 1) : (i + by + list.length) % list.length];
    },
    // Fly to a node from the side with the least in the way (see
    // clearestView); what hangs from it doesn't count.
    focus(node) {
      const dist = { session: 115, core: 320, host: 300, project: 170, native: 130, zone: 150, resource: 140, wiki: 170, page: 90 }[node.kind] ?? 140;
      const c = posOf(node);
      const r = sizeFor(node) * 3 + 12;
      const pos = c.clone().addScaledVector(clearestView(c, { x0: -r, x1: r, y0: -r, y1: r }, dist, new Set([node, ...membersOf(node)])), dist);
      fly(pos, c, 1500);
    },
    reset() {
      fit();
    },
    pulse(nodeId) {
      const node = nodeIndex.get(nodeId);
      if (node?.__obs && !still) flash(node);
    },
    sonar: () => !still && sonar(),
    // One agent messaging others through XenoNet's engine (see main.js): a
    // packet along a bowed path to each, and a ring when it lands. An
    // actor's message is the actors layer's to draw (actorMessage).
    message(fromId, toIds) {
      const from = nodeIndex.get(fromId);
      if (still || !from?.__obs) return;
      for (const id of Array.isArray(toIds) ? toIds : []) {
        const to = nodeIndex.get(id);
        if (!to?.__obs || (visible && (!visible.has(from.id) || !visible.has(to.id)))) continue;
        packet({ from, to, color: COLORS.message, size: 4, duration: 1.8, bow: 0.35 });
        setTimeout(() => flash(to, COLORS.message, 14, 1.4), 1700);
      }
    },
    setLayout(mode) {
      graph.dagMode(mode === "free" ? null : mode);
      engineRunning = true;
      graph.d3ReheatSimulation();
      fitLater(2500);
    },
    // The actors (actors.js): their list as it changes, each event, one
    // by id, every one, the beams drawn, and for scripts and the
    // benchmark, the layer itself.
    setActors: (list) => actors.set(list),
    // `quiet`: only what it acts on changes, drawn by something else
    // already (a message's said event; see main.js).
    actorEvent: (e, { quiet = false } = {}) => actors.event(e, { still: still || quiet }),
    // A message an actor sent agents (agent-message), drawn from the actor.
    actorMessage: (from, to, at) => actors.message(from, to, { at, still }),
    // A remote actor's read (remote-read): light through its tunnel's gate
    // to what it read.
    actorRead: (e) => actors.read(e, { still }),
    actor: (id) => actors.get(id),
    actors: () => actors.records(),
    actorLayer: actors,
    // The camera as an ear, for sounds placed on the map (ambient.js,
    // sound-space.js): where it is, which ways are right, up and ahead,
    // how far it is from what it looks at, and how close in it's flown.
    ear() {
      camera.updateMatrixWorld();
      const e = camera.matrixWorld.elements;
      const p = camera.position;
      return { pos: [p.x, p.y, p.z], right: [e[0], e[1], e[2]], up: [e[4], e[5], e[6]], fwd: [-e[8], -e[9], -e[10]], orbit: p.distanceTo(controls.target), focus: focusLevel };
    },
    // Where a drawn node or an actor is, by id, or null.
    where(id) {
      if (isActorId(id)) {
        const rec = actors.get(id);
        return rec?.placed ? [rec.x, rec.y, rec.z] : null;
      }
      const n = nodeIndex.get(id);
      if (!n || !Number.isFinite(n.x) || (id !== "core" && !drawnNode(id))) return null;
      const p = posOf(n);
      return [p.x, p.y, p.z];
    },
    getNode: (id) => nodeIndex.get(id),
    nodes: () => [...nodeIndex.values()],
    // Other machines' constellations and the bridges drawn (hosts-draw.js).
    hosts: () => hosts.state(),
    homeOf,
    hover(node) {
      if (node && isActorId(node.id)) return hoverActor(actors.get(node.id));
      hoverActor(null);
      hoverNode(node ? (nodeIndex.get(node.id) ?? null) : null);
    },

    // For scripts and tests (src/driver.js): what's still moving, holding the
    // view still, what's gathered, and where a node is on screen.
    motion,
    setStill,
    get still() {
      return still;
    },
    gathering: () => (fan ? { root: fan.root.id, members: fan.order.map((n) => n.id), sessions: fan.sessions } : null),
    view: () => ({ camera: camera.position.toArray().map(Math.round), target: controls.target.toArray().map(Math.round), autoRotate: controls.autoRotate }),
    // In CSS pixels from the page's top left; null if it isn't drawn.
    screenPos(id) {
      const rec = isActorId(id) ? actors.get(id) : null;
      const n = rec ? null : nodeIndex.get(id);
      if (!rec && (!n?.__obs || !Number.isFinite(n.x) || (visible && !visible.has(id)))) return null;
      const p = (rec ? rec.group.position.clone() : posOf(n)).project(camera);
      const r = renderer.domElement.getBoundingClientRect();
      const onScreen = p.z < 1 && Math.abs(p.x) <= 1 && Math.abs(p.y) <= 1;
      return { x: Math.round(r.left + ((p.x + 1) / 2) * r.width), y: Math.round(r.top + ((1 - p.y) / 2) * r.height), onScreen };
    },

    // Wallpaper: no reticle, faded labels, slower orbit and a breathing dolly.
    setWallpaper(on) {
      if (on) {
        release();
        fit(2400);
        cameraUntil = Math.max(cameraUntil, performance.now() + 2500);
        setTimeout(() => (wallpaper = { distance: camera.position.distanceTo(controls.target) }), 2500);
        controls.autoRotate = !still;
        controls.autoRotateSpeed = 0.16;
        labelFade = 0.25;
        reticle.visible = false;
      } else {
        wallpaper = null;
        controls.autoRotateSpeed = 0.28;
        labelFade = 1;
        reticle.visible = !!selected;
      }
    },
    setSky,
    setRest(on) {
      restTarget = on ? 1 : 0;
    },

    // One frame at poster resolution. Lines keep their on-screen width; the
    // pixel ratio is raised so everything else renders at `width` pixels.
    renderHighRes(width = 3840) {
      const pr0 = renderer.getPixelRatio();
      const pr = width / innerWidth;
      const ret = reticle.visible;
      reticle.visible = false;
      renderer.setPixelRatio(pr);
      composer.setPixelRatio(pr);
      composer.render();
      const url = renderer.domElement.toDataURL("image/png");
      renderer.setPixelRatio(pr0);
      composer.setPixelRatio(pr0);
      reticle.visible = ret;
      return { url, width: Math.round(innerWidth * pr), height: Math.round(innerHeight * pr) };
    },
  };
  onTheme(retheme);
  return api;
}
