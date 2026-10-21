// A2A in the Actors panel (actors-panel.js; crates/xenonet-service/src/a2a):
// the switch that lets agents that speak A2A reach XenoNet through the
// gateway, the Agent Card's address to give them, and each agent that has
// called, with the target its messages go to when they name none.
//
// Markup only. The panel reads /api/a2a, which is the owner's, and answers
// the buttons; a page that can't read it shows nothing of this.
import { escapeHtml as h, timeAgo } from "./format.js";

// What an agent's message goes to when it names no target.
const TARGET_HINT = "claude:…, codex:… or repo:<name>";

// The switch, the card's address and the agents, for `a2a` as /api/a2a
// answers it; `canDecide` is whether this window may change it.
export function a2aHtml(a2a, { canDecide }) {
  const yours = canDecide ? "" : "disabled";
  const toggle = `<button class="toggle ${a2a.on ? "on" : ""}" data-ac-a2a-on aria-pressed="${a2a.on}" data-yours ${yours}>${a2a.on ? "On" : "Off"}</button>`;
  const lines = [`<div class="ac-gw-sub">Each A2A agent is an actor with its own token and scopes. Connect… on its actor gives it a token.</div>`];
  if (a2a.problem) lines.push(`<div class="ac-gw-sub bad">${h(a2a.problem)}</div>`);
  if (a2a.on && a2a.card)
    lines.push(
      `<div class="ac-gw-sub">Agent Card <code class="ac-gw-addr" data-ac-a2a-card>${h(a2a.card)}</code> <button class="ac-link" data-ac-gateway-copy="${h(a2a.card)}" data-ac-copied="Card address copied">Copy</button></div>`,
    );
  else if (a2a.on) lines.push(`<div class="ac-gw-sub">The card's address comes with the gateway's: set the gateway up to give agents one.</div>`);
  if (a2a.on)
    for (const g of a2a.agents ?? [])
      lines.push(
        `<div class="ac-gw-sub ac-a2a-agent" data-ac-a2a-agent="${h(g.id)}"><span>${h(g.name ?? g.id)}</span> <span class="dim">· last call <span data-since="${h(g.lastAt)}">${timeAgo(g.lastAt)}</span> · its default target</span> <input class="ac-input" data-ac-a2a-target="${h(g.id)}" value="${h(g.target ?? "")}" placeholder="${h(TARGET_HINT)}" spellcheck="false" autocomplete="off" ${yours}> <button class="ac-link" data-ac-a2a-save="${h(g.id)}" data-yours ${yours}>Save</button></div>`,
      );
  return `<div class="ac-gw" data-ac-a2a data-state="${a2a.on ? "on" : "off"}"><div class="ac-gw-line"><span class="ac-gw-word">Let A2A agents reach XenoNet</span><span class="grow"></span>${toggle}</div>${lines.join("")}</div>`;
}
