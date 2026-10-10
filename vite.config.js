import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, searchForWorkspaceRoot } from "vite";
import { PAGE_HEADERS } from "./server/static.js";

const here = path.dirname(fileURLToPath(import.meta.url));
// Agents' git worktrees (.claude/worktrees) link node_modules to the main
// checkout's, outside the folder the dev server serves from, so it would
// refuse the fonts there and draw every label in a fallback.
const modules = (() => {
  try {
    return fs.realpathSync(path.join(here, "node_modules"));
  } catch {
    return null;
  }
})();

export default defineConfig({
  build: { target: "es2022", chunkSizeWarningLimit: 2000 },
  // Prebundled dependencies stay in this checkout's .xenonet/, which git
  // ignores, not in node_modules/.vite: worktrees link node_modules to the
  // main checkout's, so every worktree's dev server (the ones npm test
  // starts among them) would rebuild the one cache under XenoNet Dev's page.
  cacheDir: path.join(here, ".xenonet", "vite"),
  // Transcripts and project data are served by server/index.js, not Vite.
  // Build output and captures stay unwatched: on Windows a watched folder
  // can't be renamed, which breaks electron-builder. So do agents' git
  // worktrees in .claude/, where any .html edit would reload this page.
  server: {
    host: "127.0.0.1",
    // The page's Content-Security-Policy (server/static.js), as a packaged
    // XenoNet sends it, so XenoNet Dev runs under the same rules.
    headers: PAGE_HEADERS,
    watch: { ignored: ["**/release/**", "**/dist/**", "**/.xenonet/**", "**/build/**", "**/.claude/**"] },
    fs: { allow: [searchForWorkspaceRoot(here), ...(modules ? [modules] : [])] },
  },
});
