// End to end: A2A agents through the actor gateway, on XenoNet Dev's test
// instance, whose data is made up. The switch in the Actors panel is off
// until the window turns it on; a program without the window's key can't;
// once the gateway has an address the panel shows the Agent Card's; and
// Connect… on an actor gives an A2A agent the card's address and its token
// as a bearer. The protocol itself is tested in Rust
// (crates/xenonet-service/src/a2a). The window stays hidden;
// XENONET_E2E_SHOW=inactive shows it to watch.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";
import { limit } from "../harness/software.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });

const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

// A call from XenoNet's own page, with its key, as the Actors panel makes
// them.
const fromWindow = (code) =>
  s.eval(`return (async () => {
    const key = await window.xenonetDesktop.windowKey();
    const call = async (method, path, body) => {
      const r = await fetch(path, { method, headers: { "x-xenonet-window": key, ...(body ? { "content-type": "application/json" } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
      return { status: r.status, body: await r.json() };
    };
    ${code}
  })()`);
const click = (sel) => s.eval(`return (() => { const b = document.querySelector(${JSON.stringify(sel)}); if (!b || b.disabled) return false; b.click(); return true; })()`);
const text = (sel) => s.eval(`return document.querySelector(${JSON.stringify(sel)})?.innerText ?? null`);
const until = async (fn, ms = 8000) => {
  const end = Date.now() + ms;
  for (;;) {
    const got = await fn();
    if (got) return got;
    if (Date.now() > end) return got;
    await new Promise((r) => setTimeout(r, 150));
  }
};

before(async () => {
  // The gateway checks its address on this computer alone.
  process.env.XENONET_GATEWAY_PROBE = "local";
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: limit(180000) });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("A2A is off until the window turns it on; the card's address comes with the gateway's; Connect… gives an A2A agent the card and its token", { timeout: limit(90000) }, shoot("a2a", async () => {
  // Off, and only the window may look or change it.
  assert.equal((await fetch(`${s.base}/api/a2a`, { method: "POST", headers: { "content-type": "application/json", "x-xenonet-window": "not-its-key" }, body: JSON.stringify({ on: true }) })).status, 403);
  const before = await fromWindow(`return await call("GET", "/api/a2a");`);
  assert.deepEqual([before.status, before.body.on, before.body.card], [200, false, null]);
  await s.driver("command", "actors");
  assert.equal(await until(() => s.eval(`return document.querySelector("[data-ac-a2a]")?.dataset.state ?? null`)), "off");
  assert.match((await text("[data-ac-a2a]")) ?? "", /Let A2A agents reach XenoNet[\s\S]*Each A2A agent is an actor with its own token and scopes/);
  // On, from the panel: with no gateway address yet, no card's either.
  assert.equal(await click("[data-ac-a2a-on]"), true);
  assert.equal(await until(() => s.eval(`return document.querySelector("[data-ac-a2a]")?.dataset.state === "on" || null`)), true);
  assert.match((await text("[data-ac-a2a]")) ?? "", /comes with the gateway's/);
  assert.equal((await fromWindow(`return await call("GET", "/api/a2a");`)).body.on, true);
  // The gateway at an address of your own: the card is there.
  const set = await fromWindow(`return await call("POST", "/api/actors/gateway", { on: true, tunnel: { kind: "own", host: "https://XenoNet.Example.com/" } });`);
  assert.equal(set.status, 200);
  await s.input({ keys: "Escape" });
  await s.driver("command", "actors");
  const card = await until(() => text("[data-ac-a2a-card]"));
  assert.equal(card, "https://xenonet.example.com/.well-known/agent-card.json");
  // Connect… on an actor of your own, as an A2A agent: its token, masked,
  // and the card's address.
  const wren = await fromWindow(`return await call("POST", "/api/actors", { schema: "actor/registration/v1", name: "Wren", capabilities: ["message-agents"], heartbeatMs: 60000 });`);
  assert.equal(wren.status, 201);
  assert.equal(await until(() => click('[data-ac-gateway="actor:wren"]')), true);
  assert.equal(await until(() => click('[data-ac-gateway-make="actor:wren"][data-ac-kind="a2a"]')), true);
  assert.match((await until(() => text("[data-ac-token-text]"))) ?? "", /^xenonet_gateway_•+$/);
  assert.equal(await s.eval(`return /xenonet_gateway_[0-9a-f]{64}/.test(document.documentElement.outerHTML)`), false);
  assert.match((await text(".ac-steps")) ?? "", /Agent Card URL[\s\S]*https:\/\/xenonet\.example\.com\/\.well-known\/agent-card\.json[\s\S]*Authorization: Bearer <token>/);
  await click("[data-ac-done]");
  await s.input({ keys: "Escape" });
  // Off again; the gateway too.
  assert.equal((await fromWindow(`return await call("POST", "/api/a2a", { on: false });`)).body.on, false);
  await fromWindow(`return await call("POST", "/api/actors/gateway", { on: false, tunnel: null });`);
  await fromWindow(`return await call("DELETE", "/api/actors/wren");`);
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
