// A film of the day, to share: the replay of the last 24 hours (replay.js)
// framed for the screen, with a title card, running counts while it plays,
// and an end card with the day's numbers (the cards: film-cards.js). The
// panels step aside; the map turns slowly. It shows the day's constellation
// (core/redact.js): star names, the archetype, no clock, unless the film
// reveals more. "Make a film of today" records it in the desktop app
// (electron/recorder.js), as a story (9:16); xenonet film records it from a
// hidden window.
import { fetchConstellation, fetchExportLabels } from "./poster.js";
import { titleCard, overlay, endCard } from "./film-cards.js";
import { filmPlaySeconds } from "../core/frames.js";

const TITLE_MS = 3200;
const END_MS = 4800;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const count = (n) => Number(n ?? 0).toLocaleString("en-US");

export function createFilm({ scene, replay }) {
  const root = document.createElement("div");
  root.className = "film";
  root.hidden = true;
  document.body.append(root);
  let state = "idle"; // idle, title, playing, end
  let raf = 0;

  // Plays the day for `seconds` (the cards add 8 more; core/frames.js),
  // revealing what `reveal` turns on (names, tokens).
  async function play({ seconds = filmPlaySeconds(), reveal = {} } = {}) {
    if (state !== "idle") return;
    const [day, history, labels] = await Promise.all([fetchConstellation(reveal), fetch("/api/history").then((r) => r.json()), reveal.names ? null : fetchExportLabels()]);
    const events = history.events ?? [];
    if (!events.length) throw new Error("nothing happened today to make a film of");
    document.body.classList.add("filming");
    // The map's own labels follow the cards: star names, and none on a
    // session, unless the film reveals the names (core/export-labels.js).
    scene.setRedaction(labels);
    root.hidden = false;
    scene.reset();

    // Title.
    state = "title";
    root.innerHTML = titleCard(day, reveal);
    await wait(TITLE_MS);

    // The day, played back.
    state = "playing";
    root.innerHTML = overlay(day, reveal);
    const els = { progress: root.querySelector("[data-fl-progress]"), tool: root.querySelector('[data-fl-n="tool"]'), prompt: root.querySelector('[data-fl-n="prompt"]'), sessions: root.querySelector('[data-fl-n="sessions"]') };
    await replay.enter({ seconds });
    let i = 0;
    const seen = { tool: 0, prompt: 0, sessions: new Set() };
    await new Promise((resolve) => {
      const tick = () => {
        const cursor = replay.cursor;
        while (i < events.length && events[i].ts <= cursor) {
          const e = events[i++];
          if (e.kind === "tool") seen.tool++;
          if (e.kind === "prompt") seen.prompt++;
          if (e.sessionId) seen.sessions.add(e.sessionId);
        }
        const { start, end } = replay.range;
        els.progress.style.setProperty("--p", String(Math.max(0, Math.min(1, (cursor - start) / Math.max(1, end - start)))));
        els.tool.textContent = count(seen.tool);
        els.prompt.textContent = count(seen.prompt);
        els.sessions.textContent = count(seen.sessions.size);
        if (!replay.active || cursor >= end) return resolve();
        raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    });

    // The end card.
    state = "end";
    root.innerHTML = endCard(day, reveal);
    await wait(END_MS);
    stop();
  }

  function stop() {
    cancelAnimationFrame(raf);
    if (replay.active) replay.exit();
    scene.setRedaction(null);
    document.body.classList.remove("filming");
    root.hidden = true;
    root.innerHTML = "";
    state = "idle";
  }

  return {
    play,
    stop,
    get state() {
      return state;
    },
  };
}
