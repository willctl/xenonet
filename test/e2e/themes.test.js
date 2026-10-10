// End to end: themes switch live, without a reload, on a real XenoNet Dev
// (the test instance, with the demo's made-up data; see xenonet.test.js),
// and the switcher (Theme…) and the gallery show each one before it's
// kept. Each theme's look is saved to .xenonet/e2e-theme-<id>.png on the way.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

// What the page shows: the same page (its time origin), and the theme its
// stylesheet has.
const look = () => s.eval(`return { origin: performance.timeOrigin, theme: document.documentElement.dataset.theme, accent: getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() }`);
// Waits for the page to show what `want` says.
async function until(want, ms = 10000) {
  const end = Date.now() + ms;
  let got;
  while (Date.now() < end && !want((got = await look()))) await sleep(200);
  return got;
}

let all = [];
let first = null;

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
  ({ themes: all, current: first } = await s.driver("themes"));
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("switching through every theme and back is live: drawn in place, never a reload", { timeout: 180000 }, shoot("themes-live", async () => {
  assert.ok(all.length >= 5, `themes: ${all.map((t) => t.id)}`);
  const { origin } = await look();
  for (const t of [...all.filter((x) => x.id !== first), all.find((x) => x.id === first)]) {
    const st = await s.driver("theme", t.id);
    assert.deepEqual([st.theme.id, st.theme.inUse, st.theme.preview, st.theme.pending], [t.id, t.id, null, false]);
    assert.ok(st.motion.settled, `settled after ${t.id}`);
    const l = await look();
    assert.equal(l.origin, origin, `switching to ${t.id} reloaded the page`);
    assert.equal(l.theme, t.id);
    assert.equal(l.accent, (await s.api(`/api/themes/${t.id}`)).colors.accent);
    assert.equal((await s.api("/api/themes")).current, t.id, "the server keeps the choice");
  }
}));

test("every theme draws with no page errors", { timeout: 180000 }, shoot("themes-draw", async () => {
  const { cursor } = await s.logs({ limit: 1 });
  for (const t of all) {
    await s.driver("theme", t.id);
    await s.screenshot({ name: `e2e-theme-${t.id}`, inline: false });
  }
  await s.driver("theme", first);
  const { entries } = await s.logs({ since: cursor, level: "error", source: "page" });
  assert.deepEqual(entries.map((e) => e.text), []);
  assert.deepEqual(await s.driver("errors"), []);
}));

test("what you did survives a switch: the op log, with each switch logged once", { timeout: 60000 }, shoot("themes-ops", async () => {
  await s.driver("select", null);
  await s.driver("clearOps");
  const { nodes } = await s.driver("find", { query: "atlas-api", kind: "project", limit: 1 });
  await s.driver("select", nodes[0].id);
  await s.driver("theme", "phosphor");
  // From the palette, as a person would: one step, not a command and a step.
  await s.driver("command", "theme-gridline");
  await until((l) => l.theme === "gridline");
  const ops = (await s.driver("ops")).map(({ op, args }) => [op, ...args]);
  assert.deepEqual(ops, [["select", nodes[0].id], ["theme", "phosphor"], ["theme", "gridline"]]);
  assert.equal((await s.driver("state")).selection.id, nodes[0].id, "the selection stays");
  await s.driver("select", null);
  await s.driver("theme", first);
}));

test("a preview shows a theme in this window alone, and goes back or is kept", { timeout: 60000 }, shoot("themes-preview", async () => {
  let st = await s.driver("previewTheme", "gridline");
  assert.deepEqual([st.theme.id, st.theme.preview, st.theme.inUse], ["gridline", "gridline", first]);
  assert.equal((await s.api("/api/themes")).current, first, "a preview isn't saved");
  st = await s.driver("revertTheme");
  assert.deepEqual([st.theme.id, st.theme.preview], [first, null]);
  assert.equal((await look()).theme, first);
  await s.driver("previewTheme", "ochre");
  st = await s.driver("keepTheme");
  assert.deepEqual([st.theme.id, st.theme.inUse, st.theme.preview], ["ochre", "ochre", null]);
  assert.equal((await s.api("/api/themes")).current, "ochre");
  await s.driver("theme", first);
}));

test("a theme file of yours named for a built-in takes its place, live", { timeout: 60000 }, shoot("themes-yours", async () => {
  const { folder } = await s.api("/api/themes");
  const builtIn = (await s.api("/api/themes/phosphor")).colors.accent;
  await s.driver("theme", "phosphor");
  const file = path.join(folder, "phosphor.json");
  const { origin } = await look();
  fs.writeFileSync(file, JSON.stringify({ colors: { accent: "#12ab34" } }));
  try {
    const l = await until((x) => x.accent === "#12ab34");
    assert.deepEqual([l.accent, l.origin], ["#12ab34", origin], "drawn in place");
    const { themes } = await s.driver("themes");
    assert.deepEqual(themes.filter((t) => t.id === "phosphor").map((t) => [t.builtIn, !!t.replacesBuiltIn]), [[false, true]]);
  } finally {
    fs.rmSync(file, { force: true });
  }
  assert.equal((await until((x) => x.accent === builtIn)).accent, builtIn, "and the built-in comes back when it's gone");
  await s.driver("theme", first);
}));

test("a theme file that tries to get past the checks is drawn as XenoNet's would be, in the gallery and on the map", { timeout: 60000 }, shoot("themes-hostile", async () => {
  const { folder } = await s.api("/api/themes");
  const file = path.join(folder, "hostile.json");
  // Made up, as a theme someone shared might be: it starts from a name
  // every object has, puts markup where colors go and declarations where
  // fonts go, and asks for rings without end and a sonar every frame (a
  // millisecond apart: 0 would send none).
  fs.writeFileSync(file, JSON.stringify({
    name: "Hostile",
    about: "made up",
    extends: "toString",
    colors: { bg: `"><img src=x onerror="window.pwned=1">`, accent: "red;position:fixed;inset:0" },
    fonts: { display: "Optima;position:fixed;inset:0;z-index:99999", body: "serif;position:fixed" },
    actors: { form: "orb", params: { rings: 1e9 } },
    sonar: { every: 0.001 },
  }));
  try {
    const xenonetTheme = await s.api("/api/themes/xenonet");
    const theme = await s.api("/api/themes/hostile");
    assert.deepEqual([theme.colors.bg, theme.colors.accent, theme.fonts.display, theme.fonts.body], [xenonetTheme.colors.bg, xenonetTheme.colors.accent, xenonetTheme.fonts.display, undefined], "XenoNet's, where its values don't fit");
    await s.driver("themeGallery", "hostile");
    const card = await s.eval(`return (() => { const c = document.querySelector('.tg-card[data-id="hostile"]'); return { pwned: window.pwned ?? null, injected: document.querySelectorAll(".tgal img, .tgal [onerror], .tgal [onmouseover]").length, position: getComputedStyle(c).position, rings: c.querySelectorAll("ellipse").length, bg: c.style.getPropertyValue("--tg-bg") }; })()`);
    assert.deepEqual(card, { pwned: null, injected: 0, position: "relative", rings: 8, bg: xenonetTheme.colors.bg }, "its card is a card like any other");
    // Looked at, it's on the map: a sonar wave a second at most, not one a
    // frame (each lives about six seconds).
    await sleep(3000);
    const waves = await s.eval(`return window.__xenonet.scene.graph.scene().children.filter((o) => o.type === "Group" && o.children.some((c) => c.geometry?.type === "SphereGeometry") && o.children.some((c) => c.geometry?.type === "RingGeometry")).length`);
    assert.ok(waves >= 1 && waves <= 8, `${waves} sonar waves at once`);
    assert.deepEqual(await s.driver("errors"), []);
  } finally {
    await s.driver("themeGallery", false);
    fs.rmSync(file, { force: true });
    await s.driver("theme", first);
  }
}));

// Polls the page's state until `want` holds.
async function untilState(want, ms = 10000) {
  const end = Date.now() + ms;
  let st;
  while (Date.now() < end && !want((st = await s.driver("state")))) await sleep(150);
  return st;
}

test("Theme… shows each theme on the map as you move through it: Esc puts yours back, Enter keeps one", { timeout: 90000 }, shoot("themes-switcher", async () => {
  await s.driver("theme", first);
  const { origin } = await look();
  // From the palette, with keys, as a person would.
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "Theme…" });
  await s.input({ keys: "Enter" });
  let st = await untilState((x) => x.overlays.switcher && x.themes.switcher);
  assert.equal(st.themes.switcher.active, first, "it starts at the theme in use");
  assert.deepEqual(st.themes.switcher.items, all.map((t) => t.id));
  const items = st.themes.switcher.items, at = items.indexOf(first);
  for (const k of [1, 2]) {
    await s.input({ keys: "ArrowDown" });
    const want = items[(at + k) % items.length];
    st = await untilState((x) => x.theme.id === want && !x.theme.pending);
    assert.deepEqual([st.themes.switcher.active, st.theme.preview, st.theme.inUse], [want, want, first]);
    assert.equal((await look()).theme, want, "drawn in this window");
    assert.equal((await s.api("/api/themes")).current, first, "and nothing saved");
  }
  await s.input({ keys: "Escape" });
  st = await untilState((x) => !x.overlays.switcher && x.theme.id === first);
  assert.deepEqual([st.theme.preview, st.theme.inUse], [null, first]);

  // Through the driver, then Enter keeps what's shown, everywhere.
  st = await s.driver("stepTheme", 1);
  const kept = st.themes.switcher.active;
  assert.deepEqual([st.theme.id, st.theme.preview], [kept, kept]);
  await s.input({ keys: "Enter" });
  st = await untilState((x) => !x.overlays.switcher && x.theme.inUse === kept && !x.theme.pending);
  assert.deepEqual([st.theme.id, st.theme.preview], [kept, null]);
  assert.equal((await s.api("/api/themes")).current, kept);

  // Typing narrows the list to the best match first, and shows it.
  await s.driver("themeSwitcher");
  await s.input({ text: "eclipse" });
  st = await untilState((x) => x.theme.id === "eclipse" && !x.theme.pending);
  assert.equal(st.themes.switcher.items[0], "eclipse");
  st = await s.driver("revertTheme");
  assert.deepEqual([st.overlays.switcher, st.theme.id], [false, kept]);
  assert.equal((await look()).origin, origin, "never a reload");
  await s.driver("theme", first);
}));

test("the gallery: a card for every theme, yours too; looking at one shows it, a click keeps it, and you make your own from it", { timeout: 120000 }, shoot("themes-gallery", async () => {
  await s.driver("theme", first);
  const { origin } = await look();
  let st = await s.driver("command", "theme-gallery");
  assert.ok(st.overlays.gallery);
  assert.deepEqual(st.themes.gallery.cards, all.map((t) => t.id));
  assert.deepEqual([st.themes.gallery.focused, st.theme.preview], [first, null]);
  // Each card in its own colors, with its sky and actors, swatches, type and mood.
  const cards = await s.eval(`return [...document.querySelectorAll(".tg-card[data-id]")].map((c) => ({ id: c.dataset.id, bg: c.style.getPropertyValue("--tg-bg"), sky: !!c.querySelector(".tg-sky svg"), swatches: c.querySelectorAll(".tg-sw i").length, mood: c.querySelector(".tg-mood").textContent }))`);
  for (const t of all) {
    const c = cards.find((x) => x.id === t.id);
    assert.equal(c.bg, (await s.api(`/api/themes/${t.id}`)).colors.bg, `${t.id}: in its own colors`);
    assert.ok(c.sky && c.swatches >= 8, `${t.id}: a sky, actors and swatches`);
    assert.equal(c.mood, t.about);
  }

  // Moving with the keys looks at the next card, in this window alone.
  const at = all.findIndex((t) => t.id === first), next = all[(at + 1) % all.length].id;
  await s.input({ keys: "ArrowDown" });
  st = await untilState((x) => x.theme.id === next && !x.theme.pending);
  assert.deepEqual([st.themes.gallery.focused, st.theme.preview, st.theme.inUse], [next, next, first]);
  assert.equal((await s.api("/api/themes")).current, first);

  // Pointing at a card looks at it too, and a click keeps it, everywhere.
  const target = all.find((t) => ![first, next].includes(t.id)).id;
  const spot = await s.eval(`return (() => { const c = document.querySelector('.tg-card[data-id="${target}"]'); c.scrollIntoView({ block: "center" }); const r = c.getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + 30) }; })()`);
  await s.input({ move: spot });
  st = await untilState((x) => x.theme.id === target && !x.theme.pending);
  assert.deepEqual([st.themes.gallery.focused, st.theme.preview], [target, target]);
  await s.input({ click: spot });
  st = await untilState((x) => x.theme.inUse === target && !x.theme.pending);
  assert.equal((await s.api("/api/themes")).current, target);
  assert.ok(st.overlays.gallery, "it stays open to look further");

  // Make your own: a copy of the card looked at, saved as your file and
  // put in use, with the file shown.
  await s.driver("focusTheme", "rime");
  st = await s.driver("makeTheme");
  try {
    assert.equal(st.made.id, "rime-mine");
    assert.equal(JSON.parse(fs.readFileSync(st.made.file, "utf8")).extends, "rime");
    assert.deepEqual([st.theme.inUse, st.theme.preview, st.themes.gallery.focused], ["rime-mine", null, "rime-mine"]);
    assert.ok(st.themes.gallery.cards.includes("rime-mine"));
    assert.match(await s.eval(`return document.querySelector(".tg-make code")?.textContent ?? ""`), /rime-mine\.json$/);
    assert.equal((await s.driver("themes")).themes.find((t) => t.id === "rime-mine").builtIn, false);

    // Esc closes it, keeping the theme in use.
    await s.input({ keys: "Escape" });
    st = await untilState((x) => !x.overlays.gallery);
    assert.equal(st.theme.id, "rime-mine");
    assert.equal((await look()).origin, origin, "never a reload");
    assert.deepEqual(await s.driver("errors"), []);
  } finally {
    await s.driver("theme", first);
    if (st.made?.file) fs.rmSync(st.made.file, { force: true });
  }
}));
