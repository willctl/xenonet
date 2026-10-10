import { escapeHtml as h, renderText, clock, stamp, timeAgo } from "./format.js";
import { APP_LABEL } from "./theme.js";

const ROLE = {
  user: { tag: "USER", cls: "user" },
  assistant: { tag: "AGENT", cls: "agent" },
  tool: { tag: "TOOL", cls: "tool" },
  reasoning: { tag: "THINK", cls: "think" },
  system: { tag: "SYS", cls: "sys" },
};

// Full-screen holographic transcript reader. Pages load newest-first; "earlier"
// pulls the previous page and prepends it.
export function createPreview({ onOpenApp }) {
  const root = document.getElementById("preview");
  let node = null;
  let entries = [];
  let cursor = null;
  let loading = false;

  function entryHtml(e, i) {
    const r = ROLE[e.role] ?? ROLE.system;
    const collapsible = e.role === "tool" || e.role === "reasoning";
    const body = e.text ? (e.role === "tool" ? `<pre class="out">${h(e.text)}</pre>` : `<div class="txt">${renderText(e.text)}</div>`) : "";
    const head = e.title ? `<span class="e-title">${h(e.title)}</span>` : "";
    const meta = [e.status, e.meta].filter(Boolean).map((m) => `<span class="tag ${m === "failed" ? "bad" : ""}">${h(m)}</span>`).join(" ");
    const content = collapsible
      ? `<details><summary>${head || (e.role === "reasoning" ? "<span class='e-title'>reasoning</span>" : "")} ${meta}</summary>${body}</details>`
      : `${head ? `<div>${head} ${meta}</div>` : ""}${body}`;
    return `<article class="entry ${r.cls}" id="e-${i}" style="--i:${Math.min(i, 30)}">
      <div class="e-time">${h(clock(e.ts))}</div>
      <div class="e-role">${r.tag}</div>
      <div class="e-body">${content}</div>
    </article>`;
  }

  function render(scrollTo) {
    const list = root.querySelector(".p-list");
    const strip = root.querySelector(".p-strip");
    const prevHeight = list.scrollHeight;
    const prevTop = list.scrollTop;
    list.innerHTML =
      (cursor ? `<button class="earlier" data-earlier>▲ Load earlier messages</button>` : `<div class="origin">Start of session</div>`) +
      entries.map(entryHtml).join("");
    strip.innerHTML = entries.map((e, i) => `<i class="${(ROLE[e.role] ?? ROLE.system).cls}" data-jump="${i}"></i>`).join("");
    root.querySelector(".p-count").textContent = `${entries.length} entries${cursor ? "+" : ""}`;
    if (scrollTo === "bottom") list.scrollTop = list.scrollHeight;
    else if (scrollTo === "keep") list.scrollTop = list.scrollHeight - prevHeight + prevTop;
  }

  async function load(more = false) {
    if (loading) return;
    loading = true;
    root.classList.add("loading");
    try {
      const params = new URLSearchParams({ limit: "120" });
      if (more && cursor) params.set("cursor", cursor);
      const res = await fetch(`/api/session/${encodeURIComponent(node.id)}/transcript?${params}`);
      const page = await res.json();
      if (!res.ok) throw new Error(page.error);
      entries = more ? [...page.entries, ...entries] : page.entries;
      cursor = page.cursor;
      render(more ? "keep" : "bottom");
    } catch (err) {
      root.querySelector(".p-list").innerHTML = `<div class="origin bad">Couldn't load the transcript: ${h(err.message)}</div>`;
    } finally {
      loading = false;
      root.classList.remove("loading");
    }
  }

  function open(n) {
    node = n;
    entries = [];
    cursor = null;
    root.style.setProperty("--accent", n.app === "claude" ? "var(--claude)" : "var(--codex)");
    root.innerHTML = `
      <div class="p-backdrop" data-pclose></div>
      <section class="p-panel">
        <header class="p-head">
          <div>
            <div class="p-chip">${h(APP_LABEL[n.app])} · transcript · <span class="p-count">…</span></div>
            <h2>${h(n.title)}</h2>
            <div class="p-meta">${h(n.model ?? "")} ${n.branch ? `· ${h(n.branch)}` : ""} · updated ${h(timeAgo(n.updatedAt))} · started ${h(stamp(n.createdAt))}</div>
          </div>
          <div class="p-actions">
            ${n.actions?.some((a) => a.id === "open-app") ? `<button class="act primary ${h(n.app)}" data-popen>${h(n.actions.find((a) => a.id === "open-app").label)}</button>` : ""}
            <button class="act" data-pclose>Close ✕</button>
          </div>
        </header>
        <div class="p-body">
          <nav class="p-strip" title="Jump to a message"></nav>
          <div class="p-list"><div class="origin">Loading the transcript…</div></div>
        </div>
        <div class="p-scan"></div>
      </section>`;
    root.hidden = false;
    requestAnimationFrame(() => root.classList.add("open"));
    load();
  }

  function close() {
    root.classList.remove("open");
    setTimeout(() => (root.hidden = true), 220);
    node = null;
  }

  root.addEventListener("click", (e) => {
    if (e.target.closest("[data-pclose]")) return close();
    if (e.target.closest("[data-popen]")) return onOpenApp(node);
    if (e.target.closest("[data-earlier]")) return load(true);
    const jump = e.target.closest("[data-jump]");
    if (jump) root.querySelector(`#e-${jump.dataset.jump}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  });
  addEventListener("keydown", (e) => {
    if (e.key === "Escape" && node) {
      e.stopImmediatePropagation();
      close();
    }
  }, true);

  return {
    open,
    close,
    get isOpen() {
      return !!node;
    },
  };
}
