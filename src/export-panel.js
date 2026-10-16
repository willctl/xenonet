// The export panel: today's constellation as a poster to save or copy, and
// as a film. A preview comes first, at the size picked (core/frames.js).
// What an export reveals (the projects' names, the tokens) is off each
// time the panel opens, until you turn it on here. The film is a story
// (9:16) of 15 to 30 seconds, which the desktop app records. Everything
// stays on this computer, and the panel says so.
import { escapeHtml as h } from "./format.js";
import { POSTER_SIZES, DEFAULT_SIZE, FILM_SECONDS, posterSize } from "../core/frames.js";
import { renderPoster, savePoster, copyPoster, posterBlob } from "./poster.js";

export const LOCAL_LINE = "Made on this computer. Nothing is uploaded, no account is needed, and the file carries no metadata.";
export const REDACTED_LINE = "Projects show as star names, and no clock appears, unless you reveal more for this export.";
export const REVEALED_LINE = "This export shows what you turned on below. The next one starts hidden again.";
const LENGTHS = [15, 20, 30];

export function createExportPanel({ modals, scene, sfx, toast, onFilm = null, canFilm = false }) {
  let st = null; // while open: { size, reveal, seconds, poster, url, key }
  let panel = null;
  let pending = 0;

  const html = () => `
    <header class="m-head">
      <div><div class="m-chip">▣ share today</div><h2>Constellation of the day</h2><div class="m-sub">${h(LOCAL_LINE)}</div></div>
      <button class="act" data-mclose>Close ✕</button>
    </header>
    <div class="m-body xp">
      <div class="xp-preview busy" data-xp-preview><span class="xp-wait">Rendering…</span></div>
      <div class="xp-row" data-xp-sizes>${POSTER_SIZES.map((s) => `<button class="xp-pick${s.id === st.size ? " on" : ""}" data-xp-size="${s.id}">${h(s.label)}<small>${s.width}×${s.height}</small></button>`).join("")}</div>
      <details class="xp-reveal" data-xp-reveal>
        <summary>Reveal more…</summary>
        <label><input type="checkbox" data-xp-names /> The projects' names, in place of the star names</label>
        <label><input type="checkbox" data-xp-tokens /> The tokens processed</label>
      </details>
      <p class="xp-note" data-xp-redacted>${h(REDACTED_LINE)}</p>
      <div class="m-foot">
        <button class="act" data-xp-copy>Copy as image</button>
        <button class="act primary" data-xp-save>Save PNG</button>
        <span class="grow"></span>
        <span class="xp-film">
          <span class="xp-label">Film · 9:16</span>
          ${LENGTHS.map((n) => `<button class="xp-pick xp-sec${n === st.seconds ? " on" : ""}" data-xp-seconds="${n}">${n} s</button>`).join("")}
          ${canFilm ? `<button class="act" data-xp-film>Record the film</button>` : `<span class="xp-note">Films record in the desktop app.</span>`}
        </span>
      </div>
    </div>`;

  const key = () => `${st.size}:${st.reveal.names ? 1 : 0}${st.reveal.tokens ? 1 : 0}`;

  // The poster for what's picked now: the preview's, or rendered again.
  async function current() {
    if (st.poster && st.key === key()) return st.poster;
    const poster = await renderPoster({ scene, size: st.size, reveal: { ...st.reveal } });
    return { ...poster, bytes: (await posterBlob(poster)).size };
  }

  // Renders the preview for what's picked; a later pick drops an earlier
  // render still on its way.
  async function preview() {
    if (!st || !panel) return;
    const mine = ++pending;
    const box = panel.querySelector("[data-xp-preview]");
    box.classList.add("busy");
    const at = key();
    try {
      const poster = await renderPoster({ scene, size: st.size, reveal: { ...st.reveal } });
      const blob = await posterBlob(poster);
      if (!st || mine !== pending) return;
      if (st.url) URL.revokeObjectURL(st.url);
      st.url = URL.createObjectURL(blob);
      st.poster = { ...poster, bytes: blob.size };
      st.key = at;
      box.innerHTML = `<img alt="A preview of today's poster" src="${st.url}" />`;
    } catch (err) {
      if (st && mine === pending) box.innerHTML = `<span class="xp-wait">${h(err.message)}</span>`;
    } finally {
      if (mine === pending) box.classList.remove("busy");
    }
  }

  function wire() {
    panel.querySelector("[data-xp-sizes]").addEventListener("click", (e) => {
      const b = e.target.closest("[data-xp-size]");
      if (!b) return;
      st.size = posterSize(b.dataset.xpSize).id;
      for (const el of panel.querySelectorAll("[data-xp-size]")) el.classList.toggle("on", el.dataset.xpSize === st.size);
      preview();
    });
    panel.querySelector("[data-xp-reveal]").addEventListener("change", () => {
      st.reveal = { names: panel.querySelector("[data-xp-names]").checked, tokens: panel.querySelector("[data-xp-tokens]").checked };
      panel.querySelector("[data-xp-redacted]").textContent = st.reveal.names || st.reveal.tokens ? REVEALED_LINE : REDACTED_LINE;
      preview();
    });
    panel.querySelector(".xp-film").addEventListener("click", (e) => {
      const b = e.target.closest("[data-xp-seconds]");
      if (!b) return;
      st.seconds = Number(b.dataset.xpSeconds);
      for (const el of panel.querySelectorAll("[data-xp-seconds]")) el.classList.toggle("on", Number(el.dataset.xpSeconds) === st.seconds);
    });
    panel.querySelector("[data-xp-save]").addEventListener("click", async () => {
      try {
        const p = await current();
        await savePoster(p);
        sfx("shutter");
        toast(`Saved a ${p.width}×${p.height} poster`);
      } catch (err) {
        toast(`The poster didn't save: ${err.message}`, true);
      }
    });
    panel.querySelector("[data-xp-copy]").addEventListener("click", async () => {
      try {
        const p = await current();
        await copyPoster(p);
        sfx("shutter");
        toast(`Copied the ${p.width}×${p.height} poster as an image`);
      } catch (err) {
        toast(`Couldn't copy the poster: ${err.message}`, true);
      }
    });
    panel.querySelector("[data-xp-film]")?.addEventListener("click", () => {
      const opts = { seconds: st.seconds, reveal: { ...st.reveal } };
      modals.close();
      onFilm?.(opts);
    });
  }

  function open({ size = DEFAULT_SIZE } = {}) {
    st = { size: posterSize(size).id, reveal: { names: false, tokens: false }, seconds: FILM_SECONDS.default, poster: null, url: null, key: null };
    panel = modals.open(html(), {
      accent: "var(--accent)",
      cls: "xp-panel",
      onClose: () => {
        if (st?.url) URL.revokeObjectURL(st.url);
        st = null;
        panel = null;
      },
    });
    wire();
    preview();
    return state();
  }

  const state = () => (st ? { size: st.size, reveal: { ...st.reveal }, seconds: st.seconds, preview: !!st.poster, local: LOCAL_LINE } : null);

  return {
    open,
    close: () => modals.close(),
    state,
    get isOpen() {
      return !!st;
    },
    // For the driver (driver.js): renders the poster now, at `size` and
    // with `reveal`, and says what it drew; `save` saves it too.
    async render({ size = st?.size ?? DEFAULT_SIZE, reveal = {}, save = false } = {}) {
      const poster = await renderPoster({ scene, size, reveal: { names: !!reveal.names, tokens: !!reveal.tokens } });
      const blob = await posterBlob(poster);
      if (save) await savePoster(poster);
      return { width: poster.width, height: poster.height, size: poster.size, bytes: blob.size, lines: poster.lines, labels: poster.labels, revealed: { names: !!reveal.names, tokens: !!reveal.tokens }, saved: !!save };
    },
  };
}
