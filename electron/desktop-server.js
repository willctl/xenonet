// Desktop serves its own frontend and streams API requests to the service.
// Keeping one origin preserves browser storage and the existing page API.
//
// The service takes a request from this process as one from XenoNet's window
// (core/engine/verify.js, desktop), so only the page's own requests go
// through: Desktop adds `secret` to them as they leave its window
// (pageHeaders), and an API request without it is refused. Other programs
// talk to the service directly, where it sees who they are.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { staticFile } from "../server/static.js";

export const PAGE_HEADER = "x-xenonet-desktop";

const same = (a, b) => typeof a === "string" && a.length === b.length && crypto.timingSafeEqual(Buffer.from(a), Buffer.from(b));

// `dev` (XenoNet Dev, from a checkout): the page comes from Vite, which
// reloads it as src/ is edited, rather than from dist/; `dev.hot` false
// keeps it as it loaded (XenoNet Dev's test instance).
export async function desktopServer({ root, port, service, secret, dev = null }) {
  if (!/^[a-f0-9]{64}$/.test(secret ?? "")) throw new Error("Desktop's frontend needs a secret for its page.");
  const base = `http://127.0.0.1:${port}`;
  let vite = null;
  const server = http.createServer((req, res) => {
    const refuse = (status) => { res.writeHead(status); res.end(); };
    if (req.headers.host !== `127.0.0.1:${port}`) return refuse(421);
    if (req.headers.origin && req.headers.origin !== base) return refuse(403);
    const url = new URL(req.url, base);
    if (url.pathname.startsWith("/api/")) {
      if (!same(req.headers[PAGE_HEADER], secret)) return refuse(403);
      const target = new URL(url.pathname + url.search, service);
      const { [PAGE_HEADER]: _, ...rest } = req.headers;
      const headers = { ...rest, host: target.host, ...(req.headers.origin ? { origin: target.origin } : {}) };
      const upstream = http.request(target, { method: req.method, headers }, (reply) => {
        res.writeHead(reply.statusCode, reply.headers); reply.pipe(res);
      });
      upstream.on("error", () => { if (!res.headersSent) res.writeHead(503, { "content-type": "application/json" }); res.end(JSON.stringify({ error: "XenoNet Service is restarting." })); });
      res.on("close", () => upstream.destroy());
      req.pipe(upstream);
      return;
    }
    if (vite) return vite.middlewares(req, res);
    const found = staticFile(path.join(root, "dist"), url.pathname);
    if (found.status) return refuse(found.status);
    res.writeHead(200, { "content-type": found.type, ...found.headers });
    fs.createReadStream(found.file).pipe(res);
  });
  if (dev) {
    const { createServer } = await import("vite");
    vite = await createServer({ root, server: { middlewareMode: true, ws: { server }, ...(dev.hot === false ? { hmr: false, watch: null } : {}) }, appType: "spa" });
  }
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  return { base, close: () => { vite?.close(); server.closeAllConnections(); server.close(); } };
}

// Adds the secret to the window's API requests to `base`, in the session
// the window uses (Electron keeps one such listener per session).
export function pageHeaders(session, { base, secret }) {
  session.webRequest.onBeforeSendHeaders((details, done) => {
    if (details.url.startsWith(`${base}/api/`)) details.requestHeaders[PAGE_HEADER] = secret;
    done({ requestHeaders: details.requestHeaders });
  });
}
