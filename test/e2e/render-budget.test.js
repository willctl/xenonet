// End to end: how often the console draws (src/render-budget.js), on a
// real XenoNet Dev (the test instance, with the demo's made-up data; see
// xenonet.test.js): at rest it keeps to the rest pace instead of the
// display's rate, a recording gets every frame (films, the showcase and
// the X cuts all record through the desktop app's recorder), a replay
// draws at the moving pace, the palette's frame rate choice holds, plain
// panels takes the glass blur away, a window nobody can see draws nothing,
// and a frame does no more work than it needs (fx.js, scene.js).
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { findFfmpeg } from "../../core/dirs.js";
import { limit, software } from "../harness/software.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
// Drawn in software (as on a Linux runner with no GPU), the map draws a
// frame or two a second. There these tests check the pace each state asks
// for and that no more is drawn, but not that the frame rate reaches it.
const film = path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-e2e-budget-${port}.mp4`);

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  await s.driver("settled", { timeout: 60000 });
}, { timeout: limit(180000) });

after(async () => {
  await s.record({ action: "stop" }).catch(() => {});
  fs.rmSync(film, { force: true });
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

// The frames drawn over 1.5 s, settled first so nothing is moving.
async function measure() {
  await s.driver("settled", { timeout: 30000 });
  return s.driver("render", { ms: 1500 });
}

// The demo's sessions come and go, and a new one sets the layout moving
// for a few seconds, at the moving pace: what was drawn is checked against
// the rates kept meanwhile (`rates`), with room for a frame forced out of
// turn and the timer's edges.
function within(r, rest, moving) {
  assert.ok(r.rates.every((x) => x === rest || x === moving), `only the rest and moving rates: ${JSON.stringify(r)}`);
  const most = Math.max(...r.rates), least = Math.min(...r.rates);
  assert.ok(r.perSecond <= most + 2 + r.forcedDrawn, `no more than ${most} a second: ${JSON.stringify(r)}`);
  if (!software) assert.ok(r.perSecond >= least * 0.6, `about ${least} a second or more: ${JSON.stringify(r)}`);
}

test("at rest the map keeps to the rest pace, not the display's rate", { timeout: limit(60000) }, shoot("budget-rest", async () => {
  const st = await s.driver("state");
  assert.equal(st.render.pace, "balanced");
  const r = await measure();
  // Worked through the control channel, the window counts as in use.
  assert.equal(r.driven, true);
  assert.ok(r.rates.includes(30), JSON.stringify(r));
  within(r, 30, 60);
}));

test("camera moves still run in the paced loop: selecting a project flies to its gathering", { timeout: limit(60000) }, shoot("budget-camera", async () => {
  await s.driver("select", null);
  await s.driver("reset");
  const before = (await s.driver("state")).view;
  const { nodes } = await s.driver("find", { query: "atlas-api", kind: "project", limit: 1 });
  const st = await s.driver("select", nodes[0].id);
  assert.ok(st.gathered?.members.length, "it gathered");
  const at = await s.eval(`return window.__xenonet.scene.where(${JSON.stringify(nodes[0].id)})`);
  const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
  assert.notDeepEqual(st.view.camera, before.camera, "the camera moved");
  assert.ok(dist(st.view.target, at) < dist(before.target, at) / 2, `it looks at the gathering: ${JSON.stringify({ before, after: st.view, at })}`);
  await s.driver("select", null);
}));

test("a recording draws every frame the display offers, and the rest pace comes back after", { timeout: limit(90000), skip: !findFfmpeg() && "needs ffmpeg" }, shoot("budget-recording", async () => {
  await s.record({ action: "start", file: film, fps: 30 });
  let r;
  try {
    r = await measure();
  } finally {
    await s.record({ action: "stop" });
  }
  assert.deepEqual(r.recording, ["desktop"]);
  assert.equal(r.fps, null, "every frame");
  if (!software) assert.ok(r.perSecond > 45, `more than the moving pace's share at 60 Hz: ${r.perSecond}`);
  const after = await measure();
  assert.deepEqual(after.recording, []);
  within(after, 30, 60);
}));

test("a replay draws at the moving pace while it plays", { timeout: limit(60000) }, shoot("budget-replay", async () => {
  await s.driver("command", "replay");
  try {
    const r = await s.driver("render", { ms: 1000 });
    assert.deepEqual(r.lively, ["replay"]);
    assert.equal(r.fps, 60);
  } finally {
    await s.input({ keys: "Escape" });
  }
  assert.equal((await s.driver("state")).overlays.replay, false);
}));

test("the frame rate choice in the palette: lower GPU use, every frame, then balanced again", { timeout: limit(60000) }, shoot("budget-pace", async () => {
  const commands = (await s.driver("commands")).map((c) => c.id);
  for (const id of ["pace-balanced", "pace-low", "pace-smooth"]) assert.ok(commands.includes(id), id);
  try {
    await s.driver("command", "pace-low");
    let r = await measure();
    assert.equal(r.pace, "low");
    assert.ok(r.rates.includes(15), JSON.stringify(r));
    within(r, 15, 30);
    await s.driver("command", "pace-smooth");
    r = await measure();
    assert.equal(r.pace, "smooth");
    assert.equal(r.fps, null);
  } finally {
    await s.driver("command", "pace-balanced");
  }
  assert.equal((await s.driver("state")).render.pace, "balanced");
}));

test("plain panels: off unless turned on, and then nothing blurs what's behind it", { timeout: limit(60000) }, shoot("budget-plain-panels", async () => {
  // Every element whose backdrop-filter isn't none, by its class.
  const blurred = () => s.eval(`return [...document.querySelectorAll("*")].filter((e) => getComputedStyle(e).backdropFilter !== "none").map((e) => String(e.className || e.tagName))`);
  assert.equal((await s.driver("state")).look.plainPanels, false, "off unless you turn it on");
  assert.ok((await blurred()).some((c) => c.split(" ").includes("panel")), "the panels are glass by default");
  assert.ok((await s.driver("commands")).some((c) => c.id === "plain-panels"), "the palette has the switch");
  try {
    assert.equal((await s.driver("plainPanels", true)).look.plainPanels, true);
    assert.deepEqual(await blurred(), []);
    // Drawn solid, and kept for this window as the frame rate is.
    assert.match(await s.eval(`return getComputedStyle(document.querySelector(".panel")).backgroundColor`), /^rgb\(/);
    assert.equal(await s.eval(`return localStorage.getItem("xenonet.plainPanels")`), "on");
    await s.screenshot({ name: "e2e-plain-panels", inline: false });
    // The palette's switch turns it off again.
    await s.driver("command", "plain-panels");
    assert.equal((await s.driver("state")).look.plainPanels, false);
    assert.ok((await blurred()).length > 0, "glass again");
  } finally {
    await s.driver("plainPanels", false);
  }
  assert.deepEqual((await s.logs({ source: "page", level: "error" })).entries, []);
}));

test("a window nobody can see draws nothing, and draws again once it can", { timeout: limit(60000) }, shoot("budget-unseen", async () => {
  // As the page would hear it from the desktop app (the window hidden, and
  // no script working it), counted in the page so no request to the window
  // lands in the middle; then put back as the desktop app says.
  const r = await s.eval(`return await (async () => {
    const scene = window.__xenonet.scene;
    scene.setDrawing({ seen: false, driven: false });
    const from = scene.drawingState().frames;
    await new Promise((go) => setTimeout(go, 1200));
    const unseen = scene.drawingState().frames - from;
    scene.setDrawing({ seen: true, driven: true });
    const back = scene.drawingState().frames;
    await new Promise((go) => setTimeout(go, ${software ? 3000 : 600}));
    return { unseen, seen: scene.drawingState().frames - back };
  })()`);
  assert.equal(r.unseen, 0, "no frames while unseen");
  assert.ok(r.seen >= (software ? 1 : 8), `frames again once seen: ${r.seen}`);
  await s.driver("settled", { timeout: 30000 });
}));

test("a frame draws each glow once, adds the bloom in the lens and keeps to one target", { timeout: limit(60000) }, shoot("budget-frame", async () => {
  await s.driver("settled", { timeout: 30000 });
  const r = await s.eval(`return (() => {
    const scene = window.__xenonet.scene, renderer = scene.graph.renderer(), composer = scene.graph.postProcessingComposer();
    // Added light on both faces, still drawn in two passes (fx.js).
    const twice = new Set();
    scene.graph.scene().traverse((o) => {
      for (const m of [o.material].flat()) if (m?.transparent && m.side === 2 && m.blending === 2 && !m.depthWrite && !m.forceSinglePass) twice.add(m.type);
    });
    const [, bloom, lens] = composer.passes;
    const allocated = (target) => !!renderer.properties.get(target).__webglFramebuffer;
    return { twice: [...twice], passes: composer.passes.map((p) => p.constructor.name), bloomInLens: !!lens.uniforms.tBloom && lens.uniforms.tBloom.value === bloom.renderTargetsHorizontal[0].texture, first: allocated(composer.renderTarget1), second: allocated(composer.renderTarget2) };
  })()`);
  assert.deepEqual(r.twice, [], "every additive double-sided material draws in one pass");
  assert.deepEqual(r.passes, ["RenderPass", "UnrealBloomPass", "OutputPass"]);
  assert.equal(r.bloomInLens, true, "the lens adds the bloom");
  assert.equal([r.first, r.second].filter(Boolean).length, 1, `one target is drawn into, the other never allocated: ${JSON.stringify(r)}`);
}));
