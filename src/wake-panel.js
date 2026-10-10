// How an actor that reports itself is woken when a message arrives
// (core/actors/wake.js), in the Actors panel (actors-panel.js): a word in
// its row, and the Wake view, which sets a webhook, a notification to you,
// or nothing. Never a webhook's URL or key, which stay in the credential
// store.
//
// Markup only. The panel reads the wakes (`w`, one actor's, from
// /api/actors/wakes), holds what's typed in the Wake view, and answers the
// buttons.
import { escapeHtml as h, timeAgo } from "./format.js";

const WAKE_WORDS = { webhook: "wake: webhook", notify: "wake: notifies you" };

// An actor's wake in its row, with Wake… to set it; a webhook whose last
// call failed says why, with Fix.
export function wakeHtml(id, w) {
  const set = `<button class="ac-link" data-ac-wake="${h(id)}" title="How XenoNet wakes it when a message arrives">Wake…</button>`;
  if (!w) return set;
  if (w.kind === "webhook" && w.last && !w.last.ok) return `<div class="ac-gate warn" data-ac-wake-state="failed">Its last wake failed: ${h(w.last.why)}. <button class="ac-link" data-ac-wake="${h(id)}">Fix</button></div>`;
  const words = w.listening ? `<span data-ac-wake-state="listening">listening</span>` : WAKE_WORDS[w.kind] ? `<span data-ac-wake-state="${h(w.kind)}">${WAKE_WORDS[w.kind]}${w.kind === "webhook" && w.lastAt ? `, last <span data-since="${h(w.lastAt)}">${timeAgo(w.lastAt)}</span>` : ""}</span>` : "";
  return `${words}${set}`;
}

// Waking an actor: a webhook (for Grok Bot, a routine's, with how to make
// one: `help`, from wakeHelp in core/actors/handoff.js), a notification to
// you, or nothing. A webhook's key is typed into a field that hides it, and
// the panel holds it only until it's saved: of what was typed before a
// redraw (`draft`), only the URL goes back into the page.
export function wakeViewHtml(v, { wake = null, help = null, draft }) {
  const w = wake ?? { kind: "none", listening: 0 };
  const pick = v.pick ?? w.kind;
  const kept = w.kind === "webhook" && w.host;
  const fields = `<div class="ac-form">
        <label class="ac-field"><span class="ac-k">URL</span><input class="ac-input" data-ac-wake-url value="${h(draft.url)}" placeholder="${kept ? "unchanged" : "https://…"}" spellcheck="false" autocomplete="off"></label>
        <label class="ac-field"><span class="ac-k">Key</span><input class="ac-input" type="password" data-ac-wake-key placeholder="${kept ? "unchanged" : "its webhook key"}" spellcheck="false" autocomplete="off"></label>
      </div>`;
  const how = help
    ? `<ol class="ac-steps">${help.steps.map((t, i) => `<li><div class="ac-step-head"><span class="ac-step-text">${h(t)}</span>${i === 0 ? `<button class="act" data-ac-gateway-copy="${h(help.message)}" data-ac-copied="Message copied">Copy message</button>` : ""}</div>${i === 0 ? `<pre class="ac-message">${h(help.message)}</pre>` : fields}</li>`).join("")}</ol>`
    : fields;
  const opts = [
    ["webhook", "Webhook", help ? "A Grok Bot routine that starts from a webhook" : "XenoNet calls a URL you give it"],
    ["notify", "Notify me", "A notification when messages wait"],
    ["none", "Nothing", "It reads its inbox when it checks"],
  ];
  const card = ([kind, label, short]) => {
    const on = pick === kind;
    const more =
      on && kind === "webhook"
        ? `<div class="ac-pick-more">${kept ? `<p>Calls <code>${h(w.host)}</code>; its key is kept in your credential store. Enter both again to change them.</p>` : ""}${how}${help ? "" : `<p class="dim">XenoNet sends the key as Authorization: Bearer, or signs with it (Standard Webhooks) when it starts whsec_. The call says how many messages wait, nothing more.</p>`}</div>`
        : "";
    return `<li class="ac-pick${on ? " on" : ""}"><button class="ac-pick-head" data-ac-wake-kind="${kind}" aria-pressed="${on}"><span class="ac-radio"></span><span class="ac-pick-name">${label}</span><span class="ac-pick-short">${short}</span>${w.kind === kind && w.set !== false ? `<span class="ac-badge on">In use</span>` : ""}</button>${more}</li>`;
  };
  const listening = w.listening ? `<p class="pj-note">A program of its own is listening, so messages reach it at once.</p>` : "";
  const result = v.result ? `<p class="pj-note${v.result.ok ? "" : " bad"}" data-ac-wake-result>${v.result.ok ? `✓ ${h(w.host ?? "Its webhook")} answered HTTP ${h(v.result.status)}` : h(v.result.why ?? "That didn't work")}</p>` : "";
  return `${listening}<ol class="ac-picks">${opts.map(card).join("")}</ol>${result}
      <p class="ac-error bad" data-ac-error hidden></p>
      <div class="m-foot"><button class="act" data-ac-back>${v.saved ? "Done" : "Cancel"}</button>${w.kind === "webhook" ? `<button class="act" data-ac-wake-test data-yours title="Call its webhook now">Test</button>` : ""}<span class="grow"></span><button class="act primary" data-ac-wake-save="${h(v.id)}" data-yours>Save</button></div>`;
}

// What's typed in the Wake view, else what was typed before (`draft`).
export const readWake = (body, draft) => ({ url: body.querySelector("[data-ac-wake-url]")?.value.trim() ?? draft.url, key: body.querySelector("[data-ac-wake-key]")?.value.trim() ?? draft.key });
