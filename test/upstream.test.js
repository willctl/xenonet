// XenoNet Dev reading another XenoNet (server/upstream.js): only an address on
// this computer, reads passed on without the page's own headers, an event
// stream passed as it comes, anything else refused before it gets there,
// and the dev server's own routes left to it. The other XenoNet is made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { createUpstream, upstreamOf } from "../server/upstream.js";

const listen = (handler) =>
  new Promise((resolve) => {
    const s = http.createServer(handler).listen(0, "127.0.0.1", () => resolve(s));
  });

test("only an http address on this computer: another machine is reached by forwarding its port here", () => {
  assert.equal(upstreamOf(""), null);
  assert.equal(upstreamOf("http://127.0.0.1:47850").port, "47850");
  assert.equal(upstreamOf("http://localhost:47850").hostname, "localhost");
  assert.throws(() => upstreamOf("not a url"), /isn't a URL/);
  assert.throws(() => upstreamOf("https://127.0.0.1:47850"), /http:\/\//);
  assert.throws(() => upstreamOf("http://192.0.2.10:47840"), /on this computer/);
});

test("reads go to the other XenoNet without the page's own headers; changes are refused before they get there", async () => {
  const seen = [];
  const other = await listen((req, res) => {
    seen.push({ method: req.method, url: req.url, headers: req.headers });
    if (req.url === "/api/events") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.write("event: live\ndata: {\"n\":1}\n\n");
      setTimeout(() => res.end("event: live\ndata: {\"n\":2}\n\n"), 50);
      return;
    }
    res.writeHead(200, { "content-type": "application/json", "x-from": "other" });
    res.end(JSON.stringify({ sessions: 3 }));
  });
  const up = createUpstream(upstreamOf(`http://127.0.0.1:${other.address().port}`));
  const dev = await listen((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    if (up.handles(url.pathname)) return up.pass(req, res, url);
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ local: url.pathname }));
  });
  const base = `http://127.0.0.1:${dev.address().port}`;
  try {
    const r = await fetch(`${base}/api/state?scope=7d`, { headers: { origin: base, cookie: "a=b", "x-xenonet-window": "the-window-key", "x-xenonet-api-version": "1" } });
    assert.deepEqual([r.status, r.headers.get("x-from"), await r.json()], [200, "other", { sessions: 3 }]);
    const got = seen.find((x) => x.url === "/api/state?scope=7d");
    assert.equal(got.headers.host, `127.0.0.1:${other.address().port}`);
    for (const h of ["origin", "cookie", "x-xenonet-window"]) assert.equal(got.headers[h], undefined, `${h} isn't passed on`);
    assert.equal(got.headers["x-xenonet-api-version"], "1", "the API version is");

    const events = await (await fetch(`${base}/api/events`)).text();
    assert.match(events, /data: \{"n":1\}[\s\S]*data: \{"n":2\}/, "the event stream, as it comes");

    const before = seen.length;
    for (const method of ["POST", "PUT", "DELETE"]) {
      const w = await fetch(`${base}/api/acks`, { method, body: method === "DELETE" ? undefined : "{}" });
      assert.equal(w.status, 403);
      assert.equal((await w.json()).code, "read-only");
    }
    assert.equal(seen.length, before, "nothing reached the other XenoNet");

    for (const route of ["/api/health", "/api/dev/control", "/api/logs/page"]) assert.deepEqual(await (await fetch(`${base}${route}`)).json(), { local: route }, `${route} stays here`);
    assert.equal(up.handles("/src/main.js"), false, "the page itself comes from this checkout");
  } finally {
    dev.close();
    other.close();
  }
});

test("a XenoNet that turns the address away (a port forwarded to another number) is a 502 that says how to forward it", async () => {
  const other = await listen((req, res) => {
    res.writeHead(421, { "content-type": "application/json" });
    res.end(JSON.stringify({ error: "bad host" }));
  });
  const up = createUpstream(upstreamOf(`http://127.0.0.1:${other.address().port}`));
  const dev = await listen((req, res) => up.pass(req, res, new URL(req.url, "http://127.0.0.1")));
  try {
    const r = await fetch(`http://127.0.0.1:${dev.address().port}/api/live`);
    assert.equal(r.status, 502);
    const body = await r.json();
    assert.equal(body.code, "upstream-port");
    assert.match(body.error, /same port/);
  } finally {
    dev.close();
    other.close();
  }
});

test("another XenoNet that doesn't answer is a 502 that says so", async () => {
  const gone = await listen(() => {});
  const port = gone.address().port;
  await new Promise((r) => gone.close(r));
  const up = createUpstream(upstreamOf(`http://127.0.0.1:${port}`));
  const dev = await listen((req, res) => up.pass(req, res, new URL(req.url, "http://127.0.0.1")));
  try {
    const r = await fetch(`http://127.0.0.1:${dev.address().port}/api/state`);
    assert.equal(r.status, 502);
    assert.match((await r.json()).error, /didn't answer/);
  } finally {
    dev.close();
  }
});
