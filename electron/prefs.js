// Small per-machine answers the desktop shell remembers (desktop.json in
// userData): hints already shown, questions already asked.
import { app } from "electron";
import fs from "node:fs";
import path from "node:path";

export function prefs() {
  const file = path.join(app.getPath("userData"), "desktop.json");
  let data = {};
  try {
    data = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {}
  return {
    get: (key) => data[key],
    set(key, value) {
      data[key] = value;
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, JSON.stringify(data, null, 2));
      } catch {}
    },
  };
}
