#!/usr/bin/env node
// A run in a sandbox, shown in XenoNet: a small, dependency-free client for
// XenoNet's actor gateway, for a harness running somewhere else, such as in
// a SPACE sandbox (SPACE, by Perplexity, runs code in Firecracker microVMs;
// https://github.com/perplexityai/SPACE).
//
// A session is one run of a harness; an actor is the identity that lasts.
// So the gateway token is the actor's (one you add in XenoNet's window),
// and each sandbox reports its run as a session under that actor: the
// actor's heartbeat says it's at work, and report_run says which run, with
// which harness. Every minute it reports both; stopped, it reports the run
// ended and leaves the actor as it is, since other runs may go on.
//
// Inside the sandbox:
//
//   XENONET_GATEWAY_URL         the gateway's address, https://<host>/mcp
//   XENONET_GATEWAY_TOKEN_FILE  a file holding the actor's gateway token
//   (or XENONET_GATEWAY_TOKEN)  the token itself, from the command's env
//   XENONET_RUN_ID              this run's name, the same each time (the
//                               sandbox's id, say)
//   XENONET_RUN_APP             the harness it runs: claude, codex,
//                               gemini-cli… (core/schema/ids.js)
//   XENONET_STATUS              the run's title line (optional)
//   XENONET_HEARTBEAT_SECONDS   how often (optional; 60, at least 5)
//
//   node presence.mjs [--once]
//
// The token is made in XenoNet's window (docs/guide/related-work.md says
// how) and works only at the gateway, which takes messages and presence and
// nothing else. This client never prints it, refuses http:// except on this
// machine's loopback, and doesn't follow redirects, so the token goes only
// where you sent it.
//
// From the machine that starts the sandbox, `node presence.mjs space-command
// <sandbox id> <harness>` asks the SPACE API to start this script in the sandbox,
// with the gateway's address and token in that command's own env, never in
// the sandbox's metadata or template (see spaceCommand below), and the
// sandbox's id as the run's name.
import fs from "node:fs";
import { pathToFileURL } from "node:url";

const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]"]);

export function checkUrl(raw, what = "XENONET_GATEWAY_URL") {
  let u;
  try {
    u = new URL(raw);
  } catch {
    throw new Error(`${what} isn't an address`);
  }
  if (u.username || u.password) throw new Error(`${what} can't carry a user or password`);
  if (u.protocol !== "https:" && !(u.protocol === "http:" && LOOPBACK.has(u.hostname))) throw new Error(`${what} must be https:// (http:// only on loopback)`);
  return u;
}

export function tokenFrom(env = process.env, files = fs) {
  const t = env.XENONET_GATEWAY_TOKEN_FILE ? files.readFileSync(env.XENONET_GATEWAY_TOKEN_FILE, "utf8").trim() : (env.XENONET_GATEWAY_TOKEN ?? "").trim();
  if (!t) throw new Error("no gateway token: set XENONET_GATEWAY_TOKEN_FILE or XENONET_GATEWAY_TOKEN");
  return t;
}

let seq = 0;
// One tool call at the gateway, as bare JSON-RPC over HTTP: the tool's
// answer, parsed, or an error that never includes the token.
export async function callTool(url, token, name, args = {}, { fetch: f = fetch, timeoutMs = 15000 } = {}) {
  const res = await f(url, {
    method: "POST",
    redirect: "manual",
    signal: AbortSignal.timeout(timeoutMs),
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream", authorization: `Bearer ${token}` },
    body: JSON.stringify({ jsonrpc: "2.0", id: ++seq, method: "tools/call", params: { name, arguments: args } }),
  });
  if (res.status >= 300 && res.status < 400) throw new Error(`the gateway answered with a redirect (${res.status}); not followed`);
  const text = await res.text();
  if (!res.ok) throw new Error(`the gateway answered ${res.status}`);
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    // An event-stream answer: its last data line.
    const data = text.split("\n").filter((l) => l.startsWith("data:")).pop();
    body = data ? JSON.parse(data.slice(5)) : null;
  }
  if (!body || body.error) throw new Error(`the gateway refused ${name}: ${body?.error?.message ?? "no answer"}`);
  const out = body.result?.content?.[0]?.text;
  if (body.result?.isError) throw new Error(`the gateway refused ${name}: ${out ?? ""}`);
  try {
    return JSON.parse(out);
  } catch {
    return out ?? null;
  }
}

// The SPACE API request that starts this script in a sandbox: POST
// /v1/sandboxes/{id}/commands, with the management token as Bearer, and
// the gateway's address and token in the command's env alone.
export function spaceCommand({ sandbox, gatewayUrl, gatewayToken, app, status = null, script = "/opt/xenonet/presence.mjs", node = "node" }) {
  if (!/^sbx_[A-Za-z0-9_-]{1,60}$/.test(sandbox ?? "")) throw new Error("a SPACE sandbox id starts with sbx_");
  if (typeof app !== "string" || !/^[a-z][a-z0-9-]{1,31}$/.test(app)) throw new Error("say which harness runs in it: claude, codex, gemini-cli…");
  checkUrl(gatewayUrl);
  const env = { XENONET_GATEWAY_URL: gatewayUrl, XENONET_GATEWAY_TOKEN: gatewayToken, XENONET_RUN_ID: sandbox, XENONET_RUN_APP: app, ...(status ? { XENONET_STATUS: status } : {}) };
  return { path: `/v1/sandboxes/${sandbox}/commands`, body: { command: [node, script], env } };
}

export async function startInSpace({ api, apiToken, idempotencyKey, fetch: f = fetch, ...rest }) {
  const base = checkUrl(api, "SPACE_API_URL");
  const { path, body } = spaceCommand(rest);
  const res = await f(new URL(path, base), {
    method: "POST",
    redirect: "manual",
    headers: { "content-type": "application/json", accept: "application/json", authorization: `Bearer ${apiToken}`, ...(idempotencyKey ? { "idempotency-key": idempotencyKey } : {}) },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`the SPACE API answered ${res.status}`);
  const out = await res.json();
  return { session: out.session_id ?? null, process: out.process_id ?? null };
}

async function main(argv) {
  if (argv[0] === "space-command") {
    const r = await startInSpace({
      api: process.env.SPACE_API_URL ?? "",
      apiToken: process.env.SPACE_API_TOKEN ?? "",
      sandbox: argv[1],
      gatewayUrl: process.env.XENONET_GATEWAY_URL ?? "",
      gatewayToken: tokenFrom(),
      app: argv[2] ?? process.env.XENONET_RUN_APP,
      status: process.env.XENONET_STATUS ?? null,
      ...(process.env.XENONET_PRESENCE_SCRIPT ? { script: process.env.XENONET_PRESENCE_SCRIPT } : {}),
    });
    console.log(`started in ${argv[1]}: ${r.process ?? "a process"}`);
    return;
  }
  const url = checkUrl(process.env.XENONET_GATEWAY_URL ?? "").href;
  const token = tokenFrom();
  const run = process.env.XENONET_RUN_ID ?? "";
  const app = process.env.XENONET_RUN_APP ?? "";
  if (!run || !app) throw new Error("set XENONET_RUN_ID (this run's name) and XENONET_RUN_APP (its harness)");
  const title = (process.env.XENONET_STATUS ?? "").slice(0, 90) || undefined;
  const every = Math.max(5, Number(process.env.XENONET_HEARTBEAT_SECONDS) || 60) * 1000;
  const me = await callTool(url, token, "whoami");
  const beat = async (presence = "working") => {
    await callTool(url, token, "heartbeat", { presence: "working" });
    return callTool(url, token, "report_run", { run, app, presence, ...(title ? { title } : {}) });
  };
  const r = await beat();
  console.log(`XenoNet shows this run as ${r?.session ?? "a session"}, under ${me?.you?.id ?? "its actor"}`);
  if (argv.includes("--once")) return;
  const timer = setInterval(() => beat().catch((e) => console.error(e.message)), every);
  const bye = async () => {
    clearInterval(timer);
    await callTool(url, token, "report_run", { run, app, presence: "ended" }).catch(() => {});
    process.exit(0);
  };
  process.on("SIGTERM", bye);
  process.on("SIGINT", bye);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href)
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
