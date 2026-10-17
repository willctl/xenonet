// core/themefiles.js in Rust (crates/xenonet-app/src/themefiles.rs): your
// own themes, JSON files in the themes folder of XenoNet's state folder,
// read fresh each time; a file that isn't a JSON object is reported.
import { native } from "../index.js";
import { settle } from "../settle.js";

export function readThemes(dir) {
  const { themes, problems } = native.appReadThemes(String(dir));
  return { themes: Object.assign(Object.create(null), themes), problems };
}

export const copyTheme = (dir, from, id, name) => settle(native.appCopyTheme(String(dir), from == null ? null : String(from), String(id), name));
