// The actor gateway's tunnels (server/gateway-tunnel.js) and the check
// that its address reaches it (server/gateway-reach.js), with the tools
// and the network stood in for: Tailscale Funnel set up with the command
// that matches the running Tailscale, what it already serves left alone,
// a child tunnel's address learned from its output and started again when
// it exits, and each way a check can fail said in words.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import path from "node:path";
import { createTunnel, tailscaleSelf, funnelOf, readLine, childArgs, findTools, KIND_IDS } from "../server/gateway-tunnel.js";
import { checkReach } from "../server/gateway-reach.js";
import { handoff } from "../core/actors/handoff.js";

const PORT = 48850;
const HOST = "mac.example-tailnet.ts.net";
const APP = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const BREW = "/opt/homebrew/bin/tailscale";

const status = ({ state = "Running", version = "1.104.1-t7f4", funnel = true, host = `${HOST}.` } = {}) =>
  JSON.stringify({ BackendState: state, Version: version, Self: { DNSName: host, CapMap: funnel ? { funnel: null, "https://tailscale.com/cap/funnel-ports?ports=443,8443,10000": null } : {} } });
const served = (proxy, { allow = true, more = false } = {}) =>
  JSON.stringify({ TCP: { 443: { HTTPS: true } }, Web: { [`${HOST}:443`]: { Handlers: { "/": { Proxy: proxy }, ...(more ? { "/grafana": { Proxy: "http://127.0.0.1:3000" } } : {}) } } }, ...(allow ? { AllowFunnel: { [`${HOST}:443`]: true } } : {}) });

// A pretend computer with these Tailscale commands, whose funnel answers
// as `funnel` says: every command run is in `ran`.
function tailscaleHere({ tools = { [BREW]: "1.98.8", [APP]: "1.104.1" }, self = status(), funnel = "{}", onFunnel = null } = {}) {
  const ran = [];
  let conf = funnel;
  const run = async (file, args) => {
    ran.push([file, ...args]);
    if (args[0] === "version") return { ok: true, stdout: `${tools[file]}\n  tailscale commit: abc\n`, stderr: "" };
    if (args[0] === "status") return { ok: true, stdout: self, stderr: "" };
    if (args[0] === "funnel" && args[1] === "status") return { ok: true, stdout: conf, stderr: "" };
    if (args[0] === "funnel") {
      const out = onFunnel?.(args) ?? { ok: true, stdout: "Available on the internet", stderr: "" };
      if (out.conf !== undefined) conf = out.conf;
      return out;
    }
    return { ok: false, stdout: "", stderr: "unknown" };
  };
  return { ran, run, exists: (f) => f in tools };
}

test("Tailscale's status and Funnel's settings, as the gateway needs them", () => {
  assert.deepEqual(tailscaleSelf(status()), { up: true, version: "1.104.1", host: HOST, funnel: true });
  assert.equal(tailscaleSelf(status({ funnel: false })).funnel, false);
  assert.deepEqual(tailscaleSelf(status({ state: "NeedsLogin" })), { up: false, version: "1.104.1", why: "Tailscale isn't signed in on this computer" });
  assert.equal(tailscaleSelf(status({ state: "Stopped" })).why, "Tailscale is turned off on this computer");
  assert.equal(tailscaleSelf("not json").up, false);
  assert.deepEqual(funnelOf({}, HOST, PORT), { state: "none" });
  assert.deepEqual(funnelOf(JSON.parse(served(`http://127.0.0.1:${PORT}`)), HOST, PORT), { state: "ours" });
  assert.deepEqual(funnelOf(JSON.parse(served(`http://localhost:${PORT}`, { allow: false })), HOST, PORT), { state: "tailnet" });
  assert.deepEqual(funnelOf(JSON.parse(served("http://127.0.0.1:48852")), HOST, PORT), { state: "elsewhere", target: "http://127.0.0.1:48852" });
  assert.deepEqual(funnelOf(JSON.parse(served(`http://127.0.0.1:${PORT}`, { more: true })), HOST, PORT), { state: "shared", target: "/, /grafana" });
});

test("each tool is found on PATH and where it's usually installed, whatever PATH a service started at login has", () => {
  // PATH as this computer writes it, for the test to run anywhere.
  const onpath = path.join("/home/me/bin", "ngrok");
  const files = new Set(["/opt/homebrew/bin/ngrok", "/usr/local/bin/ngrok", onpath]);
  const exists = (f) => files.has(f);
  assert.deepEqual(findTools("ngrok", { env: { PATH: ["/home/me/bin", "/usr/bin"].join(path.delimiter) }, platform: "darwin", exists }), [onpath, "/opt/homebrew/bin/ngrok", "/usr/local/bin/ngrok"]);
  assert.deepEqual(findTools("ngrok", { env: { PATH: "" }, platform: "darwin", exists: () => false }), []);
  assert.deepEqual(KIND_IDS, ["tailscale", "cloudflare", "cloudflare-quick", "ngrok", "own"]);
});

test("Tailscale Funnel: set up with the command that matches the running Tailscale, the name learned, and nothing done when it's already there", async () => {
  const ts = tailscaleHere({ onFunnel: () => ({ ok: true, stdout: "", stderr: "", conf: served(`http://127.0.0.1:${PORT}`) }) });
  const hosts = [];
  const tunnel = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: ts.exists, run: ts.run, onHost: (h) => hosts.push(h) });
  const v = await tunnel.up({ kind: "tailscale" });
  assert.deepEqual([v.kind, v.state, v.why], ["tailscale", "up", null]);
  assert.deepEqual(hosts, [HOST]);
  // Homebrew's older command is passed over for the app's, which matches.
  assert.deepEqual(ts.ran.find((r) => r[1] === "funnel" && r[2] === "--bg"), [APP, "funnel", "--bg", "--yes", String(PORT)]);
  const before = ts.ran.length;
  await tunnel.ensure();
  assert.equal(ts.ran.slice(before).filter((r) => r[1] === "funnel" && r[2] !== "status").length, 0, "already set up: nothing run but status");
  // Off turns Funnel off, as it forwards to this gateway alone.
  await tunnel.down();
  assert.deepEqual(ts.ran.at(-1), [APP, "funnel", "--yes", "--https=443", "off"]);
  assert.equal(tunnel.view().state, "off");
});

test("Tailscale Funnel: what it already forwards elsewhere is left alone until Repair, other paths never, and what Tailscale asks for is passed on", async () => {
  const elsewhere = tailscaleHere({ funnel: served("http://127.0.0.1:48852"), onFunnel: () => ({ ok: true, stdout: "", stderr: "", conf: served(`http://127.0.0.1:${PORT}`) }) });
  const t1 = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: elsewhere.exists, run: elsewhere.run });
  assert.match((await t1.up({ kind: "tailscale" })).why, /already forwards to http:\/\/127\.0\.0\.1:48852, not to this XenoNet's gateway: Repair points it here/);
  assert.equal(elsewhere.ran.some((r) => r[2] === "--bg"), false);
  assert.equal((await t1.up({ kind: "tailscale" }, { takeOver: true })).state, "up");
  // Turning off leaves what isn't this gateway's alone.
  const shared = tailscaleHere({ funnel: served(`http://127.0.0.1:${PORT}`, { more: true }) });
  const t2 = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: shared.exists, run: shared.run });
  assert.match((await t2.up({ kind: "tailscale" }, { takeOver: true })).why, /already serves other paths on this computer's port 443 \(\/, \/grafana\)/);
  await t2.down();
  assert.equal(shared.ran.some((r) => r.includes("off")), false);
  const blocked = tailscaleHere({ self: status({ funnel: false }) });
  const t3 = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: blocked.exists, run: blocked.run });
  assert.match((await t3.up({ kind: "tailscale" })).why, /your tailnet doesn't allow Funnel for this computer yet/);
  const asks = tailscaleHere({ onFunnel: () => ({ ok: false, stdout: "Funnel is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/funnel?node=abc\n", stderr: "", timedOut: true }) });
  const t4 = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: asks.exists, run: asks.run });
  assert.equal((await t4.up({ kind: "tailscale" })).why, "Tailscale asks you to allow Funnel for this computer at https://login.tailscale.com/f/funnel?node=abc; then Repair");
  const none = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: () => false, run: async () => ({ ok: false, stdout: "", stderr: "" }) });
  assert.equal((await none.up({ kind: "tailscale" })).why, "XenoNet can't find Tailscale on this computer");
  const out = tailscaleHere({ self: status({ state: "NeedsLogin" }) });
  const t5 = createTunnel({ port: PORT, env: { PATH: "" }, platform: "darwin", exists: out.exists, run: out.run });
  assert.equal((await t5.up({ kind: "tailscale" })).why, "Tailscale isn't signed in on this computer");
});

test("a tunnel tool's lines: the address it got, that it's up, and what went wrong", () => {
  assert.deepEqual(readLine("cloudflare-quick", "2026-10-08T10:00:00Z INF |  https://brave-otter-sails.trycloudflare.com  |"), { msg: "2026-10-08T10:00:00Z INF |  https://brave-otter-sails.trycloudflare.com  |", host: "brave-otter-sails.trycloudflare.com" });
  assert.equal(readLine("cloudflare", "2026-10-08T10:00:01Z INF Registered tunnel connection connIndex=0 location=ord01").ready, true);
  assert.equal(readLine("cloudflare", "2026-10-08T10:00:01Z ERR Couldn't start tunnel error=\"no such tunnel\"").error, true);
  assert.deepEqual(readLine("ngrok", JSON.stringify({ lvl: "info", msg: "started tunnel", url: "https://xenonet-demo.ngrok-free.app" })), { msg: "started https://xenonet-demo.ngrok-free.app", host: "xenonet-demo.ngrok-free.app", ready: true });
  assert.deepEqual(readLine("ngrok", JSON.stringify({ lvl: "eror", msg: "session closing", err: "authentication failed: ERR_NGROK_4018" })), { msg: "authentication failed: ERR_NGROK_4018", error: true });
  assert.equal(readLine("ngrok", ""), null);
  assert.deepEqual(childArgs("cloudflare", { port: PORT, name: "xenonet" }), ["tunnel", "--no-autoupdate", "run", "--url", `http://127.0.0.1:${PORT}`, "xenonet"]);
  assert.deepEqual(childArgs("cloudflare-quick", { port: PORT }), ["tunnel", "--no-autoupdate", "--url", `http://127.0.0.1:${PORT}`]);
  assert.deepEqual(childArgs("ngrok", { port: PORT, host: "xenonet-demo.ngrok-free.app" }), ["http", `127.0.0.1:${PORT}`, "--log", "stdout", "--log-format", "json", "--url", "https://xenonet-demo.ngrok-free.app"]);
  assert.deepEqual(childArgs("ngrok", { port: PORT }).slice(-2), ["--log-format", "json"]);
});

// A child that answers as `script` says, and exits when told to.
function fakeSpawn() {
  const children = [];
  const spawn = (file, args) => {
    const c = Object.assign(new EventEmitter(), { file, args, pid: 4000 + children.length, stdout: new PassThrough(), stderr: new PassThrough(), killed: false });
    c.kill = () => {
      c.killed = true;
      setImmediate(() => c.emit("exit", null));
    };
    children.push(c);
    return c;
  };
  return { children, spawn };
}

test("a tunnel XenoNet runs: its address learned from what it writes, started again after it exits, failed after too many exits, stopped with XenoNet", async () => {
  const { children, spawn } = fakeSpawn();
  const timers = [];
  const hosts = [];
  let ready = 0;
  let t = 1_000_000;
  const tunnel = createTunnel({
    port: PORT,
    env: { PATH: "" },
    platform: "darwin",
    exists: (f) => f === "/opt/homebrew/bin/ngrok",
    run: async () => ({ ok: true, stdout: "ngrok version 3.24.0", stderr: "" }),
    spawn,
    now: () => t,
    setTimer: (fn, ms) => (timers.push({ fn, ms }), timers.length),
    clearTimer: () => {},
    onHost: (h) => hosts.push(h),
    onReady: () => ready++,
  });
  assert.equal((await tunnel.up({ kind: "ngrok" })).state, "starting");
  assert.deepEqual([children[0].file, children[0].args.slice(0, 2)], ["/opt/homebrew/bin/ngrok", ["http", `127.0.0.1:${PORT}`]]);
  children[0].stdout.write(JSON.stringify({ lvl: "info", msg: "started tunnel", url: "https://xenonet-demo.ngrok-free.app" }) + "\n");
  await new Promise((r) => setImmediate(r));
  assert.deepEqual([tunnel.view().state, hosts, ready], ["up", ["xenonet-demo.ngrok-free.app"], 1]);
  // It exits: XenoNet waits, then starts it again, longer each time.
  for (const [i, wait] of [5e3, 20e3, 60e3].entries()) {
    children.at(-1).stdout.write(JSON.stringify({ lvl: "eror", msg: "x", err: "failed to reconnect session" }) + "\n");
    children.at(-1).emit("exit", 1);
    assert.match(tunnel.view().why, new RegExp(`it exited with code 1: failed to reconnect session; trying again in ${wait / 1000} s`));
    assert.equal(timers.at(-1).ms, wait);
    t += 1000;
    timers.at(-1).fn();
    await new Promise((r) => setImmediate(r));
    assert.equal(children.length, i + 2);
  }
  // A fourth exit in ten minutes: it stays failed until a check ten minutes on.
  children.at(-1).emit("exit", 1);
  assert.deepEqual([tunnel.view().state, timers.length], ["failed", 3]);
  await tunnel.ensure();
  assert.equal(children.length, 4, "not yet");
  t += 10 * 60e3;
  await tunnel.ensure();
  assert.equal(children.length, 5);
  // Another kind stops it first; XenoNet going stops it too.
  await tunnel.up({ kind: "own" });
  assert.deepEqual([children[4].killed, tunnel.view().state], [true, "yours"]);
  await tunnel.up({ kind: "cloudflare-quick" });
  tunnel.stop();
  assert.equal(children.at(-1).killed, true);
});

test("the check, in words: no name in public DNS, nothing answering, the wrong certificate, a changed Host, the tunnel without the gateway, and yes", async () => {
  const words = [];
  const check = (lookup, answer) => checkReach({ host: "xenonet.example.com", nonce: "n0", expect: (w) => words.push(w), lookup: async () => lookup, request: async () => answer });
  const at = { address: "203.0.113.7", family: 4 };
  assert.deepEqual(await check({ error: "missing" }), { ok: false, stage: "dns", why: "xenonet.example.com isn't in public DNS yet. A new tunnel's name can take a few minutes." });
  assert.equal((await check({ error: "ETIMEOUT" })).why, "Public DNS didn't answer (ETIMEOUT). Is this computer online?");
  assert.equal((await check(at, { error: "ECONNREFUSED" })).why, "Nothing answered at xenonet.example.com (ECONNREFUSED). Is the tunnel running?");
  assert.equal((await check(at, { error: "ERR_TLS_CERT_ALTNAME_INVALID" })).stage, "tls");
  assert.equal((await check(at, { status: 421 })).stage, "host");
  assert.equal((await check(at, { status: 502 })).why, "The tunnel answers, but can't reach the gateway on this computer (HTTP 502).");
  assert.equal((await check(at, { status: 401, echoed: "" })).why, "Something else answers at xenonet.example.com (HTTP 401).");
  assert.deepEqual(await check(at, { status: 204, echoed: "n0" }), { ok: true, stage: "ok", why: null });
  assert.deepEqual(words, Array(6).fill("n0"), "the gateway is told the word before each request");
  assert.equal((await checkReach({ host: null })).stage, "address");
});

test("what to tell each app: numbered steps with what each copies, the token never in a chat, and the address alone when it changed", () => {
  const url = "https://xenonet.example.com/mcp";
  const grok = handoff("grok-bot", { url, token: "xenonet_gateway_x" });
  assert.deepEqual(grok.steps.map((s) => s.copy), ["message", "token", "wake"]);
  assert.equal(grok.message.includes("xenonet_gateway_x"), false, "Grok Bot's token goes in its secret card, never its chat");
  assert.match(grok.message, /Add an MCP server named "xenonet" by URL: https:\/\/xenonet\.example\.com\/mcp\. Ask me for its token with your secure secret card, named XENONET_GATEWAY_TOKEN/);
  const muse = handoff("muse", { url, token: "xenonet_gateway_x" });
  assert.deepEqual(muse.steps.map((s) => s.copy), ["message", "token"]);
  assert.equal(muse.message.includes("xenonet_gateway_x"), false, "Muse's key goes in its card, never its chat");
  assert.match(muse.message, /for the host xenonet\.example\.com/);
  assert.deepEqual(handoff(null, { url, token: "t" }), { app: null, steps: [{ text: "In the app, add an MCP server with this URL.", copy: "url" }, { text: "Send the token in the header Authorization: Bearer <token>.", copy: "token" }], message: null });
  assert.match(handoff("grok-bot", { url }).message, /Change the URL of your MCP server "xenonet" to https:\/\/xenonet\.example\.com\/mcp and keep its Authorization header/);
  assert.match(handoff("muse", { url }).message, /Change your "XenoNet" connector's host to xenonet\.example\.com and keep its key/);
  assert.deepEqual(handoff(null, { url }).steps.map((s) => s.copy), ["url"]);
});
