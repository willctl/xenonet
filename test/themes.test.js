// Themes (core/themes.js) and your own theme files (core/themefiles.js).
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { themes, merge, conform, complete, contrast, colorDistance, ringColors, sonarGap, STYLES, BUILT_IN, DEFAULT_THEME } from "../core/themes.js";
import { readThemes, copyTheme } from "../core/themefiles.js";
import { themeRoutes } from "../server/routes/themes.js";

const HEX = /^#[0-9a-f]{6}$/i;
// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-themes-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

// Every leaf of `shape`, as "a.b.c" paths.
const paths = (o, at = "") => Object.entries(o).flatMap(([k, v]) => (v && typeof v === "object" && !Array.isArray(v) ? paths(v, `${at}${k}.`) : [`${at}${k}`]));

test("every built-in theme has everything XenoNet's own has, with real colors", () => {
  const all = themes();
  assert.deepEqual(Object.keys(all), BUILT_IN);
  assert.equal(all[DEFAULT_THEME].name, "XenoNet");
  const want = paths(all.xenonet).filter((p) => !["id", "builtIn"].includes(p));
  for (const t of Object.values(all)) {
    for (const p of want) assert.notEqual(p.split(".").reduce((o, k) => o?.[k], t), undefined, `${t.id} lacks ${p}`);
    for (const [k, v] of Object.entries({ ...t.colors, ...t.categories, ...t.pages, ...t.charts })) assert.match(v, HEX, `${t.id}: ${k} is ${v}`);
    for (const part of ["night", "dawn", "day", "dusk"]) assert.equal(t.sky[part].length, 4, `${t.id}: sky.${part}`);
    assert.ok(t.sound.scale.length >= 5 && t.sound.chords.every((ch) => ch.length === 5), `${t.id}: music`);
    assert.ok(t.name && t.about, `${t.id}: a name and a line about it`);
  }
  // Themes differ where they say they do, and only there.
  assert.notEqual(all.phosphor.colors.accent, all.xenonet.colors.accent);
  assert.deepEqual(all.phosphor.charts, all.xenonet.charts, "the checked chart colors carry over");
});

// Today's look in the tokens added for the next phase (docs/design/contracts.md, 4).
const TODAY = { "stars.shape": "point", "stars.drift": [0, 0, 0], "stars.twinkle": 0.5, "grid.style": "polar", "sweep.shape": "beam", "sonar.style": "ring", "lines.style": "solid", "lines.curvature": 0, "lines.width": 1, "motion.pace": 1, "motion.ease": "smooth", "motion.breathe": 0.1 };
const at = (o, p) => p.split(".").reduce((x, k) => x?.[k], o);

// The themes XenoNet had before the tokens, which keep their look in them.
const FIRST_FOUR = ["xenonet", "phosphor", "ochre", "gridline"];

test("the new tokens: every built-in has them, the first four with today's look, and actors in its own colors", () => {
  for (const t of Object.values(themes())) {
    if (FIRST_FOUR.includes(t.id)) for (const [p, v] of Object.entries(TODAY)) assert.deepEqual(at(t, p), v, `${t.id}: ${p}`);
    for (const [p, values] of Object.entries(STYLES)) assert.ok(values.includes(at(t, p)), `${t.id}: ${p} is ${at(t, p)}`);
    assert.ok(t.actors.palette.length >= 4 && t.actors.palette.every((c) => HEX.test(c)), `${t.id}: an actor palette`);
    assert.ok(Number.isFinite(t.actors.glow.opacity) && Number.isFinite(t.actors.breathe.period) && Number.isFinite(t.actors.beam.width));
  }
  assert.equal(themes().phosphor.actors.form, "cursor");
  assert.notEqual(themes().phosphor.colors.human, themes().xenonet.colors.human);
});

test("the four newer themes: each draws its world in styles of its own", () => {
  const all = themes();
  const looks = (id) => ["stars.shape", "grid.style", "sweep.shape", "sonar.style", "lines.style"].map((p) => at(all[id], p));
  assert.deepEqual(looks("helios"), ["point", "cells", "flare", "ripple", "arc"]);
  assert.deepEqual(looks("rime"), ["flake", "hex", "aurora", "ring", "hairline"]);
  assert.deepEqual(looks("trench"), ["mote", "polar", "beam", "ping", "filament"]);
  assert.deepEqual(looks("eclipse"), ["point", "polar", "none", "ring", "solid"]);
  // Snow falls and marine snow drifts down.
  assert.ok(all.rime.stars.drift[1] < 0 && all.trench.stars.drift[1] < 0);
  // Eclipse: Okabe and Ito's colors where something needs you, and calm.
  const okabeIto = ["#e69f00", "#56b4e9", "#009e73", "#f0e442", "#0072b2", "#d55e00", "#cc79a7"];
  for (const role of ["claude", "codex", "attention", "review", "stuck", "danger", "collision"]) assert.ok(okabeIto.includes(all.eclipse.colors[role]), `eclipse: ${role}`);
  assert.deepEqual([all.eclipse.motion.breathe, all.eclipse.stars.twinkle, all.eclipse.sweep.shape], [0, 0, "none"]);
  assert.ok(all.eclipse.motion.pace < 0.5);
});

// The stats panel's chart colors (src/stats.js), on its surface, which is
// half its surface and half the low one: model colors and the one-series
// color 3:1 and apart from their neighbors, token kinds one ramp from dark
// to light whose dark end still shows, and the rest readable. Checked
// with the dataviz rules when they were picked; these are the parts a
// slip would break.
test("every theme's chart colors stand out on the stats panel", () => {
  const mix = (a, b) => `#${[1, 3, 5].map((i) => Math.round((parseInt(a.slice(i, i + 2), 16) + parseInt(b.slice(i, i + 2), 16)) / 2).toString(16).padStart(2, "0")).join("")}`;
  for (const t of Object.values(themes())) {
    const on = mix(t.colors.surface, t.colors.surfaceLow), c = t.charts;
    const models = [c.m1, c.m2, c.m3, c.m4, c.m5];
    for (const [k, v] of Object.entries({ m1: c.m1, m2: c.m2, m3: c.m3, m4: c.m4, m5: c.m5, one: c.one, bad: c.bad, input: c.input, output: c.output })) assert.ok(contrast(v, on) >= 3, `${t.id}: ${k} ${v} is ${contrast(v, on).toFixed(2)}:1`);
    assert.ok(contrast(c.other, on) >= 2.9 && contrast(c.cacheRead, on) >= 2, `${t.id}: other and the ramp's dark end`);
    models.slice(1).forEach((m, i) => assert.ok(colorDistance(m, models[i]) >= 20, `${t.id}: ${models[i]} and ${m} look alike`));
    const ramp = [c.cacheRead, c.input, c.cacheWrite, c.output].map((x) => contrast(x, on));
    assert.ok(ramp.every((r, i) => !i || r > ramp[i - 1]), `${t.id}: token kinds from dark to light`);
  }
});

test("conforming: a value that doesn't fit keeps the one underneath, so a slip costs one value", () => {
  const base = themes().phosphor;
  const t = conform(base, {
    colors: { accent: "red", text: "#ABC", danger: 7 },
    grid: { style: "spiral", opacity: "lots" },
    sweep: { shape: "flare" },
    stars: { tints: "white", drift: [0, -0.6], twinkle: 0.9 },
    sky: { night: ["#000000", "#111111", "#222222"], dawn: ["#000", "#111", "#222", "#333"] },
    sound: { chords: [[50, "x"]], scale: [60, 62, 64], pad: "custom" },
    motion: { pace: Infinity, ease: "stepped" },
    actors: { form: "kraken", params: { rings: 3, spin: true, name: "no" }, palette: [] },
    lattice: null,
    mine: { anything: 1 },
  });
  assert.equal(t.colors.accent, base.colors.accent, "not a color: Phosphor's");
  assert.equal(t.colors.text, "#aabbcc", "short colors are written out");
  assert.equal(t.colors.danger, base.colors.danger);
  assert.deepEqual([t.grid.style, t.grid.opacity, t.grid.center], ["polar", base.grid.opacity, base.grid.center]);
  assert.equal(t.sweep.shape, "flare", "a style XenoNet knows");
  assert.deepEqual([t.stars.tints, t.stars.drift, t.stars.twinkle], [base.stars.tints, base.stars.drift, 0.9]);
  assert.deepEqual([t.sky.night, t.sky.dawn], [base.sky.night, ["#000000", "#111111", "#222222", "#333333"]], "the sky's four colors, no fewer");
  assert.deepEqual([t.sound.chords, t.sound.scale, t.sound.pad], [base.sound.chords, [60, 62, 64], base.sound.pad]);
  assert.deepEqual([t.motion.pace, t.motion.ease], [1, "stepped"]);
  assert.deepEqual([t.actors.form, t.actors.params, t.actors.palette], ["cursor", { rings: 3, spin: true }, base.actors.palette]);
  assert.deepEqual(t.lattice, base.lattice);
  assert.deepEqual(t.mine, { anything: 1 }, "fields XenoNet doesn't use pass through");
});

test("a sonar.every of 0 or less sends no sonar, which a theme keeps; any other is at least a second", () => {
  assert.equal(sonarGap(16), 16);
  assert.equal(sonarGap(0.25), 1, "never more than one wave a second");
  for (const off of [0, -3]) {
    assert.equal(sonarGap(off), null, `${off}: no sonar`);
    // Kept as written, in a theme of yours and in one that extends another.
    assert.equal(complete({ sonar: { every: off } }).sonar.every, off);
    const quiet = themes({ quiet: { extends: "trench", sonar: { every: off } } }).quiet;
    assert.deepEqual([quiet.sonar.every, quiet.sonar.style], [off, "ping"]);
  }
  // What isn't a number never reaches it (conform keeps the one underneath),
  // and means no sonar if it did.
  assert.equal(complete({ sonar: { every: "never" } }).sonar.every, 16);
  for (const bad of [NaN, Infinity, "16", null, undefined]) assert.equal(sonarGap(bad), null, String(bad));
  for (const t of Object.values(themes())) assert.ok(sonarGap(t.sonar.every) >= 1, `${t.id} sends a sonar`);
});

// A face plainer than a theme's titles, for its panels' text.
const PLAIN = '"Gill Sans", "Gill Sans MT", "Segoe UI", sans-serif';

test("a body font for the panels: a theme may set its text plainer than its titles; the built-ins leave it to the display font", () => {
  const all = themes();
  for (const t of Object.values(all)) assert.equal(t.fonts.body, undefined, `${t.id} sets its panels in its display font`);
  // A name, or nothing: a slip leaves what's underneath.
  const plain = complete({ fonts: { display: '"Futura", sans-serif', body: PLAIN } });
  assert.equal(conform(plain, { fonts: { body: 7 } }).fonts.body, PLAIN);
  assert.equal(conform(all.xenonet, { fonts: { body: " " } }).fonts.body, undefined);
  assert.equal(complete({ fonts: { body: "Optima, serif" } }).fonts.body, "Optima, serif");
  const dir = tmp();
  fs.writeFileSync(path.join(dir, "plain.json"), JSON.stringify({ extends: "rime", fonts: { body: PLAIN } }));
  fs.writeFileSync(path.join(dir, "moss.json"), JSON.stringify({ extends: "plain", fonts: { display: "Optima, sans-serif" } }));
  const moss = readThemes(dir).themes.moss;
  assert.deepEqual([moss.fonts.display, moss.fonts.body], ["Optima, sans-serif", PLAIN], "kept from the theme it extends");
});

test("a theme from before the new tokens, kept or in a file, gets every one of them", () => {
  const old = { id: "old", name: "Old", colors: { ...themes().phosphor.colors }, sky: themes().phosphor.sky, stars: { count: 10, tints: ["#ffffff"] }, grid: { center: "#111111", lines: "#222222", opacity: 0.5 } };
  delete old.colors.actor;
  delete old.colors.human;
  const t = complete(old);
  for (const [p, v] of Object.entries(TODAY)) assert.deepEqual(at(t, p), v, p);
  assert.deepEqual([t.colors.actor, t.colors.human, t.actors.form, t.stars.count, t.grid.center], [themes().xenonet.colors.actor, themes().xenonet.colors.human, "orb", 10, "#111111"]);
  const dir = tmp();
  fs.writeFileSync(path.join(dir, "old.json"), JSON.stringify({ extends: "ochre", colors: { accent: "#123456" } }));
  const fromFile = readThemes(dir).themes.old;
  assert.deepEqual([fromFile.colors.accent, fromFile.sweep.shape, fromFile.actors.form], ["#123456", "beam", "swarm"]);
});

test("merging: objects field by field, arrays and values replaced", () => {
  assert.deepEqual(merge({ a: { x: 1, y: 2 }, b: [1, 2], c: 1 }, { a: { y: 3 }, b: [9] }), { a: { x: 1, y: 3 }, b: [9], c: 1 });
});

test("your own themes: on top of what they extend, problems reported, copies complete", () => {
  const dir = tmp();
  fs.writeFileSync(path.join(dir, "ember.json"), JSON.stringify({ name: "Ember", extends: "helios", colors: { accent: "#ff0000" } }));
  fs.writeFileSync(path.join(dir, "ash.json"), JSON.stringify({ extends: "ember", glow: { bloom: 2 } }));
  fs.writeFileSync(path.join(dir, "loop-a.json"), JSON.stringify({ extends: "loop-b" }));
  fs.writeFileSync(path.join(dir, "loop-b.json"), JSON.stringify({ extends: "loop-a" }));
  fs.writeFileSync(path.join(dir, "broken.json"), "{ not json");
  fs.writeFileSync(path.join(dir, "Bad Name.json"), "{}");
  const { themes: all, problems } = readThemes(dir);
  assert.equal(all.ember.colors.accent, "#ff0000");
  assert.equal(all.ember.colors.claude, all.helios.colors.claude, "the rest from Helios");
  assert.deepEqual([all.ash.name, all.ash.glow.bloom, all.ash.colors.accent], ["ash", 2, "#ff0000"]);
  assert.equal(all.ember.builtIn, false);
  assert.ok(all["loop-a"].colors, "a loop ends at XenoNet's own");
  assert.equal(problems.length, 2);
  assert.ok(problems.some((p) => p.startsWith("broken.json")) && problems.some((p) => p.startsWith("Bad Name.json")));

  const file = copyTheme(dir, "phosphor", "mine");
  const copy = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.equal(copy.extends, "phosphor");
  assert.deepEqual(Object.keys(copy.colors), Object.keys(all.xenonet.colors), "every color, to edit");
  assert.equal(readThemes(dir).themes.mine.colors.accent, all.phosphor.colors.accent);
  assert.throws(() => copyTheme(dir, "xenonet", "mine"), /already a theme called mine/);
  assert.throws(() => copyTheme(dir, "xenonet", "Not ok"), /lowercase/);
});

// The contrast every theme keeps (docs/design/next-phase.md, Budgets): text
// and labels 4.5:1 on the page and on panels, muted text 4.5:1 and dim text
// 3:1, and the alert colors 3:1 on panels.
const BUDGET = [
  ["text", ["bg", "surface"], 4.5],
  ["label", ["bg", "surface"], 4.5],
  ["muted", ["bg", "surface"], 4.5],
  ["dim", ["bg", "surface"], 3],
  ["attention", ["surface"], 3],
  ["danger", ["surface"], 3],
  ["collision", ["surface"], 3],
];

test("each app's sessions have a color of their own in every theme, readable on the page and the panels", () => {
  const others = ["claude", "codex", "grok", "project", "satellite", "native", "resource", "actor", "gold", "attention", "danger", "collision", "stuck", "lineage", "pressure", "review", "collab"];
  for (const t of Object.values(themes())) {
    for (const app of ["claude", "codex", "grok"]) {
      for (const bg of ["bg", "surface"]) assert.ok(contrast(t.colors[app], t.colors[bg]) >= 4.5, `${t.id}: ${app} (${t.colors[app]}) on ${bg} is ${contrast(t.colors[app], t.colors[bg]).toFixed(2)}:1`);
    }
    for (const k of others.filter((x) => x !== "grok")) assert.ok(colorDistance(t.colors.grok, t.colors[k]) >= 20, `${t.id}: grok ${t.colors.grok} is close to ${k} ${t.colors[k]}`);
  }
});

test("contrast as WCAG measures it", () => {
  assert.equal(contrast("#000000", "#ffffff"), 21);
  assert.equal(contrast("#ffffff", "#000000"), 21, "either way round");
  assert.equal(contrast("#5fe1ff", "#5fe1ff"), 1);
  assert.equal(Math.round(contrast("#767676", "#ffffff") * 100) / 100, 4.54, "the classic gray that just passes on white");
});

test("every theme, built in or copied to make your own, keeps its text readable", () => {
  const dir = tmp();
  for (const id of BUILT_IN) copyTheme(dir, id, `${id}-mine`);
  const all = readThemes(dir).themes;
  assert.equal(Object.keys(all).length, BUILT_IN.length * 2);
  for (const t of Object.values(all)) {
    for (const [fg, on, min] of BUDGET) {
      for (const bg of on) {
        const r = contrast(t.colors[fg], t.colors[bg]);
        assert.ok(r >= min, `${t.id}: ${fg} (${t.colors[fg]}) on ${bg} (${t.colors[bg]}) is ${r.toFixed(2)}:1, under ${min}:1`);
      }
    }
  }
});

test("a theme of yours named for a built-in takes its place, and changes no other", () => {
  const dir = tmp();
  const builtIn = themes();
  fs.writeFileSync(path.join(dir, "phosphor.json"), JSON.stringify({ colors: { accent: "#123456" }, sound: { bpm: 70 } }));
  fs.writeFileSync(path.join(dir, "ochre.json"), JSON.stringify({ extends: "gridline", colors: { accent: "#654321" } }));
  fs.writeFileSync(path.join(dir, "xenonet.json"), JSON.stringify({ about: "Mine", colors: { accent: "#ff0000" }, charts: { m1: "#000000" }, lattice: { opacity: 0.2 } }));
  fs.writeFileSync(path.join(dir, "ember.json"), JSON.stringify({ name: "Ember", extends: "phosphor", colors: { claude: "#abcdef" } }));
  fs.writeFileSync(path.join(dir, "list.json"), "[1, 2]");
  const { themes: all, problems } = readThemes(dir);
  // Yours wins, starting from the built-in, and is marked as yours.
  assert.deepEqual([all.phosphor.colors.accent, all.phosphor.sound.bpm, all.phosphor.colors.claude, all.phosphor.name], ["#123456", 70, builtIn.phosphor.colors.claude, "Phosphor"]);
  assert.deepEqual([all.phosphor.builtIn, all.phosphor.replacesBuiltIn, all.ember.replacesBuiltIn], [false, true, undefined]);
  // Unless it says otherwise.
  assert.deepEqual([all.ochre.colors.accent, all.ochre.colors.claude, all.ochre.sky], ["#654321", builtIn.gridline.colors.claude, builtIn.gridline.sky]);
  // XenoNet's own too, without touching the built-ins that start from it.
  assert.deepEqual([all.xenonet.colors.accent, all.xenonet.about, all.xenonet.charts.m1, all.xenonet.builtIn], ["#ff0000", "Mine", "#000000", false]);
  assert.deepEqual([all.gridline.charts, all.gridline.lattice, all.gridline.builtIn], [builtIn.gridline.charts, builtIn.gridline.lattice, true]);
  // A theme of yours that extends one you've replaced starts from your version.
  assert.deepEqual([all.ember.colors.accent, all.ember.colors.claude, all.ember.name], ["#123456", "#abcdef", "Ember"]);
  // One of each id, and a file that isn't a theme is reported.
  assert.equal(Object.keys(all).length, BUILT_IN.length + 1);
  assert.deepEqual(problems, ["list.json: a theme is a JSON object of fields"]);
  // A copy of a theme that replaces a built-in is a theme of yours like any other.
  const copy = JSON.parse(fs.readFileSync(copyTheme(dir, "phosphor", "mine"), "utf8"));
  assert.equal(copy.replacesBuiltIn, undefined);
  assert.equal(readThemes(dir).themes.mine.colors.accent, "#123456");
});

test("identity rings: each theme's own five, clear of every color with a meaning", () => {
  const all = themes();
  assert.deepEqual(ringColors(all.xenonet), ["#8fa8ff", "#ff8fb3", "#f2ead3", "#7ef0dd", "#b69bff"]);
  assert.equal(Math.round(colorDistance("#000000", "#ffffff")), 100);
  const meaningful = ["project", "satellite", "claude", "codex", "grok", "gold", "pressure", "attention", "danger", "collision", "stuck", "lineage"];
  for (const t of Object.values(all)) {
    const rings = ringColors(t);
    assert.deepEqual(rings, t.identities, `${t.id}: its own five, none swapped`);
    assert.equal(new Set(rings).size, 5, `${t.id}: five rings`);
    for (const r of rings) for (const k of meaningful) assert.ok(colorDistance(r, t.colors[k]) >= 20, `${t.id}: the ring ${r} is close to ${k} ${t.colors[k]}`);
    rings.forEach((a, i) => rings.slice(i + 1).forEach((b) => assert.ok(colorDistance(a, b) >= 15, `${t.id}: rings ${a} and ${b} look alike`)));
    for (const r of rings) assert.ok(contrast(r, t.colors.surface) >= 4.5, `${t.id}: the ring ${r} reads on the panels`);
  }
  // The first four keep their rings: Phosphor's questions are cream, and
  // Ochre's Codex is blue, so those two rings were moved over.
  assert.notEqual(ringColors(all.phosphor)[2], "#f2ead3");
  assert.notEqual(ringColors(all.ochre)[0], "#8fa8ff");
  assert.deepEqual(ringColors(all.phosphor).filter((_, i) => i !== 2), ringColors(all.xenonet).filter((_, i) => i !== 2), "the rest stay where they are");
  assert.deepEqual(all.gridline.identities, all.xenonet.identities);
});

test("identity rings in a theme of yours: inherited, five or none, and moved over where your colors give one a meaning", () => {
  const all = themes();
  const dir = tmp();
  fs.writeFileSync(path.join(dir, "moss.json"), JSON.stringify({ extends: "helios" }));
  fs.writeFileSync(path.join(dir, "pink.json"), JSON.stringify({ colors: { codex: "#ff8fb3" } }));
  fs.writeFileSync(path.join(dir, "few.json"), JSON.stringify({ extends: "rime", identities: ["#ff0000", "#00ff00"] }));
  fs.writeFileSync(path.join(dir, "mine.json"), JSON.stringify({ identities: ["#112233", "#ABC", "#ffeedd", "#00aa88", "#aa00aa"] }));
  const yours = readThemes(dir).themes;
  assert.deepEqual(ringColors(yours.moss), all.helios.identities, "from the theme it extends");
  assert.deepEqual(yours.few.identities, all.rime.identities, "five or none");
  assert.deepEqual(yours.mine.identities, ["#112233", "#aabbcc", "#ffeedd", "#00aa88", "#aa00aa"]);
  // Your Codex in the second ring's pink: that ring moves over, the rest stay.
  const rings = ringColors(yours.pink);
  assert.notEqual(rings[1], "#ff8fb3");
  assert.deepEqual(rings.filter((_, i) => i !== 1), all.xenonet.identities.filter((_, i) => i !== 1));
});

// Markup where a color goes: what a theme file shared with you might carry.
const MARKUP = `"><img src=x onerror=pwned()>`;

test("a theme starts only from a theme: a name every object has is none, so its fields are checked like any other", () => {
  const dir = tmp();
  const evil = { colors: { bg: MARKUP, accent: MARKUP }, sky: { night: [MARKUP, MARKUP, MARKUP, MARKUP] }, actors: { palette: [MARKUP] } };
  const names = ["toString", "constructor", "__proto__", "valueOf", "hasOwnProperty", "isPrototypeOf"];
  names.forEach((name, i) => fs.writeFileSync(path.join(dir, `evil-${i}.json`), JSON.stringify({ extends: name, ...evil })));
  // A file may be named like one too, and is then a theme of yours like any other.
  fs.writeFileSync(path.join(dir, "constructor.json"), JSON.stringify({ extends: "constructor", ...evil }));
  const { themes: all } = readThemes(dir);
  const xenonet = themes().xenonet;
  for (const id of [...names.map((_, i) => `evil-${i}`), "constructor"]) {
    const t = all[id];
    assert.deepEqual([t.colors.bg, t.colors.accent, t.sky.night, t.actors.palette], [xenonet.colors.bg, xenonet.colors.accent, xenonet.sky.night, xenonet.actors.palette], `${id} starts from XenoNet's, its markup refused`);
    assert.equal(t.builtIn, false);
  }
  assert.equal(all.constructor.replacesBuiltIn, undefined, "constructor isn't a built-in to replace");
  // Looking one up by id finds only themes.
  for (const name of names) assert.equal(themes()[name], undefined, `${name} isn't a theme`);
  assert.equal(readThemes(tmp()).themes.constructor, undefined);
});

test("a key every object has is never a theme's field, so nothing comes in through what a theme inherits", () => {
  const t = complete(JSON.parse(`{ "__proto__": { "name": "x" }, "colors": { "__proto__": { "bg": "#000000", "glow": 1 }, "accent": "#123456" }, "fonts": { "__proto__": { "body": "${MARKUP.replaceAll('"', '\\"')}" } }, "toString": "x" }`));
  assert.equal(Object.getPrototypeOf(t), Object.prototype);
  assert.equal(Object.getPrototypeOf(t.colors), Object.prototype);
  assert.equal(Object.getPrototypeOf(t.fonts), Object.prototype);
  assert.deepEqual([t.colors.accent, t.colors.glow, t.fonts.body, Object.hasOwn(t, "toString")], ["#123456", undefined, undefined, false]);
});

test("the theme route serves only themes, never what every object has", async () => {
  const dir = tmp();
  const route = themeRoutes({ dir, send: (res, status, body) => Object.assign(res, { status, body }), readJson: async () => ({}), quiet: true });
  const get = async (id) => {
    const res = {};
    await route({ method: "GET" }, res, new URL(`http://127.0.0.1/api/themes/${id}`));
    return res;
  };
  assert.deepEqual([(await get("xenonet")).status, (await get("xenonet")).body.id], [200, "xenonet"]);
  assert.equal((await get("constructor")).status, 404, "a name every object has");
});

test("a theme's fonts are lists of font names: one that would add a declaration to a style keeps the font underneath", () => {
  const all = themes();
  const sneaky = "Optima;position:fixed;inset:0;z-index:99999;background:url(http://127.0.0.1:9/seen)";
  const plain = complete({ fonts: { body: PLAIN } });
  for (const bad of [sneaky, "Optima}body{display:none", "serif<img src=x>", "a\\3b b", "x/*", "  "]) {
    const t = complete({ fonts: { display: bad, mono: bad, body: bad } });
    assert.deepEqual([t.fonts.display, t.fonts.mono, t.fonts.body], [all.xenonet.fonts.display, all.xenonet.fonts.mono, undefined], bad);
    assert.equal(conform(plain, { fonts: { body: bad } }).fonts.body, PLAIN, "what's underneath");
  }
  // Every built-in's, and names as people write them, quoted or not.
  for (const t of Object.values(all)) for (const f of [t.fonts.display, t.fonts.mono, t.fonts.body].filter(Boolean)) assert.equal(complete({ fonts: { display: f } }).fonts.display, f, `${t.id}: ${f}`);
  for (const f of ["Papyrus, fantasy", "'Fira Code', monospace", '"M+ 1p", sans-serif', "-apple-system, BlinkMacSystemFont", '"ヒラギノ角ゴ ProN", "Noto Sans CJK JP"', "Optima"]) assert.equal(complete({ fonts: { body: f } }).fonts.body, f);
});
