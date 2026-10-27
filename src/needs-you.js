// The Needs you panel: everything waiting on you in one list, the longest
// wait first (core/needs-you.js), and what answers each. A session's
// question or permission is answered in its own app, so Open goes there,
// Show selects it on the map and ✓ puts its wait away, as the strip's card
// does. An actor waiting on you shows on the map, or you write to it. A
// program asking to be an actor, or a session's ask for a gateway token,
// has its Allow and Deny here, as in the strip and the Actors panel: Allow
// waits a moment after its row shows, and is marked data-yours, which
// XenoNet's control channel never presses outside the demo.
//
// It opens from the palette, with N, from the strip's Needs you button and
// from the tray's menu. While it's open, rows are reconciled rather than
// redrawn, so a button never swaps out from under the pointer.
import { escapeHtml as h, timeAgo } from "./format.js";
import { askedBy, asksFor, leftWords, JOINS_ALLOW, ARM_MS } from "./actor-requests.js";
import { CAPABILITY_WORDS, PRODUCT_NAMES } from "./actors-model.js";
import { gist } from "./alerts.js";
import { hereId } from "./hosts.js";
import { waitsWords, countOf } from "../core/needs-you.js";
import "./needs-you.css";

// A dismissed wait stays hidden this long for the server to drop it, as
// the strip's does, then shows again if it didn't.
const DISMISS_MS = 8000;

// `items()` is the queue as it is now (needsYouQueue), `request(id)` the
// window's view of a registration or a gateway ask by its id, for the
// details its row shows. The rest answer: select(id), openApp(id),
// ack(ids), message(id), decide(id, "allow" | "deny"), armed(id).
export function createNeedsYou({ modals, sfx, items, request = () => null, select, openApp, ack, message, decide, armed = () => false }) {
  let panel = null;
  let list = null;
  let armTimer = null;
  const rows = new Map(); // key → { el, sig }
  const shownAt = new Map(); // key → when its row first showed, for Allow's wait
  const dismissed = new Map(); // key → timer

  const visible = () => items().filter((it) => !dismissed.has(it.key));

  function head(queue) {
    const n = countOf(queue);
    return `<header class="m-head"><div><div class="m-chip">◉ needs you</div><h2>Needs you</h2><div class="m-sub" data-ny-sub>${n ? `${n} waiting · the longest wait first` : "Nothing waits on you"}</div></div><button class="act" data-mclose>Close ✕</button></header>`;
  }

  function open() {
    if (panel?.isConnected) return refresh();
    rows.clear();
    shownAt.clear();
    const queue = visible();
    panel = modals.open(`${head(queue)}<div class="m-body ny"><ol class="ny-list" data-ny-list></ol><p class="ny-empty" data-ny-empty hidden>Nothing waits on you. Questions, permissions, actors waiting and programs asking to join show here as they come.</p><p class="pj-note">A session's question or permission is answered in its own app: Open goes there. A session XenoNet runs itself asks here instead, with Allow and Deny. Allow is yours alone, from this window.</p></div>`, {
      accent: "var(--attention)",
      cls: "ny-panel",
      onClose: () => {
        clearTimeout(armTimer);
        panel = null;
        list = null;
        rows.clear();
      },
    });
    list = panel.querySelector("[data-ny-list]");
    panel.addEventListener("click", onClick);
    render(queue);
    return true;
  }

  function close() {
    if (panel?.isConnected) modals.close();
  }

  // ---------- rows ----------

  const when = (it) =>
    it.expiresAt != null ? `<span class="ny-time" data-until="${h(it.expiresAt)}">${leftWords(it.expiresAt)}</span>` : it.since != null ? `<span class="ny-time" data-since="${h(it.since)}">${timeAgo(it.since)}</span>` : "";
  const btn = (attr, label, title, { primary = false, yours = false } = {}) => `<button class="act${primary ? " primary" : ""}" ${attr}${yours ? " data-yours disabled" : ""} title="${h(title)}">${label}</button>`;
  const asker = (who) => (who ? `asked by <code>${h(who.program)}</code>${who.from.length ? ` ← ${h(who.from.join(" ← "))}` : ""}${who.folder ? ` · in <code>${h(who.folder)}</code>` : ""}` : "XenoNet couldn't see which program asked");

  function rowHtml(it, n) {
    const key = h(it.key);
    const mark = it.kind === "question" || it.kind === "permission" || it.kind === "ask" ? `<i class="sw ${h(it.app ?? (it.kind === "ask" ? "claude" : ""))}"></i>` : `<span class="ny-mark" title="${it.kind === "actor" ? "An actor" : "A program"}">◇</span>`;
    // Its machine: another's, or in network mode this one's, marked here.
    const session = it.kind === "question" || it.kind === "permission";
    const host = it.host ? `<span class="ny-host${it.stale ? " stale" : ""}">${it.stale ? `${h(it.host)} not live` : `on ${h(it.host)}`}</span>` : session && hereId() ? `<span class="ny-host here">on ${h(hereId())} · here</span>` : "";
    let lines = "";
    let acts = "";
    if (it.kind === "question" || it.kind === "permission") {
      const text = it.host && !it.text ? `The question is on ${it.host}: preview the transcript to read it.` : (it.text ?? "");
      lines = `<div class="ny-text" title="${h(text)}">${h(gist(text))}</div>`;
      acts = btn(`data-ny-show="${key}"`, "Show", "Select it on the map") + (it.actions.includes("open") ? btn(`data-ny-open="${key}"`, "Open", "Open it in its app, to answer it", { primary: true }) + btn(`data-ny-dismiss="${key}"`, "✓", "Dismiss") : "");
    } else if (it.kind === "actor") {
      lines = `<div class="ny-text">${h(it.text)}</div>`;
      acts = btn(`data-ny-show="${key}"`, "Show", "Select it on the map, where its inspector says the rest") + (it.actions.includes("message") ? btn(`data-ny-message="${key}"`, "Message", "Write to it, marked as from you", { primary: true }) : "");
    } else if (it.kind === "request" || it.kind === "gateway") {
      const r = request(it.id) ?? {};
      const who = askedBy(r.asker);
      if (it.kind === "request") {
        const product = r.product?.id ? ` as ${h(PRODUCT_NAMES[r.product.id] ?? `“${r.product.id}”, its own word`)}` : "";
        lines = `<div class="ny-text">${h(asksFor(r, CAPABILITY_WORDS))}${product ? ` · ${product.slice(1)}` : ""}${r.joins ? ` · it joins ${h(r.actor)}, which XenoNet knows: ${h(JOINS_ALLOW)}` : ""}</div><div class="ny-asker">${asker(who)}</div>`;
      } else lines = `<div class="ny-text">${r.replaces ? "Allow makes a new token, and the old one stops working." : "Allow makes its token."} It shows in the Actors panel, masked. The session that asked never sees it.</div><div class="ny-asker">${asker(who)}</div>`;
      acts = btn(`data-ny-deny="${key}"`, "Deny", it.kind === "request" ? "Its token never works" : "Nothing is made, and the old token, if any, keeps working") + btn(`data-ny-allow="${key}"`, "Allow", it.kind === "request" ? "Its token starts working, and it joins the map" : "Makes the token and shows it masked in the Actors panel", { primary: true, yours: true });
    } else if (it.kind === "ask") {
      // A run XenoNet started asks to use a tool: what, and where, as its
      // machine words it. Deny is safe; Allow waits a moment.
      lines = `<div class="ny-text" title="${h(it.text ?? "")}"><code>${h(it.tool ?? "a tool")}</code> ${h(gist(it.text ?? ""))}</div><div class="ny-asker">${h([it.host ? `on ${it.host}` : "here", it.session, it.run ? `run ${it.run}` : null].filter(Boolean).join(" · "))}</div>`;
      acts = btn(`data-ny-deny="${key}"`, "Deny", "It doesn't run, and the session hears why") + btn(`data-ny-allow="${key}"`, "Allow", "It runs this once", { primary: true, yours: true });
    } else if (it.kind === "sign-in") {
      lines = `<div class="ny-text">${h(it.text ?? "")}</div>`;
      acts = btn(`data-ny-dismiss="${key}"`, "✓", "Dismiss");
    } else if (it.kind === "requests") lines = `<div class="ny-text">Allow or deny ${it.count === 1 ? "it" : "them"} in XenoNet's own window, the desktop app.</div>`;
    return `<li class="ny-row ny-${h(it.kind)}" data-key="${key}"><span class="ny-n">${n}</span><div class="ny-main"><div class="ny-name">${mark}<span class="ny-title">${h(it.title)}</span><span class="ny-what">${h(it.kind === "ask" && it.tool ? `wants to run ${it.tool}` : waitsWords(it))}</span>${host}${when(it)}</div>${lines}</div><div class="ny-acts">${acts}</div></li>`;
  }

  const tpl = document.createElement("template");
  const fromHtml = (html) => ((tpl.innerHTML = html.trim()), tpl.content.firstElementChild);

  function render(queue = visible()) {
    if (!list) return;
    const keep = new Set(queue.map((it) => it.key));
    for (const [key, row] of rows) {
      if (keep.has(key)) continue;
      row.el.remove();
      rows.delete(key);
      shownAt.delete(key);
    }
    let prev = null;
    const t = Date.now();
    queue.forEach((it, i) => {
      const html = rowHtml(it, i + 1);
      const sig = html.replace(/<span class="ny-time"[^>]*>[^<]*<\/span>/, "");
      let row = rows.get(it.key);
      if (!row) {
        row = { el: fromHtml(html), sig };
        rows.set(it.key, row);
        shownAt.set(it.key, t);
      } else if (row.sig !== sig) {
        // The same wait with new details: changed in place, buttons too.
        const next = fromHtml(html);
        row.el.className = next.className;
        row.el.innerHTML = next.innerHTML;
        row.sig = sig;
      }
      const want = prev ? prev.nextElementSibling : list.firstElementChild;
      if (want !== row.el) list.insertBefore(row.el, want);
      prev = row.el;
    });
    panel.querySelector("[data-ny-empty]").hidden = queue.length > 0;
    const sub = panel.querySelector("[data-ny-sub]");
    const n = countOf(queue);
    sub.textContent = n ? `${n} waiting · the longest wait first` : "Nothing waits on you";
    arm();
  }

  // Allow is ready a moment after its row shows here, and after the
  // request's own wait (actor-requests.js), whichever is later.
  const ready = (key, id) => armed(id) && Date.now() - (shownAt.get(key) ?? Infinity) >= ARM_MS;
  function arm() {
    clearTimeout(armTimer);
    if (!list) return;
    let pending = false;
    for (const b of list.querySelectorAll("[data-ny-allow]")) {
      const it = items().find((x) => x.key === b.dataset.nyAllow);
      const ok = !!it && ready(it.key, it.id) && b.dataset.busy !== "1";
      b.disabled = !ok;
      if (!ok) pending = true;
    }
    if (pending) armTimer = setTimeout(arm, 200);
  }

  // Ages and time left tick on their own.
  setInterval(() => {
    if (!list) return;
    for (const el of list.querySelectorAll(".ny-time[data-since]")) el.textContent = timeAgo(Number(el.dataset.since));
    for (const el of list.querySelectorAll(".ny-time[data-until]")) el.textContent = leftWords(Number(el.dataset.until));
  }, 15000);

  function hide(key) {
    dismissed.set(key, setTimeout(() => (dismissed.delete(key), refresh()), DISMISS_MS));
    render();
  }

  // ---------- answering ----------

  const itemOf = (key) => items().find((it) => it.key === key) ?? null;

  // Does what an item's action does, as its button would: show, open,
  // dismiss, message, allow or deny. { ok, error } after.
  async function act(key, action) {
    const it = itemOf(key);
    if (!it) return { ok: false, error: "nothing waits by that key" };
    if (!it.actions.includes(action)) return { ok: false, error: `${it.title} can't be answered with ${action} here` };
    if (action === "show") {
      close();
      await select(it.id);
      return { ok: true };
    }
    if (action === "open") {
      const res = await openApp(it.id);
      return { ok: res?.ok !== false, error: res?.error ?? null };
    }
    if (action === "dismiss") {
      sfx("select");
      hide(key);
      await ack([it.id], it);
      return { ok: true };
    }
    if (action === "message") return (message(it.id), { ok: true });
    const res = await decide(it.id, action, it);
    return { ok: !!res?.ok, error: res?.error ?? null };
  }

  async function onClick(e) {
    const b = e.target.closest("button[data-ny-show], button[data-ny-open], button[data-ny-dismiss], button[data-ny-message], button[data-ny-allow], button[data-ny-deny]");
    if (!b || b.disabled) return;
    e.stopPropagation();
    const [action, key] = Object.entries({ show: b.dataset.nyShow, open: b.dataset.nyOpen, dismiss: b.dataset.nyDismiss, message: b.dataset.nyMessage, allow: b.dataset.nyAllow, deny: b.dataset.nyDeny }).find(([, v]) => v != null);
    if (action === "allow" && !ready(key, itemOf(key)?.id)) return;
    if (action === "allow" || action === "deny") {
      // Allow and Deny wait for their answer; one that didn't go through
      // can be pressed again.
      b.disabled = true;
      b.dataset.busy = "1";
      const res = await act(key, action);
      if (!res.ok && b.isConnected) {
        delete b.dataset.busy;
        b.disabled = false;
      }
      return;
    }
    act(key, action);
  }

  function refresh() {
    if (panel?.isConnected) render();
    return !!panel?.isConnected;
  }

  return {
    open,
    close,
    toggle: () => (panel?.isConnected ? (close(), false) : open()),
    refresh,
    act,
    get isOpen() {
      return !!panel?.isConnected;
    },
    // The queue as the panel has it, for the driver: each item, whether
    // its row shows now, and for a request whether Allow is ready yet.
    state() {
      const queue = visible();
      return {
        open: !!panel?.isConnected,
        count: countOf(queue),
        items: queue.map((it) => ({ ...it, waitsWords: waitsWords(it), row: rows.has(it.key), ...(it.actions.includes("allow") ? { armed: rows.has(it.key) && ready(it.key, it.id) } : {}) })),
      };
    },
  };
}
