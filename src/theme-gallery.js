// The gallery: a card for every theme, yours included, in a panel beside
// the map. Each card shows the theme in its own colors: the sky through the
// day with its actors drawn over it, swatches, its name in its display font
// and a line of type in its body font,
// and its mood. Pointing at a card, or moving to it with the keys, shows
// the theme on the map in this window alone, as the switcher does; a click
// or Enter keeps it, everywhere, and Esc or Close puts yours back. The last
// card makes a theme of your own from the one you're looking at, and shows
// the file to edit.
import "./themes.css";
import { escapeHtml as h, tildePath } from "./format.js";
import { keyLabel } from "./platform.js";
import { complete } from "../core/themes.js";

// How an agent looks in each actor form, in a few words.
const FORMS = {
  orb: "luminous orbs, ringed",
  cursor: "block cursors that blink faster as they work",
  crystal: "crystals that grow facets as they work",
  flame: "small stars whose corona flares when they act",
  swarm: "swarms that gather and scatter",
  jelly: "jellyfish trailing tendrils to what they touch",
};

const rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(" ");

// A small sketch of two agents and you, in the theme's actor form and
// colors, beaming into a project: SVG over the card's sky.
function actorSketch(t) {
  const a = t.actors;
  const one = (x, y, color, r) => {
    const g = [`<circle cx="${x}" cy="${y}" r="${r * 2.4}" fill="${color}" opacity="${0.18 * (a.glow?.opacity ?? 0.5) * 2}"/>`];
    switch (a.form) {
      case "cursor":
        // A block cursor in a ring of memory cells, some full, some empty.
        g.push(`<rect x="${(x - r * 0.3).toFixed(1)}" y="${(y - r * 0.5).toFixed(1)}" width="${(r * 0.6).toFixed(1)}" height="${r.toFixed(1)}" fill="${color}"/>`);
        for (let i = 0; i < 6; i++) {
          const th = (i / 6) * Math.PI * 2, cx = x + Math.cos(th) * r * 1.7, cy = y + Math.sin(th) * r * 1.7;
          g.push(`<rect x="${(cx - r * 0.18).toFixed(1)}" y="${(cy - r * 0.3).toFixed(1)}" width="${(r * 0.36).toFixed(1)}" height="${(r * 0.6).toFixed(1)}" ${i % 2 ? `fill="none" stroke="${color}" stroke-width=".6"` : `fill="${color}"`}/>`);
        }
        break;
      case "crystal": {
        const s = r * 1.2;
        g.push(`<polygon points="${x},${y - s * 1.4} ${x + s},${y} ${x},${y + s * 1.4} ${x - s},${y}" fill="${color}" opacity=".9"/><polyline points="${x - s},${y} ${x},${y - s * 0.3} ${x + s},${y}" fill="none" stroke="#fff" stroke-width=".7" opacity=".7"/>`);
        break;
      }
      case "flame":
        g.push(`<path d="M${x} ${y - r * 2} C${x + r * 1.5} ${y - r * 0.5} ${x + r * 1.1} ${y + r * 1.2} ${x} ${y + r * 1.2} C${x - r * 1.1} ${y + r * 1.2} ${x - r * 1.5} ${y - r * 0.5} ${x} ${y - r * 2}Z" fill="${color}" opacity=".9"/><circle cx="${x}" cy="${y + r * 0.2}" r="${r * 0.45}" fill="#fff" opacity=".8"/>`);
        break;
      case "swarm":
        for (let i = 0; i < 18; i++) {
          const th = i * 2.4, rr = Math.sqrt((i + 0.5) / 18) * r * 1.8;
          g.push(`<circle cx="${(x + Math.cos(th) * rr).toFixed(1)}" cy="${(y + Math.sin(th) * rr * 0.8).toFixed(1)}" r="${(0.6 + (i % 3) * 0.35).toFixed(2)}" fill="${color}"/>`);
        }
        break;
      case "jelly":
        g.push(`<path d="M${x - r * 1.3} ${y} A${r * 1.3} ${r * 1.2} 0 0 1 ${x + r * 1.3} ${y} Q${x} ${y + r * 0.45} ${x - r * 1.3} ${y}Z" fill="${color}" opacity=".85"/>`);
        for (let i = -2; i <= 2; i++) g.push(`<path d="M${x + i * r * 0.45} ${y + r * 0.2} q${r * 0.35} ${r * 1.1} 0 ${r * 2.2}" fill="none" stroke="${color}" stroke-width=".7" opacity=".7"/>`);
        break;
      default:
        // An orb, or with the eclipse knob, a black disc in a white corona.
        if (a.params?.eclipse) g.push(`<circle cx="${x}" cy="${y}" r="${r * 1.2}" fill="none" stroke="#fff" stroke-width="1.8" opacity=".9"/><circle cx="${x}" cy="${y}" r="${r}" fill="#000"/>`);
        else g.push(`<circle cx="${x}" cy="${y}" r="${r}" fill="${color}"/>`);
        // As many rings as the map draws (looks/actors.js): a theme's knob
        // is any number, and a card mustn't take forever to draw.
        const rings = Math.max(0, Math.min(4, Math.round(a.params?.rings ?? (a.params?.eclipse ? 0 : 1))));
        for (let i = 0; i < rings; i++) g.push(`<ellipse cx="${x}" cy="${y}" rx="${r * 2}" ry="${r * (0.55 + i * 0.5)}" fill="none" stroke="${color}" stroke-width=".8" opacity=".8" transform="rotate(${-18 + i * 50} ${x} ${y})"/>`);
    }
    return g.join("");
  };
  const human = t.colors.human, project = t.colors.project;
  const beam = (x1, y1, color, w) => `<line x1="${x1}" y1="${y1}" x2="150" y2="44" stroke="${color}" stroke-width="${w}" stroke-dasharray="3 4" opacity=".75"/>`;
  return `<svg class="tg-actors-svg" viewBox="0 0 200 56" aria-hidden="true">
    ${beam(46, 24, a.palette[0], 1.2)}${beam(96, 18, a.palette[1] ?? a.palette[0], 1)}${beam(176, 20, human, 0.8)}
    <circle cx="150" cy="44" r="3.2" fill="${project}"/>
    ${one(46, 24, a.palette[0], 7)}${one(96, 18, a.palette[1] ?? a.palette[0], 5.5)}
    <circle cx="176" cy="20" r="6" fill="${human}" opacity=".14"/><circle cx="176" cy="20" r="2.8" fill="${human}"/><circle cx="176" cy="20" r="4.8" fill="none" stroke="${human}" stroke-width=".7" opacity=".7"/>
  </svg>`;
}

// The sky across the day, night to night, fading into the card below.
const skyOf = ({ sky, colors }) =>
  `linear-gradient(180deg, transparent 35%, ${colors.bg} 100%), linear-gradient(90deg, ${sky.night[1]}, ${sky.dawn[1]} 25%, ${sky.day[1]} 50%, ${sky.dusk[1]} 75%, ${sky.night[1]})`;

const SWATCHES = ["bg", "surface", "text", "accent", "claude", "codex", "attention", "danger", "collision", "gold"];

// A theme's card. Its colors go into style attributes, so the theme is
// completed first, as the map completes the one it draws (theme.js): a
// value that isn't a color is XenoNet's, whatever the server sent.
export function card(theme, { inUse, focused }) {
  const t = complete(theme);
  const c = t.colors;
  const tags = [inUse && "in use", !t.builtIn && "yours"].filter(Boolean);
  const p = t.actors.params ?? {};
  const form = p.eclipse ? "black discs in a corona of light" : FORMS[t.actors.form] ?? t.actors.form;
  return `<article class="tg-card${inUse ? " in-use" : ""}${focused ? " focused" : ""}" tabindex="0" role="option" aria-selected="${focused}" data-id="${h(t.id)}"
      style="--tg-bg:${c.bg};--tg-surface:${c.surface};--tg-text:${c.text};--tg-muted:${c.muted};--tg-accent:${c.accent};--tg-accent-rgb:${rgb(c.accent)};font-family:${h(t.fonts.body ?? t.fonts.display)}">
    <div class="tg-sky" style="background:${skyOf(t)}" title="The sky through the day: night, dawn, day and dusk, and how actors look">${actorSketch(t)}</div>
    <div class="tg-body">
      <div class="tg-head"><span class="tg-name" style="font-family:${h(t.fonts.display)}">${h(t.name)}</span>${tags.map((x) => `<em>${x}</em>`).join("")}</div>
      <p class="tg-mood">${h(t.about)}</p>
      <div class="tg-sw">${SWATCHES.map((k) => `<i title="${k} ${c[k]}" style="background:${c[k]}"></i>`).join("")}</div>
      <div class="tg-sample">2 sessions need you · atlas-api<small style="font-family:${h(t.fonts.mono)};color:${c.accent}">claude ▸ editing src/parser.ts</small></div>
      <div class="tg-actors"><span class="tg-pal">${[...t.actors.palette.slice(0, 6), c.human].map((x, i, all) => `<i class="${i === all.length - 1 ? "you" : ""}" style="background:${x}"></i>`).join("")}</span><span>Actors: ${h(form)}</span></div>
    </div>
  </article>`;
}

export function createGallery({ themes, sfx, toast }) {
  const root = document.createElement("aside");
  root.className = "tgal";
  root.hidden = true;
  root.setAttribute("aria-label", "Theme gallery");
  root.innerHTML = `
    <header class="tgal-head">
      <div><div class="tgal-chip">◐ themes</div><h2>Gallery</h2><div class="tgal-sub">Point at a theme, or move with ↑↓, to see it on the map in this window. ${h(keyLabel("Enter"))} or a click keeps it; Esc puts yours back.</div></div>
      <button class="act" data-tgal-close>Close ✕</button>
    </header>
    <div class="tgal-cards" role="listbox" aria-label="Themes"></div>`;
  document.body.append(root);
  const cards = root.querySelector(".tgal-cards");
  let open = false;
  let whole = [];
  let focused = null; // the card looked at: its theme is the one drawn
  let made = null; // { id, file } after making one

  function makeCard() {
    const from = whole.find((t) => t.id === focused) ?? whole.find((t) => t.id === themes.current);
    const body = made
      ? `<p>Saved <code>${h(tildePath(made.file))}</code>. Edit it in any editor and XenoNet follows each save; it's in use now.</p>
         <div class="tg-make-acts"><button class="act" data-tgal-copy>Copy the path</button><button class="act" data-tgal-reveal>Show the file</button></div>`
      : `<p>Copies <b data-from>${h(from?.name ?? "the theme")}</b>, the one you're looking at, to a file of your own with every field written out, to change as you like.</p>`;
    return `<article class="tg-card tg-make" tabindex="0" role="option" data-make><div class="tg-plus">+</div><div class="tg-body"><div class="tg-head"><span class="tg-name">Make your own</span></div>${body}</div></article>`;
  }

  // Drawn again whole when the list or the theme in use changes. The key
  // focus, if it was on a card, goes back to the one looked at.
  function render() {
    const at = cards.contains(document.activeElement) ? document.activeElement : null;
    cards.innerHTML = whole.map((t) => card(t, { inUse: t.id === themes.current, focused: t.id === focused })).join("") + makeCard();
    if (at) (at.matches("[data-make]") ? cards.querySelector("[data-make]") : cards.querySelector(".tg-card.focused"))?.focus({ preventScroll: true });
  }

  // Shows `id` on the map, unless it's drawn already; a click or Enter keeps it.
  function look(id, { scroll = true } = {}) {
    if (!whole.some((t) => t.id === id)) return Promise.resolve();
    const changed = id !== focused;
    focused = id;
    for (const el of cards.querySelectorAll("[data-id]")) {
      el.classList.toggle("focused", el.dataset.id === id);
      el.setAttribute("aria-selected", String(el.dataset.id === id));
    }
    const el = cards.querySelector(`[data-id="${CSS.escape(id)}"]`);
    if (scroll) el?.scrollIntoView({ block: "nearest" });
    // The key focus follows what's looked at, so the arrows go on from it.
    if (cards.contains(document.activeElement) && document.activeElement !== el && !document.activeElement.matches("[data-make]")) el?.focus({ preventScroll: true });
    if (changed) sfx("hover");
    const from = cards.querySelector("[data-from]");
    if (from) from.textContent = whole.find((t) => t.id === id).name;
    return themes.shown === id ? Promise.resolve() : themes.preview(id).catch(() => {});
  }

  async function keep(id = focused) {
    if (!id) return;
    await look(id);
    if (id !== themes.current) await themes.set(id).catch(() => {});
    else sfx("select");
    render();
  }

  async function make() {
    const from = focused ?? themes.current;
    const res = await themes.copy(from);
    if (!res) return null;
    made = res;
    await themes.load();
    whole = await themes.whole();
    focused = res.id;
    render();
    cards.querySelector("[data-make]")?.scrollIntoView({ block: "nearest" });
    return res;
  }

  async function show(id) {
    if (!open) {
      open = true;
      made = null;
      root.hidden = false;
      requestAnimationFrame(() => root.classList.add("open"));
      sfx("open");
    }
    await themes.load();
    whole = await themes.whole();
    focused = id ?? themes.shown ?? themes.current;
    render();
    await look(focused);
    cards.querySelector(".tg-card.focused")?.focus({ preventScroll: true });
  }

  // Closing puts the theme in use back, if another was being looked at.
  async function hide() {
    if (!open) return;
    open = false;
    root.classList.remove("open");
    root.hidden = true;
    sfx("close");
    await themes.revert();
  }

  // A change to the list (a theme file added or edited) while it's open.
  async function refresh() {
    if (!open) return;
    whole = await themes.whole();
    render();
  }

  // Pointing at a card shows it, after a moment, so sweeping across the
  // list doesn't draw every theme it passes. Only the pointer moving
  // counts: cards redrawn or scrolled under a still one don't.
  let hover = null;
  const still = { x: null, y: null };
  cards.addEventListener("pointermove", (e) => {
    if (e.clientX === still.x && e.clientY === still.y) return;
    Object.assign(still, { x: e.clientX, y: e.clientY });
    const el = e.target.closest("[data-id]");
    clearTimeout(hover);
    if (el && el.dataset.id !== focused) hover = setTimeout(() => look(el.dataset.id, { scroll: false }), 140);
  });
  cards.addEventListener("mouseleave", () => clearTimeout(hover));
  cards.addEventListener("focusin", (e) => {
    const el = e.target.closest("[data-id]");
    if (el && el.dataset.id !== focused) look(el.dataset.id, { scroll: false });
  });
  root.addEventListener("click", async (e) => {
    if (e.target.closest("[data-tgal-close]")) return hide();
    if (e.target.closest("[data-tgal-copy]")) {
      await navigator.clipboard?.writeText(made.file).catch(() => {});
      return toast("Copied the theme's path");
    }
    if (e.target.closest("[data-tgal-reveal]")) return themes.reveal(made.id);
    if (e.target.closest("[data-make]")) return made ? null : make();
    const el = e.target.closest("[data-id]");
    if (el) keep(el.dataset.id);
  });
  // Its own keys stay its own: ↑↓ would step through a gathering on the
  // map, and Esc let go of the selection.
  root.addEventListener("keydown", (e) => {
    const all = [...cards.querySelectorAll(".tg-card")];
    const at = all.indexOf(document.activeElement);
    if (["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft", "Home", "End", "Enter", "Escape"].includes(e.key)) e.stopPropagation();
    if (["ArrowDown", "ArrowUp", "ArrowRight", "ArrowLeft", "Home", "End"].includes(e.key)) {
      e.preventDefault();
      const to = e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : Math.max(0, Math.min(all.length - 1, (at < 0 ? 0 : at) + (e.key === "ArrowDown" || e.key === "ArrowRight" ? 1 : -1)));
      all[to]?.focus();
    } else if (e.key === "Enter" && !e.target.closest("button")) {
      e.preventDefault();
      if (document.activeElement?.matches("[data-make]")) {
        if (!made) make();
      } else keep(document.activeElement?.dataset.id ?? focused);
    } else if (e.key === "Escape") {
      e.preventDefault();
      hide();
    }
  });

  return {
    show,
    hide,
    keep,
    make,
    refresh,
    // Looks at a card's theme, as pointing at it does; resolves once drawn.
    focus: (id) => look(id),
    get isOpen() {
      return open;
    },
    // Its cards, the one looked at, and the file made, for scripts and tests.
    state: () => (open ? { focused, cards: whole.map((t) => t.id), made: made ? { id: made.id, file: made.file } : null } : null),
  };
}
