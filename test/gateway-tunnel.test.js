// What to tell each app about the actor gateway (src/shared/handoff.js):
// numbered steps, the token never in a chat.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import { handoff } from "../src/shared/handoff.js";

const PORT = 48850;
const HOST = "mac.example-tailnet.ts.net";
const APP = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const BREW = "/opt/homebrew/bin/tailscale";

const status = ({ state = "Running", version = "1.104.1-t7f4", funnel = true, host = `${HOST}.` } = {}) =>
  JSON.stringify({ BackendState: state, Version: version, Self: { DNSName: host, CapMap: funnel ? { funnel: null, "https://tailscale.com/cap/funnel-ports?ports=443,8443,10000": null } : {} } });
const served = (proxy, { allow = true, more = false, at = "/" } = {}) =>
  JSON.stringify({ TCP: { 443: { HTTPS: true } }, Web: { [`${HOST}:443`]: { Handlers: { [at]: { Proxy: proxy }, ...(more ? { "/grafana": { Proxy: "http://127.0.0.1:3000" } } : {}) } } }, ...(allow ? { AllowFunnel: { [`${HOST}:443`]: true } } : {}) });
// Funnel as XenoNet sets it up: the gateway's /mcp alone.
const mcpAlone = (opts = {}) => served(`http://127.0.0.1:${PORT}/mcp`, { at: "/mcp", ...opts });

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
