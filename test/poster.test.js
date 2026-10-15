// A poster of the day (src/poster-draw.js), drawn onto a context that
// records what it writes: at each of the four sizes, with a day whose
// projects have real-looking names, nothing drawn is a project name, a
// path, a host or a time, the mark and the site are there, and there's no
// border. Revealing more is an export's own choice. The mark's polygons
// (src/mark.js) match the brand SVG.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { POSTER_SIZES } from "../core/frames.js";
import { drawPoster, posterLines } from "../src/poster-draw.js";
import { MARK_POLYGONS, MARK_BAND, MARK_URL } from "../src/mark.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// A 2D context that keeps what's drawn: the text with where it went, the
// rectangles stroked, and the paths' extents.
function recorder() {
  const g = {
    font: "",
    fillStyle: "",
    strokeStyle: "",
    letterSpacing: "0px",
    texts: [],
    strokedRects: [],
    fills: 0,
    fillText(text, x, y) {
      this.texts.push({ text: String(text), x, y, font: this.font });
    },
    measureText(text) {
      return { width: String(text).length * (parseFloat(this.font.match(/(\d+(?:\.\d+)?)px/)?.[1] ?? 16) * 0.62 + parseFloat(this.letterSpacing)) };
    },
    strokeRect(...args) {
      this.strokedRects.push(args);
    },
    createLinearGradient: () => ({ addColorStop() {} }),
    drawImage() {},
    fillRect() {},
    beginPath() {},
    closePath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    fill() {
      this.fills++;
    },
    stroke() {},
  };
  return g;
}

const theme = { colors: { bg: "#05070a", accent: "#39ff9f", bright: "#e9fff4", muted: "#7d8c99", label: "#a8d8c0" }, fonts: { display: '"Chakra Petch", sans-serif', mono: '"JetBrains Mono", monospace' } };
const SECRETS = ["atlas-api", "ledger", "harbor-api", "dana", "/Users", "feat/refund", "mbp.local", "partial refunds"];
const day = {
  date: "Friday, October 3",
  archetype: "cluster",
  totals: { sessions: 3, turns: 7, toolCalls: 1250, files: 12 },
  tokens: 1234567,
  stars: [
    { name: "Thaloren", toolCalls: 40, project: "atlas-api" },
    { name: "Vesku", toolCalls: 22, project: "ledger" },
    { name: "Drashi", toolCalls: 9, project: "harbor-api" },
  ],
  apps: { claude: 2, codex: 1 },
};

test("at every size: the band, the site, the archetype and star names, and nothing of the projects, paths or times", () => {
  for (const size of POSTER_SIZES) {
    const g = recorder();
    const lines = drawPoster(g, { width: size.width, height: size.height, image: null, data: day, theme });
    const texts = g.texts.map((t) => t.text);
    const all = texts.join("\n");
    for (const s of SECRETS) assert.ok(!all.toLowerCase().includes(s.toLowerCase()), `${size.id}: ${s} in ${all}`);
    assert.doesNotMatch(all, /\b\d{1,2}:\d{2}\b/, `${size.id}: a clock`);
    assert.ok(texts.includes(MARK_BAND) && texts.includes(MARK_URL), `${size.id}: the mark`);
    assert.ok(texts.includes("Cluster") && texts.includes("Friday, October 3"), `${size.id}: the headline and the date`);
    assert.ok(texts.some((t) => t.startsWith("BRIGHTEST STARS") && t.includes("THALOREN")), `${size.id}: the stars`);
    assert.ok(texts.includes("CLAUDE CODE 2") && texts.includes("CODEX 1"), `${size.id}: the apps`);
    assert.deepEqual(
      lines.stats.map(([v, k]) => `${v} ${k}`),
      ["3 sessions", "7 turns", "1.3k tool calls", "12 files edited"],
    );
    assert.ok(!texts.some((t) => /tokens/i.test(t)), `${size.id}: tokens stay off`);
    for (const t of g.texts) assert.ok(t.x >= 0 && t.x < size.width && t.y > 0 && t.y <= size.height, `${size.id}: "${t.text}" at ${t.x},${t.y}`);
    assert.equal(g.strokedRects.length, 0, `${size.id}: no border`);
    assert.ok(g.fills > 0, `${size.id}: the mark and the glyphs are drawn`);
  }
});

test("revealing more is per export: the projects' names in place of the stars, and the tokens", () => {
  const g = recorder();
  drawPoster(g, { width: 1080, height: 1350, image: null, data: day, reveal: { names: true, tokens: true }, theme });
  const texts = g.texts.map((t) => t.text);
  assert.ok(texts.some((t) => t.startsWith("BUSIEST PROJECTS") && t.includes("ATLAS-API") && t.includes("LEDGER")));
  assert.ok(!texts.some((t) => t.includes("THALOREN")));
  assert.ok(texts.includes("TOKENS") && texts.includes("1.2M"));
  // Without the names in the data (the server wasn't asked), the stars stay.
  const lines = posterLines({ ...day, stars: day.stars.map(({ project, ...s }) => s) }, { names: true });
  assert.deepEqual(lines.stars.names, ["Thaloren", "Vesku", "Drashi"]);
});

test("the stars line drops names until it fits the frame", () => {
  const long = { ...day, stars: Array.from({ length: 4 }, (_, i) => ({ name: `Thalorenvesku${i}`, toolCalls: 1 })) };
  const g = recorder();
  const lines = drawPoster(g, { width: 1080, height: 1920, image: null, data: long, theme });
  assert.ok(lines.stars.shown.length >= 1 && lines.stars.shown.length < 4, `${lines.stars.shown.length} shown`);
  const wide = recorder();
  assert.equal(drawPoster(wide, { width: 1920, height: 1080, image: null, data: long, theme }).stars.shown.length, 4);
});

test("a day without an archetype or stars: the date leads, and the band stays", () => {
  const g = recorder();
  const lines = drawPoster(g, { width: 1200, height: 630, image: null, data: { date: "Monday, October 6", totals: {}, stars: [] }, theme });
  assert.equal(lines.headline, "Monday, October 6");
  assert.equal(lines.date, "");
  assert.deepEqual(lines.stars.names, []);
  assert.deepEqual(lines.apps, []);
  assert.ok(g.texts.some((t) => t.text === MARK_BAND));
});

test("the mark's polygons are the brand SVG's path", () => {
  const svg = fs.readFileSync(path.join(root, "public", "brand", "xenonet.svg"), "utf8");
  const d = svg.match(/<path d="([^"]+)"/)[1];
  // M x y, L x y, H x, V y and Z, as the SVG uses them.
  const polys = [];
  let cur = null, x = 0, y = 0;
  for (const [, op, args] of d.matchAll(/([MLHVZ])([^MLHVZ]*)/g)) {
    const nums = args.trim().split(/[\s,]+/).filter(Boolean).map(Number);
    if (op === "M") polys.push((cur = [[(x = nums[0]), (y = nums[1])]]));
    else if (op === "L") cur.push([(x = nums[0]), (y = nums[1])]);
    else if (op === "H") cur.push([(x = nums[0]), y]);
    else if (op === "V") cur.push([x, (y = nums[0])]);
  }
  assert.deepEqual(MARK_POLYGONS, polys);
});
