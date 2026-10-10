// Your own themes: JSON files in the themes folder of XenoNet's state folder,
// one per theme (<id>.json), with the fields of a theme in core/themes.js
// and "extends" naming the theme they start from. A file named for a
// built-in theme takes its place (see themes()). Read fresh each time, so
// an edit shows on the next look; a file that isn't valid JSON is skipped
// and reported.
import fs from "node:fs";
import path from "node:path";
import { themes, DEFAULT_THEME } from "./themes.js";

const ID = /^[a-z0-9][a-z0-9-]{0,40}$/;

export function readThemes(dir) {
  const custom = {};
  const problems = [];
  let files = [];
  try {
    files = fs.readdirSync(dir).filter((f) => f.endsWith(".json"));
  } catch {}
  for (const f of files) {
    const id = f.slice(0, -5).toLowerCase();
    if (!ID.test(id)) {
      problems.push(`${f}: a theme's file name is its id: lowercase letters, digits and dashes`);
      continue;
    }
    try {
      const t = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
      if (!t || typeof t !== "object" || Array.isArray(t)) throw new Error("a theme is a JSON object of fields");
      custom[id] = t;
    } catch (err) {
      problems.push(`${f}: ${err.message}`);
    }
  }
  return { themes: themes(custom), problems };
}

// A theme of your own to start from: `from` with nothing changed yet, saved
// as <id>.json. Returns its file.
export function copyTheme(dir, from, id, name) {
  if (!ID.test(id)) throw new Error("a theme's id is lowercase letters, digits and dashes");
  const file = path.join(dir, `${id}.json`);
  if (fs.existsSync(file)) throw new Error(`there's already a theme called ${id}`);
  const base = readThemes(dir).themes[from] ?? readThemes(dir).themes[DEFAULT_THEME];
  const { id: _id, builtIn: _b, replacesBuiltIn: _r, extends: _e, name: _n, about: _a, ...fields } = base;
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(file, JSON.stringify({ name: name ?? id, about: `Starts from ${base.name}`, extends: base.builtIn ? from : DEFAULT_THEME, ...fields }, null, 2) + "\n");
  return file;
}
