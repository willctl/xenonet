// The client the xenonet command and the MCP servers reach a running XenoNet
// with (core/client.js), against a stand-in server.
import { test } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { candidates, find, need, nameOf, get, post, del, PORTS, HEALTH_MS, NAMED_HEALTH_MS, healthMs } from "../core/client.js";

test("which ports to try: the one named, the instance named, XENONET_PORT, or XenoNet then XenoNet Dev", () => {
  assert.deepEqual(candidates({ port: "5000" }), [5000]);
  assert.deepEqual(candidates({ demo: true }), [PORTS.demo]);
  assert.deepEqual(candidates({ dev: true }), [PORTS.dev]);
  assert.deepEqual(candidates({ env: { XENONET_PORT: "6000" } }), [6000]);
  assert.deepEqual(candidates({ env: {} }), [PORTS.xenonet, PORTS.dev]);
  assert.equal(nameOf({ port: PORTS.dev }), "XenoNet Dev");
  assert.equal(nameOf({ port: PORTS.demo }), "XenoNet (demo)");
  assert.equal(nameOf({ port: PORTS.xenonet }), "XenoNet");
  assert.equal(nameOf({ port: 1, health: { name: "XenoNet Dev (test)" } }), "XenoNet Dev (test)");
});

test("how long a XenoNet has to answer: a quick probe when looking for one, longer for the one named", () => {
  assert.ok(NAMED_HEALTH_MS > HEALTH_MS);
  assert.equal(healthMs({ env: {} }), HEALTH_MS);
  for (const opts of [{ port: "5000" }, { demo: true }, { dev: true }, { env: { XENONET_PORT: "6000" } }]) assert.equal(healthMs(opts), NAMED_HEALTH_MS, JSON.stringify(opts));
});

test("calls to a running XenoNet, its errors, and one that stops answering", async () => {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      seen.push([req.method, req.url, req.headers["content-type"] ?? null, body]);
      const json = (status, data) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
      if (req.url === "/api/health") return json(200, { ok: true, name: "Stand-in" });
      if (req.url === "/api/broken") return json(500, {});
      if (req.url === "/api/refused") return json(403, { error: "an agent can't do that" });
      if (req.url === "/api/not-json") return res.writeHead(200).end("<html>");
      json(200, { ok: true });
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  const s = await find({ port });
  assert.equal((await need({ port })).port, port);
  assert.deepEqual([s.port, s.base, s.health.name], [port, `http://127.0.0.1:${port}`, "Stand-in"]);
  assert.deepEqual(await get(s, "/api/x"), { ok: true });
  assert.deepEqual(await post(s, "/api/x", { a: 1 }, { "x-xenonet-agent": "claude:x" }), { ok: true });
  assert.deepEqual(seen.at(-1), ["POST", "/api/x", "application/json", '{"a":1}']);
  assert.deepEqual(await del(s, "/api/x"), { ok: true });
  assert.deepEqual(await get(s, "/api/not-json"), {}, "an answer that isn't JSON reads as empty");
  await assert.rejects(get(s, "/api/refused"), (err) => err.status === 403 && err.message === "an agent can't do that");
  await assert.rejects(get(s, "/api/broken"), /\/api\/broken: 500 Internal Server Error/);
  await new Promise((r) => server.close(r));
  await assert.rejects(get(s, "/api/x"), new RegExp(`Stand-in stopped answering on 127.0.0.1:${port}`));
  assert.equal(await find({ port }), null);
  await assert.rejects(need({ port }), new RegExp(`XenoNet isn't running \\(nothing answers on 127.0.0.1:${port}\\)`));
});

test("a XenoNet that's slow to answer is still found, and one that never answers is given up on", async () => {
  // Slower than the 600 ms the client used to wait, which a busy machine
  // (or a XenoNet starting up) can take.
  let hang = false;
  const server = http.createServer((req, res) => {
    if (hang) return;
    setTimeout(() => res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify({ ok: true, name: "Slow" })), 900);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const port = server.address().port;
  try {
    assert.ok(HEALTH_MS >= 900, "long enough for it");
    assert.equal((await find({ port })).health.name, "Slow");
    hang = true;
    const started = Date.now();
    assert.equal(await find({ port }, { answerMs: 300 }), null);
    assert.ok(Date.now() - started < 3000, "not waited on past the time given");
  } finally {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
});
