// Command palette (Ctrl+K, ⌘K on macOS): jump to any project, session, page or
// system, run actions on it, run commands, or search every transcript ("?"
// prefix).
import { escapeHtml as h, timeAgo } from "./format.js";
import { APP_LABEL } from "./theme.js";
import { isMac, isMod, keyLabel } from "./platform.js";

const KIND_ICON = { project: "◇", session: "●", page: "✦", wiki: "◈", resource: "⬡", zone: "⬠", native: "◎", core: "✺", host: "✺" };

// Subsequence fuzzy match: contiguous runs and word starts score higher.
export function fuzzy(query, text) {
  const q = query.toLowerCase(), t = text.toLowerCase();
  if (!q) return 1;
  const at = t.indexOf(q);
  if (at >= 0) return 100 - Math.min(at, 50) + (at === 0 || /\W/.test(t[at - 1]) ? 20 : 0);
  let ti = 0, score = 0, run = 0;
  for (const ch of q) {
    const i = t.indexOf(ch, ti);
    if (i < 0) return 0;
    run = i === ti ? run + 1 : 0;
    score += 1 + run * 2 + (i === 0 || /\W/.test(t[i - 1]) ? 3 : 0);
    ti = i + 1;
  }
  return score;
}

export function createPalette({ sfx, nodes, commands, actions, onSearchResults }) {
  const root = document.getElementById("palette");
  root.innerHTML = `
    <div class="p-backdrop" data-pal-close></div>
    <section class="pal">
      <label class="pal-input"><span>›</span><input placeholder="Go to anything · ? search transcripts · > commands" spellcheck="false" autocomplete="off"/></label>
      <ol class="pal-list"></ol>
      <footer class="pal-foot"></footer>
    </section>`;
  const input = root.querySelector("input");
  const list = root.querySelector(".pal-list");
  const foot = root.querySelector(".pal-foot");
  let items = [];
  let active = 0;
  let searchSeq = 0;

  function nodeItem(n) {
    const label = n.title ?? n.name;
    const sub = n.kind === "session" ? `${APP_LABEL[n.app]} · ${timeAgo(n.updatedAt)}${n.cwd ? ` · ${n.cwd.split(/[\\/]/).pop()}` : ""}` : n.kind === "page" ? `wiki page · ${n.type}${n.space ? ` · ${n.space}` : ""}` : n.kind === "project" ? `${n.group ? "group of projects" : "project"}${n.rel?.includes("/") ? ` · ${n.rel}` : ""}${n.repo?.dirty ? ` · Δ${n.repo.dirty} uncommitted` : ""}` : n.kind === "host" ? `another machine · ${n.state}` : n.kind;
    // Another machine's (with XENONET_HOSTS=1) says which.
    const where = n.host && n.kind !== "host" ? `on ${n.host} · ` : "";
    return { type: "node", node: n, label, sub: where + sub, icon: n.group ? "▣" : KIND_ICON[n.kind] ?? "•", cls: n.app ?? n.kind };
  }

  function build(q) {
    const raw = q.trim();
    if (raw.startsWith("?")) return searchTranscripts(raw.slice(1).trim());
    const cmdOnly = raw.startsWith(">");
    const query = cmdOnly ? raw.slice(1).trim() : raw;
    const cmds = commands()
      .map((c) => ({ type: "command", ...c, score: fuzzy(query, c.label + " " + (c.keywords ?? "")) }))
      .filter((c) => c.score > 0);
    const ns = cmdOnly
      ? []
      : nodes()
          .filter((n) => n.kind !== "core")
          .map((n) => ({ ...nodeItem(n), score: fuzzy(query, `${n.title ?? n.name} ${n.cwd ?? n.rel ?? ""} ${n.branch ?? ""} ${n.aliases?.join(" ") ?? ""}`) +(n.kind === "session" ? Math.max(0, 8 - (Date.now() - (n.updatedAt ?? 0)) / 3600e3) : 0) }))
          .filter((n) => n.score > 0);
    let out = query ? [...ns, ...cmds].sort((a, b) => b.score - a.score) : [...cmds.slice(0, 6), ...ns.sort((a, b) => (b.node.updatedAt ?? 0) - (a.node.updatedAt ?? 0))];
    out = out.slice(0, 40);
    if (query && !cmdOnly) out.push({ type: "search", label: `Search transcripts for "${query}"`, sub: "full text in Claude Code and Codex", icon: "⌕", q: query });
    return out;
  }

  async function searchTranscripts(q) {
    const seq = ++searchSeq;
    if (!q) return [{ type: "hint", label: "Type words to search every Claude Code and Codex transcript", icon: "⌕" }];
    list.innerHTML = `<li class="pal-hint">Searching…</li>`;
    const res = await fetch(`/api/search?q=${encodeURIComponent(q)}`).then((r) => r.json()).catch(() => ({ results: [] }));
    if (seq !== searchSeq) return null;
    onSearchResults?.(res.results);
    sfx(res.results.length ? "select" : "error");
    const all = nodes();
    const byId = new Map(all.map((n) => [n.id, n]));
    return res.results.length
      ? res.results.map((r) => ({
          type: "hit",
          node: byId.get(r.sessionId) ?? null,
          label: r.title,
          sub: `${r.hits} match${r.hits === 1 ? "" : "es"} · ${r.app}${r.inGraph ? "" : " · not on the map"}`,
          snippet: r.snippets[0]?.text,
          icon: "⌕",
          cls: r.app,
        }))
      : [{ type: "hint", label: `No transcript mentions "${q}"`, icon: "∅" }];
  }

  function render() {
    list.innerHTML = items
      .map(
        (it, i) => `<li class="pal-item ${it.type} ${i === active ? "on" : ""}" data-i="${i}">
          <span class="pal-icon ${h(it.cls ?? "")}">${h(it.icon ?? "")}</span>
          <span class="pal-text"><span class="pal-label">${h(it.label)}</span>${it.sub ? `<span class="pal-sub">${h(it.sub)}</span>` : ""}${it.snippet ? `<span class="pal-snip">${h(it.snippet).replace(/⟦/g, "<b>").replace(/⟧/g, "</b>")}</span>` : ""}</span>
          ${it.key ? `<kbd>${h(keyLabel(it.key))}</kbd>` : ""}
        </li>`,
      )
      .join("");
    const it = items[active];
    const acts = it?.node ? actions(it.node) : [];
    foot.innerHTML = it?.node
      ? acts.map((a) => `<span><kbd>${h(keyLabel(a.key))}</kbd>${h(a.label)}</span>`).join("")
      : `<span><kbd>↑↓</kbd>move</span><span><kbd>${keyLabel("Enter")}</kbd>run</span><span><kbd>?</kbd>search transcripts</span><span><kbd>&gt;</kbd>commands</span><span><kbd>Esc</kbd>close</span>`;
    list.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }

  async function refresh() {
    const next = await build(input.value);
    if (next === null) return;
    items = next;
    active = Math.min(active, Math.max(0, items.length - 1));
    render();
  }

  function run(it, mod) {
    if (!it || it.type === "hint") return;
    if (it.type === "search") {
      input.value = `? ${it.q}`;
      active = 0;
      return refresh();
    }
    sfx("select");
    if (it.type === "command") {
      close(false);
      return it.run();
    }
    if (it.node) {
      const acts = actions(it.node);
      const a = acts.find((x) => x.mod === mod) ?? acts[0];
      close(false);
      a?.run();
    }
  }

  function open(prefill = "") {
    root.hidden = false;
    requestAnimationFrame(() => root.classList.add("open"));
    input.value = prefill;
    active = 0;
    refresh();
    input.focus();
    sfx("palette");
  }

  function close(sound = true) {
    root.classList.remove("open");
    setTimeout(() => (root.hidden = true), 160);
    input.blur();
    if (sound) sfx("close");
  }

  let debounce;
  input.addEventListener("input", () => {
    active = 0;
    clearTimeout(debounce);
    debounce = setTimeout(refresh, input.value.trim().startsWith("?") ? 220 : 0);
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      active = (active + (e.key === "ArrowDown" ? 1 : -1) + items.length) % Math.max(1, items.length);
      sfx("hover");
      render();
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(items[active], isMod(e) ? "ctrl" : e.shiftKey ? "shift" : e.altKey ? "alt" : null);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (isMod(e) && ["o", "p", "i", "h", "t", "n"].includes(e.key.toLowerCase()) && items[active]?.node) {
      e.preventDefault();
      run(items[active], `ctrl-${e.key.toLowerCase()}`);
    }
  });
  root.addEventListener("click", (e) => {
    if (e.target.closest("[data-pal-close]")) return close();
    const li = e.target.closest("[data-i]");
    if (li) run(items[Number(li.dataset.i)], null);
  });
  root.addEventListener("mousemove", (e) => {
    const li = e.target.closest("[data-i]");
    if (li && Number(li.dataset.i) !== active) {
      active = Number(li.dataset.i);
      render();
    }
  });

  addEventListener(
    "keydown",
    (e) => {
      // ⌘K on macOS, where shells never see ⌘. Elsewhere Ctrl+K, except in the
      // terminal, where Ctrl+K belongs to the shell; use Ctrl+Shift+K there.
      const hit = isMac ? e.metaKey && !e.ctrlKey : (e.ctrlKey || e.metaKey) && (e.shiftKey || !e.target.closest?.(".xterm"));
      if (hit && e.key.toLowerCase() === "k") {
        e.preventDefault();
        e.stopPropagation();
        root.hidden ? open() : close();
      }
    },
    true,
  );

  return { open, close, get isOpen() { return !root.hidden; } };
}
