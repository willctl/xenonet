// A film's cards (src/film-cards.js): the mark, the archetype, star names
// and running counts, with nothing of the projects, paths or times, unless
// the film reveals more.
import { test } from "node:test";
import assert from "node:assert/strict";
import { titleCard, overlay, endCard, filmLines } from "../src/film-cards.js";
import { MARK_BAND, MARK_URL } from "../src/mark.js";

const SECRETS = ["atlas-api", "ledger", "harbor-api", "dana", "/Users", "mbp.local"];
const day = {
  date: "Friday, October 3",
  archetype: "pulsar",
  totals: { sessions: 3, turns: 7, toolCalls: 1250, files: 12 },
  tokens: 1234567,
  stars: [
    { name: "Thaloren", toolCalls: 40, project: "atlas-api" },
    { name: "Vesku", toolCalls: 22, project: "ledger" },
    { name: "Drashi", toolCalls: 9, project: "harbor-api" },
    { name: "Kaelis", toolCalls: 2, project: "docs" },
  ],
};
const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();

test("each card: the mark, the archetype and the date, star names, and nothing else of the day", () => {
  for (const [name, html] of [["title", titleCard(day)], ["overlay", overlay(day)], ["end", endCard(day)]]) {
    const t = text(html);
    for (const s of SECRETS) assert.ok(!t.toLowerCase().includes(s.toLowerCase()), `${name}: ${s} in ${t}`);
    assert.doesNotMatch(t, /\b\d{1,2}:\d{2}\b/, `${name}: a clock`);
    assert.ok(t.includes(MARK_BAND) && t.includes(MARK_URL), `${name}: the mark`);
    assert.ok(t.includes("Pulsar"), `${name}: the archetype`);
    assert.ok(!/tokens/i.test(t), `${name}: tokens stay off`);
    assert.ok(html.includes('<svg class="fl-x"'), `${name}: the node-X`);
  }
  assert.ok(text(titleCard(day)).includes("Friday, October 3") && text(titleCard(day)).includes("3 sessions · 1,250 tool calls"));
  assert.ok(overlay(day).includes('data-fl-progress') && !overlay(day).includes("fl-clock"), "a line fills instead of a clock");
  const end = text(endCard(day));
  assert.ok(end.includes("brightest stars") && end.includes("Thaloren 40 tool calls") && end.includes("Drashi") && !end.includes("Kaelis"), end);
  assert.ok(end.includes("3 sessions · 7 turns · 1,250 tool calls"));
});

test("revealing more: the projects' names and the tokens", () => {
  const reveal = { names: true, tokens: true };
  const end = text(endCard(day, reveal));
  assert.ok(end.includes("busiest projects") && end.includes("atlas-api 40 tool calls") && !end.includes("Thaloren"));
  assert.ok(end.includes("1.2M tokens") && text(titleCard(day, reveal)).includes("1.2M tokens"));
  assert.deepEqual(filmLines({ ...day, stars: day.stars.map(({ project, ...s }) => s) }, { names: true }).stars.map((s) => s.name), ["Thaloren", "Vesku", "Drashi"], "no names from the server: the stars stay");
});

test("markup in the data never comes out as markup", () => {
  const odd = { ...day, date: "<b>x</b>", archetype: "<i>y</i>", stars: [{ name: '"><img src=x onerror=1>', toolCalls: 1 }] };
  for (const html of [titleCard(odd), overlay(odd), endCard(odd)]) assert.ok(!html.includes("<b>") && !html.includes("<i>") && !html.includes("<img"), html);
});

test("a day without an archetype: the date leads", () => {
  const l = filmLines({ date: "Monday, October 6", totals: {}, stars: [] });
  assert.equal(l.headline, "Monday, October 6");
  assert.equal(l.date, "");
  assert.equal(text(endCard({ date: "Monday, October 6", totals: {}, stars: [] })).includes("brightest"), false);
});
