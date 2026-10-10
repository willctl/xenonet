// The real thing, on this Mac: Grok Bot and Muse send each other a message
// through XenoNet and read it. Nothing is made up and nothing is simulated, so
// it isn't part of npm test or test:e2e: it needs both apps installed and
// signed in, a person at XenoNet's window, and someone to ask each agent to do
// what it prints. Two routes:
//
//   gateway (the default): through XenoNet's actor gateway, from their
//     makers' clouds (server/actor-gateway.js). The gateway is on, the
//     tunnel picked in XenoNet's window puts it on the internet and its
//     address checks out, each has a gateway token made with Connect…,
//     and each app has the gateway as an MCP server (Grok Bot) or a custom
//     connector (Muse).
//   local: each registers through a command its own app runs on this Mac,
//     and uses the xenonet command. Muse can't run commands here yet, so only
//     Grok Bot can take this route today.
//
//   npm run test:real                       # through the gateway
//   XENONET_REAL_ROUTE=local npm run test:real
//
// It runs against XenoNet Dev (the dev instance, port 47842) unless
// XENONET_REAL_PORT says another; XENONET_REAL_WORD sets the code word the
// messages carry (a new one each run otherwise), XENONET_REAL_MINUTES how
// long each step waits for the agents (10), and XENONET_REAL_GATEWAY_URL the
// gateway's address when XenoNet won't say it (it says it to you, not to a
// program under an agent). It only reads: XenoNet's health, its actors and
// the messages of the last day, never a token.
import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const PORT = Number(process.env.XENONET_REAL_PORT) || 47842;
const BASE = `http://127.0.0.1:${PORT}`;
const WORD = process.env.XENONET_REAL_WORD || `xenonet-${crypto.randomBytes(3).toString("hex")}`;
const WAIT_MS = (Number(process.env.XENONET_REAL_MINUTES) || 10) * 60e3;
const ROUTE = process.env.XENONET_REAL_ROUTE === "local" ? "local" : "gateway";
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

// What each agent runs locally: this checkout's xenonet command, under the
// Node that runs this test, pointed at the XenoNet under test. Each keeps its
// token in a file of its own outside any repo, readable only by you.
const PORT_FLAG = PORT === 47842 ? "--dev" : PORT === 47841 ? "--demo" : `--port ${PORT}`;
const XENONET = `${process.execPath} ${path.join(ROOT, "cli", "xenonet.js")} ${PORT_FLAG}`;
const TOKEN = (slug) => path.join(os.homedir(), ".config", "xenonet", "actors", `${slug}.token`);
const AGENTS = {
  "grok-bot": { name: "Grok Bot", other: "muse" },
  muse: { name: "Muse", other: "grok-bot" },
};
const register = (slug) => `${XENONET} actor register --product ${slug} --name "${AGENTS[slug].name}" --capabilities message-agents --heartbeat 3600 --token-file ${TOKEN(slug)}`;
const asActor = (slug, rest) => `XENONET_ACTOR_TOKEN_FILE=${TOKEN(slug)} ${XENONET} actor ${rest}`;
const said = (from) => `${WORD}: ${AGENTS[from].name} to ${AGENTS[AGENTS[from].other].name}`;
// How each message came, as XenoNet wrote it down (its envelope's method).
const HOW = { gateway: "gateway", local: "token" };

// What to ask an agent: through the gateway, its own XenoNet tools; locally,
// the xenonet command.
const ask = (slug, op, to = null) =>
  ROUTE === "gateway"
    ? op === "send"
      ? `With your XenoNet tools (the gateway), send actor:${to} the message "${said(slug)}".`
      : "With your XenoNet tools (the gateway), read your inbox and tell me what's there."
    : op === "send"
      ? asActor(slug, `send actor:${to} "${said(slug)}"`)
      : asActor(slug, "inbox");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function api(route) {
  const res = await fetch(`${BASE}${route}`, { signal: AbortSignal.timeout(10e3) });
  if (!res.ok) throw new Error(`${route} answered ${res.status}`);
  return res.json();
}
// Asks `fn` every 2 s until it answers, for up to WAIT_MS.
async function until(what, fn) {
  const end = Date.now() + WAIT_MS;
  for (;;) {
    const got = await fn().catch(() => null);
    if (got) return got;
    if (Date.now() > end) assert.fail(`waited ${WAIT_MS / 60e3} minutes for ${what}`);
    await sleep(2000);
  }
}
const say = (lines) => console.log(["", ...lines, ""].join("\n"));
const actor = async (slug) => (await api("/api/actors")).actors.find((a) => a.id === `actor:${slug}`) ?? null;
// The message from one actor to the other carrying this run's word, once
// the recipient has read it.
const read = (from, to) => async () => {
  const { messages } = await api("/api/engine/state");
  const m = messages.find((x) => x.from === `actor:${from}` && x.text.includes(said(from)) && x.to.some((d) => d.id === `actor:${to}`));
  return m && m.to.find((d) => d.id === `actor:${to}`).seen ? m : null;
};
// The gateway's address, from XenoNet when it says, else from the setting.
async function gatewayUrl() {
  const st = await api("/api/actors/gateway").catch(() => null);
  return st?.url ?? process.env.XENONET_REAL_GATEWAY_URL ?? "https://<the gateway's address>/mcp";
}
const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return "<the gateway's host>";
  }
};

test("XenoNet is running, and isn't a demo", async () => {
  const health = await api("/api/health");
  assert.equal(health.demo, false, "a demo's actors are made up");
  say([`Testing the XenoNet on port ${PORT} (${health.version ?? "unknown version"}), ${ROUTE === "gateway" ? "through the actor gateway" : "with local commands"}. This run's word: ${WORD}`]);
});

test("XenoNet observes both apps on this Mac", async () => {
  for (const slug of Object.keys(AGENTS)) {
    const a = await until(`actor:${slug} (is ${AGENTS[slug].name}'s app installed, and its observer on?)`, () => actor(slug));
    assert.equal(a.product?.id, slug);
    assert.ok(a.via.includes("observed"), `${slug} is observed`);
  }
});

test("through the gateway: each has a gateway token from XenoNet's window, so it reports itself", { skip: ROUTE !== "gateway", timeout: 2 * WAIT_MS + 60e3 }, async () => {
  const url = await gatewayUrl();
  say([
    "In XenoNet's window, Actors:",
    "  - under actor gateway, Set up…: pick a tunnel this computer has, then Use it, and wait for ✓ reachable;",
    "  - Connect… on Grok Bot, then on Muse: a token for each, with the message to give each app.",
    `Grok Bot: paste its message into a new chat (an MCP server by URL, ${url}, with the token in its header).`,
    `Muse: paste its message into a chat, approve the connector card for ${hostOf(url)}, and paste the token into the card's key field.`,
  ]);
  for (const slug of Object.keys(AGENTS)) {
    const done = await until(`${AGENTS[slug].name} to report itself (a gateway token makes it)`, async () => {
      const a = await actor(slug);
      return a?.via.includes("registered") ? a : null;
    });
    assert.ok(done.capabilities.includes("message-agents"), `${slug} may message agents`);
  }
});

test("locally: each registers from a command its own app runs, and you allow it", { skip: ROUTE !== "local", timeout: 2 * WAIT_MS + 60e3 }, async () => {
  for (const slug of Object.keys(AGENTS)) {
    const a = await actor(slug);
    if (a.via.includes("registered")) continue;
    say([`Ask ${AGENTS[slug].name} to run this on your Mac, then allow it in XenoNet's window:`, `  ${register(slug)}`]);
    const done = await until(`${AGENTS[slug].name} to register`, async () => {
      const now = await actor(slug);
      return now?.via.includes("registered") ? now : null;
    });
    assert.ok(done.capabilities.includes("message-agents"), `${slug} may message agents`);
  }
});

test("Grok Bot sends Muse a message, and Muse reads it", { timeout: WAIT_MS + 60e3 }, async () => {
  say(["Ask Grok Bot:", `  ${ask("grok-bot", "send", "muse")}`, "Then ask Muse:", `  ${ask("muse", "inbox")}`]);
  const m = await until("Muse to read Grok Bot's message", read("grok-bot", "muse"));
  assert.equal(m.target, "actor:muse");
  assert.equal(m.how, HOW[ROUTE], `it came ${ROUTE === "gateway" ? "through the gateway" : "from a command under Grok Bot"}`);
});

test("Muse answers, and Grok Bot reads the answer", { timeout: WAIT_MS + 60e3 }, async () => {
  say(["Ask Muse:", `  ${ask("muse", "send", "grok-bot")}`, "Then ask Grok Bot:", `  ${ask("grok-bot", "inbox")}`]);
  const m = await until("Grok Bot to read Muse's answer", read("muse", "grok-bot"));
  assert.equal(m.target, "actor:grok-bot");
  assert.equal(m.how, HOW[ROUTE]);
  // XenoNet wrote down who sent each, from the token and the route it came
  // by, not from what the message says.
  const { messages } = await api("/api/engine/state");
  const pair = messages.filter((x) => x.text.includes(WORD)).map((x) => [x.from, x.target, x.how]);
  assert.deepEqual(pair, [
    ["actor:grok-bot", "actor:muse", HOW[ROUTE]],
    ["actor:muse", "actor:grok-bot", HOW[ROUTE]],
  ]);
});
