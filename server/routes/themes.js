// Themes by id (core/themes.js, and yours in the state folder's themes/,
// which may stand in for a built-in): GET /api/themes/<id> returns one
// whole, as the console draws it, for looking at a theme before switching
// to it (the switcher, the gallery); POST /api/themes/reveal {id} shows a
// theme of yours in the file manager, to edit (not in the demo, which
// leaves your other apps alone). The list, the one in use, switching and
// copying are in server/index.js.
import fs from "node:fs";
import path from "node:path";
import { readThemes } from "../../core/themefiles.js";
import { reveal as revealFile } from "../../core/open.js";

const ID = /^[a-z0-9][a-z0-9-]{0,40}$/;

export function themeRoutes({ dir, send, readJson, reveal = revealFile, quiet = false }) {
  // Handles the request and returns true, or returns false for another route.
  return async function route(req, res, url) {
    if (req.method === "POST" && url.pathname === "/api/themes/reveal") {
      // Only a file in the themes folder, named by its id: never a path
      // from the page.
      const { id } = await readJson(req).catch(() => ({}));
      const file = ID.test(id ?? "") ? path.join(dir, `${id}.json`) : null;
      if (!file || !fs.existsSync(file)) send(res, 404, { error: `there's no theme file called ${id}` });
      else {
        if (!quiet) reveal(file);
        send(res, 200, { ok: true, file, shown: !quiet });
      }
      return true;
    }
    const m = /^\/api\/themes\/([a-z0-9][a-z0-9-]{0,40})$/.exec(url.pathname);
    if (req.method !== "GET" || !m) return false;
    const theme = readThemes(dir).themes[m[1]];
    send(res, theme ? 200 : 404, theme ?? { error: `there's no theme called ${m[1]}` });
    return true;
  };
}
