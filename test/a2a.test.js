// A2A agents through the gateway, as the window shows them: the steps that
// give an agent that speaks A2A its token and XenoNet's Agent Card
// (core/actors/handoff.js), and the Actors panel's switch with the card's
// address and the agents that called (src/a2a-panel.js). The protocol
// itself is tested in Rust (crates/xenonet-service/src/a2a/tests.rs, and
// tests/a2a.rs against test/fixtures/a2a/cases.json).
import { test } from "node:test";
import assert from "node:assert/strict";
import { handoff } from "../core/actors/handoff.js";
import { a2aHtml } from "../src/a2a-panel.js";

const CARD = "https://gw.example.com/.well-known/agent-card.json";

test("an A2A agent is given the card's address and the token as its bearer", () => {
  const h = handoff("a2a", { url: CARD, token: "xenonet_gateway_x" });
  assert.equal(h.app, "A2A agent");
  assert.deepEqual(
    h.steps.map((s) => s.copy),
    ["url", "token"],
  );
  assert.match(h.steps[1].text, /Authorization: Bearer <token>/);
  assert.equal(h.message, null, "nothing to paste into a chat, so the token stays out of one");
  const moved = handoff("a2a", { url: CARD });
  assert.deepEqual(
    moved.steps.map((s) => s.copy),
    ["url"],
  );
  assert.match(moved.steps[0].text, /token still works/);
});

test("the panel's switch says it's off, and shows nothing more", () => {
  const html = a2aHtml({ on: false, card: CARD, agents: [{ id: "actor:wren", name: "Wren", lastAt: 1, target: null }] }, { canDecide: true });
  assert.match(html, /Let A2A agents reach XenoNet/);
  assert.match(html, /data-ac-a2a-on aria-pressed="false"/);
  assert.match(html, /Each A2A agent is an actor with its own token and scopes/);
  assert.doesNotMatch(html, /agent-card\.json/);
  assert.doesNotMatch(html, /data-ac-a2a-agent/);
});

test("on, it shows the card's address and each agent's default target", () => {
  const html = a2aHtml({ on: true, card: CARD, agents: [{ id: "actor:wren", name: "Wren <3", lastAt: Date.now(), target: "claude:x" }] }, { canDecide: true });
  assert.match(html, /data-ac-a2a-card>https:\/\/gw\.example\.com\/\.well-known\/agent-card\.json</);
  assert.match(html, /data-ac-gateway-copy="https:\/\/gw\.example\.com\/\.well-known\/agent-card\.json"/);
  assert.match(html, /data-ac-a2a-target="actor:wren" value="claude:x"/);
  assert.match(html, /Wren &lt;3/, "names are escaped");
  const noCard = a2aHtml({ on: true, card: null, agents: [] }, { canDecide: false });
  assert.match(noCard, /comes with the gateway's/);
  assert.match(noCard, /data-ac-a2a-on aria-pressed="true" data-yours disabled/, "a page that can't change it can only look");
});
