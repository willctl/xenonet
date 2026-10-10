// End to end: a remote actor paired in XenoNet Dev's test instance, whose
// data is made up (docs/guide/cloud-assistants.md). Pairing is the
// window's alone, with its key; the Actors panel lists the connection and
// the actor; `xenonet mcp assistant --grant`, as a tunnel client would run
// it, reads the demo's sessions through the private endpoint, and nothing
// after the connection is revoked. The window stays hidden;
// XENONET_E2E_SHOW=inactive shows it to watch.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { xenonet } from "../../core/control.js";

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

const fromWindow = (code) =>
  s.eval(`return (async () => {
    const key = await window.xenonetDesktop.windowKey();
    const call = async (method, path, body) => {
      const r = await fetch(path, { method, headers: { "x-xenonet-window": key, ...(body ? { "content-type": "application/json" } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
      return { status: r.status, body: await r.json() };
    };
    ${code}
  })()`);

// The adapter, as a tunnel client runs it, against this instance.
function adapter(grant) {
  const proc = spawn(process.execPath, ["cli/xenonet.js", "mcp", "assistant", "--grant", grant, "--port", String(port)], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  let buf = "";
  let seq = 0;
  const pending = new Map();
  proc.stdout.on("data", (d) => {
    buf += d;
    for (let e; (e = buf.indexOf("\n")) >= 0; ) {
      const m = JSON.parse(buf.slice(0, e));
      buf = buf.slice(e + 1);
      pending.get(m.id)?.(m);
    }
  });
  const call = (method, params) => new Promise((resolve) => {
    const id = ++seq;
    pending.set(id, resolve);
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  return {
    init: async () => (await call("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e", version: "1" } }), proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n")),
    tool: async (name, args = {}) => (await call("tools/call", { name, arguments: args })).result,
    close: () => (proc.stdin.end(), proc.kill()),
  };
}

before(async () => {
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("a dot paired in the window reads the demo's sessions through xenonet mcp assistant, and nothing once revoked", { timeout: 120000 }, shoot("assistant", async () => {
  const outside = await fetch(`${s.base}/api/assistant/pair`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ product: "dots" }) });
  assert.equal(outside.status, 403, "only the window pairs");

  const places = await fromWindow(`return (await call("GET", "/api/assistant/places")).body.places;`);
  const atlas = places.find((p) => p.name === "atlas-api");
  assert.ok(atlas, "the demo's atlas-api is there to pick");
  const paired = await fromWindow(`return await call("POST", "/api/assistant/pair", ${JSON.stringify({ product: "dots", label: "E2E dot", transport: "openai-secure-mcp-tunnel", audience: { description: "Only my account", verification: "owner_verified", scope: "owner_endpoint" }, projects: [atlas.place] })});`);
  assert.equal(paired.status, 201, JSON.stringify(paired.body));
  const grant = paired.body.grant_id;
  assert.deepEqual(paired.body.launch.args, ["mcp", "assistant", "--grant", grant, "--port", String(port)]);

  // The panel and the map know it.
  await s.driver("command", "actors");
  const remote = await s.driver("remoteConnections");
  assert.deepEqual(remote.connections.map((c) => [c.grant, c.actor, c.state, c.projects, c.labels]), [[grant, "actor:dots", "active", 1, false]]);
  assert.match(remote.connections[0].row, /active/);
  assert.ok((await s.api("/api/actors")).actors.some((a) => a.id === "actor:dots" && a.via.includes("remote") && a.remote?.active));

  const a = adapter(grant);
  try {
    await a.init();
    const status = await a.tool("xenonet_status");
    assert.ok(!status.isError, JSON.stringify(status));
    assert.deepEqual([status.structuredContent.data.authorized_project_count, status.structuredContent.data.data_mode, status.structuredContent.data.actor.actor_id], [1, "real", "actor:dots"]);
    const sessions = await a.tool("xenonet_sessions");
    assert.ok(sessions.structuredContent.data.sessions.length >= 1, JSON.stringify(sessions.structuredContent));
    assert.ok(sessions.structuredContent.data.sessions.every((x) => x.label === null && x.project_id === sessions.structuredContent.data.sessions[0].project_id), "one project, no labels");
    const attention = await a.tool("xenonet_attention");
    assert.ok(!attention.isError);
    // The read shows on the map: the dot was at work, through its bridge.
    const dot = (await s.api("/api/actors")).actors.find((x) => x.id === "actor:dots");
    assert.deepEqual([dot.presence, dot.remote.bridge], ["working", "connected"]);

    const revoked = await fromWindow(`return await call("POST", "/api/assistant/${grant}/revoke");`);
    assert.equal(revoked.status, 200);
    const after = await a.tool("xenonet_sessions");
    assert.equal(after.isError, true, "nothing after revoking");
    assert.ok(["GRANT_REVOKED", "NOT_AUTHORIZED"].includes(JSON.parse(after.content[0].text).code));
  } finally {
    a.close();
  }
  assert.equal((await s.driver("remoteConnections")).connections[0].state, "revoked");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("a tunnel for a dot is set up in the window: its tunnel, a key kept and never shown, and Start says what's missing", { timeout: 120000 }, shoot("assistant-tunnel", async () => {
  const places = await fromWindow(`return (await call("GET", "/api/assistant/places")).body.places;`);
  const paired = await fromWindow(`return await call("POST", "/api/assistant/pair", ${JSON.stringify({ product: "dots", label: "Tunnel dot", transport: "openai-secure-mcp-tunnel", audience: { description: "Only my account", verification: "unverified", scope: "owner_endpoint" }, projects: [places[0].place] })});`);
  assert.equal(paired.status, 201, JSON.stringify(paired.body));
  const grant = paired.body.grant_id;
  // A made-up key, for this test instance's memory only.
  const KEY = "sk-e2e-MADEUP0123456789abcdefghij";
  const TUNNEL = "tunnel_00112233445566778899aabbccddeeff";
  const outside = await fetch(`${s.base}/api/assistant/${grant}/tunnel/key`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ key: KEY }) });
  assert.equal(outside.status, 403, "only the window keeps a key");

  await s.driver("command", "actors");
  let remote = await s.driver("remoteConnections");
  const mine = () => remote.tunnels.tunnels.find((t) => t.grant === grant);
  assert.equal(remote.tunnels.client.found, false, "no tunnel-client in the test instance");
  assert.deepEqual([mine().tunnelId, mine().key, mine().state], [null, "missing", "off"]);
  assert.match(remote.connections.find((c) => c.grant === grant).row, /tunnel not running · no tunnel set · no key/);

  const click = (sel) => s.eval(`return (() => { const b = document.querySelector(${JSON.stringify(sel)}); if (!b) return false; b.click(); return true; })()`);
  const until = async (fn, ms = 6000) => {
    const end = Date.now() + ms;
    let got;
    while (!(got = await fn()) && Date.now() < end) await new Promise((r) => setTimeout(r, 150));
    return got;
  };
  assert.ok(await click(`[data-rc-tunnel="${grant}"]`), "Tunnel… in its row");
  assert.ok(await until(() => s.eval(`return !!document.querySelector("[data-rc-tid]")`)), "the tunnel view");
  assert.match(await s.eval(`return document.querySelector(".ac .m-body").textContent`), /XenoNet can't find tunnel-client/);
  await s.eval(`return (() => { document.querySelector("[data-rc-tid]").value = ${JSON.stringify(TUNNEL)}; document.querySelector("[data-rc-key]").value = ${JSON.stringify(KEY)}; })()`);
  await click("[data-rc-tkey]");
  remote = await until(async () => {
    const r = await s.driver("remoteConnections");
    return r.tunnels.tunnels.find((t) => t.grant === grant)?.key === "stored" ? r : null;
  });
  assert.ok(remote, "the key is kept");
  assert.equal(mine().tunnelId, TUNNEL, "with the tunnel it's for");
  const shown = await s.eval(`return document.querySelector(".ac .m-body").innerHTML + [...document.querySelectorAll("input")].map((i) => i.value).join(" ")`);
  assert.ok(!shown.includes(KEY), "the key isn't on the page, nor left in its field");
  const status = await fromWindow(`return await call("GET", "/api/assistant/tunnels");`);
  assert.ok(!JSON.stringify(status.body).includes(KEY), "nor in what the window reads");

  await until(() => s.eval(`return !!document.querySelector("[data-rc-tstart]:not([disabled])")`));
  await click("[data-rc-tstart]");
  const error = await until(() => s.eval(`return (() => { const e = document.querySelector("[data-rc-error]"); return e && !e.hidden ? e.textContent : null; })()`));
  assert.match(error ?? "", /can't find tunnel-client/, "Start says what's missing");
  await click("[data-rc-tauto]");
  assert.ok(await until(async () => (await s.driver("remoteConnections")).tunnels.tunnels.find((t) => t.grant === grant)?.autostart), "it can start with XenoNet");

  // Revoking it takes the tunnel off the list.
  await fromWindow(`return await call("POST", "/api/assistant/${grant}/revoke");`);
  assert.ok(!(await s.driver("remoteConnections")).tunnels?.tunnels.some((t) => t.grant === grant));
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));
