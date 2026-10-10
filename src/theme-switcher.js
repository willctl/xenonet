// "Theme…" in the palette: a list of the themes, mine included. Moving
// through it with ↑ and ↓ shows each on the real map, in this window alone
// (nothing is saved, no other window changes); Enter keeps the one shown,
// which every window follows, and Esc puts the old one back. Typing narrows
// the list. Never a reload: a theme is drawn in place (see theme.js).
import "./themes.css";
import { escapeHtml as h } from "./format.js";
import { fuzzy } from "./palette.js";
import { keyLabel } from "./platform.js";

// A swatch's color goes into a style attribute, where escaping wouldn't
// stop a declaration: only a color does.
const swatch = (c) => (/^#[0-9a-f]{6}$/i.test(c) ? c : "transparent");

export function createSwitcher({ themes, sfx }) {
  const root = document.createElement("div");
  root.className = "tswitch";
  root.hidden = true;
  root.innerHTML = `
    <div class="tsw-catch" data-tsw-cancel></div>
    <section class="tsw" role="dialog" aria-label="Themes">
      <label class="tsw-input"><span aria-hidden="true">◐</span><input placeholder="Theme…" spellcheck="false" autocomplete="off" aria-label="Find a theme" aria-controls="tsw-list"/></label>
      <ol class="tsw-list" id="tsw-list" role="listbox" aria-label="Themes"></ol>
      <footer class="tsw-foot"><span><kbd>↑↓</kbd>look</span><span><kbd>${h(keyLabel("Enter"))}</kbd>keep</span><span><kbd>Esc</kbd>put back</span><span class="grow"></span><button class="tsw-more" data-tsw-gallery>Gallery</button></footer>
    </section>`;
  document.body.append(root);
  const input = root.querySelector("input");
  const list = root.querySelector(".tsw-list");
  let items = [];
  let active = 0;
  let open = false;

  // The themes in the order the server lists them, or those that match
  // what's typed, best first: a name before a word of its mood.
  function filter() {
    const q = input.value.trim();
    if (!q) return [...themes.list];
    const score = (t) => fuzzy(q, `${t.name} ${t.id}`) * 2 + fuzzy(q, t.about);
    return themes.list
      .map((t) => ({ t, s: score(t) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .map((x) => x.t);
  }

  function render() {
    list.innerHTML = items
      .map(
        (t, i) => `<li class="tsw-item ${i === active ? "on" : ""}" id="tsw-${i}" role="option" aria-selected="${i === active}" data-i="${i}">
          <span class="tsw-sw">${t.swatch.map((c) => `<i style="background:${swatch(c)}"></i>`).join("")}</span>
          <span class="tsw-text"><span class="tsw-name">${h(t.name)}${t.id === themes.current ? ` <em>in use</em>` : ""}${t.builtIn ? "" : ` <em>yours</em>`}</span><span class="tsw-about">${h(t.about)}</span></span>
        </li>`,
      )
      .join("");
    if (!items.length) list.innerHTML = `<li class="tsw-none">No theme matches</li>`;
    // Screen readers follow the highlight while the key focus stays in the box.
    input.setAttribute("aria-activedescendant", items.length ? `tsw-${active}` : "");
    list.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  }

  // Shows the highlighted theme. Holding an arrow key skips through the
  // list: a theme still being drawn finishes, then only the latest one
  // asked for is drawn, so redraws never queue up behind the keys.
  let wanted = null;
  let looking = null;
  // A frame, or a tenth of a second where frames have stopped (a hidden window).
  const frame = () => new Promise((r) => (requestAnimationFrame(r), setTimeout(r, 100)));
  function look() {
    wanted = items[active]?.id ?? null;
    looking ??= (async () => {
      for (let id; (id = wanted) && open; ) {
        await frame();
        if (id !== wanted || !open) continue;
        await themes.preview(id).catch(() => {});
        if (id === wanted) break;
      }
    })().finally(() => (looking = null));
    return looking;
  }

  function move(to) {
    if (!items.length) return Promise.resolve();
    active = (to + items.length) % items.length;
    sfx("hover");
    render();
    return look();
  }

  async function show() {
    if (open) return;
    open = true;
    root.hidden = false;
    requestAnimationFrame(() => root.classList.add("open"));
    input.value = "";
    await themes.load();
    items = filter();
    // Start at what's drawn: the theme in use, or one being looked at.
    active = Math.max(0, items.findIndex((t) => t.id === (themes.shown ?? themes.current)));
    render();
    input.focus();
    sfx("palette");
    // Each theme whole, and its fonts, so moving to one draws it at once.
    themes.warm();
  }

  function hide() {
    if (!open) return;
    open = false;
    wanted = null;
    root.classList.remove("open");
    root.hidden = true;
    input.blur();
  }

  // Enter: the theme shown is the one in use, everywhere.
  async function keep() {
    const id = items[active]?.id;
    hide();
    sfx("select");
    await looking;
    if (id && id !== themes.current) await themes.set(id).catch(() => {});
    else await themes.revert();
  }

  // Esc, or a click away: back to the theme in use.
  async function cancel() {
    hide();
    sfx("close");
    await looking;
    await themes.revert();
  }

  // Typing shows the best match; clearing it goes back to where it was.
  input.addEventListener("input", () => {
    const id = items[active]?.id;
    items = filter();
    active = input.value.trim() ? 0 : Math.max(0, items.findIndex((t) => t.id === id));
    render();
    if (items[active] && items[active].id !== id) look();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      move(active + (e.key === "ArrowDown" ? 1 : -1));
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      move(e.key === "Home" ? 0 : items.length - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      keep();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  });
  root.addEventListener("click", (e) => {
    if (e.target.closest("[data-tsw-cancel]")) return cancel();
    if (e.target.closest("[data-tsw-gallery]")) {
      const id = items[active]?.id;
      hide();
      return themes.gallery.show(id);
    }
    const li = e.target.closest("[data-i]");
    if (!li) return;
    active = Number(li.dataset.i);
    render();
    keep();
  });
  // The pointer moves the highlight too, and shows the theme under it;
  // only when it moves, not when the list is drawn again under it.
  const still = { x: null, y: null };
  root.addEventListener("pointermove", (e) => {
    if (e.clientX === still.x && e.clientY === still.y) return;
    Object.assign(still, { x: e.clientX, y: e.clientY });
    const li = e.target.closest("[data-i]");
    if (li && Number(li.dataset.i) !== active) move(Number(li.dataset.i));
  });

  return {
    show,
    hide,
    keep,
    cancel,
    // ↓ (by 1) or ↑ (by -1), as the keys do; resolves once it's drawn.
    step: (by = 1) => move(active + by),
    get isOpen() {
      return open;
    },
    // What it lists and which is highlighted, for scripts and tests.
    state: () => (open ? { active: items[active]?.id ?? null, items: items.map((t) => t.id) } : null),
  };
}
