// A film of the day, to share: the replay of the last 24 hours (replay.js)
// framed for the screen, with a title card, the time of day and running
// counts while it plays, and an end card with the day's numbers. The panels
// step aside; the map turns slowly. "Make a film of today" in the palette
// records it in the desktop app (electron/recorder.js); xenonet film records
// it from a hidden window.
import { escapeHtml as h } from "./format.js";
import { tokens } from "../core/units.js";

const TITLE_MS = 3200;
const END_MS = 4800;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const clock = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
const count = (n) => Number(n ?? 0).toLocaleString("en-US");

export function createFilm({ scene, replay }) {
  const root = document.createElement("div");
  root.className = "film";
  root.hidden = true;
  document.body.append(root);
  let state = "idle"; // idle, title, playing, end
  let raf = 0;

  function overlay(day) {
    root.innerHTML = `
      <div class="fl-top"><span class="fl-mark">XENONET</span><span class="fl-date">${h(day.date)}</span></div>
      <div class="fl-clock" data-fl-clock>--:--</div>
      <div class="fl-counts">
        <div><b data-fl-n="tool">0</b><span>tool calls</span></div>
        <div><b data-fl-n="prompt">0</b><span>prompts</span></div>
        <div><b data-fl-n="sessions">0</b><span>sessions at work</span></div>
      </div>`;
  }

  async function play({ seconds = 45 } = {}) {
    if (state !== "idle") return;
    const [day, history] = await Promise.all([fetch("/api/daylog").then((r) => r.json()), fetch("/api/history").then((r) => r.json())]);
    const events = history.events ?? [];
    if (!events.length) throw new Error("nothing happened today to make a film of");
    document.body.classList.add("filming");
    root.hidden = false;
    scene.reset();

    // Title.
    state = "title";
    const t = day.totals ?? {};
    root.innerHTML = `<div class="fl-card"><div class="fl-mark">XENONET</div><h1>${h(day.date)}</h1><p>${count(t.sessions)} sessions · ${count(t.toolCalls)} tool calls · ${tokens(day.tokens ?? 0)} tokens</p></div>`;
    await wait(TITLE_MS);

    // The day, played back.
    state = "playing";
    overlay(day);
    const els = { clock: root.querySelector("[data-fl-clock]"), tool: root.querySelector('[data-fl-n="tool"]'), prompt: root.querySelector('[data-fl-n="prompt"]'), sessions: root.querySelector('[data-fl-n="sessions"]') };
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
        els.clock.textContent = clock(cursor);
        els.tool.textContent = count(seen.tool);
        els.prompt.textContent = count(seen.prompt);
        els.sessions.textContent = count(seen.sessions.size);
        if (!replay.active || cursor >= replay.range.end) return resolve();
        raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    });

    // The end card.
    state = "end";
    const top = (day.topProjects ?? []).slice(0, 3).map((p) => `<li>${h(p.name)}<span>${count(p.toolCalls)} tool calls</span></li>`).join("");
    root.innerHTML = `<div class="fl-card end"><div class="fl-mark">XENONET</div><h1>${h(day.date)}</h1><p>${count(t.sessions)} sessions · ${count(t.turns)} turns · ${count(t.toolCalls)} tool calls · ${tokens(day.tokens ?? 0)} tokens</p>${top ? `<ol>${top}</ol>` : ""}</div>`;
    await wait(END_MS);
    stop();
  }

  function stop() {
    cancelAnimationFrame(raf);
    if (replay.active) replay.exit();
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
