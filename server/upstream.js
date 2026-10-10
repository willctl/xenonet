// XenoNet Dev reading another XenoNet: with XENONET_UPSTREAM set (a URL, such as
// a forwarded port to the XenoNet on the computer the agents run on), the dev
// server serves the page from this checkout and passes its reads to that
// XenoNet, so a page in development shows real sessions. Only reads go
// through: anything that would change something there is refused, so a
// build in development can look but never act. The dev server's own routes
// (its health, the window's control and the page's error reports) stay here.
import http from "node:http";

const LOCAL = (route) => route === "/api/health" || route.startsWith("/api/dev/") || route === "/api/logs/page";
const READS = new Set(["GET", "HEAD"]);
// What the page's request says about itself, which the other XenoNet has no
// use for and might take as a page of its own.
const DROP = new Set(["host", "origin", "referer", "cookie", "x-xenonet-window", "authorization", "connection"]);

export function upstreamOf(value) {
  if (!value) return null;
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`XENONET_UPSTREAM isn't a URL: ${value}`);
  }
  if (url.protocol !== "http:") throw new Error("XENONET_UPSTREAM must be an http:// address, such as a forwarded port");
  if (!["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)) throw new Error("XENONET_UPSTREAM must be on this computer (127.0.0.1): forward the other XenoNet's port here, over SSH, rather than reach it across a network");
  return url;
}

export function createUpstream(url, { request = http.request } = {}) {
  return {
    url,
    // Whether this request is the other XenoNet's to answer.
    handles: (route) => route.startsWith("/api/") && !LOCAL(route),
    pass(req, res, target) {
      if (!READS.has(req.method)) {
        res.writeHead(403, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: `this XenoNet Dev reads ${url.host} and changes nothing there`, code: "read-only" }));
        return;
      }
      const headers = Object.fromEntries(Object.entries(req.headers).filter(([k]) => !DROP.has(k)));
      const out = request({ host: url.hostname.replace(/^\[|\]$/g, ""), port: url.port || 80, method: req.method, path: target.pathname + target.search, headers: { ...headers, host: url.host } }, (up) => {
        // XenoNet answers only to its own address (against DNS rebinding), so
        // a port forwarded to a different number here is turned away.
        if (up.statusCode === 421) {
          up.resume();
          res.writeHead(502, { "content-type": "application/json" });
          res.end(JSON.stringify({ error: `${url.host} turned the request away: forward the other XenoNet's port to the same port here (47840 to 47840), since XenoNet answers only to its own address`, code: "upstream-port" }));
          return;
        }
        res.writeHead(up.statusCode ?? 502, up.headers);
        up.pipe(res);
      });
      out.on("error", (err) => {
        if (res.headersSent) return res.destroy();
        res.writeHead(502, { "content-type": "application/json" });
        res.end(JSON.stringify({ error: `${url.host} didn't answer: ${err.message}`, code: "upstream" }));
      });
      // The page going away ends the read there too (an event stream, say).
      res.on("close", () => out.destroy());
      out.end();
    },
  };
}
