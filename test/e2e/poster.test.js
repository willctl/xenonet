// End to end: posters and films of the day on a real XenoNet Dev (the test
// instance, with the demo's made-up data; see xenonet.test.js). The export
// panel opens from the palette with a preview and says exports stay on
// this computer; each size renders at its own pixels with star names and
// no clock, and nothing of the demo's project names unless an export
// reveals them; and a film plays in a story-shaped window with the mark
// and no clock. Nothing is saved: a save would go to the pictures folder.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { POSTER_SIZES } from "../../core/frames.js";
import { MARK_BAND, MARK_URL } from "../../src/mark.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const api = (route) => fetch(`http://127.0.0.1:${port}${route}`).then((r) => r.json());

// Runs a test body, and on failure saves what the window looked like.
const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

// Polls `fn` until `ok` holds of what it returns, or the time's up.
async function until(fn, ok, ms = 20000) {
  for (const end = Date.now() + ms; ; await sleep(250)) {
    const got = await fn();
    if (ok(got) || Date.now() > end) return got;
  }
}

const CLOCK = /\b\d{1,2}:\d{2}\b/;
// Everything a poster's lines say, as one text.
const said = (lines) => [lines.band, lines.headline, lines.date, lines.url, ...lines.stars.names, ...lines.stats.flat(), ...lines.apps.map((a) => `${a.label} ${a.count}`)].join("\n");

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("the export panel opens from the palette with a preview, hiding names and times until told, and says exports stay here", { timeout: 60000 }, shoot("export-panel", async () => {
  const st = await s.driver("command", "poster");
  assert.ok(st.overlays.modal && st.overlays.export, "the panel is open");
  const picked = await until(() => s.driver("exportPanel"), (p) => p?.preview);
  assert.ok(picked.preview, "a preview rendered");
  assert.deepEqual(picked.reveal, { names: false, tokens: false }, "nothing revealed until told");
  assert.equal(picked.size, "portrait");
  assert.equal(picked.seconds, 20);
  const text = await s.eval(`return document.getElementById("modal").innerText`);
  assert.match(text, /Nothing is uploaded, no account is needed, and the file carries no metadata/);
  assert.match(text, /star names/);
  // innerText carries the stylesheet's capitals, so the matches ignore case.
  assert.match(text, /reveal more/i);
  assert.match(text, /copy as image/i);
  assert.match(text, /save png/i);
  for (const size of POSTER_SIZES) assert.ok(text.includes(`${size.width}×${size.height}`), size.id);
  const boxes = await s.eval(`return [...document.querySelectorAll("#modal input[type=checkbox]")].map((b) => b.checked)`);
  assert.deepEqual(boxes, [false, false]);
  const preview = await s.eval(`return (() => { const i = document.querySelector("#modal .xp-preview img"); return i ? { w: i.naturalWidth, h: i.naturalHeight } : null; })()`);
  assert.deepEqual(preview, { w: 1080, h: 1350 }, "the preview is the poster itself");
  const closed = await s.input({ keys: "Escape" });
  await until(() => s.driver("state"), (x) => !x.overlays.export);
  assert.ok(!(await s.driver("state")).overlays.export, JSON.stringify(closed.overlays));
  assert.equal((await s.driver("exportPanel")), null);
}));

test("each size renders at its own pixels, with the mark, star names and no clock, and the projects' names only when revealed", { timeout: 120000 }, shoot("poster-sizes", async () => {
  const day = await api("/api/daylog");
  const projects = day.topProjects.map((p) => p.name);
  assert.ok(projects.length >= 2 && day.busiestHour != null, "the demo has busy projects and a busiest hour");
  for (const size of POSTER_SIZES) {
    const r = await s.driver("poster", { size: size.id });
    assert.deepEqual([r.width, r.height, r.size], [size.width, size.height, size.id]);
    assert.ok(r.bytes > 20000, `${size.id}: a PNG of ${r.bytes} bytes`);
    const text = said(r.lines);
    assert.ok(text.includes(MARK_BAND) && text.includes(MARK_URL), `${size.id}: the mark`);
    assert.doesNotMatch(text, CLOCK, `${size.id}: a clock`);
    for (const p of projects) assert.ok(!text.toLowerCase().includes(p.toLowerCase()), `${size.id}: ${p} in ${text}`);
    assert.ok(r.lines.stars.names.length >= 2 && r.lines.stars.names.every((n) => /^[A-Z][a-z]+( \d+)?$/.test(n)), `${size.id}: star names ${r.lines.stars.names}`);
    assert.ok(!r.lines.stats.some(([, k]) => k === "tokens"), `${size.id}: tokens stay off`);
    assert.deepEqual(r.revealed, { names: false, tokens: false });
    assert.equal(r.saved, false);
  }
  // The same star names from one render to the next, all day.
  const a = await s.driver("poster", { size: "card" });
  const b = await s.driver("poster", { size: "wide" });
  assert.deepEqual(a.lines.stars.names, b.lines.stars.names);
  // Revealed, for this one export: the projects' names, and the tokens.
  const shown = await s.driver("poster", { size: "card", reveal: { names: true, tokens: true } });
  assert.ok(shown.lines.stars.names.some((n) => projects.includes(n)), `${shown.lines.stars.names} has a project`);
  assert.ok(shown.lines.stats.some(([, k]) => k === "tokens"));
  assert.deepEqual(shown.revealed, { names: true, tokens: true });
  const next = await s.driver("poster", { size: "card" });
  assert.ok(!next.lines.stars.names.some((n) => projects.includes(n)), "the next export starts hidden again");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("a film plays in a story-shaped window with the mark, star names, running counts and no clock", { timeout: 90000 }, shoot("film", async () => {
  const projects = (await api("/api/daylog")).topProjects.map((p) => p.name);
  // The frame the desktop app records a film in, at half size: narrow
  // enough for the page's narrow layout.
  await s.window({ size: "540x960" });
  try {
    const started = await s.driver("film", { seconds: 7 });
    assert.ok(started.started);
    const st = await until(() => s.driver("state"), (x) => x.overlays.film === "playing", 30000);
    assert.equal(st.overlays.film, "playing", "the day plays");
    assert.ok(st.viewport.width < 900, `a narrow viewport: ${st.viewport.width}`);
    const text = await s.eval(`return document.querySelector(".film").innerText`);
    assert.ok(text.includes(MARK_BAND) && text.includes(MARK_URL), text);
    assert.doesNotMatch(text, CLOCK, "a clock");
    assert.match(text, /tool calls/i);
    assert.match(text, /sessions at work/i);
    for (const p of projects) assert.ok(!text.toLowerCase().includes(p.toLowerCase()), `${p} in ${text}`);
    assert.equal(await s.eval(`return document.querySelector(".film .fl-clock")`), null);
    assert.ok(await s.eval(`return !!document.querySelector(".film [data-fl-progress]")`), "a line fills instead");
    // The end card, with the day's stars.
    const end = await until(() => s.driver("state"), (x) => x.overlays.film === "end", 30000);
    assert.equal(end.overlays.film, "end");
    const card = await s.eval(`return document.querySelector(".film").innerText`);
    assert.match(card, /brightest stars/i);
    assert.doesNotMatch(card, CLOCK);
    for (const p of projects) assert.ok(!card.toLowerCase().includes(p.toLowerCase()), `${p} in ${card}`);
    await until(() => s.driver("state"), (x) => !x.overlays.film, 20000);
  } finally {
    await s.driver("stopFilm").catch(() => {});
    await s.window({ size: null });
  }
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
