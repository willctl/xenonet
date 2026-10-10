// The gallery's cards (src/theme-gallery.js): each theme drawn in its own
// colors, fonts and actor form. A theme comes from the server whole, but a
// theme file is anyone's to write or share, so a card takes nothing on
// trust: made-up themes carrying markup and absurd values here must come
// out as a card like any other.
import { test } from "node:test";
import assert from "node:assert/strict";
import { register } from "node:module";
import { themes } from "../core/themes.js";

// The gallery imports its stylesheet, which only Vite loads: here it's empty.
register("data:text/javascript," + encodeURIComponent(`export async function load(url, context, next) { return url.endsWith(".css") ? { format: "module", source: "", shortCircuit: true } : next(url, context); }`));
const { card } = await import("../src/theme-gallery.js");

const MARKUP = `x" onmouseover="pwned()"><img src=x onerror=pwned()>`;
const xenonet = themes().xenonet;

// No element or attribute of the theme's own, and nothing of it in a style.
function clean(html) {
  assert.doesNotMatch(html, /<img|onmouseover="/, "the theme's markup came out as markup");
  for (const [, style] of html.matchAll(/style="([^"]*)"/g)) assert.doesNotMatch(style, /pwned/, `the theme reached a style: ${style}`);
}

test("a card draws a theme's colors only as colors, whatever the theme sent", () => {
  const evil = {
    id: "evil",
    name: "Evil",
    about: "made up",
    builtIn: false,
    colors: Object.fromEntries(Object.keys(xenonet.colors).map((k) => [k, MARKUP])),
    sky: { night: [MARKUP, MARKUP, MARKUP, MARKUP], dawn: xenonet.sky.dawn, day: xenonet.sky.day, dusk: xenonet.sky.dusk },
    actors: { ...xenonet.actors, palette: [MARKUP, "#123456"] },
    fonts: xenonet.fonts,
  };
  const html = card(evil, { inUse: false, focused: true });
  assert.match(html, /data-id="evil"/);
  clean(html);
  assert.ok(html.includes(`--tg-bg:${xenonet.colors.bg}`), "a color that isn't one is XenoNet's");
});

test("a card's fonts can't add declarations to its styles", () => {
  const sneaky = "Optima;position:fixed;inset:0;z-index:99999;background:url(http://127.0.0.1:9/pwned)";
  const html = card({ ...xenonet, id: "sneaky", builtIn: false, fonts: { display: sneaky, mono: sneaky, body: sneaky } }, { inUse: false, focused: false });
  clean(html);
  assert.ok(html.includes(`font-family:${xenonet.fonts.display.replaceAll('"', "&quot;")}`), "the card is in XenoNet's font instead");
});

test("a card draws as many rings round an orb as the map does, however many a theme asks for", () => {
  const rings = (params) => card({ ...xenonet, id: "orbs", actors: { ...xenonet.actors, form: "orb", params } }, { inUse: false, focused: false }).match(/<ellipse /g)?.length ?? 0;
  // Two agents in the sketch, each ringed.
  assert.equal(rings({ rings: 1e5 }), 8, "four each at most");
  assert.equal(rings({ rings: -3 }), 0);
  assert.equal(rings({ rings: 2 }), 4);
  assert.equal(rings({}), 2, "one each by default");
  assert.equal(rings({ eclipse: true }), 0, "none round an eclipse unless asked, as on the map");
});
