// The attention strip: programs asking to be actors, agents and actors
// waiting on you, edit collisions, stuck agents, sessions ready for review,
// and problems XenoNet's doctor found. Also the tab-title badge, and notifications:
// native ones in the desktop app, the browser's (opt-in) otherwise.
//
// Live snapshots arrive several times a second, so the strip is reconciled
// rather than redrawn: a card that hasn't changed keeps its element, so it
// doesn't replay its entrance or swap its buttons out from under the pointer.
// New cards slide in, resolved ones fold away, and dismissing is immediate.
import { escapeHtml as h, timeAgo } from "./format.js";
import { askedBy, asksFor, leftWords, JOINS_ALLOW } from "./actor-requests.js";
import { CAPABILITY_WORDS, PRODUCT_NAMES, actorNeeds } from "./actors-model.js";
import { alertFrom, alertHostHtml, alertText, collisionFrom } from "./hosts.js";

const MAX_VISIBLE = 4;
const LEAVE_MS = 240;
// An alert older than this when it first shows up (after a reconnect, or a
// session coming into the map's scope) isn't news: no sound, no notification.
const FRESH_MS = 10 * 60e3;

// The part of an agent's last message worth a line or two: its last question
// if it asks one, else its last sentence.
export function gist(text) {
  const t = String(text ?? "").replace(/\s+/g, " ").replace(/^…\s*/, "").trim();
  const sentences = t.match(/[^.!?]+[.!?]+["')\]*_`]*|[^.!?]+$/g)?.map((s) => s.trim()).filter(Boolean) ?? [];
  const pick = [...sentences].reverse().find((s) => s.includes("?")) ?? sentences[sentences.length - 1];
  return pick || t;
}

const keyOf = (it) => (it.kind === "review" || it.kind === "requests" ? it.kind : `${it.kind}:${it.id}`);

// `requests` answers the request cards (see actor-requests.js): allow(id),
// deny(id), open() for the Actors panel, and armed(id) / armsIn(id) for
// Allow's moment of wait after a card shows.
export function createAlerts({ sfx, select, ack, openApp, collision, doctor = () => {}, requests = null, native = null }) {
  const el = document.getElementById("alerts");
  const tpl = document.createElement("template");
  const cards = new Map(); // key → { el, sig }
  const dismissed = new Map(); // key → timer, while an ack is on its way
  const browserNotes = new Map();
  let seen = null; // keys (with their timestamps) already announced
  let index = new Map();
  let expanded = false;
  let lastItems = [];
  let lastSnap = null;
  // The doctor's problems (see setDoctor), and the ones you've put away,
  // which stay away until they change.
  let doctorItems = [];
  const doctorHidden = new Set();
  const doctorKey = (it) => `${it.id}|${it.text}`;
  // Registrations waiting for you (see setRequests): a card each, first in
  // the strip so nothing pushes them about, or one card with how many wait
  // in a window that can't answer them.
  let requestItems = [];
  let armTimer = null;
  // Actors waiting on you (see setActors), after the sessions that are.
  let actorItems = [];

  const title = (id) => index.get(id)?.title ?? id;

  // ---------- notifications ----------

  function notify(n) {
    if (native) return native.notify(n);
    if (!("Notification" in window) || Notification.permission !== "granted" || document.hasFocus()) return;
    browserNotes.get(n.key)?.close();
    const note = new Notification(n.subtitle ? `${n.title} · ${n.subtitle}` : n.title, { body: n.body, tag: n.key, silent: true });
    note.onclick = () => {
      window.focus();
      select(n.id);
      note.close();
    };
    browserNotes.set(n.key, note);
  }

  function withdraw(keys) {
    if (!keys.length) return;
    if (native) return native.clear(keys);
    for (const k of keys) {
      browserNotes.get(k)?.close();
      browserNotes.delete(k);
    }
  }

  // Announces what's new since the last snapshot: one sound per kind, and a
  // notification each. With the window in front the strip and the sound are
  // enough; behind it, the notification carries the sound instead.
  function announce(items) {
    const now = Date.now();
    const stamp = (it) => `${keyOf(it)}@${it.since ?? ""}`;
    const current = new Set(items.map(stamp));
    if (seen) {
      const fresh = items.filter((it) => !seen.has(stamp(it)) && it.kind !== "review" && !it.stale && now - (it.since ?? it.at ?? now) < FRESH_MS);
      const front = document.hasFocus();
      const notifies = !!native || ("Notification" in window && Notification.permission === "granted");
      for (const kind of ["collision", "need", "stuck", "request", "actor-need"]) {
        // Stuck sessions have no notification to carry a sound, and the
        // server's for a request goes up silent.
        // From where it happened on the map: the session (or actor)
        // waiting, a collision's two sessions.
        const first = fresh.find((it) => it.kind === kind);
        if ((front || !notifies || kind === "stuck" || kind === "request") && first) sfx(kind === "collision" || kind === "stuck" ? kind : "alert", kind === "collision" ? { id: first.a, to: first.b } : kind === "request" ? undefined : { id: first.id });
      }
      for (const it of fresh) {
        if (it.kind === "need") notify({ key: keyOf(it), id: it.id, title: it.host ? `${it.host} · ${title(it.id)}` : title(it.id), subtitle: it.sub === "asks" ? "is waiting for you" : "needs permission", body: gist(alertText(it)), sound: !front });
        else if (it.kind === "actor-need") notify({ key: keyOf(it), id: it.id, title: it.name, subtitle: it.sub === "needs permission" ? "needs permission" : "is waiting for you", body: it.text, sound: !front });
        else if (it.kind === "collision") notify({ key: keyOf(it), id: it.a, title: it.host ? `${it.host} · Two sessions edited the same file` : "Two sessions edited the same file", subtitle: it.files.join(", "), body: `${title(it.a)} and ${title(it.b)}`, sound: !front });
      }
      // Answered, dismissed or gone: take its notification back.
      const live = new Set(items.map(keyOf));
      withdraw([...seen].map((s) => s.slice(0, s.lastIndexOf("@"))).filter((k) => !live.has(k) && (k.startsWith("need:") || k.startsWith("actor-need:") || k.startsWith("collision:"))));
    }
    seen = current;
  }

  // ---------- strip ----------

  function update(snap, graphIndex) {
    index = graphIndex;
    lastSnap = snap;
    const items = [...requestItems];
    const need = snap.sessions.filter((s) => s.needsYou && index.has(s.id));
    const strong = need.filter((s) => s.needsYou.kind !== "review").sort((a, b) => b.needsYou.since - a.needsYou.since);
    const review = need.filter((s) => s.needsYou.kind === "review");
    for (const s of strong) items.push({ kind: "need", id: s.id, app: s.app, text: s.needsYou.text, sub: s.needsYou.kind === "permission" ? "needs permission" : "asks", since: s.needsYou.since, ...alertFrom(s) });
    items.push(...actorItems);
    for (const c of snap.collisions ?? []) items.push({ kind: "collision", id: `${c.a}|${c.b}`, a: c.a, b: c.b, files: c.files, sameBranch: c.sameBranch, branches: c.branches, at: c.ts, ...collisionFrom(c) });
    for (const s of snap.sessions.filter((x) => x.stuck && index.has(x.id))) items.push({ kind: "stuck", id: s.id, app: s.app, text: s.stuck.reasons.join(" · "), ...alertFrom(s) });
    // Only this machine's can be dismissed from here: hosts are read-only.
    if (review.length) items.push({ kind: "review", ids: review.map((s) => s.id), count: review.length, mine: review.filter((s) => !s.host).map((s) => s.id) });
    items.push(...doctorItems.filter((it) => !doctorHidden.has(doctorKey(it))));

    announce(items);
    // A dismissal is done once the server drops the alert.
    for (const [key, timer] of dismissed) {
      if (!items.some((it) => keyOf(it) === key)) {
        clearTimeout(timer);
        dismissed.delete(key);
      }
    }
    const waiting = strong.filter((s) => !s.stale).length + actorItems.length;
    document.title = waiting ? `(${waiting}) XenoNet` : "XenoNet";
    lastItems = items;
    render();
  }

  function chip(it) {
    if (it.kind === "request") {
      const r = it.request;
      const who = askedBy(r.asker);
      const product = r.product?.id ? ` as ${h(PRODUCT_NAMES[r.product.id] ?? `“${r.product.id}”, its own word`)}` : "";
      const asked = who ? `asked by <code>${h(who.program)}</code>${who.from.length ? ` ← ${h(who.from.join(" ← "))}` : ""}${who.folder ? ` · in <code>${h(who.folder)}</code>` : ""}` : "XenoNet couldn't see which program asked";
      // Allow is drawn disabled and armed a moment after the card shows
      // (see arm), so the card's look stays the same while it waits. One
      // that joins an actor XenoNet knows says what Allow replaces, on a line
      // of its own so it's never cut off.
      return `<div class="al request" data-al-request="${h(r.id)}">
        <span class="al-icon">◇</span>
        <div class="al-body"><div class="al-title"><span class="al-name">${h(r.name)}</span> <em>asks to join XenoNet${product}</em><span class="al-time" data-until="${h(r.expiresAt)}">${leftWords(r.expiresAt)}</span></div><div class="al-text">${h(asksFor(r, CAPABILITY_WORDS))}</div>${r.joins ? `<div class="al-text">joins ${h(r.actor)}, which XenoNet knows: ${h(JOINS_ALLOW)}</div>` : ""}<div class="al-text al-asker">${asked}</div></div>
        <button class="al-btn" data-al-deny="${h(r.id)}" title="Its token never works">deny</button>
        <button class="al-btn al-allow" data-al-allow="${h(r.id)}" data-yours disabled title="Its token starts working, and it joins the map">allow</button>
      </div>`;
    }
    if (it.kind === "requests")
      return `<div class="al request" data-al-requests>
        <span class="al-icon">◇</span>
        <div class="al-body"><div class="al-title"><span class="al-name">${it.count === 1 ? "A program asks" : `${it.count} programs ask`} to join XenoNet</span></div><div class="al-text">Allow or deny ${it.count === 1 ? "it" : "them"} in XenoNet's own window, the desktop app.</div></div>
      </div>`;
    // An actor waiting on you: selecting it shows what it waits for. It
    // has nothing to dismiss; it goes when the actor stops waiting.
    if (it.kind === "actor-need")
      return `<div class="al need actor-need" data-al-goto="${h(it.id)}">
        <span class="al-icon">◉</span>
        <div class="al-body"><div class="al-title"><span class="al-actor" title="An actor">◇</span><span class="al-name">${h(it.name)}</span> <em>${h(it.sub)}</em><span class="al-time" data-since="${h(it.since)}">${timeAgo(it.since)}</span></div><div class="al-text">${h(it.text)}</div></div>
      </div>`;
    if (it.kind === "need")
      return `<div class="al need" data-al-goto="${h(it.id)}">
        <span class="al-icon">◉</span>
        <div class="al-body"><div class="al-title"><i class="sw ${h(it.app)}"></i><span class="al-name">${h(title(it.id))}</span> <em>${h(it.sub)}</em>${alertHostHtml(it)}<span class="al-time" data-since="${h(it.since)}">${timeAgo(it.since)}</span></div><div class="al-text" title="${h(alertText(it))}">${h(gist(alertText(it)))}</div></div>
        ${it.host ? "" : `<button class="al-btn" data-al-open="${h(it.id)}" title="Open in the desktop app">open</button>
        <button class="al-btn" data-al-ack="${h(it.id)}" title="Dismiss">✓</button>`}
      </div>`;
    if (it.kind === "collision")
      return `<div class="al collision" data-al-collision="${h(it.a)}|${h(it.b)}">
        <span class="al-icon">⚠</span>
        <div class="al-body"><div class="al-title"><span class="al-name">Collision${it.files.length ? ` · ${h(it.files.slice(0, 2).join(", "))}${it.files.length > 2 ? ` +${it.files.length - 2}` : ""}` : ""}</span>${alertHostHtml(it)}</div><div class="al-text">${h(title(it.a))} ↔ ${h(title(it.b))} · ${it.sameBranch ? `same branch ${h(it.branches[0])}` : `${h(it.branches[0] ?? "?")} vs ${h(it.branches[1] ?? "?")}`}</div></div>
        <button class="al-btn">diff</button>
      </div>`;
    if (it.kind === "doctor")
      return `<div class="al doctor ${it.status}" data-al-doctor>
        <span class="al-icon">✚</span>
        <div class="al-body"><div class="al-title"><span class="al-name">${h(it.group)}</span> <em>${h(it.text)}</em></div><div class="al-text">${h(it.fix ?? "Open XenoNet's doctor for the details.")}</div></div>
        <button class="al-btn" data-al-doctor-hide="${h(doctorKey(it))}" title="Hide until it changes">✓</button>
      </div>`;
    if (it.kind === "stuck")
      return `<div class="al stuck" data-al-goto="${h(it.id)}">
        <span class="al-icon">⟳</span>
        <div class="al-body"><div class="al-title"><i class="sw ${h(it.app)}"></i><span class="al-name">${h(title(it.id))}</span> <em>may be stuck</em>${alertHostHtml(it)}</div><div class="al-text">${h(it.text)}</div></div>
      </div>`;
    return `<div class="al review" data-al-review>
        <span class="al-icon">▣</span>
        <div class="al-body"><div class="al-title"><span class="al-name">${it.count} ready for review</span></div><div class="al-text">${it.ids.slice(0, 3).map((id) => h(title(id))).join(" · ")}${it.count > 3 ? " …" : ""}</div></div>
        ${it.mine.length ? `<button class="al-btn" data-al-ackall title="Dismiss all">✓ all</button>` : ""}
      </div>`;
  }

  const fromHtml = (html) => {
    tpl.innerHTML = html.trim();
    return tpl.content.firstElementChild;
  };

  // Folds a card away, then removes it.
  function leave(node) {
    node.style.height = `${node.offsetHeight}px`;
    node.classList.add("leaving");
    requestAnimationFrame(() => node.classList.add("gone"));
    setTimeout(() => {
      node.remove();
      if (!el.querySelector(".al")) el.hidden = true;
    }, LEAVE_MS);
  }

  let more = null;
  function render() {
    const items = lastItems.filter((it) => !dismissed.has(keyOf(it)));
    const shown = expanded ? items : items.slice(0, MAX_VISIBLE);
    const keep = new Set(shown.map(keyOf));
    for (const [key, card] of cards) {
      if (keep.has(key)) continue;
      cards.delete(key);
      leave(card.el);
    }
    let prev = null;
    for (const it of shown) {
      const key = keyOf(it);
      const html = chip(it);
      const sig = html.replace(/<span class="al-time"[^>]*>[^<]*<\/span>/, "");
      let card = cards.get(key);
      if (!card) {
        card = { el: fromHtml(html), sig };
        cards.set(key, card);
      } else if (card.sig !== sig) {
        // Same alert, new details: change it in place, without a new entrance.
        const next = fromHtml(html);
        card.el.className = next.className;
        card.el.innerHTML = next.innerHTML;
        card.sig = sig;
      }
      let want = prev ? prev.nextElementSibling : el.firstElementChild;
      while (want?.classList.contains("leaving")) want = want.nextElementSibling;
      if (want !== card.el) el.insertBefore(card.el, want);
      prev = card.el;
    }
    const hidden = items.length - shown.length;
    if (hidden > 0 || expanded) {
      more ??= Object.assign(document.createElement("button"), { className: "al-more" });
      more.dataset.alMore = "";
      more.textContent = expanded ? "show fewer" : `+${hidden} more`;
      if (more.parentNode !== el || el.lastElementChild !== more) el.appendChild(more);
    } else more?.remove();
    if (shown.length) el.hidden = false;
    else if (!el.querySelector(".al")) el.hidden = true;
    arm();
  }

  // Allow on each request card is ready for a click a moment after the
  // card shows; until then it waits, disabled.
  function arm() {
    clearTimeout(armTimer);
    let next = Infinity;
    for (const b of el.querySelectorAll("[data-al-allow]")) {
      const ready = !!requests?.armed(b.dataset.alAllow);
      b.disabled = !ready;
      if (!ready) next = Math.min(next, requests?.armsIn(b.dataset.alAllow) ?? Infinity);
    }
    if (next < Infinity) armTimer = setTimeout(arm, next + 20);
  }

  // Ages tick on their own; nothing else needs a redraw for them.
  setInterval(() => {
    for (const t of el.querySelectorAll(".al-time[data-since]")) t.textContent = timeAgo(Number(t.dataset.since));
    for (const t of el.querySelectorAll(".al-time[data-until]")) t.textContent = leftWords(Number(t.dataset.until));
  }, 15000);

  // Hidden at once; if the server hasn't dropped it in a few seconds, back it comes.
  function dismiss(keys, ids) {
    for (const key of keys) dismissed.set(key, setTimeout(() => (dismissed.delete(key), render()), 8000));
    sfx("select");
    render();
    ack(ids);
  }

  el.addEventListener("click", (e) => {
    // Allow and Deny wait for their answer; one that didn't go through can
    // be clicked again.
    const answer = async (button, how) => {
      button.disabled = true;
      const res = await how(button.dataset.alAllow ?? button.dataset.alDeny);
      if (!res?.ok && button.isConnected) button.disabled = false;
    };
    const allow = e.target.closest("[data-al-allow]");
    if (allow) {
      e.stopPropagation();
      if (allow.disabled || !requests?.armed(allow.dataset.alAllow)) return;
      return answer(allow, requests.allow);
    }
    const deny = e.target.closest("[data-al-deny]");
    if (deny) {
      e.stopPropagation();
      if (deny.disabled || !requests) return;
      return answer(deny, requests.deny);
    }
    if (e.target.closest("[data-al-request], [data-al-requests]")) return requests?.open();
    const hideDoctor = e.target.closest("[data-al-doctor-hide]");
    if (hideDoctor) {
      e.stopPropagation();
      doctorHidden.add(hideDoctor.dataset.alDoctorHide);
      sfx("select");
      return lastSnap && update(lastSnap, index);
    }
    if (e.target.closest("[data-al-doctor]")) return doctor();
    const ackBtn = e.target.closest("[data-al-ack]");
    if (ackBtn) {
      e.stopPropagation();
      return dismiss([`need:${ackBtn.dataset.alAck}`], [ackBtn.dataset.alAck]);
    }
    const ackAll = e.target.closest("[data-al-ackall]");
    if (ackAll) {
      e.stopPropagation();
      return dismiss(["review"], lastItems.find((i) => i.kind === "review")?.mine ?? []);
    }
    const openBtn = e.target.closest("[data-al-open]");
    if (openBtn) {
      e.stopPropagation();
      return openApp(openBtn.dataset.alOpen);
    }
    if (e.target.closest("[data-al-more]")) {
      expanded = !expanded;
      sfx("toggle");
      return render();
    }
    const col = e.target.closest("[data-al-collision]");
    if (col) {
      const [a, b] = col.dataset.alCollision.split("|");
      return collision(a, b);
    }
    const rev = e.target.closest("[data-al-review]");
    if (rev) {
      const it = lastItems.find((i) => i.kind === "review");
      if (it) select(it.ids[0]);
      return;
    }
    const go = e.target.closest("[data-al-goto]");
    if (go) select(go.dataset.alGoto);
  });

  return {
    update,
    get count() {
      return lastItems.filter((i) => (i.kind === "need" && !i.stale) || i.kind === "actor-need").length;
    },
    // The registrations waiting (actor-requests.js's view): a card each
    // with Allow and Deny, or, in a window without the key, one that says
    // how many wait.
    setRequests(view) {
      const list = view?.off ? [] : (view?.requests ?? []);
      requestItems = view?.canDecide ? list.map((r) => ({ kind: "request", id: r.id, request: r, since: r.at })) : view?.waiting > 0 && !view.off ? [{ kind: "requests", count: view.waiting }] : [];
      if (lastSnap) update(lastSnap, index);
    },
    // The actors' list (actor/list/v1): those waiting on you join the strip.
    setActors(list) {
      actorItems = actorNeeds(list);
      if (lastSnap) update(lastSnap, index);
    },
    // A doctor run (see core/doctor): its warnings and failures join the strip.
    setDoctor(result) {
      doctorItems = (result?.results ?? []).filter((r) => r.status === "fail" || r.status === "warn").map((r) => ({ kind: "doctor", id: r.id, group: r.group, text: r.detail, fix: r.fix, status: r.status }));
      if (lastSnap) update(lastSnap, index);
    },
    // What the strip shows, dismissed alerts left out (for src/driver.js).
    get items() {
      return lastItems.filter((it) => !dismissed.has(keyOf(it)));
    },
    async enableNotifications() {
      if (!("Notification" in window)) return "unsupported";
      return Notification.requestPermission();
    },
  };
}
