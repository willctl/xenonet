// Desktop's frontend (electron/desktop-server.js): it serves the page and
// passes the page's own API requests to the service, which takes them for
// the window's; a request without the page's secret never reaches it.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { desktopServer, pageHeaders, PAGE_HEADER } from "../electron/desktop-server.js";

const freePort = () => new Promise((r) => { const s = net.createServer().listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => r(port)); }); });
const SECRET = "a".repeat(64);

test("Desktop passes on only its page's API requests, without the secret, and serves the page to anyone", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-desktop-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "dist"));
  fs.writeFileSync(path.join(root, "dist", "index.html"), "<!doctype html><title>XenoNet</title>");
  const seen = [];
  const service = http.createServer((req, res) => { seen.push({ url: req.url, headers: req.headers }); res.end(JSON.stringify({ ok: true })); });
  await new Promise((r) => service.listen(0, "127.0.0.1", r));
  t.after(() => service.close());
  const desktop = await desktopServer({ root, port: await freePort(), service: `http://127.0.0.1:${service.address().port}`, secret: SECRET });
  t.after(() => desktop.close());

  for (const headers of [{}, { [PAGE_HEADER]: "b".repeat(64) }, { [PAGE_HEADER]: "short" }]) {
    const r = await fetch(`${desktop.base}/api/engine/connect`, { method: "DELETE", headers });
    assert.equal(r.status, 403, JSON.stringify(headers));
  }
  assert.deepEqual(seen, [], "nothing without the page's secret reaches the service");

  const r = await fetch(`${desktop.base}/api/actors/you?x=1`, { headers: { [PAGE_HEADER]: SECRET, "x-xenonet-window": "key" } });
  assert.equal(r.status, 200);
  assert.equal(seen.length, 1);
  assert.equal(seen[0].url, "/api/actors/you?x=1");
  assert.equal(seen[0].headers["x-xenonet-window"], "key");
  assert.equal(seen[0].headers[PAGE_HEADER], undefined, "the secret stays with Desktop");

  assert.equal((await fetch(`${desktop.base}/`)).status, 200, "the page itself needs no secret");
  await assert.rejects(desktopServer({ root, port: await freePort(), service: "http://127.0.0.1:1", secret: "" }), /secret/);
});

test("the secret goes only on the window's API requests to Desktop", () => {
  let listener;
  pageHeaders({ webRequest: { onBeforeSendHeaders: (fn) => (listener = fn) } }, { base: "http://127.0.0.1:48840", secret: SECRET });
  const send = (url) => { let out; listener({ url, requestHeaders: { accept: "*/*" } }, (r) => (out = r.requestHeaders)); return out; };
  assert.equal(send("http://127.0.0.1:48840/api/summary")[PAGE_HEADER], SECRET);
  for (const url of ["http://127.0.0.1:48840/assets/index.js", "http://127.0.0.1:47840/api/summary", "https://github.com/willctl/xenonet/releases/latest"]) {
    assert.equal(send(url)[PAGE_HEADER], undefined, url);
  }
});
