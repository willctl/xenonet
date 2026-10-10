// The actor gateway in the Actors panel (actors-panel.js;
// server/actor-gateway.js): the gateway at a glance, each actor's app
// connection under its row, picking the tunnel, and connecting an app:
// a question first, then numbered steps, each with its Copy, that wait for
// the app's first call (core/actors/handoff.js).
//
// Markup only. The panel keeps the gateway's state, reads it again and
// answers the buttons; what it knows (whether this window can change
// anything, an actor's name) comes in as arguments.
import { escapeHtml as h, timeAgo } from "./format.js";
import { handoff } from "../core/actors/handoff.js";
import { keepSecret, secretHtml } from "./secret.js";

const REFUSED_WORDS = { rotated: "replaced", removed: "taken away", expired: "expired" };
const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// An address, hidden while identities are (Hide identities): a tunnel's
// name can carry the computer's.
const addrHtml = (text, host = null) => `<span class="addr-real">${h(text)}</span><span class="addr-masked">${h(host ? String(text).split(host).join("•••") : "address hidden")}</span>`;
const hostIn = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
};

// Whether an app has called since its token was made (`v.madeAt`), from its
// connection `a` (the gateway's record of it).
export const connected = (a, v) => !!a?.lastAt && a.lastAt >= (v.madeAt ?? 0) - 2000;
export const waitWords = (done, name) => (done ? `${name} is connected` : `Waiting for ${name}'s first call…`);

// The gateway at a glance: a dot and a word, its address, and the one
// thing to do next.
export function gatewayHtml(g, { canDecide }) {
  const yours = canDecide ? "" : "disabled";
  const t = g.tunnel ?? {};
  const r = g.reach ?? {};
  const btn = (attr, label, { primary = false, title = "" } = {}) => `<button class="act${primary ? " primary" : ""}" ${attr} data-yours ${yours}${title ? ` title="${h(title)}"` : ""}>${label}</button>`;
  if (!t.kind)
    return `<div class="ac-gw" data-state="none"><div class="ac-gw-line"><span class="ac-gw-dot"></span><span class="ac-gw-word">Not set up</span><span class="grow"></span>${btn("data-ac-gateway-setup", "Set up…", { primary: true })}${g.on ? `<button class="toggle on" data-ac-gateway-on aria-pressed="true" data-yours ${yours}>On</button>` : ""}</div>
        <div class="ac-gw-sub">SI in other clouds, such as Muse or Grok Bot, can message the sessions here.</div></div>`;
  const state = !g.on ? "off" : g.problem ? "bad" : r.state === "ok" ? "ok" : "wait";
  const word = { off: "Off", bad: "Not reachable", ok: "Online", wait: "Checking…" }[state];
  const addr = g.url ? `<code class="ac-gw-addr">${addrHtml(g.url, hostIn(g.url))}</code>` : "";
  const check = `<button class="ac-link" data-ac-gateway-check data-yours ${yours} title="Check the address now">Check now</button>`;
  const sub =
    state === "bad"
      ? `<span class="ac-gw-why" data-ac-gateway-problem>${g.host ? addrHtml(g.problem, g.host) : h(g.problem)}</span>`
      : [t.through && `via ${h(t.through)}`, t.stable === false && `<span class="warn">new address each start</span>`, state === "ok" && r.at && `checked <span data-since="${h(r.at)}">${timeAgo(r.at)}</span>${r.local ? " on this computer" : ""}`, g.calls ? h(plural(g.calls, "call")) : null, state === "ok" && check].filter(Boolean).join(" · ");
  const acts = [
    state === "bad" ? btn("data-ac-gateway-repair", "Repair", { primary: true, title: "Set the tunnel up again, and check" }) : "",
    btn("data-ac-gateway-setup", "Change…", { title: "Pick another tunnel" }),
    `<button class="toggle ${g.on ? "on" : ""}" data-ac-gateway-on aria-pressed="${g.on}" data-yours ${yours}>${g.on ? "On" : "Off"}</button>`,
  ].join("");
  return `<div class="ac-gw" data-state="${state}" data-ac-reach="${h(r.state ?? "")}"><div class="ac-gw-line"><span class="ac-gw-dot"></span><span class="ac-gw-word">${word}</span>${addr}<span class="grow"></span>${acts}</div>${sub ? `<div class="ac-gw-sub">${sub}</div>` : ""}</div>`;
}

// An actor's app connection through the gateway, under its row: what
// needs you, with its fix beside it, else its last call.
export function gateHtml(a) {
  if (!a) return "";
  const day = (t) => new Date(t).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const fix = (attr, label) => ` <button class="ac-link" ${attr}>${label}</button>`;
  const warn = (text) => `<div class="ac-gate warn" data-ac-gate="warn">${text}</div>`;
  if (a.refused) return warn(`Its app uses a gateway token that was ${h(REFUSED_WORDS[a.refused.reason] ?? "refused")}.${fix(`data-ac-gateway="${h(a.id)}"`, "Reconnect")}`);
  if (a.oldAddress) return warn(`Its app has the gateway's old address.${fix(`data-ac-gateway-moved="${h(a.id)}"`, "Send the new one")}`);
  if (a.token && a.expiring) return warn(`Its gateway token expires ${h(day(a.expiresAt))} unless its app calls.`);
  return `<div class="ac-gate" data-ac-gate="ok">gateway: ${a.lastAt ? `last call <span data-since="${h(a.lastAt)}">${timeAgo(a.lastAt)}</span>` : "no calls yet"}</div>`;
}

// Picking the tunnel: the ones this computer has first, each a line; what
// one needs, and what it asks for, once it's picked.
export function gatewaySetupHtml(v, gateway) {
  const kinds = gateway?.kinds ?? [];
  const pick = v.pick ?? {};
  const port = gateway?.port ?? "";
  const card = (k) => {
    const on = pick.kind === k.kind;
    const badge = gateway?.tunnel?.kind === k.kind ? `<span class="ac-badge on">In use</span>` : k.tool && k.found ? `<span class="ac-badge">Installed</span>` : "";
    const fields = [
      k.asks?.name ? `<label class="ac-field"><span class="ac-k">Tunnel</span><input class="ac-input" data-ac-tunnel-name value="${h(pick.name ?? "")}" placeholder="xenonet" spellcheck="false" autocomplete="off"></label>` : "",
      k.asks?.host ? `<label class="ac-field"><span class="ac-k">${k.kind === "ngrok" ? "Domain" : "Address"}</span><input class="ac-input" data-ac-tunnel-host value="${h(pick.host ?? "")}" placeholder="${k.kind === "ngrok" ? "optional: your-name.ngrok-free.app" : "xenonet.example.com"}" spellcheck="false" autocomplete="off"></label>` : "",
    ].join("");
    const more = on
      ? `<div class="ac-pick-more"><p>${h(k.needs.replace("the gateway's port", `port ${port}`))}${k.docs ? ` <a href="${h(k.docs)}" target="_blank" rel="noreferrer">Guide ↗</a>` : ""}</p>${k.note ? `<p class="dim">${h(k.note)}</p>` : ""}${fields ? `<div class="ac-form">${fields}</div>` : ""}</div>`
      : "";
    return `<li class="ac-pick${on ? " on" : ""}"><button class="ac-pick-head" data-ac-tunnel-kind="${h(k.kind)}" aria-pressed="${on}"><span class="ac-radio"></span><span class="ac-pick-name">${h(k.label)}</span><span class="ac-pick-short">${h(k.short ?? "")}</span>${badge}</button>${more}</li>`;
  };
  const here = kinds.filter((k) => k.found);
  const missing = kinds.filter((k) => !k.found);
  const label = kinds.find((k) => k.kind === pick.kind)?.label;
  return `<ol class="ac-picks">${here.map(card).join("")}</ol>
      ${missing.length ? `<details class="ac-more"${missing.some((k) => k.kind === pick.kind) ? " open" : ""}><summary>Not installed (${missing.length})</summary><ol class="ac-picks">${missing.map(card).join("")}</ol></details>` : ""}
      <p class="ac-error bad" data-ac-error hidden></p>
      <div class="m-foot"><button class="act" data-ac-back>Cancel</button>${gateway?.tunnel?.kind ? `<button class="ac-link" data-ac-tunnel-none data-yours title="The gateway answers on this computer only">Remove the tunnel</button>` : ""}<span class="grow"></span><button class="act primary" data-ac-tunnel-use data-yours ${pick.kind ? "" : "disabled"}>${pick.kind === "own" ? "Use this address" : label ? `Use ${h(label)}` : "Pick one"}</button></div>`;
}
export const readPick = (body, pick) => ({ ...pick, ...(body.querySelector("[data-ac-tunnel-name]") ? { name: body.querySelector("[data-ac-tunnel-name]").value.trim() } : {}), ...(body.querySelector("[data-ac-tunnel-host]") ? { host: body.querySelector("[data-ac-tunnel-host]").value.trim() } : {}) });

// Connecting an actor's app through the gateway: a question first; then,
// once, numbered steps with what each copies, and a last one that waits
// for the app's first call; and, when the gateway's address changed, the
// steps that give an app the new one. `a` is the actor, if XenoNet knows it,
// and `again` whether its app has a gateway token already.
export function gatewayAskHtml(v, { gateway, a, name: who, again }) {
  const name = h(who);
  if (!gateway?.on || !gateway?.url)
    return `<p class="ac-lead">Set up the gateway first</p><p class="pj-note">${name}'s app needs an address on the internet to reach.</p>
        <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary" data-ac-gateway-setup data-yours>Set up…</button></div>`;
  const joins = a && !a.via?.includes("registered") ? ` ${name} reports itself from then on.` : "";
  return `<p class="ac-lead">Let ${name} message SI here from its cloud?</p>
      <p class="pj-note">XenoNet makes a token for ${name}'s app${again ? ", and the old one stops working" : ""}.${joins}</p>
      <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary" data-ac-gateway-make="${h(v.id)}" data-yours>${again ? "Make a new token" : "Connect"}</button></div>`;
}

// The steps, each with its Copy and what it copies: the message (masked
// when it carries the token), the token, or the address.
function stepsHtml(how, { url, v = null, name = "", done = false }) {
  const host = hostIn(url);
  const copy = { message: v?.token ? `data-secret-copy="message"` : `data-ac-gateway-copy="${h(how.message ?? "")}" data-ac-copied="Message copied"`, token: `data-secret-copy="token"`, url: `data-ac-gateway-copy="${h(url)}" data-ac-copied="Address copied"` };
  const label = { message: "Copy message", token: "Copy token", url: "Copy URL" };
  const body = {
    message: () => (v?.token ? secretHtml("message", { attrs: "data-ac-handoff", block: true }) : `<pre class="ac-message" data-ac-handoff>${addrHtml(how.message, host)}</pre>`),
    token: () => secretHtml("token", { attrs: "data-ac-token-text" }),
    url: () => `<code class="ac-url">${addrHtml(url, host)}</code>`,
  };
  const step = (s, i) => `<li><div class="ac-step-head"><span class="ac-step-text">${h(s.text)}</span>${s.copy ? `<button class="act${i === 0 ? " primary" : ""}" ${copy[s.copy]}>${label[s.copy]}</button>` : ""}</div>${s.copy ? body[s.copy]() : ""}</li>`;
  const items = how.steps.filter((s) => s.copy !== "wake").map(step);
  if (v?.token) items.push(`<li class="ac-wait" data-ac-wait="${done ? "done" : "waiting"}"><span class="ac-step-text">${h(waitWords(done, name))}</span></li>`);
  // Then, once it's connected, a way to wake it.
  for (const s of how.steps.filter((x) => x.copy === "wake")) items.push(`<li><div class="ac-step-head"><span class="ac-step-text">${h(s.text)}</span><button class="act" data-ac-wake="${h(v?.id ?? "")}">Set up its wake…</button></div></li>`);
  return `<ol class="ac-steps">${items.join("")}</ol>`;
}

// The steps for an app given a token (`v.token`, kept as a secret for the
// page to show and copy), at the gateway's address `url`: `product` is the
// actor's, `name` what to call it when its app has no name of its own,
// and `done` whether the app has called yet.
export function gatewayTokenHtml(v, { url, product, name, done }) {
  const how = handoff(product, { url, token: v.token });
  keepSecret("token", v.token);
  if (how.message) keepSecret("message", v.token, how.message);
  return `${stepsHtml(how, { url, v, name: how.app ?? name, done })}
      <div class="m-foot"><span class="pj-foot">XenoNet keeps only its hash, so it can't show it again.</span><span class="grow"></span><button class="act${done ? " primary" : ""}" data-ac-done>Done</button></div>`;
}

// The steps that give an app the gateway's new address; its token still
// works.
export function gatewayMovedHtml(url, product) {
  return `${stepsHtml(handoff(product, { url }), { url })}
      <div class="m-foot"><span class="grow"></span><button class="act" data-ac-done>Done</button></div>`;
}
