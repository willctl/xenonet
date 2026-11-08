// The console page as `npm run build` leaves it in dist/, which a packaged
// XenoNet serves. (XenoNet Dev serves it through vite instead; see index.js.)
import fs from "node:fs";
import path from "node:path";

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".woff": "font/woff", ".png": "image/png", ".ico": "image/x-icon", ".json": "application/json" };

// The page's Content-Security-Policy, so a slip in escaping what the page
// writes can't run script: scripts come only from XenoNet's own server, and
// nothing inline or evaluated runs. WebAssembly may compile, which the
// terminal's image decoding does (it evaluates no script). Styles may be
// inline, since the page sets colors in style attributes and Vite's dev
// server adds stylesheets as style elements; images may be data: and blob:
// URLs, which the poster and the terminal make, and fonts data: URLs, which
// a build may inline. The page can't be framed, has no forms, and connects
// only to XenoNet's own server (its WebSocket, in XenoNet Dev, included).
// vite.config.js sends the same for XenoNet Dev.
export const CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  // Vite's reconnect monitor is a locally created SharedWorker.
  "worker-src 'self' blob:",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self' data:",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'none'",
].join("; ");
export const PAGE_HEADERS = { "content-security-policy": CSP };

// The file a request's path names in `dist`, with its type, and for the
// page its headers. A path that isn't a file there gets the page, which
// routes for itself; otherwise { status, error }.
export function staticFile(dist, pathname) {
  let rel;
  try {
    rel = decodeURIComponent(pathname);
  } catch {
    return { status: 400, error: "bad path" };
  }
  let file = path.normalize(path.join(dist, rel));
  // Inside dist, not in a folder beside it whose name starts the same.
  if (file !== dist && !file.startsWith(dist + path.sep)) return { status: 403, error: "forbidden" };
  if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(dist, "index.html");
  if (!fs.existsSync(file)) return { status: 503, error: "Run `npm run build` first, or use `npm run dev`." };
  const type = MIME[path.extname(file)] ?? "application/octet-stream";
  return { file, type, ...(type === "text/html" ? { headers: PAGE_HEADERS } : {}) };
}
