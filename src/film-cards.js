// What a film of the day shows over the map (src/film.js): the title card,
// the overlay while the day plays, and the end card, from the day's
// constellation (core/redact.js). So the cards carry the mark, the day's
// archetype and its star names, and no clock: a line fills as the day
// plays instead. `reveal.names` puts the projects' names in, and
// `reveal.tokens` the tokens; both are off for every film until it turns
// them on. Markup only, so the tests read it.
import { escapeHtml as h } from "./format.js";
import { tokens } from "../core/units.js";
import { MARK_BAND, MARK_URL, markSvg } from "./mark.js";

const count = (n) => Number(n ?? 0).toLocaleString("en-US");
const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : "");

// What the cards say: the headline (the archetype, else the date), the
// date, a short and a long line of the day's numbers, and the brightest
// stars.
export function filmLines(data, reveal = {}) {
  const t = data.totals ?? {};
  const more = reveal.tokens ? [`${tokens(data.tokens ?? 0)} tokens`] : [];
  return {
    headline: data.archetype ? cap(data.archetype) : (data.date ?? ""),
    date: data.archetype ? (data.date ?? "") : "",
    summary: [`${count(t.sessions)} sessions`, `${count(t.toolCalls)} tool calls`, ...more].join(" · "),
    numbers: [`${count(t.sessions)} sessions`, `${count(t.turns)} turns`, `${count(t.toolCalls)} tool calls`, ...more].join(" · "),
    starsLabel: reveal.names ? "busiest projects" : "brightest stars",
    stars: (data.stars ?? []).slice(0, 3).map((s) => ({ name: reveal.names && s.project ? s.project : s.name, toolCalls: s.toolCalls ?? 0 })),
  };
}

// The mark: the band top left, the node-X and the site top right.
const mark = () => `<div class="fl-top"><span class="fl-mark">${h(MARK_BAND)}</span></div><div class="fl-site">${markSvg("fl-x")}<span>${h(MARK_URL)}</span></div>`;

export function titleCard(data, reveal) {
  const l = filmLines(data, reveal);
  return `${mark()}<div class="fl-card"><h1>${h(l.headline)}</h1>${l.date ? `<p class="fl-date">${h(l.date)}</p>` : ""}<p>${h(l.summary)}</p></div>`;
}

// While the day plays: the headline and a line that fills as it goes
// (data-fl-progress, --p from 0 to 1), and the running counts.
export function overlay(data, reveal) {
  const l = filmLines(data, reveal);
  return `${mark()}
    <div class="fl-day"><span class="fl-headline">${h(l.headline)}</span><i class="fl-progress" data-fl-progress style="--p:0"></i></div>
    <div class="fl-counts">
      <div><b data-fl-n="tool">0</b><span>tool calls</span></div>
      <div><b data-fl-n="prompt">0</b><span>prompts</span></div>
      <div><b data-fl-n="sessions">0</b><span>sessions at work</span></div>
    </div>`;
}

export function endCard(data, reveal) {
  const l = filmLines(data, reveal);
  const top = l.stars.map((s) => `<li>${h(s.name)}<span>${count(s.toolCalls)} tool calls</span></li>`).join("");
  return `${mark()}<div class="fl-card end"><h1>${h(l.headline)}</h1>${l.date ? `<p class="fl-date">${h(l.date)}</p>` : ""}<p>${h(l.numbers)}</p>${top ? `<div class="fl-label">${h(l.starsLabel)}</div><ol>${top}</ol>` : ""}</div>`;
}
