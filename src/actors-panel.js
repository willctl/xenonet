// The Actors panel (docs/design/actors/interface.md, sections 2 and 7):
//
//   - the registrations waiting for you, each with what it asks and who
//     asked, and Allow and Deny;
//   - the actors beside the map, each name showing it on the map, with how
//     XenoNet knows each one: how its token stands, New token and Remove for
//     those that report themselves, and Show me on the map for you;
//   - the observers' switches, one per product XenoNet looks at;
//   - the actor gateway at a glance (a dot, a word, its address, the one
//     thing to do next), the tunnel you pick for it (Set up…, Change…), and
//     for each actor that uses it, how its app's connection stands, with
//     its fix beside it, and Connect…: numbered steps, each with its Copy,
//     that wait for the app's first call (gateway-panel.js);
//   - how each actor that reports itself is woken when a message arrives
//     (core/actors/wake.js, wake-panel.js), in its row, and Wake… to set
//     it: a webhook, whose URL and key go to the credential store, a
//     notification, or nothing;
//   - remote connections (remote-panel.js): who reads what through each,
//     Revoke, the switch that turns them all off, and Pair a remote
//     actor, which shows what would leave before anything is approved;
//   - Add an actor, whose token shows once.
//
// The palette's Actors opens it, and so do a request's card and its
// notification. Allowing, adding, new tokens, removing and the observers'
// switches go through the window's own routes, with the key only this
// window holds (actor-requests.js); a browser tab can only look.
//
// A token shows once, masked (secret.js): its value stays out of the
// page's HTML until you choose Show, and then for ten seconds, and Copy
// copies it unseen. It's never in the op log, a toast or the driver's
// state, and the page lets it go when the view or the panel closes, or once
// Save to a file… has handed it to the desktop app, which writes it where
// only you can read it. Rows are
// put in place rather than redrawn, so the actors' list coming in every
// few seconds never swaps a button out from under the pointer.
import "./actors.css";
import { escapeHtml as h, timeAgo } from "./format.js";
import { THEME } from "./theme.js";
import { actorActions, actorOrder, colorOf, presenceWord, PRESENCE_WORDS, CAPABILITY_WORDS, VIA_WORDS, PRODUCT_NAMES } from "./actors-model.js";
import { askedBy, asksFor, leftWords, tokenParts, JOINS_ALLOW } from "./actor-requests.js";
import { actorWhat } from "./actor-inspector.js";
import { productLine } from "./actor-words.js";
import { handoff, wakeHelp } from "../core/actors/handoff.js";
import { keepSecret, forgetSecrets, secretHtml, secretClick, hideOnBlur } from "./secret.js";
import { gatewayHtml, gateHtml, gatewaySetupHtml, readPick, gatewayAskHtml, gatewayTokenHtml, gatewayMovedHtml, connected, waitWords } from "./gateway-panel.js";
import { wakeHtml, wakeViewHtml, readWake } from "./wake-panel.js";
import { remoteSectionHtml, remoteRow, tunnelStateHtml, tunnelFootHtml } from "./remote-panel.js";
import { createRemoteViews } from "./remote-views.js";

const CAPABILITIES = Object.keys(CAPABILITY_WORDS);
const slugOf = (id) => String(id).replace(/^actor:/, "");
const presenceOf = (s) => (PRESENCE_WORDS[s.presence] ? s.presence : "unknown");
const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// `decide(id, decision)` answers a request and says so (main.js), the same
// as from its card; `onShown()` draws the map again once you've turned
// Show me on the map on or off; `tokens` is how each token stands
// (createActorTokens in actor-requests.js).
export function createActorsPanel({ modals, sfx, toast, calls, requests, decide, you, list, tokens = null, remote = null, off = () => false, onShown = () => {}, select = () => {}, desktop = null }) {
  let panel = null;
  // list, add, token ({ id, token, saved }), rotate ({ id }), remove ({ id }),
  // for the gateway gateway-setup ({ pick }), gateway ({ id }), gateway-token
  // ({ id, token, url }) and gateway-moved ({ id }), and for remote
  // connections pair, preview ({ preview }), paired ({ connection }) and
  // unpair ({ grant })
  let view = { name: "list" };
  // The remote connections' views and buttons (remote-views.js), with what
  // they need of the panel.
  const remoteViews = remote && createRemoteViews(remote, { view: () => view, go, render, drawList, act, sfx, toast });
  let canDecide = false;
  let observers = null;
  let ticker = null;
  let armTimer = null;
  // Whether what the list needs (the key, the observers, the requests, the
  // tokens) has come in since the panel opened.
  let loaded = false;
  // The form's fields between a refusal and the next try.
  let draft = { name: "", slug: "", capabilities: ["run-commands"], beat: 60 };

  // Actors removed here, left out until the list comes without them.
  const removed = new Set();

  const isOpen = () => !!panel?.isConnected;
  function actors() {
    const all = list()?.actors ?? [];
    for (const id of removed) if (!all.some((a) => a.id === id)) removed.delete(id);
    return all.filter((a) => !removed.has(a.id)).sort(actorOrder);
  }
  const actorNamed = (id) => actors().find((a) => a.id === id) ?? null;
  const nameOf = (id) => {
    const a = actorNamed(id);
    return a ? (a.kind === "human" ? "You" : a.name) : id;
  };
  const observedProducts = () => new Set((observers?.observers ?? []).map((o) => o.product));

  // The actors that report themselves when their tokens were read last:
  // they're read again once that changes, as allowing, adding and removing
  // one do, and after a new token.
  let tokensFor = null;
  const reporting = () => actors().filter((a) => a.via?.includes("registered")).map((a) => a.id).sort().join(" ");
  async function readTokens() {
    tokensFor = reporting();
    await tokens?.refresh();
  }

  // ---------- the list ----------

  // What a row says now, apart from its buttons: whether it's there, what
  // it's doing, and when XenoNet last heard from it.
  function liveHtml(s, actions) {
    if (actions.you && !you.shown()) return `<span class="presence offline">off the map</span> <span class="dim">· this window reports nothing about you, and you're not drawn</span>`;
    const pres = presenceOf(s);
    const status = s.status ? ` · <span class="ac-status">${h(s.status)}</span>` : "";
    const seen = s.lastSeen ? ` <span class="dim">· seen <span data-since="${h(s.lastSeen)}">${timeAgo(s.lastSeen)}</span></span>` : "";
    const need = s.needsYou ? ` · <span class="ac-need">◉ waiting for you</span>` : "";
    return `<span class="presence ${pres}">${h(presenceWord(s))}</span>${need}${status}${seen}`;
  }

  function viaHtml(s, actions) {
    if (actions.you) return "Only this window sees what you do. SIs see a placeholder.";
    const via = (s.via ?? []).filter((v) => v !== "demo" || s.via.length === 1);
    const words = via.map((v) => VIA_WORDS[v] ?? v);
    if (s.via?.includes("demo") && s.via.length > 1) words.push(VIA_WORDS.demo);
    return h(words.join(" · ") || "XenoNet doesn't say how it knows it");
  }

  function actsHtml(s, actions) {
    if (actions.you) return `<button class="toggle ${you.shown() ? "on" : ""}" data-ac-you aria-pressed="${you.shown()}" title="Off, this window reports nothing about you, and you're not drawn">Show me on the map</button>`;
    const again = !!gateOf(s.id)?.token;
    return [
      actions.gateway ? `<button class="act" data-ac-gateway="${h(s.id)}" title="${again ? "A new gateway token for its app, and what to tell it" : "Let its app reach XenoNet from outside this machine"}">${again ? "Reconnect…" : "Connect…"}</button>` : "",
      actions.token ? `<button class="act" data-ac-token="${h(s.id)}" data-yours title="A new token, shown once; the old one stops working at once">New token</button>` : "",
      actions.remove ? `<button class="act" data-ac-remove="${h(s.id)}" title="Its token stops working, and it leaves XenoNet">Remove</button>` : "",
    ].join("");
  }

  // How its token stands, for an actor that reports itself: whether it
  // works, when it expires and when it was last used, that age kept up to
  // date by the ticker. Nothing in a window without the key.
  function tokenHtmlFor(t) {
    const p = tokenParts(t);
    if (!p) return "";
    const used = p.used != null ? `last used <span data-since="${h(p.used)}">${timeAgo(p.used)}</span>` : p.usedWords && h(p.usedWords);
    return `<div class="ac-token-state ${h(t.state)}" data-ac-token-state="${h(t.state)}">${[h(p.state), p.expires && h(p.expires), used].filter(Boolean).join(" · ")}</div>`;
  }

  function actorRow(s) {
    const actions = actorActions(s, { canDecide, observed: observedProducts() });
    const color = colorOf(s, THEME);
    // What it is, with its maker and version; a product XenoNet doesn't know
    // says they're its own words (actor-words.js).
    const what = actions.you ? "the person at this console" : (productLine(s.product) ?? actorWhat(s));
    // Its name shows it on the map, as Show did.
    const title = actions.you ? `<span class="ac-title">You</span>` : `<button class="ac-title ac-show" data-ac-show="${h(s.id)}" title="Show it on the map">${h(s.name)}</button>`;
    const head = `<div class="ac-name">${title}<span class="ac-what">${h(what)}</span></div>`;
    // The row is drawn again when its token changes, not as its ages pass.
    const t = tokens?.of(s.id) ?? null;
    const gate = gateHtml(gateOf(s.id));
    const wake = canDecide && !actions.you && s.via?.includes("registered") ? wakeHtml(s.id, wakeOf(s.id)) : "";
    const stat = `${head}<div class="ac-via">${viaHtml(s, actions)}</div>|${actsHtml(s, actions)}|${color}|${t ? `${t.state}|${t.expiresAt}|${t.usedAt}` : ""}|${gate}|${wake}`;
    return {
      key: s.id,
      sig: stat,
      live: liveHtml(s, actions),
      // How XenoNet knows it, its token, its gateway connection and how it's
      // woken, on one line.
      html: `<li class="ac-row" style="--c:${color}"><span class="ac-dot"></span><div class="ac-main">${head}<div class="ac-live"></div><div class="ac-meta"><div class="ac-via">${viaHtml(s, actions)}</div>${t ? tokenHtmlFor(t) : ""}${gate}${wake}</div></div><div class="ac-acts">${actsHtml(s, actions)}</div></li>`,
    };
  }

  function requestRow(r) {
    const who = askedBy(r.asker);
    const product = r.product?.id ? ` as ${h(PRODUCT_NAMES[r.product.id] ?? `“${r.product.id}”, its own word`)}` : "";
    const asker = who
      ? `asked by <code>${h(who.program)}</code>${who.parents.length ? `, run from ${who.parents.map((p) => `<code>${h(p)}</code>`).join(" ← ")}` : ""}${who.folder ? ` · in <code>${h(who.folder)}</code>` : ""}`
      : "XenoNet couldn't see which program asked: there's no process check here";
    // In the attention color, as its card in the strip is: it waits on you.
    const html = `<li class="ac-row ac-request" style="--c:var(--attention)"><span class="ac-dot"></span><div class="ac-main">
        <div class="ac-name"><span class="ac-title">${h(r.name)}</span><span class="ac-what">asks to join XenoNet${product}</span><span class="ac-left" data-until="${h(r.expiresAt)}">${leftWords(r.expiresAt)}</span></div>
        <div class="ac-ask">${h(asksFor(r, CAPABILITY_WORDS))}${r.joins ? ` · it joins <code>${h(r.actor)}</code>, which XenoNet knows: ${h(JOINS_ALLOW)}` : ` · it would be <code>${h(r.actor)}</code>`}</div>
        <div class="ac-asker">${asker}</div>
      </div><div class="ac-acts"><button class="act" data-ac-deny="${h(r.id)}" title="Its token never works">Deny</button><button class="act primary ac-allow" data-ac-allow="${h(r.id)}" data-yours disabled title="Its token starts working, and it joins the map">Allow</button></div></li>`;
    return { key: r.id, sig: html.replace(/<span class="ac-left"[^>]*>[^<]*<\/span>/, ""), live: null, html };
  }

  function observerRow(o) {
    const known = o.actors.map((id) => (actorNamed(id) ? nameOf(id) : id)).join(", ");
    const first = o.actors.map(actorNamed).find(Boolean);
    const html = `<li class="ac-row ac-observer" style="--c:${first ? colorOf(first, THEME) : "var(--actor)"}"><span class="ac-dot"></span><div class="ac-main"><div class="ac-name"><span class="ac-title">${h(o.name)}</span><span class="ac-what">${o.on ? (known ? h(known) : "nothing seen yet") : "XenoNet isn't looking"}</span></div></div><div class="ac-acts"><button class="toggle ${o.on ? "on" : ""}" data-ac-observe="${h(o.product)}" aria-pressed="${o.on}" ${canDecide ? "" : "disabled"}>${o.on ? "On" : "Off"}</button></div></li>`;
    return { key: o.product, sig: html, live: null, html };
  }

  // Puts rows ({ key, sig, live, html }) in `box` in order, keeping each
  // one's element while it says the same, and changing only its live part
  // when that's all that changed.
  const tpl = document.createElement("template");
  function reconcile(box, rows) {
    if (!box) return;
    const old = new Map([...box.children].map((el) => [el.dataset.key, el]));
    let prev = null;
    for (const row of rows) {
      let el = old.get(row.key);
      old.delete(row.key);
      if (!el || el.dataset.sig !== row.sig) {
        tpl.innerHTML = row.html.trim();
        const fresh = tpl.content.firstElementChild;
        fresh.dataset.key = row.key;
        fresh.dataset.sig = row.sig;
        if (el) el.replaceWith(fresh);
        el = fresh;
        el.dataset.live = "";
      }
      if (row.live != null && el.dataset.live !== row.live) {
        el.querySelector(".ac-live").innerHTML = row.live;
        el.dataset.live = row.live;
      }
      const want = prev ? prev.nextElementSibling : box.firstElementChild;
      if (want !== el) box.insertBefore(el, want);
      prev = el;
    }
    for (const el of old.values()) el.remove();
  }

  function listHtml() {
    return `
      <p class="pj-note">You, always-on SIs and programs of your own. A program joins only after you allow it.</p>
      <div class="ac-waiting" data-ac-waiting hidden><div class="d-k need" data-ac-waiting-title>waiting for you</div><ol class="ac-list" data-ac-requests></ol></div>
      <div class="d-k">beside the map</div>
      <ol class="ac-list" data-ac-actors></ol>
      <div class="d-k">observers</div>
      <p class="pj-note">XenoNet reads these apps' processes and state files, nothing more.</p>
      <ol class="ac-list" data-ac-observers></ol>
      <div data-ac-gateway-section hidden>
        <div class="d-k">actor gateway</div>
        <div data-ac-gateway-state></div>
      </div>
      <div data-ac-remote-section hidden>${remoteSectionHtml()}</div>
      <p class="pj-note ac-keyless" data-ac-keyless hidden>This page can look, but only XenoNet's own window, the desktop app, can allow, add or remove actors, switch observers or give out tokens.</p>
      <div class="m-foot">
        <button class="act" data-ac-new ${canDecide ? "" : "disabled"}>Add an actor…</button>
        <button class="act" data-rc-pair ${canDecide ? "" : "disabled"} hidden>Pair a remote actor…</button>
      </div>`;
  }

  // The actor gateway's state, for a window that can change it, read
  // again every few seconds while something about it is on its way (a
  // tunnel starting, a check), and with the ages every 15 s.
  let gateway = null;
  let gatewayTimer = null;
  async function loadGateway() {
    const r = await calls.call("GET", "/api/actors/gateway").catch(() => null);
    gateway = r?.ok ? r.data : null;
  }
  const settling = (g) => !!g?.on && (["checking", "unknown"].includes(g.reach?.state) || g.tunnel?.state === "starting");
  function watchGateway() {
    clearTimeout(gatewayTimer);
    if (!isOpen() || !settling(gateway)) return;
    gatewayTimer = setTimeout(async () => {
      await loadGateway();
      if (isOpen() && view.name === "list") drawList();
      watchGateway();
    }, 2000);
  }
  const gateOf = (id) => gateway?.actors?.find((a) => a.id === id) ?? null;

  // How each actor that reports itself is woken when a message arrives
  // (core/actors/wake.js), for a window that can change it; never a
  // webhook's URL or key, which stay in the credential store.
  let wakes = null;
  async function loadWakes() {
    const r = canDecide ? await calls.call("GET", "/api/actors/wakes").catch(() => null) : null;
    wakes = r?.ok ? r.data.wakes : null;
  }
  const wakeOf = (id) => wakes?.find((w) => w.actor === id) ?? null;
  // A webhook's URL and key as typed in the Wake view (wake-panel.js),
  // held here only until they're saved.
  let wakeDraft = { url: "", key: "" };

  // The list as it is now, put in place.
  function drawList() {
    const body = panel.querySelector(".m-body");
    if (!body.querySelector("[data-ac-actors]")) body.innerHTML = listHtml();
    const rv = requests.view();
    const waiting = rv.canDecide ? rv.requests : [];
    const box = body.querySelector("[data-ac-waiting]");
    box.hidden = !waiting.length && !(!rv.canDecide && rv.waiting);
    body.querySelector("[data-ac-waiting-title]").textContent = rv.canDecide ? "waiting for you" : `${plural(rv.waiting, "program asks", "programs ask")} to join: answer in XenoNet's own window`;
    reconcile(body.querySelector("[data-ac-requests]"), waiting.map(requestRow));
    reconcile(body.querySelector("[data-ac-actors]"), actors().map(actorRow));
    // Tokens read again once the actors that report themselves have changed.
    if (tokens && reporting() !== tokensFor) readTokens().then(() => isOpen() && view.name === "list" && drawList());
    reconcile(body.querySelector("[data-ac-observers]"), (observers?.observers ?? []).map(observerRow));
    const gate = body.querySelector("[data-ac-gateway-section]");
    gate.hidden = !gateway;
    if (gateway) body.querySelector("[data-ac-gateway-state]").innerHTML = gatewayHtml(gateway, { canDecide });
    watchGateway();
    // Remote connections, for a window that can see them.
    const remoteView = remote?.view();
    const shown = !!remoteView?.canSee && !remoteView.off;
    body.querySelector("[data-ac-remote-section]").hidden = !shown;
    body.querySelector("[data-rc-pair]").hidden = !shown;
    body.querySelector("[data-rc-pair]").disabled = !canDecide;
    if (shown) {
      const access = body.querySelector("[data-rc-access]");
      access.classList.toggle("on", !remoteView.disabled);
      access.setAttribute("aria-pressed", String(!remoteView.disabled));
      access.textContent = remoteView.disabled ? "Cloud access off" : "Cloud access on";
      reconcile(body.querySelector("[data-ac-remote]"), remoteView.connections.map((x) => remoteRow(x, { color: actorNamed(x.actor_id) ? colorOf(actorNamed(x.actor_id), THEME) : "var(--actor)", tunnel: remote.tunnelOf(x.grant_id) })));
    }
    body.querySelector("[data-ac-keyless]").hidden = canDecide;
    body.querySelector("[data-ac-new]").disabled = !canDecide;
    // You're in the list either way, and beside the map only when shown.
    const n = actors().filter((a) => a.kind !== "human" || you.shown()).length;
    panel.querySelector(".m-sub").textContent = `${plural(n, "actor")} beside the map${waiting.length ? ` · ${waiting.length} waiting for you` : ""}`;
    arm();
  }

  // Allow is ready for a click a moment after its request first showed
  // (here or in the strip), as on the card.
  function arm() {
    clearTimeout(armTimer);
    let next = Infinity;
    for (const b of panel?.querySelectorAll("[data-ac-allow]") ?? []) {
      const ready = requests.armed(b.dataset.acAllow);
      if (b.dataset.busy !== "1") b.disabled = !ready;
      if (!ready) next = Math.min(next, requests.armsIn(b.dataset.acAllow));
    }
    if (next < Infinity) armTimer = setTimeout(arm, next + 20);
  }

  // ---------- the other views ----------

  function addHtml() {
    const caps = CAPABILITIES.map((c) => `<button class="toggle ${draft.capabilities.includes(c) ? "on" : ""}" data-ac-cap="${c}" aria-pressed="${draft.capabilities.includes(c)}">${h(CAPABILITY_WORDS[c])}</button>`).join("");
    return `
      <div class="ac-form">
        <label class="ac-field"><span class="ac-k">name</span><input class="ac-input" data-ac-name maxlength="64" placeholder="Nightly backup" spellcheck="false" autocomplete="off" value="${h(draft.name)}" /></label>
        <div class="ac-field"><span class="ac-k">it may</span><div class="ac-caps">${caps}</div></div>
      </div>
      <details class="ac-more"><summary>More options</summary>
        <div class="ac-form">
          <label class="ac-field"><span class="ac-k">id</span><input class="ac-input" data-ac-slug maxlength="32" placeholder="from the name" spellcheck="false" autocomplete="off" value="${h(draft.slug)}" /><small class="dim">actor:&lt;id&gt;, in lowercase letters, digits and dashes</small></label>
          <label class="ac-field"><span class="ac-k">heartbeat</span><input class="ac-input ac-beat" data-ac-beat type="number" min="5" max="3600" step="1" value="${h(draft.beat)}" /><small class="dim">seconds between its reports</small></label>
        </div>
        <p class="pj-note">A program can also ask to join by itself: <code>xenonet actor register</code>.</p>
      </details>
      <div class="ac-error bad" data-ac-error hidden></div>
      <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary" data-ac-add data-yours>Add</button></div>`;
  }

  function tokenHtml(t) {
    const save = desktop?.saveToken ? `<button class="act" data-ac-save>Save to a file…</button>` : "";
    if (!t.token)
      return `<p class="ac-lead">Saved to <code>${h(t.saved.file)}</code></p><p class="pj-note">Only you can read it, and the page has let the token go.</p>${t.saved.warning ? `<p class="pj-note bad">${h(t.saved.warning)}</p>` : ""}
        <div class="m-foot"><span class="grow"></span><button class="act primary" data-ac-done>Done</button></div>`;
    keepSecret("token", t.token);
    return `<p class="ac-lead"><code>${h(t.id)}</code> ${t.rotated ? "has a new token" : "is ready"}</p>
      ${secretHtml("token", { attrs: "data-ac-token-text", block: true })}
      <p class="pj-note">Give it to the program in a file, as <code>XENONET_ACTOR_TOKEN_FILE</code>, never as a command-line argument.${t.rotated ? " The old one has stopped working." : ""}</p>
      <div class="m-foot">${save}<span class="grow"></span><button class="act primary" data-secret-copy="token">Copy token</button><button class="act" data-ac-done>Done</button></div>`;
  }

  const productOf = (id) => actorNamed(id)?.product?.id ?? gateOf(id)?.product ?? null;
  // Whether an actor's app has called since its gateway token was made.
  const called = (v) => connected(gateOf(v.id), v);

  // While the steps show, the gateway is read every few seconds, for the app's
  // first call to tick the last one.
  let waitTimer = null;
  function watchConnect() {
    clearTimeout(waitTimer);
    if (!isOpen() || view.name !== "gateway-token" || called(view)) return;
    const v = view;
    waitTimer = setTimeout(async () => {
      await loadGateway();
      if (!isOpen() || view !== v) return;
      const el = panel.querySelector("[data-ac-wait]");
      if (el && called(v)) {
        el.dataset.acWait = "done";
        el.querySelector(".ac-step-text").textContent = waitWords(true, handoff(productOf(v.id), { url: "", token: v.token }).app ?? nameOf(v.id));
        panel.querySelector("[data-ac-done]")?.classList.add("primary");
        sfx("success");
      }
      watchConnect();
    }, 3000);
  }

  function confirmHtml(v) {
    const name = h(nameOf(v.id));
    if (v.name === "rotate")
      return `<p class="ac-lead">A new token for ${name}?</p><p class="pj-note">The old one stops working at once.</p>
        <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary" data-ac-rotate="${h(v.id)}" data-yours>Make a new token</button></div>`;
    const a = actorNamed(v.id);
    const observed = (a?.via ?? []).some((x) => x === "observed" || x === "attributed") && !a?.via.includes("demo");
    const remote = a?.via?.includes("remote") ? " Its remote connections are revoked too." : "";
    return `<p class="ac-lead">Remove ${name}?</p><p class="pj-note">${observed ? `Its token stops working and its own reports stop; XenoNet keeps observing it.` : `Its token stops working, its claims are let go, and its record and history leave XenoNet. A program that should come back asks again, and waits for your Allow.`}${remote}</p>
      <div class="m-foot"><button class="act" data-ac-back>Cancel</button><span class="grow"></span><button class="act primary ac-danger" data-ac-remove-yes="${h(v.id)}">Remove</button></div>`;
  }

  function offHtml() {
    return `<p class="pj-note">Actors are off here (<code>XENONET_ACTORS=off</code>): XenoNet keeps no actors, takes no registrations, and neither draws nor reports you. Unset it and start XenoNet again to turn them on.</p>`;
  }

  function render() {
    if (!isOpen()) return;
    const body = panel.querySelector(".m-body");
    const head = panel.querySelector("h2");
    if (off()) {
      head.textContent = "Actors";
      panel.querySelector(".m-sub").textContent = "Off";
      body.innerHTML = offHtml();
      return;
    }
    if (view.name === "list") {
      head.textContent = "Actors";
      return drawList();
    }
    clearTimeout(armTimer);
    const REMOTE = { pair: "Pair a remote actor", preview: "What would leave", paired: "Paired", unpair: "Revoke a connection", tunnel: "A tunnel XenoNet runs" };
    const who = view.id ? nameOf(view.id) : "";
    const GATEWAY = { "gateway-setup": "Put the gateway online", gateway: `Connect ${who}`, "gateway-token": `Connect ${who}`, "gateway-moved": `New address for ${who}`, wake: `Wake ${who}` };
    head.textContent = REMOTE[view.name] ?? GATEWAY[view.name] ?? (view.name === "add" ? "Add an actor" : view.name === "token" ? (view.rotated ? "A new token" : "An actor of your own") : view.name === "rotate" ? "A new token" : "Remove an actor");
    const SUB = { wake: "When a message arrives for it", token: "Shown once", "gateway-token": "Its token shows once", "gateway-moved": "Its token still works", "gateway-setup": "How apps in other clouds reach XenoNet", add: "A script or daemon of your own", preview: "Nothing is approved yet", pair: "Read-only, for the projects you pick" };
    panel.querySelector(".m-sub").textContent = SUB[view.name] ?? "";
    if (REMOTE[view.name]) body.innerHTML = remoteViews.html(view, actors);
    else if (view.name === "gateway") body.innerHTML = gatewayAskHtml(view, { gateway, a: actorNamed(view.id), name: nameOf(view.id), again: !!gateOf(view.id)?.token });
    else if (view.name === "gateway-token") body.innerHTML = gatewayTokenHtml(view, { url: view.url ?? gateway?.url ?? "", product: productOf(view.id), name: nameOf(view.id), done: called(view) });
    else if (view.name === "gateway-moved") body.innerHTML = gatewayMovedHtml(gateway?.url ?? "", productOf(view.id));
    else if (view.name === "gateway-setup") body.innerHTML = gatewaySetupHtml(view, gateway);
    else if (view.name === "wake") body.innerHTML = wakeViewHtml(view, { wake: wakeOf(view.id), help: wakeHelp(productOf(view.id)), draft: wakeDraft });
    else body.innerHTML = view.name === "add" ? addHtml() : view.name === "token" ? tokenHtml(view) : confirmHtml(view);
    if (view.name === "add") body.querySelector("[data-ac-name]")?.focus();
    if (view.name === "gateway-token") watchConnect();
    if (view.name === "gateway-setup") body.querySelector(".ac-pick.on input")?.focus();
    if (view.name === "pair") remoteViews.focus(body);
  }

  // Moves to a view; leaving a token's view lets the token go, and leaving
  // the Wake view a webhook key typed there.
  function go(next) {
    view = next;
    forgetSecrets();
    if (next.name !== "wake") wakeDraft = { url: "", key: "" };
    render();
  }

  // ---------- what the buttons do ----------

  async function act(call, done) {
    const r = await call();
    if (!r.ok) {
      sfx("error");
      toast(r.error ?? "That didn't work", true);
      return null;
    }
    sfx("success");
    if (done) toast(typeof done === "function" ? done(r) : done);
    return r;
  }

  async function loadObservers() {
    observers = await fetch("/api/actors/observers")
      .then((r) => (r.ok ? r.json() : null))
      .catch(() => null);
  }

  function readForm(body) {
    draft = {
      name: body.querySelector("[data-ac-name]").value,
      slug: body.querySelector("[data-ac-slug]").value.trim(),
      capabilities: [...body.querySelectorAll("[data-ac-cap].on")].map((b) => b.dataset.acCap),
      beat: body.querySelector("[data-ac-beat]").value,
    };
    const beat = Number(draft.beat);
    if (!draft.name.trim()) return { error: "It needs a name" };
    if (!Number.isFinite(beat) || beat < 5 || beat > 3600) return { error: "A heartbeat is 5 to 3,600 seconds" };
    return { body: { schema: "actor/registration/v1", name: draft.name.trim(), ...(draft.slug ? { slug: draft.slug } : {}), capabilities: draft.capabilities, heartbeatMs: Math.round(beat) * 1000 } };
  }

  async function onClick(e) {
    const t = e.target.closest("button");
    if (!t || t.disabled) return;
    const body = panel.querySelector(".m-body");
    const d = t.dataset;
    const saidCopy = (ok, what) => (sfx(ok ? "select" : "error"), toast(ok ? (what === "message" ? "Message copied" : "Token copied") : "Couldn't copy it", !ok));
    if (await secretClick(panel, t, saidCopy)) return;
    if (d.acAllow !== undefined || d.acDeny !== undefined) {
      const id = d.acAllow ?? d.acDeny;
      if (d.acAllow !== undefined && !requests.armed(id)) return;
      t.disabled = true;
      t.dataset.busy = "1";
      const res = await decide(id, d.acAllow !== undefined ? "allow" : "deny");
      if (!res.ok && t.isConnected) (t.disabled = false), delete t.dataset.busy;
      return;
    }
    if (d.acShow) {
      modals.close();
      return select(d.acShow);
    }
    if (d.acYou !== undefined) {
      const on = !you.shown();
      const done = you.setShown(on);
      onShown();
      sfx("toggle");
      render();
      if (on) return toast("You're on the map again");
      // Off deletes what XenoNet kept of you, at once where it can.
      const { deleted = false } = (await done) ?? {};
      return toast(deleted ? "You're off the map, and XenoNet has deleted what it kept of you" : "You're off the map: this window reports nothing about you");
    }
    if (d.acObserve) {
      const o = observers?.observers.find((x) => x.product === d.acObserve);
      if (!o) return;
      t.disabled = true;
      await act(() => calls.call("POST", "/api/actors/observers", { product: o.product, on: !o.on }), o.on ? `XenoNet stopped looking at ${o.name}` : `XenoNet looks at ${o.name} again`);
      await loadObservers();
      return render();
    }
    if (d.acNew !== undefined) return go({ name: "add" });
    if (await remoteViews?.click(t, d, body)) return;
    if (d.acToken) return go({ name: "rotate", id: d.acToken });
    if (d.acGateway) return go({ name: "gateway", id: d.acGateway });
    if (d.acGatewayMoved) return go({ name: "gateway-moved", id: d.acGatewayMoved });
    if (d.acWake) {
      wakeDraft = { url: "", key: "" };
      await loadWakes();
      return go({ name: "wake", id: d.acWake });
    }
    if (d.acWakeKind) {
      // Away from Webhook, what was typed for it goes.
      wakeDraft = d.acWakeKind === "webhook" ? readWake(body, wakeDraft) : { url: "", key: "" };
      sfx("toggle");
      const keep = { ...view, pick: d.acWakeKind, result: null };
      view = keep;
      return render();
    }
    if (d.acWakeSave) {
      const pick = view.pick ?? wakeOf(view.id)?.kind ?? "none";
      const draft = readWake(body, wakeDraft);
      const kept = wakeOf(view.id)?.kind === "webhook";
      // A webhook already kept, with nothing typed: nothing to change.
      if (pick === "webhook" && kept && !draft.url && !draft.key) return go({ name: "list" });
      if (pick === "webhook" && (!draft.url || !draft.key)) return Object.assign(body.querySelector("[data-ac-error]"), { hidden: false, textContent: "A webhook needs its URL and its key" });
      t.disabled = true;
      const r = await calls.call("PUT", `/api/actors/${encodeURIComponent(slugOf(d.acWakeSave))}/wake`, pick === "webhook" ? { kind: pick, url: draft.url, key: draft.key } : { kind: pick });
      wakeDraft = { url: "", key: "" };
      if (!r.ok) {
        sfx("error");
        t.disabled = false;
        return Object.assign(body.querySelector("[data-ac-error]"), { hidden: false, textContent: r.error ?? "That didn't work" });
      }
      sfx("success");
      await loadWakes();
      if (pick === "webhook") return go({ ...view, pick: "webhook", saved: true, result: null });
      toast(pick === "notify" ? `XenoNet notifies you when messages wait for ${nameOf(view.id)}` : `XenoNet doesn't wake ${nameOf(view.id)}`);
      return go({ name: "list" });
    }
    if (d.acWakeTest !== undefined) {
      t.disabled = true;
      t.textContent = "Testing…";
      const r = await calls.call("POST", `/api/actors/${encodeURIComponent(slugOf(view.id))}/wake/test`);
      await loadWakes();
      sfx(r.ok && r.data.ok ? "success" : "error");
      return go({ ...view, result: r.ok ? r.data : { ok: false, why: r.error } });
    }
    if (d.acGatewayMake) {
      t.disabled = true;
      const r = await act(() => calls.call("POST", `/api/actors/${encodeURIComponent(slugOf(d.acGatewayMake))}/gateway`), null);
      if (!r) return go({ name: "list" });
      readTokens();
      loadGateway();
      return go({ name: "gateway-token", id: r.data.id, token: r.data.token, url: r.data.url, madeAt: Date.now() });
    }
    if (d.acGatewaySetup !== undefined) {
      await loadGateway();
      const t0 = gateway?.tunnel ?? {};
      return go({ name: "gateway-setup", pick: t0.kind ? { kind: t0.kind, name: t0.name ?? "", host: t0.given ?? "" } : { kind: (gateway?.kinds ?? []).find((k) => k.tool && k.found)?.kind ?? null } });
    }
    if (d.acTunnelKind) {
      const pick = readPick(body, view.pick);
      sfx("toggle");
      return go({ ...view, pick: pick.kind === d.acTunnelKind ? pick : { kind: d.acTunnelKind, name: "", host: "" } });
    }
    if (d.acTunnelUse !== undefined || d.acTunnelNone !== undefined) {
      const pick = d.acTunnelNone !== undefined ? null : readPick(body, view.pick);
      t.disabled = true;
      const r = await calls.call("POST", "/api/actors/gateway", { on: pick ? true : gateway?.on, tunnel: pick });
      if (!r.ok) {
        sfx("error");
        t.disabled = false;
        return Object.assign(body.querySelector("[data-ac-error]"), { hidden: false, textContent: r.error ?? "That didn't work" });
      }
      sfx("success");
      gateway = r.data;
      toast(pick ? `Starting ${r.data.tunnel?.through}…` : "The gateway has no tunnel now");
      return go({ name: "list" });
    }
    // On, with no tunnel picked yet, picks one first.
    if (d.acGatewayOn !== undefined && !gateway?.on && !gateway?.tunnel?.kind) return panel.querySelector("[data-ac-gateway-setup]")?.click();
    if (d.acGatewayOn !== undefined || d.acGatewayCheck !== undefined || d.acGatewayRepair !== undefined) {
      t.disabled = true;
      const ask = d.acGatewayOn !== undefined ? { on: !gateway?.on } : d.acGatewayCheck !== undefined ? { check: true } : { repair: true };
      if (ask.check || ask.repair) t.textContent = ask.check ? "Checking…" : "Repairing…";
      const r = await act(
        () => calls.call("POST", "/api/actors/gateway", ask),
        (x) => (ask.on !== undefined ? (x.data.on ? "The gateway is on" : "The gateway is off") : x.data.reach?.state === "ok" ? "The gateway is online" : "Still not reachable"),
      );
      if (r) gateway = r.data;
      return render();
    }
    if (d.acGatewayCopy) {
      const copied = await Promise.resolve(navigator.clipboard?.writeText(d.acGatewayCopy)).then(() => !!navigator.clipboard, () => false);
      sfx(copied ? "select" : "error");
      return toast(copied ? (d.acCopied ?? "Copied") : "Couldn't copy it", !copied);
    }
    if (d.acRemove) return go({ name: "remove", id: d.acRemove });
    if (d.acBack !== undefined || d.acDone !== undefined) return go({ name: "list" });
    if (d.acCap) {
      t.classList.toggle("on");
      t.setAttribute("aria-pressed", t.classList.contains("on"));
      return sfx("toggle");
    }
    if (d.acAdd !== undefined) {
      const form = readForm(body);
      const error = body.querySelector("[data-ac-error]");
      if (form.error) return Object.assign(error, { hidden: false, textContent: form.error });
      t.disabled = true;
      const r = await calls.call("POST", "/api/actors", form.body);
      if (!r.ok) {
        sfx("error");
        t.disabled = false;
        return Object.assign(error, { hidden: false, textContent: `${r.error ?? "That didn't work"}${r.data?.suggest ? ` (actor:${r.data.suggest} is free)` : ""}` });
      }
      sfx("success");
      draft = { name: "", slug: "", capabilities: ["run-commands"], beat: 60 };
      return go({ name: "token", id: r.data.id, token: r.data.token });
    }
    if (d.acRotate) {
      t.disabled = true;
      const r = await act(() => calls.call("POST", `/api/actors/${encodeURIComponent(slugOf(d.acRotate))}/token`), null);
      if (!r) return go({ name: "list" });
      // Its new token's state, for the list to come back to.
      readTokens();
      return go({ name: "token", id: r.data.id, token: r.data.token, rotated: true });
    }
    if (d.acRemoveYes) {
      t.disabled = true;
      const name = nameOf(d.acRemoveYes);
      const r = await act(() => calls.call("DELETE", `/api/actors/${encodeURIComponent(slugOf(d.acRemoveYes))}`), `Removed ${name}`);
      // Gone from here at once; one XenoNet still observes stays.
      if (r && !r.data.observed) removed.add(d.acRemoveYes);
      return go({ name: "list" });
    }
    if (d.acSave !== undefined && view.token) {
      const saved = await desktop.saveToken(view.token, { slug: slugOf(view.id) }).catch((err) => ({ error: err.message }));
      if (!saved) return;
      if (saved.error) return sfx("error"), toast(`Couldn't save it: ${saved.error}`, true);
      sfx("success");
      return go({ ...view, token: null, saved });
    }
  }

  // How tunnels stand changed (the server's assistant event): the list, or
  // the tunnel view's state alone, so nothing typed there is lost.
  async function tunnelsChanged() {
    if (!loaded || !isOpen() || (view.name !== "list" && view.name !== "tunnel")) return;
    await remote?.refresh();
    if (!isOpen()) return;
    if (view.name === "list") return render();
    const t = remote.tunnelOf(view.grant);
    const body = panel.querySelector(".m-body");
    const state = body.querySelector("[data-rc-tstate]");
    if (state) state.innerHTML = tunnelStateHtml(t);
    const foot = body.querySelector("[data-rc-tfoot]");
    if (foot) foot.innerHTML = tunnelFootHtml(t);
  }

  async function open() {
    view = { name: "list" };
    loaded = false;
    panel = modals.open(`<header class="m-head"><div><div class="m-chip">◇ actors</div><h2>Actors</h2><div class="m-sub">Loading…</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"></div>`, {
      accent: "var(--actor)",
      cls: "ac",
      // A token on screen goes with the panel.
      onClose: () => {
        view = { name: "list" };
        clearInterval(ticker);
        clearTimeout(armTimer);
        clearTimeout(gatewayTimer);
        clearTimeout(waitTimer);
        forgetSecrets();
        wakeDraft = { url: "", key: "" };
        unblur();
      },
    });
    // A shown token masks itself again as you switch to another app.
    const unblur = hideOnBlur(panel);
    panel.addEventListener("click", onClick);
    panel.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && e.target.matches?.("[data-ac-name], [data-ac-slug], [data-ac-beat]")) panel.querySelector("[data-ac-add]")?.click();
    });
    clearInterval(ticker);
    // Ages and time left tick on their own.
    ticker = setInterval(() => {
      if (!isOpen()) return clearInterval(ticker);
      for (const el of panel.querySelectorAll("[data-since]")) el.textContent = timeAgo(Number(el.dataset.since));
      for (const el of panel.querySelectorAll("[data-until]")) el.textContent = leftWords(Number(el.dataset.until));
      // The gateway's checks and its apps' calls, as they come.
      if (view.name === "list") Promise.all([gateway?.on ? loadGateway() : null, loadWakes()]).then(() => isOpen() && view.name === "list" && drawList());
    }, 15000);
    const mine = panel;
    [canDecide] = await Promise.all([calls.ready(), loadObservers(), requests.refresh(), readTokens(), remote?.refresh(), loadGateway()]);
    await loadWakes();
    if (panel !== mine) return;
    loaded = true;
    render();
  }

  return {
    open,
    get isOpen() {
      return isOpen();
    },
    // The actors or the requests changed: the list again, if it's showing,
    // with the remote connections read again.
    refresh() {
      if (!loaded || !isOpen() || view.name !== "list") return;
      render();
      remote?.refresh().then(() => isOpen() && view.name === "list" && render());
    },
    tunnelsChanged,
  };
}
