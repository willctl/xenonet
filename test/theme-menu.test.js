// The Theme menu in the tray and the macOS menu bar: the page tells the
// desktop app its themes and the one in use (src/themes-ui.js), the app
// checks what it's told (electron/theme-menu.js), and the menu's items run
// the page's own palette command for a theme. Made-up themes.
import { test } from "node:test";
import assert from "node:assert/strict";
import { themeMenuOf, themeItems } from "../electron/theme-menu.js";
import { createThemes } from "../src/themes-ui.js";

const said = {
  current: "slate",
  themes: [
    { id: "xenonet", name: "XenoNet", command: "theme-xenonet" },
    { id: "slate", name: "Slate", command: "theme-slate" },
    { id: "gallery", name: "My gallery", command: "theme-use-gallery" },
  ],
};

test("the desktop app takes the page's themes as they are, and only what fits a theme", () => {
  assert.deepEqual(themeMenuOf(said), said);
  const odd = themeMenuOf({
    current: "nowhere",
    themes: [
      { id: "moss", name: "  Moss\n  and   stone  ", command: "theme-moss" },
      { id: "moss", name: "Moss again", command: "theme-moss" },
      { id: "Bad Id", name: "x", command: "theme-bad" },
      { id: "sly", name: "Sly", command: "quit" },
      { id: "long", name: "L".repeat(200), command: "theme-long" },
      { id: "blank", name: "", command: "theme-blank" },
      null,
      "theme-x",
    ],
  });
  assert.deepEqual(odd.themes.map((t) => [t.id, t.name.length > 60 ? "long" : t.name, t.command]), [["moss", "Moss and stone", "theme-moss"], ["long", "L".repeat(60), "theme-long"], ["blank", "blank", "theme-blank"]]);
  assert.equal(odd.current, null, "a theme in use that isn't listed checks nothing");
  for (const junk of [null, "themes", { themes: "all" }, { themes: [] }, { themes: [{ id: "x", command: "rm -rf" }] }]) assert.equal(themeMenuOf(junk), null);
  assert.equal(themeMenuOf({ themes: Array.from({ length: 100 }, (_, i) => ({ id: `t${i}`, name: `T${i}`, command: `theme-t${i}` })) }).themes.length, 64, "at most 64");
});

test("the Theme submenu: a radio item for each theme, the one in use checked, a click picking it", () => {
  const picked = [];
  const items = themeItems(said, (t) => picked.push(t.command));
  assert.deepEqual(items.map((i) => [i.label, i.type, i.checked]), [["XenoNet", "radio", false], ["Slate", "radio", true], ["My gallery", "radio", false]]);
  items[2].click();
  assert.deepEqual(picked, ["theme-use-gallery"]);
  const amp = { current: null, themes: [{ id: "salt", name: "Salt & Pepper", command: "theme-salt" }] };
  assert.equal(themeItems(amp, () => {}, { mnemonics: true })[0].label, "Salt && Pepper", "an & shows as one on Windows and Linux");
  assert.equal(themeItems(amp, () => {})[0].label, "Salt & Pepper");
  assert.deepEqual(themeItems(null, () => {}), []);
});

test("the page tells the desktop app its themes whenever they load, with the palette's own command for each", async () => {
  const told = [];
  let commands = [];
  const list = { current: "slate", themes: [{ id: "xenonet", name: "XenoNet", about: "", builtIn: true }, { id: "slate", name: "Slate", about: "", builtIn: true }, { id: "gallery", name: "My gallery", about: "", builtIn: false }] };
  const fetched = globalThis.fetch;
  globalThis.fetch = async () => ({ ok: true, json: async () => list });
  try {
    const themes = createThemes({ post: async () => ({ ok: false, statusText: "no", json: async () => ({ error: "not yours" }) }), sfx() {}, toast() {}, note() {}, desktop: { setTheme() {}, setThemes: (m) => told.push(m) }, onCommands: (c) => (commands = c) });
    await themes.load();
    assert.deepEqual(told.at(-1), said, "every theme, its name and its command, and the one in use");
    for (const t of told.at(-1).themes) assert.ok(commands.some((c) => c.id === t.command), `${t.command} is a command of the palette's`);
    assert.deepEqual(themeMenuOf(told.at(-1)), told.at(-1), "and the desktop app keeps it whole");
    // A switch that didn't go through tells the menus again what's in use.
    await assert.rejects(themes.set("xenonet", { log: false }));
    assert.equal(told.length, 2);
    assert.equal(told.at(-1).current, "slate");
  } finally {
    globalThis.fetch = fetched;
  }
});
