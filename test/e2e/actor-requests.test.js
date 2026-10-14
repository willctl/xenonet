// End to end: registrations wait for XenoNet's window (core/actors/
// registration.js, server/routes/actors.js) on XenoNet Dev's test instance,
// whose data is made up. The demo's made-up requests (simulate
// actor-request) are allowed and denied from the page itself, with the
// window key only it holds, as the Actors panel will; a program without the
// key can't see or answer them; and tokens stop working when they're
// denied, replaced or their actor removed. The window stays hidden;
// XENONET_E2E_SHOW=inactive shows it to watch.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });

// Runs a test body, and on failure saves what the window looked like.
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
// them: `code` is the body of an async function with call(method, path,
// body) in scope.
const fromWindow = (code) =>
  s.eval(`return (async () => {
    const key = await window.xenonetDesktop.windowKey();
    const call = async (method, path, body) => {
      const r = await fetch(path, { method, headers: { "x-xenonet-window": key, ...(body ? { "content-type": "application/json" } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
      return { status: r.status, body: await r.json() };
    };
    ${code}
  })()`);

// An actor's own call, from this test's process: XenoNet knows the token or
// says why not before it looks at the process.
const asActor = async (token, route = "/api/actors/me") => {
  const r = await fetch(`${s.base}${route}`, { headers: { authorization: `Bearer ${token}` } });
  return { status: r.status, body: await r.json() };
};
const listed = async (id) => (await s.api("/api/actors")).actors.some((a) => a.id === id);

before(async () => {
  // One test instance at a time (it has a single-instance lock). Its
  // actor gateway checks its address on this computer alone, never the
  // internet.
  process.env.XENONET_GATEWAY_PROBE = "local";
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("a made-up registration waits for the window, which allows one and denies another", { timeout: 60000 }, shoot("actor-requests", async () => {
  const heron = await s.simulate("actor-request");
  const osprey = await s.simulate("actor-request", { a: "Osprey" });
  assert.deepEqual([heron.ok, heron.a, osprey.a], [true, "actor:heron", "actor:osprey"]);
  const outside = await fetch(`${s.base}/api/actors/requests`);
  assert.equal(outside.status, 403, "a program without the key can't see them");
  assert.equal((await outside.json()).code, "yours");

  const cards = await fromWindow(`return (await call("GET", "/api/actors/requests")).body.requests.map((r) => ({ actor: r.actor, name: r.name, folder: r.asker?.folder ?? null }));`);
  assert.deepEqual(cards.filter((c) => c.actor === heron.a || c.actor === osprey.a), [
    { actor: "actor:heron", name: "Heron", folder: "~/Projects/heron" },
    { actor: "actor:osprey", name: "Osprey", folder: "~/Projects/osprey" },
  ]);
  assert.equal(await listed(heron.a), false, "not on the list while it waits");

  const allowed = await fromWindow(`return await call("POST", "/api/actors/requests/${heron.request}", { decision: "allow" });`);
  assert.deepEqual([allowed.status, allowed.body.decision, allowed.body.actor?.id, allowed.body.actor?.presence], [200, "allow", "actor:heron", "unknown"]);
  const denied = await fromWindow(`return await call("POST", "/api/actors/requests/${osprey.request}", { decision: "deny" });`);
  assert.deepEqual([denied.status, denied.body.decision], [200, "deny"]);
  assert.equal(await listed(heron.a), true, "allowed, it joins the list");
  assert.equal(await listed(osprey.a), false);

  const revoked = await asActor(osprey.token);
  assert.deepEqual([revoked.status, revoked.body.code, revoked.body.reason], [401, "revoked", "denied"]);
  // On macOS a token never works from under an agent, which may be running
  // these tests; otherwise Heron's token now works.
  const me = await asActor(heron.token);
  assert.ok((me.status === 200 && me.body.state === "allowed") || (me.status === 403 && me.body.code === "wrong-process"), JSON.stringify(me.body));
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("an actor added from the window gets its token once; a new token stops the old; removing it stops both", { timeout: 60000 }, shoot("actor-tokens", async () => {
  const added = await fromWindow(`return await call("POST", "/api/actors", { name: "Kite", capabilities: ["run-commands"] });`);
  assert.deepEqual([added.status, added.body.state, added.body.id], [201, "allowed", "actor:kite"]);
  assert.match(added.body.token, /^xenonet_actor_[0-9a-f]{64}$/);
  const outside = await fetch(`${s.base}/api/actors`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name: "Kite 2" }) });
  assert.equal(outside.status, 403, "only the window adds actors");

  const fresh = await fromWindow(`return await call("POST", "/api/actors/kite/token");`);
  assert.equal(fresh.status, 200);
  assert.notEqual(fresh.body.token, added.body.token);
  assert.equal((await asActor(added.body.token)).body.reason, "rotated");

  const removed = await fromWindow(`return await call("DELETE", "/api/actors/kite");`);
  assert.deepEqual([removed.status, removed.body.removed], [200, "actor:kite"]);
  assert.equal((await asActor(fresh.body.token)).body.reason, "removed");
  assert.equal(await listed("actor:kite"), false);
}));

test("an observer turned off in the window takes the actor it knows off the list, and on brings it back", { timeout: 60000 }, shoot("actor-observers", async () => {
  assert.equal(await listed("actor:kestrel"), true, "the demo's Grok Bot");
  const off = await fromWindow(`return await call("POST", "/api/actors/observers", { product: "grok-bot", on: false });`);
  assert.deepEqual([off.status, off.body.on, off.body.actors], [200, false, ["actor:kestrel"]]);
  assert.equal(await listed("actor:kestrel"), false);
  assert.deepEqual((await s.api("/api/actors/observers")).observers.map((o) => [o.product, o.on]), [["hermes-agent", true], ["grok-bot", false], ["muse", true]]);
  await fromWindow(`return await call("POST", "/api/actors/observers", { product: "grok-bot", on: true });`);
  assert.equal(await listed("actor:kestrel"), true);
}));

test("the actor gateway: set up in the window with a tunnel of your own, its address checked, a gateway token made there, and a client outside this machine reaching it with only that token", { timeout: 90000 }, shoot("actor-gateway", async () => {
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
  // Set up from the panel: not set up, it offers Set up… alone; every kind
  // is offered, and your own takes an address.
  await s.driver("command", "actors");
  assert.equal(await until(() => s.eval(`return document.querySelector('[data-ac-gateway-state] .ac-gw')?.dataset.state ?? null`)), "none");
  assert.equal(await s.eval(`return !!document.querySelector("[data-ac-gateway-on]")`), false);
  assert.equal(await until(() => click("[data-ac-gateway-setup]")), true);
  // Every kind is offered, the ones this computer has first and the rest
  // folded away; which those are depends on the computer.
  assert.equal(await until(() => s.eval(`return document.querySelectorAll(".ac-pick").length`)), 5);
  assert.deepEqual((await s.eval(`return [...document.querySelectorAll(".ac-pick-name")].map((e) => e.textContent)`)).sort(), ["Cloudflare Tunnel", "Cloudflare quick tunnel", "Tailscale Funnel", "Your own", "ngrok"]);
  assert.equal(await click('[data-ac-tunnel-kind="own"]'), true);
  await until(() => s.eval(`return !!document.querySelector("[data-ac-tunnel-host]")`));
  await s.eval(`document.querySelector("[data-ac-tunnel-host]").value = "https://XenoNet.Example.com/"`);
  assert.equal(await click("[data-ac-tunnel-use]"), true);
  // XenoNet checks the address (here, on this computer alone) and says so.
  const reachable = await until(() => s.eval(`return document.querySelector('[data-ac-gateway-state] [data-ac-reach="ok"]') ? document.querySelector("[data-ac-gateway-state]").innerText : null`), 15000);
  assert.match(reachable ?? "", /Online[\s\S]*https:\/\/xenonet\.example\.com\/mcp[\s\S]*via a tunnel of your own/);
  const on = await fromWindow(`return await call("GET", "/api/actors/gateway");`);
  assert.deepEqual([on.body.on, on.body.listening, on.body.tunnel.kind, on.body.reach.state, on.body.reach.local], [true, true, "own", "ok", true]);
  // Not from a program on this machine without the window's key.
  assert.equal((await fetch(`${s.base}/api/actors/moth/gateway`, { method: "POST" })).status, 403);
  // Connect… on an actor of your own (the demo's made-up ones have none):
  // a token, shown once, and what to tell its app.
  const lark = await fromWindow(`return await call("POST", "/api/actors", { schema: "actor/registration/v1", name: "Lark", capabilities: ["message-agents"], heartbeatMs: 60000 });`);
  assert.equal(lark.status, 201);
  assert.equal(await until(() => click('[data-ac-gateway="actor:lark"]')), true);
  assert.equal(await until(() => click('[data-ac-gateway-make="actor:lark"]')), true);
  // Its token is masked, and out of the page, until Show.
  assert.match((await until(() => text("[data-ac-token-text]"))) ?? "", /^xenonet_gateway_•+$/);
  assert.equal(await s.eval(`return /xenonet_gateway_[0-9a-f]{64}/.test(document.documentElement.outerHTML)`), false);
  await click('[data-secret-show="token"]');
  const token = await until(() => text('[data-secret="token"] .secret-plain'));
  assert.match(token ?? "", /^xenonet_gateway_[0-9a-f]{64}$/);
  assert.match(await text(".ac-steps"), /add an MCP server with this URL[\s\S]*https:\/\/xenonet\.example\.com\/mcp[\s\S]*Authorization: Bearer <token>[\s\S]*Waiting for Lark's first call/);
  // As a cloud agent calls it, through a tunnel to the gateway's own port.
  const rpc = (tok, method, params = {}) =>
    fetch(`http://127.0.0.1:${on.body.port}/mcp`, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", ...(tok ? { authorization: `Bearer ${tok}` } : {}) }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
  assert.equal((await rpc(null, "tools/list")).status, 401);
  const listed = await (await rpc(token, "tools/list")).json();
  assert.deepEqual(listed.result.tools.map((t) => t.name).sort(), ["heartbeat", "inbox", "peers", "send", "whoami"]);
  const me = await (await rpc(token, "tools/call", { name: "whoami", arguments: {} })).json();
  assert.match(me.result.content[0].text, /actor:lark/);
  // Its first call ticks the last step.
  assert.equal(await until(() => s.eval(`return document.querySelector('[data-ac-wait="done"]')?.innerText ?? null`), 15000), "Lark is connected");
  await click("[data-ac-done]");
  // The actor's row says its app called.
  await click("[data-ac-gateway-check]");
  const row = await until(async () => {
    const t = await s.eval(`return document.querySelector('[data-key="actor:lark"] [data-ac-gate]')?.innerText ?? null`);
    return /last call/.test(t ?? "") ? t : null;
  }, 20000);
  assert.match(row ?? "", /gateway: last call/);
  await s.input({ keys: "Escape" });
  const off = await fromWindow(`return await call("POST", "/api/actors/gateway", { on: false });`);
  assert.equal(off.body.listening, false);
  assert.equal((await rpc(token, "tools/list").catch(() => null)), null, "its port is closed");
  await fromWindow(`return await call("DELETE", "/api/actors/lark");`);
}));

test("a session asks for an actor's gateway token: it is told none exists, the card waits in the window, and Allow shows the token masked there alone", { timeout: 90000 }, shoot("actor-gateway-ask", async () => {
  const click = (sel) => s.eval(`return (() => { const b = document.querySelector(${JSON.stringify(sel)}); if (!b || b.disabled) return false; b.click(); return true; })()`);
  const until = async (fn, ms = 8000) => {
    const end = Date.now() + ms;
    for (;;) {
      const got = await fn();
      if (got) return got;
      if (Date.now() > end) return got;
      await new Promise((r) => setTimeout(r, 150));
    }
  };
  const TOKEN = /xenonet_(gateway|actor)_[0-9a-f]{64}/;
  // A program on this machine asks, as a session or the xenonet command does.
  const asked = await fetch(`${s.base}/api/actors/moth/gateway/ask`, { method: "POST", headers: { "content-type": "application/json" }, body: "{}" });
  const answer = await asked.text();
  assert.equal(asked.status, 202, answer);
  assert.doesNotMatch(answer, TOKEN, "what the asker gets holds no token");
  const ask = JSON.parse(answer);
  assert.deepEqual([ask.id, ask.state], ["actor:moth", "waiting"]);
  // It can't answer its own ask, nor see the card.
  const own = await fetch(`${s.base}/api/actors/requests/${ask.request}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ decision: "allow" }) });
  assert.equal(own.status, 403, "only the window allows");
  assert.equal((await fetch(`${s.base}/api/actors/requests`)).status, 403);

  // The window shows the card; what the driver says of it holds no token.
  const card = await until(async () => (await s.driver("actorRequests")).gatewayAsks.find((g) => g.actor === "actor:moth" && g.card && g.armed), 15000);
  assert.ok(card, "the card is up and armed");
  assert.doesNotMatch(JSON.stringify(await s.driver("actorRequests")), TOKEN);
  assert.equal((await s.driver("state")).alerts.find((a) => a.gatewayToken)?.actor, "actor:moth");

  // The person clicks Allow: the Actors panel opens on the steps, the token masked.
  assert.equal(await click('#alerts [data-al-allow]'), true);
  const masked = await until(() => s.eval(`return document.querySelector("[data-ac-token-text]")?.innerText ?? null`));
  assert.match(masked ?? "", /^xenonet_gateway_•+$/);
  assert.equal(await s.eval(`return ${TOKEN}.test(document.documentElement.outerHTML)`), false, "out of the page until Show");
  // The card leaves the strip as the strip's own transition ends.
  assert.equal(await until(() => s.eval(`return !document.querySelector('#alerts [data-al-request="${card.id}"]')`)), true, "answered");
  assert.deepEqual((await s.driver("actorRequests")).gatewayAsks, []);
  await click('[data-secret-show="token"]');
  assert.match((await until(() => s.eval(`return document.querySelector('[data-secret="token"] .secret-plain')?.innerText ?? null`))) ?? "", /^xenonet_gateway_[0-9a-f]{64}$/, "Show reveals it, in the window alone");
  // A screenshot holds a shown token masked.
  const shot = await s.eval(`return document.documentElement.outerHTML.includes("secret-masked")`);
  assert.equal(shot, true);
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
  await click("[data-ac-done]");
  await s.input({ keys: "Escape" });
}));

test("waking an actor: a webhook set in the window, its key hidden as it's typed, a test, then a call within seconds of a message, saying only how many wait", { timeout: 90000 }, shoot("actor-wake", async () => {
  const click = (sel) => s.eval(`return (() => { const b = document.querySelector(${JSON.stringify(sel)}); if (!b || b.disabled) return false; b.click(); return true; })()`);
  const text = (sel) => s.eval(`return document.querySelector(${JSON.stringify(sel)})?.innerText ?? null`);
  const until = async (fn, ms = 10000) => {
    const end = Date.now() + ms;
    for (;;) {
      const got = await fn();
      if (got || Date.now() > end) return got;
      await new Promise((r) => setTimeout(r, 150));
    }
  };
  // A webhook on this computer that records each call.
  const calls = [];
  const hook = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d)).on("end", () => {
      calls.push({ headers: req.headers, body: JSON.parse(body) });
      res.writeHead(204).end();
    });
  });
  await new Promise((r) => hook.listen(0, "127.0.0.1", r));
  const KEY = "rk_test_made_up_wake_key";
  try {
    // Two actors outside this machine, through the gateway: Kite writes to Lark.
    const on = await fromWindow(`return await call("POST", "/api/actors/gateway", { on: true, tunnel: { kind: "own", host: "xenonet.example.com" } });`);
    for (const name of ["Lark", "Kite"]) assert.equal((await fromWindow(`return await call("POST", "/api/actors", { schema: "actor/registration/v1", name: ${JSON.stringify(name)}, capabilities: ["message-agents"], heartbeatMs: 60000 });`)).status, 201);
    const kite = (await fromWindow(`return await call("POST", "/api/actors/kite/gateway");`)).body.token;
    await fromWindow(`return await call("POST", "/api/actors/lark/gateway");`);
    // Wake… on Lark: a webhook, its URL and key.
    await s.driver("command", "actors");
    assert.equal(await until(() => click('[data-ac-wake="actor:lark"]')), true);
    assert.equal(await until(() => click('[data-ac-wake-kind="webhook"]')), true);
    await until(() => s.eval(`return !!document.querySelector("[data-ac-wake-key]")`));
    assert.equal(await s.eval(`return document.querySelector("[data-ac-wake-key]").type`), "password", "the key is hidden as it's typed");
    await s.eval(`document.querySelector("[data-ac-wake-url]").value = "http://127.0.0.1:${hook.address().port}/wake"`);
    await s.eval(`document.querySelector("[data-ac-wake-key]").value = ${JSON.stringify(KEY)}`);
    assert.equal(await click('[data-ac-wake-save="actor:lark"]'), true);
    assert.equal(await until(() => click("[data-ac-wake-test]")), true);
    assert.match((await until(() => text("[data-ac-wake-result]"))) ?? "", /127\.0\.0\.1:\d+ answered HTTP 204/);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].headers.authorization, `Bearer ${KEY}`);
    assert.equal(await s.eval(`return document.documentElement.outerHTML.includes(${JSON.stringify(KEY)})`), false, "the key isn't in the page");
    await click("[data-ac-back]");
    // Kite writes to Lark through the gateway: Lark's webhook is called.
    const rpc = (tok, method, params = {}) => fetch(`http://127.0.0.1:${on.body.port}/mcp`, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", authorization: `Bearer ${tok}` }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
    const sent = await (await rpc(kite, "tools/call", { name: "send", arguments: { to: "actor:lark", text: "the build is green" } })).json();
    assert.equal(sent.result.isError, undefined, JSON.stringify(sent));
    assert.ok(await until(() => calls.length === 2), "woken within seconds");
    assert.deepEqual(calls[1].body.data, { actor: "actor:lark", unread: 1 });
    assert.doesNotMatch(JSON.stringify(calls[1]), /green|kite/i, "never what it says or who sent it");
    // Lark's row says how it's woken.
    const row = await until(async () => {
      const t = await s.eval(`return document.querySelector('[data-key="actor:lark"] [data-ac-wake-state]')?.innerText ?? null`);
      return /wake: webhook, last/.test(t ?? "") ? t : null;
    }, 20000);
    assert.match(row ?? "", /wake: webhook, last/);
    await s.input({ keys: "Escape" });
  } finally {
    hook.close();
    for (const slug of ["lark", "kite"]) await fromWindow(`return await call("DELETE", "/api/actors/${slug}");`);
    await fromWindow(`return await call("POST", "/api/actors/gateway", { on: false });`);
  }
}));
