// xenonet theme: the console's themes (core/themes.js, and yours in XenoNet's
// state folder), which one is in use, and switching or copying one.
import os from "node:os";
import { c, print, json, table, Fail } from "../lib/out.js";
import { need, get, post } from "../../core/client.js";
import { tilde } from "../../core/paths.js";

export const help = `The console's themes: every color, the fonts, the sky, the floor grid, the
glow and the music. A change shows in every XenoNet window at once.

  xenonet theme                     the themes, and which is in use
  xenonet theme helios              use one
  xenonet theme copy ochre mine     start a theme of your own from one, as a
                                  JSON file to edit; XenoNet follows each save
  --json                          the list as JSON`;

// A swatch of a theme's colors, in terminals that show true color.
const swatch = (colors) => (c.bold("") ? colors.map((hex) => `\x1b[48;2;${[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(";")}m  \x1b[0m`).join("") : "");

export async function run([first, ...rest], flags) {
  const s = await need(flags);
  if (first === "copy") {
    const [from, id] = rest;
    if (!from || !id) throw new Fail("say which theme to copy and what to call yours: xenonet theme copy <theme> <id>", 2);
    const { file } = await post(s, "/api/themes/copy", { from, id });
    if (flags.json) return json({ file });
    print(`${c.green("✓")} ${tilde(file, os.homedir())}`);
    print(c.dim(`  Edit it, then xenonet theme ${id} to use it; XenoNet follows each save.`));
    return;
  }
  if (first) {
    const t = await post(s, "/api/theme", { id: first });
    if (flags.json) return json({ id: t.id, name: t.name });
    return print(`${c.green("✓")} ${t.name} ${c.dim(`· ${t.about ?? ""}`)}`);
  }
  const list = await get(s, "/api/themes");
  if (flags.json) return json(list);
  print(table(list.themes.map((t) => [t.id === list.current ? c.green("●") : " ", swatch(t.swatch), t.id, c.dim(t.about + (t.builtIn ? "" : t.replacesBuiltIn ? " (yours, in place of the built-in)" : " (yours)"))]), { wrap: true }));
  print(c.dim(`\nYours go in ${tilde(list.folder, os.homedir())}; xenonet theme copy starts one.`));
  for (const p of list.problems) print(c.yellow(`! ${p}`));
}
