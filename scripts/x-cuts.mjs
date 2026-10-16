// Short videos for X and phones: a 30-second hero cut and a few demo clips,
// each square (1080 by 1080, the X feed) and vertical (1080 by 1920, Shorts,
// TikTok and Reels), from the made-up showcase workspace in the Eclipse
// theme. Run it hidden, so nothing plays aloud and no notification shows:
//
//   node cli/xenonet.js script run scripts/x-cuts.mjs --instance showcase --hidden
//
// It starts the instance afresh, hidden and in UTC, so no clock on screen
// tells where the films were made, and lists each clock or date it saw. For
// the recording only, it hides the alert cards but the one a beat is about,
// and draws collisions in a brighter color; both go back at the end.
//
// What to make, from the environment:
//   XCUTS_VIDEOS   videos by name, comma-separated (default: all of VIDEOS)
//   XCUTS_FORMATS  square, vertical or both (the default)
//   XCUTS_BEATS    record only these beats, as video/beat (x-hero/diff), and
//                  take the rest from the last run's work files
//   XCUTS_RECUT=1  cut and caption the beats again from their recordings,
//                  recording only the ones that have none
//   XCUTS_OUT      where the finished videos go (default .xenonet/x-cuts/out)
//   XCUTS_CLEAN=1  delete the work files once the videos are made
//   XCUTS_DEBUG=1  say which labels overlap or are cut in each view tried
//   XCUTS_WARM     seconds to let the made-up sessions run before recording,
//                  so a beat recorded on its own shows as much history (the
//                  stats' turns) as one recorded late in a full run
//
// Captions and the end card are drawn in the page, with its own fonts, and
// burned in. The sound is XenoNet's own mix (showcase-audio.mjs), aligned to
// each beat, with short crossfades at the cuts.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { findFfmpeg } from "../core/dirs.js";
import { prepareAudio, startAudio, stopAudio } from "./showcase-audio.mjs";

const FPS = 30;
// Audio past each beat's end, crossfaded into the next beat.
const TAIL = 0.3;
const END_SECONDS = 2.5;
const VOID = "#05070A";
const TEXT = "#DCE6EE";
const XENO = "#39FF9F";
// The films' colors: BT.709 at video's limited range, as players and sites
// expect. The recorder's frames are JPEGs (full range, BT.601), and the
// captions and end card are RGB; each is converted to these on the way in.
const TO709 = "scale=out_color_matrix=bt709:out_range=tv";
const AS709 = "setparams=colorspace=bt709:color_primaries=bt709:color_trc=bt709:range=tv";
const TAGS = ["-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"];

// Every beat is recorded once, in a phone-shaped window (540 by 960 points,
// drawn at the page's density and filmed 1080 wide), so the page uses its
// narrow layout and its text is phone-sized. The vertical film is all of it
// and the square one a 1080 by 1080 crop of it. Either can move over the
// recording: a beat's `square` and `vertical` give the crop's top, as a
// number or as [seconds, top] keys it eases between, or a function of what
// the recording measured that gives one of those. Measured, in the film's
// pixels: where the open sheet starts and ends (`sheet`, `sheetBottom`), the
// beat's `subject` element ({ top, bottom }), and the height of the nodes
// it `watch`es, before it acts and after (`focus0`, `focus`).
const WINDOW = [540, 960];
const FORMATS = {
  vertical: { size: [1080, 1920] },
  square: { size: [1080, 1080] },
};
// The square's top with a height in the film 400 pixels down it, clear of
// its caption.
const centered = (y) => Math.round(Math.min(840, Math.max(0, (y ?? 905) - 400)));
// How far the vertical film moves up to bring a height above its caption,
// and, if it moves, past the alert card at the top too (`cards`, where the
// cards showing end), so no sliver of one is left.
const up = (y) => Math.round(Math.min(700, Math.max(0, (y ?? 0) - 1170)));
const clear = (y, { cards }) => (up(y) > 0 ? Math.max(up(y), cards ? cards + 40 : 0) : 0);
// The theme the films are in: Eclipse, with a brighter collision color, so a
// collision's arc reads on a phone. It's written to the showcase's own themes
// folder for the run, and taken out after.
const FILM_THEME = "eclipse-film";
const FILM_THEME_FILE = { name: "Eclipse film", extends: "eclipse", colors: { collision: "#ff3d71" } };

const sleep = (ms) => new Promise((r) => setTimeout(r, Math.max(0, ms)));

// A clock time or a date in the page's text, which the films must not show:
// even in UTC, a render time and a posting time together tell a time zone.
const CLOCK = String.raw`\b\d{1,2}:\d{2}(?::\d{2})?\b|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]* \d{1,2}\b|\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)[a-z]*, |\bGMT\b|\bUTC\b`;

// Run in the page for the recording: every date and clock time in its text
// becomes words that say nothing of when ("today", a session's span as
// minutes, kept on one line), now and as the page changes; and the demo's
// projects folder reads ~/Projects everywhere, as the map's own labels have
// it. The page's own state is untouched.
const SCRUB = String.raw`(() => {
  if (window.__xcutsScrub) return true;
  const MON = "(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*";
  const T = "(\\d{1,2}):(\\d{2})(?::\\d{2})? ?([AaPp][Mm])?";
  const minutes = (h, m, ap) => ((Number(h) % 12) + (ap && /p/i.test(ap) ? 12 : 0)) * 60 + Number(m);
  const nb = "\u00a0";
  const span = (a, b) => { const d = (b - a + 1440) % 1440; return d ? d + nb + "min" : "a" + nb + "moment"; };
  const rules = [
    [new RegExp(MON + " \\d{1,2},?(?: ·)? " + T + " to " + T, "gi"), (_, h1, m1, a1, h2, m2, a2) => "today" + nb + "·" + nb + span(minutes(h1, m1, a1 || a2), minutes(h2, m2, a2))],
    [new RegExp(T + " to " + T, "gi"), (_, h1, m1, a1, h2, m2, a2) => span(minutes(h1, m1, a1 || a2), minutes(h2, m2, a2))],
    [new RegExp(MON + " \\d{1,2},?(?: ·)? " + T, "gi"), () => "today"],
    [new RegExp("\\b(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)[a-z]*, " + MON + " \\d{1,2}\\b", "gi"), () => "today"],
    [new RegExp("\\b" + MON + " \\d{1,2}\\b", "g"), () => "today"],
    [new RegExp("\\b" + T + "\\b", "g"), () => ""],
    [/~\/projects(?=\/|\b)/g, () => "~/Projects"],
  ];
  const scrub = (node) => {
    let text = node.nodeValue;
    for (const [re, to] of rules) text = text.replace(re, to);
    if (text !== node.nodeValue) node.nodeValue = text;
  };
  const all = (root) => {
    if (root.nodeType === 3) return scrub(root);
    const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) scrub(n);
  };
  all(document.body);
  new MutationObserver((changes) => {
    for (const c of changes) {
      if (c.type === "characterData") scrub(c.target);
      else for (const n of c.addedNodes) all(n);
    }
  }).observe(document.body, { subtree: true, childList: true, characterData: true });
  window.__xcutsScrub = true;
  return true;
})()`;

// An FFmpeg expression of time for a crop's top: a number, or [seconds, top]
// keys with eased moves between them.
function track(keys) {
  if (typeof keys === "number") return String(keys);
  let expr = String(keys[keys.length - 1][1]);
  for (let i = keys.length - 2; i >= 0; i--) {
    const [t0, y0] = keys[i];
    const [t1, y1] = keys[i + 1];
    const u = `clip((t-${t0})/${t1 - t0},0,1)`;
    expr = `if(lt(t,${t1}),${y0}+${y1 - y0}*${u}*${u}*(3-2*${u}),${expr})`;
  }
  return expr;
}

// ---------- the beats ----------

// A beat is a few seconds of the console with one caption. `prepare` sets the
// scene before recording, `run` acts while recording, with `at(name)` to mark
// the time and `until(seconds)` to wait for a time on the beat's clock.
// `cut(marks)` picks what's kept as [from, to, speed] spans of the recording;
// by default its first `seconds` after a short lead. `audio: "free"` keeps the
// sound at normal speed when a span is sped up (no effects in it to follow),
// and `push` zooms in slowly on a point of the frame.

const ROUTE_SOLVER = "Speed up the route solver for 500+ stops";
const ROUNDING = "Fix rounding in monthly reconciliation";
const STAGING = "Move staging to the new cluster";
const REFUNDS = "Add partial refunds to the orders API";
const KESTREL = "actor:kestrel";
const WREN = "actor:wren";
const MOTH = "actor:moth";

// Where a beat looks: the whole map (`core` in view), a step closer, a
// project with its sessions gathered round it, or one session or actor up
// close. Selecting opens the inspector, a sheet over the map on a narrow
// screen; framing closes it again and keeps the camera. The alert cards are
// hidden, except the one a beat is about (`prepare` shows it).
const SHEET = ".dock > .panel.sheet-on";
const BEATS = {
  // The map turning under a drag, already at speed in the first frame: the
  // camera eases into a drag, so the film starts most of a second in.
  map: {
    record: 5.2,
    watch: () => ["core"],
    async run({ x, at }) {
      at("drag");
      await x.orbit({ from: 40, by: 320, steps: 75 });
    },
    cut: (m) => [[m.drag + 0.5, 99]],
  },
  // Two sessions in neighboring projects, both in view with the actors
  // that float by them away, and the arc that joins them when they edit the
  // same file; its alert alone shows, in the vertical film. The square stays
  // on the two sessions and their arc.
  collision: {
    record: 5,
    watch: (x) => Promise.all([x.id(ROUTE_SOLVER), x.id(ROUNDING)]),
    async prepare({ x }) {
      await x.aside();
      await x.frameBoth(ROUTE_SOLVER, ROUNDING);
      await x.alerts(await x.collisionCard());
    },
    async run({ x, at, until }) {
      await until(0.4);
      at("collide");
      await x.collide();
    },
    cut: (m) => [[Math.max(0.2, m.collide - 0.3), 99]],
  },
  diff: {
    record: 6,
    watch: (x) => Promise.all([x.id(ROUTE_SOLVER), x.id(ROUNDING)]),
    async prepare({ x }) {
      await x.aside();
      await x.frameBoth(ROUTE_SOLVER, ROUNDING);
      await x.alerts(await x.collisionCard());
      await x.collide();
    },
    async run({ x, at, until }) {
      await until(0.5);
      at("open");
      await x.openDiff();
    },
    cut: (m) => [[Math.max(0.2, m.open - 0.25), 99]],
    subject: ".m-panel",
    square: ({ subject }) => centered((subject.top + subject.bottom) / 2),
    vertical: (seen) => clear(seen.subject.bottom, seen),
    push: { to: 1.07, at: [0.5, 0.45] },
  },
  // A session up close turns amber when it asks, its alert alone shows, then
  // its question in the inspector.
  question: {
    record: 5,
    watch: (x) => Promise.all([x.id(STAGING)]),
    async prepare({ x }) {
      await x.focus(STAGING);
      await x.alerts(await x.questionCard());
    },
    async run({ x, at, until }) {
      await until(0.4);
      at("ask");
      await x.ask();
      await until(2.3);
      await x.inspect();
    },
    cut: (m) => [[Math.max(0.2, m.ask - 0.3), 99]],
    subject: `${SHEET} .need-box`,
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [1.9, centered(focus0)], [2.6, sheet - 24]],
    vertical: (seen) => [[0, 0], [1.9, 0], [2.6, clear(seen.subject.bottom, seen)]],
  },
  "question-appear": {
    record: 5,
    watch: (x) => Promise.all([x.id(STAGING)]),
    async prepare({ x }) {
      await x.focus(STAGING);
      await x.alerts(await x.questionCard());
    },
    async run({ x, at, until }) {
      await until(0.4);
      at("ask");
      await x.ask();
    },
    cut: (m) => [[Math.max(0.2, m.ask - 0.3), 99]],
    square: ({ focus0 }) => centered(focus0),
  },
  "question-read": {
    record: 6,
    async prepare({ x }) {
      await x.focus(STAGING);
      await x.alerts(await x.questionCard());
      await x.ask();
    },
    async run({ x, at, until }) {
      await until(0.3);
      at("open");
      await x.inspect();
    },
    cut: (m) => [[Math.max(0.2, m.open - 0.1), 99]],
    subject: `${SHEET} .need-box`,
    watch: (x) => Promise.all([x.id(STAGING)]),
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [0.3, centered(focus0)], [1.1, sheet - 24]],
    vertical: (seen) => [[0, 0], [0.3, 0], [1.1, clear(seen.subject.bottom, seen)]],
  },
  // Actors message sessions and edit files, each a flash on the map, then
  // the activity feed lists them as they come.
  actors: {
    record: 4.6,
    watch: () => ["core"],
    prepare: ({ x }) => x.closer(),
    async run({ x, at, until }) {
      await until(0.2);
      at("act");
      await x.s.simulate("actor-message", { a: MOTH, b: await x.id(ROUTE_SOLVER) });
      await until(0.7);
      await x.s.simulate("actor-edit", { a: WREN });
      await until(1.3);
      await x.s.driver("sheet", "activity");
      await until(2.5);
      await x.s.simulate("actor-message", { a: KESTREL, b: await x.id(ROUNDING) });
    },
    cut: (m) => [[Math.max(0.2, m.act - 0.1), 99]],
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [1.2, centered(focus0)], [1.9, sheet - 24]],
  },
  edits: {
    record: 4.6,
    watch: () => ["core"],
    prepare: ({ x }) => x.closer(),
    async run({ x, at, until }) {
      await until(0.2);
      at("act");
      await x.s.simulate("actor-edit", { a: WREN });
      await until(0.8);
      await x.s.simulate("actor-edit", { a: MOTH });
      await until(1.4);
      await x.s.driver("sheet", "activity");
      await until(2.6);
      await x.s.simulate("actor-edit", { a: KESTREL });
    },
    cut: (m) => [[Math.max(0.2, m.act - 0.1), 99]],
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [1.3, centered(focus0)], [2.0, sheet - 24]],
  },
  message: {
    record: 5,
    watch: () => ["core"],
    prepare: ({ x }) => x.closer(),
    async run({ x, at, until }) {
      await until(0.2);
      at("act");
      await x.s.simulate("actor-message", { a: MOTH, b: await x.id(ROUTE_SOLVER) });
      await until(0.8);
      await x.s.simulate("actor-message", { a: WREN, b: await x.id(REFUNDS) });
      await until(1.4);
      await x.s.driver("sheet", "activity");
      await until(2.7);
      await x.s.simulate("actor-message", { a: KESTREL, b: await x.id(ROUNDING) });
    },
    cut: (m) => [[Math.max(0.2, m.act - 0.1), 99]],
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [1.3, centered(focus0)], [2.0, sheet - 24]],
  },
  // Grok Bot, away, comes online through its tunnel as the camera pushes
  // in, then its card shows who it is.
  remote: {
    record: 4.8,
    watch: () => [KESTREL],
    prepare: ({ x }) => x.s.simulate("actor-offline", { a: KESTREL }),
    async run({ x, at, until }) {
      at("push");
      x.push();
      await until(0.25);
      await x.wake(KESTREL);
      await x.pushing;
      await until(1.6);
      await x.s.driver("select", KESTREL);
    },
    cut: (m) => [[m.push + 0.2, 99]],
    square: ({ sheet }) => [[0, centered(905)], [1.6, centered(905)], [2.4, sheet - 24]],
  },
  "remote-wake": {
    record: 6.6,
    watch: () => [KESTREL],
    prepare: ({ x }) => x.s.simulate("actor-offline", { a: KESTREL }),
    async run({ x, at, until }) {
      at("push");
      x.push();
      await until(0.25);
      await x.wake(KESTREL);
      await x.pushing;
      await until(2.6);
      await x.s.driver("select", KESTREL);
    },
    cut: (m) => [[m.push + 0.2, 99]],
    square: ({ sheet }) => [[0, centered(905)], [2.6, centered(905)], [3.4, sheet - 24]],
  },
  // Grok Bot up close: its tunnel's gate, and reads going through it.
  // You and Wren, which float by pathfinder too, are away (the next beat
  // wakes Wren); the view is the clearest one of pathfinder with Kestrel
  // and its gate above it.
  reach: {
    record: 6.6,
    watch: async (x) => [KESTREL, await x.place("pathfinder")],
    async prepare({ x }) {
      await x.wake(KESTREL);
      await x.aside(["actor:you", WREN]);
      const pathfinder = await x.place("pathfinder");
      const starts = [0, 1].map((tilt) => ({ name: "pathfinder", tilt, look: () => x.look(pathfinder) }));
      await x.bestView("reach", { starts, targets: [KESTREL, pathfinder] });
    },
    async run({ x, at, until }) {
      at("read");
      for (const t of [0.2, 1.3, 2.4, 3.5, 4.6, 5.7]) {
        await until(t);
        await x.s.simulate("actor-read", { a: KESTREL });
      }
    },
    cut: () => [[0.2, 99]],
  },
  "wake-hermes": {
    record: 6.2,
    watch: () => [WREN],
    async prepare({ x }) {
      await x.wake(KESTREL);
      await x.s.simulate("actor-offline", { a: WREN });
      await x.closer();
    },
    async run({ x, at, until }) {
      await until(0.1);
      at("wake");
      await x.wake(WREN);
      await until(1.9);
      await x.s.driver("select", WREN);
    },
    cut: (m) => [[Math.max(0.1, m.wake - 0.05), 99]],
    square: ({ sheet }) => [[0, centered(905)], [1.9, centered(905)], [2.7, sheet - 24]],
  },
  // The camera flies to a session that keeps failing, its halo flickering
  // orange, and its alert alone shows. A session stuck in the demo's own
  // data: a simulated loop on a live session is undone by its next step, and
  // an idle session's halo is too faint to show the flicker.
  stuck: {
    record: 5.6,
    watch: (x) => Promise.all([x.id(ROUNDING)]),
    prepare: async ({ x }) => x.alerts(await x.stuckCard()),
    async run({ x, at, until }) {
      await until(0.3);
      at("fly");
      await x.focus(ROUNDING);
    },
    cut: (m) => [[Math.max(0.2, m.fly - 0.15), 99]],
    square: ({ focus0, focus }) => [[0, centered(focus0)], [0.3, centered(focus0)], [1.8, centered(focus)]],
  },
  "stuck-read": {
    record: 4.6,
    watch: (x) => Promise.all([x.id(ROUNDING)]),
    async prepare({ x }) {
      await x.focus(ROUNDING);
      await x.alerts(await x.stuckCard());
    },
    async run({ x, at, until }) {
      await until(0.3);
      at("open");
      await x.inspect();
    },
    cut: (m) => [[Math.max(0.2, m.open - 0.1), 99]],
    subject: `${SHEET} .stuck-box`,
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [0.2, centered(focus0)], [1.0, sheet - 24]],
    vertical: (seen) => [[0, 0], [0.2, 0], [1.0, clear(seen.subject.bottom, seen)]],
  },
  // The sheet sits at the bottom, under the caption: the vertical film moves
  // up to it, leaving the top of the map.
  tokens: {
    record: 5.4,
    watch: () => ["core"],
    async run({ x, at, until }) {
      await until(0.4);
      at("open");
      await x.s.driver("sheet", "tokens");
    },
    cut: (m) => [[Math.max(0.2, m.open - 0.2), 99]],
    square: ({ focus0, sheet }) => [[0, centered(focus0)], [0.8, centered(focus0)], [1.6, sheet - 24]],
    vertical: ({ sheetBottom }) => [[0, 0], [0.8, 0], [1.6, up(sheetBottom)]],
  },
  // A session's stats, scrolled down to its cost per turn.
  stats: {
    record: 6,
    async run({ x, at, until }) {
      await until(0.3);
      // Marked once the panel is up: it waits for its numbers first, and
      // the cut shouldn't show the bare map meanwhile.
      await x.s.driver("stats", await x.id(REFUNDS));
      at("open");
      // A slow scroll through to the end, so the panel never stands still.
      for (let i = 0; i < 11; i++) {
        await until(1.2 + i * 0.3);
        await x.s.input({ scroll: { x: 270, y: 520, dy: 34 }, settle: false });
      }
    },
    cut: (m) => [[Math.max(0.2, m.open - 0.1), 99]],
    square: 0,
  },
  // Themes, each kept for a moment once it's drawn.
  themes: {
    record: 9,
    watch: () => ["core"],
    async run({ x, at }) {
      at("eclipse0");
      await sleep(1000);
      for (const id of ["phosphor", "helios", "gridline", FILM_THEME]) {
        await x.s.driver("theme", id);
        at(id);
        await sleep(1100);
      }
    },
    cut: (m) => [[m.eclipse0 + 0.2, m.eclipse0 + 1], ...["phosphor", "helios", "gridline", FILM_THEME].map((id) => [m[id], m[id] + 0.8])],
  },
  "all-themes": {
    record: 14,
    watch: () => ["core"],
    async run({ x, at }) {
      at("eclipse0");
      await sleep(1000);
      for (const id of ["xenonet", "phosphor", "ochre", "gridline", "helios", "rime", "trench", FILM_THEME]) {
        await x.s.driver("theme", id);
        at(id);
        await sleep(1000);
      }
    },
    cut: (m) => [[m.eclipse0 + 0.15, m.eclipse0 + 0.8], ...["xenonet", "phosphor", "ochre", "gridline", "helios", "rime", "trench", FILM_THEME].map((id) => [m[id], m[id] + 0.85])],
  },
  environments: {
    record: 9,
    watch: () => ["core"],
    async run({ x, at }) {
      at("nebula0");
      await sleep(900);
      for (const id of ["aurora", "trench", "vault", "nebula"]) {
        await x.s.driver("environment", id);
        at(id);
        await sleep(1300);
      }
    },
    cut: (m) => [[m.nebula0 + 0.1, m.nebula0 + 0.8], ...["aurora", "trench", "vault", "nebula"].map((id) => [m[id], m[id] + 1.05])],
  },
};

// The videos: beats with their captions and lengths, then the end card.
// Captions follow the copy rules: plain words, no dashes. "SI" appears only
// on the end card ("The network for SI."); captions say what's on screen.
export const VIDEOS = {
  "x-hero": {
    poster: 5.6,
    beats: [
      ["map", 3, "See every session on one live map."],
      ["collision", 4, "XenoNet flags two sessions editing one file."],
      ["diff", 4, "See both edits side by side."],
      ["question", 4, "A session waiting on you turns amber."],
      ["actors", 4, "Messages and edits show up live."],
      ["remote", 4, "Grok Bot comes online."],
      ["themes", 4, "Pick one of eight themes."],
    ],
  },
  "clip-collision": {
    poster: 5.5,
    beats: [
      ["map", 3, "See every session on one live map."],
      ["collision", 4.5, "Two sessions edit the same file."],
      ["diff", 5, "See both edits side by side."],
    ],
  },
  "clip-question": {
    poster: 2.5,
    beats: [
      ["question-appear", 4.5, "A session has a question for you."],
      ["question-read", 5, "Read it before you switch apps."],
    ],
  },
  "clip-talk": {
    poster: 2.5,
    beats: [
      ["edits", 4, "Edits appear live in the activity feed."],
      ["collision", 4, "XenoNet flags two sessions editing one file."],
      ["message", 4.5, "Messages to sessions appear in the feed."],
    ],
  },
  "clip-remote": {
    poster: 4.5,
    beats: [
      ["remote-wake", 6, "Grok Bot comes online."],
      ["reach", 6, "Its tunnel reaches into pathfinder."],
      ["wake-hermes", 5.5, "Hermes Agent comes online too."],
    ],
  },
  "clip-stuck": {
    poster: 2.5,
    beats: [
      ["stuck", 4.5, "Stuck in a loop? It flickers orange."],
      ["stuck-read", 3, "See what keeps failing."],
    ],
  },
  "clip-cost": {
    poster: 2.5,
    beats: [
      ["tokens", 4.5, "Watch spend and plan limits live."],
      ["stats", 5, "See each session's cost, turn by turn."],
    ],
  },
  "clip-looks": {
    poster: 3,
    beats: [
      ["all-themes", 7.5, "Pick one of eight themes."],
      ["environments", 5, "Then pick the surroundings."],
    ],
  },
};

// ---------- the run ----------

export default async function ({ xenonet: s }) {
  if (s.id !== "showcase") throw new Error("Use --instance showcase, whose data is made up.");
  const ffmpeg = findFfmpeg();
  if (!ffmpeg) throw new Error("Put ffmpeg on PATH first.");
  // A hidden window is muted and sends no notifications; a shown one could
  // play the recording's sound aloud.
  if ((await s.ping())?.window?.visible) throw new Error("Run this with --hidden, so the instance stays silent.");

  const root = path.resolve(import.meta.dirname, "..");
  const work = path.join(root, ".xenonet", "x-cuts");
  const out = path.resolve(process.env.XCUTS_OUT || path.join(work, "out"));
  fs.mkdirSync(out, { recursive: true });
  const list = (v) => (v ? v.split(",").map((t) => t.trim()).filter(Boolean) : null);
  const videos = list(process.env.XCUTS_VIDEOS) ?? Object.keys(VIDEOS);
  const formats = list(process.env.XCUTS_FORMATS) ?? Object.keys(FORMATS);
  const only = list(process.env.XCUTS_BEATS);
  for (const v of videos) if (!VIDEOS[v]) throw new Error(`no video "${v}"; there's ${Object.keys(VIDEOS).join(", ")}`);
  for (const f of formats) if (!FORMATS[f]) throw new Error(`no format "${f}"; there's square and vertical`);
  if (only) for (const b of only) if (!videos.some((v) => VIDEOS[v].beats.some(([id]) => `${v}/${id}` === b))) throw new Error(`no beat ${b} in ${videos.join(", ")}`);

  const mark = fs.readFileSync(path.join(root, "public", "brand", "xenonet.svg"), "utf8").match(/<path d="([^"]+)"/)[1];

  // FFmpeg runs alongside the recording, never blocking it: a blocked script
  // lets the server close the connection the next control request reuses.
  // One encode at a time, so the page keeps its frame rate.
  let queue = Promise.resolve();
  // `report` returns what FFmpeg printed, for a filter's measurements.
  const run = (args, { report = false } = {}) => new Promise((resolve, reject) => {
    const p = spawn(ffmpeg, ["-hide_banner", "-loglevel", report ? "info" : "error", "-y", ...args], { stdio: ["ignore", "inherit", report ? "pipe" : "inherit"] });
    let said = "";
    p.stderr?.on("data", (d) => (said += d));
    p.on("error", reject);
    p.on("exit", (code) => (code === 0 ? resolve(said) : reject(new Error(`ffmpeg exited with ${code}${said ? `: ${said.trim().split("\n").pop()}` : ""}`))));
  });
  const encode = (args) => (queue = queue.then(() => run(args)));

  // A fresh instance each run: its audio graph has to be new (prepareAudio),
  // and its clock is UTC, so no time on screen tells where it was recorded.
  // The page's time zone is set to UTC as well, and checked.
  process.env.TZ = "UTC";
  console.log("Starting the showcase instance afresh, hidden, in UTC");
  await s.quit();
  await s.launch({ show: "hidden" });
  await s.call({ op: "cdp", method: "Emulation.setTimezoneOverride", params: { timezoneId: "UTC" } });
  const zone = await s.eval("return { zone: Intl.DateTimeFormat().resolvedOptions().timeZone, offset: new Date().getTimezoneOffset() }");
  if (zone.zone !== "UTC" || zone.offset !== 0) throw new Error(`the page's clock isn't UTC: ${JSON.stringify(zone)}`);
  await prepareAudio(s);
  // The page counts as in front, so an alert plays its own sound (which is
  // recorded) instead of leaving it to a notification (which a hidden
  // window doesn't send).
  await s.call({ op: "cdp", method: "Emulation.setFocusEmulationEnabled", params: { enabled: true } });
  await s.driver("themeGallery", false);
  await s.driver("revertEnvironment");
  await s.driver("maskIdentities", true);
  await s.eval(`return ${SCRUB}`);
  const themes = (await s.api("/api/themes")).folder;
  const themeFile = path.join(themes, `${FILM_THEME}.json`);
  fs.writeFileSync(themeFile, JSON.stringify(FILM_THEME_FILE, null, 2));
  for (let i = 0; !(await s.api("/api/themes")).themes.some((t) => t.id === FILM_THEME); i++) {
    if (i > 50) throw new Error(`the showcase didn't pick up ${themeFile}`);
    await sleep(100);
  }

  // ---------- acting on the console ----------

  const ids = new Map();
  const x = {
    s,
    async id(title) {
      if (!ids.has(title)) {
        const found = (await s.driver("find", { query: title, kind: "session", limit: 5 })).nodes.find((n) => n.title === title);
        if (!found) throw new Error(`the showcase has no session "${title}"`);
        ids.set(title, found.id);
      }
      return ids.get(title);
    },
    collide: async () => s.simulate("collision", { a: await x.id(ROUTE_SOLVER), b: await x.id(ROUNDING) }),
    ask: async () => s.simulate("needs-you", { a: await x.id(STAGING) }),
    // The alert cards: all hidden, or only those `only` (a CSS selector)
    // matches, for the recording only; nothing is dismissed.
    alerts: (only) =>
      s.eval(`return (() => {
        let style = document.getElementById('x-cuts-alerts');
        if (!style) { style = document.createElement('style'); style.id = 'x-cuts-alerts'; document.head.append(style); }
        style.textContent = ${JSON.stringify(only)} ? '#alerts > :not(' + ${JSON.stringify(only)} + ') { display: none !important; }' : '#alerts { display: none !important; }';
        return true;
      })()`),
    // The cards a beat is about: the collision it makes, the question it
    // asks, the stuck session.
    collisionCard: async () => `.collision[data-al-collision*="${await x.id(ROUTE_SOLVER)}"][data-al-collision*="${await x.id(ROUNDING)}"]`,
    questionCard: async () => `.need[data-al-goto="${await x.id(STAGING)}"]`,
    stuckCard: async () => `.stuck[data-al-goto="${await x.id(ROUNDING)}"]`,
    // The camera on a project, its sessions gathered, or on one session or
    // actor, with the inspector's sheet closed so the map shows.
    frame: async (project) => x.look(await x.place(project)),
    focus: async (title) => x.look(await x.id(title)),
    // Two sessions in view, between the alert card and the caption, framed
    // by either one's project from the clearest view (see bestView), with
    // the two furthest apart counting in a view's favor.
    async frameBoth(a, b) {
      const pair = [await x.id(a), await x.id(b)];
      const starts = [];
      for (const project of ["ledger", "pathfinder"]) for (const tilt of [0, 1]) starts.push({ name: project, tilt, look: async () => x.frame(project) });
      await x.bestView(pair.join("|"), { starts, targets: pair, apart: true });
    },
    // The clearest view that keeps `targets` between the alert card and the
    // caption: from each start (a framing, from the map's own angle or
    // tilted), turned a step at a time and further out until the targets
    // show. Fewest labels overlapping or cut by the frame's edge wins, then
    // the nearest, then (with `apart`) the two targets furthest apart. The
    // first time, every view is tried and the best is refined once the
    // actors are at rest; it's kept, by `key`, for the rest of the run, with
    // the camera's exact place, since replaying the drags lands a little
    // differently each time.
    async bestView(key, { starts, targets, apart = false }) {
      const begin = async (start) => {
        await s.driver("select", null);
        await s.driver("reset");
        await start.look();
        if (start.tilt) await x.turn({ up: true });
      };
      // In the frame, and (with `apart`) at least 160 points between two
      // targets, so neither sits on the other.
      const fits = (at) => at.every((p) => p?.onScreen && p.x > 40 && p.x < WINDOW[0] - 40 && p.y > 140 && p.y < 560) && (!apart || Math.hypot(at[0].x - at[1].x, at[0].y - at[1].y) >= 160);
      const show = async () => {
        for (let k = 0; ; k++) {
          const at = await Promise.all(targets.map((id) => s.driver("screenPos", id)));
          const ok = fits(at);
          if (ok || k >= 10) return ok ? { at, out: k } : null;
          await x.zoom(160);
        }
      };
      x.views ??= new Map();
      if (!x.views.has(key)) {
        let best = null;
        for (const start of starts) {
          await begin(start);
          for (let turn = 0; turn < 6; turn++) {
            if (turn) await x.turn();
            const seen = await show();
            if (seen) {
              const bad = await x.overlaps();
              // Apart, but not so far up and down that the square can't
              // hold both above its caption.
              const spread = apart && seen.at.length > 1 ? Math.hypot(seen.at[0].x - seen.at[1].x, seen.at[0].y - seen.at[1].y) / 120 - (Math.abs(seen.at[0].y - seen.at[1].y) > 200 ? 3 : 0) : 0;
              const score = bad * 4 + seen.out * 1.5 - spread;
              if (!best || score < best.score) best = { start, turn, bad, out: seen.out, score, camera: await x.camera() };
            }
            // Back in as far as this view went out, so the next turn
            // starts as near as this one did.
            for (let k = 0; k < (seen?.out ?? 10); k++) await x.zoom(-160);
          }
        }
        if (!best) throw new Error(`couldn't frame ${key}`);
        // Then, with the actors at rest, a little nearer or further, a few
        // degrees round and a step to either side, to push a label the edge
        // cuts wholly in or out.
        await begin(best.start);
        await x.camera(best.camera);
        await x.still();
        best.bad = await x.overlaps();
        const [cx, cy, cz, tx, ty, tz] = best.camera;
        const reach = Math.hypot(cx - tx, cz - tz) || 1;
        const [rx, rz] = [(cz - tz) / reach, -(cx - tx) / reach];
        const nudges = [];
        for (const pan of [0, -1, 1, -2, 2, -3, 3]) for (const dz of [0, -1, 1, -2, 2]) for (const deg of [0, -6, 6]) if (pan || dz || deg) nudges.push([pan, dz, deg]);
        for (const [pan, dz, deg] of nudges) {
          if (!best.bad) break;
          const a = (deg * Math.PI) / 180;
          const [vx, vz] = [(cx - tx) * Math.cos(a) - (cz - tz) * Math.sin(a), (cx - tx) * Math.sin(a) + (cz - tz) * Math.cos(a)];
          const k = 1 + dz * 0.05;
          const [sx, sz] = [rx * pan * reach * 0.04, rz * pan * reach * 0.04];
          const camera = [tx + sx + vx * k, ty + (cy - ty) * k, tz + sz + vz * k, tx + sx, ty, tz + sz];
          await x.camera(camera);
          await sleep(80);
          const at = await Promise.all(targets.map((id) => s.driver("screenPos", id)));
          if (!fits(at)) continue;
          const bad = await x.overlaps();
          if (bad < best.bad) best = { ...best, bad, camera };
        }
        console.log(`  ${key}: from ${best.start.name}${best.start.tilt ? ", tilted" : ""}, turned ${best.turn}, ${best.out} steps out: ${best.bad} labels overlapping or cut`);
        x.views.set(key, best);
      }
      const best = x.views.get(key);
      await begin(best.start);
      const back = await x.camera(best.camera);
      await x.still();
      if (process.env.XCUTS_DEBUG) console.log(`  ${key} restored`, JSON.stringify(best.camera.map(Math.round)), JSON.stringify(back.map(Math.round)), await x.overlaps());
    },
    // The camera's place and what it looks at, or (given one) back there,
    // held: the map's own slow turn stays off for the beat.
    camera: (to) =>
      s.eval(`return (() => {
        const g = window.__xenonet.scene.graph;
        const camera = g.camera(), controls = g.controls();
        const to = ${JSON.stringify(to ?? null)};
        if (to) {
          controls.autoRotate = false;
          camera.position.set(to[0], to[1], to[2]);
          controls.target.set(to[3], to[4], to[5]);
          controls.update();
        }
        return [camera.position.x, camera.position.y, camera.position.z, controls.target.x, controls.target.y, controls.target.z];
      })()`),
    // The map's labels on screen, in points: [owner, left, top, right,
    // bottom, size], measured from its own sprites and camera, with each
    // actor's orb as a box too (`orbs`). Size is the label's height in the
    // map: places and actors are 3 and up, sessions and pages smaller.
    labels: (orbs = true) =>
      s.eval(`return (() => {
        const g = window.__xenonet.scene.graph;
        const camera = g.camera();
        const W = innerWidth, H = innerHeight;
        const V = camera.position.constructor;
        const screen = (v) => { const p = v.clone().project(camera); return { x: ((p.x + 1) / 2) * W, y: ((1 - p.y) / 2) * H, z: p.z }; };
        const up = new V(0, 1, 0).applyQuaternion(camera.quaternion);
        const right = new V(1, 0, 0).applyQuaternion(camera.quaternion);
        const shown = (o) => { for (let q = o; q; q = q.parent) if (!q.visible) return false; return true; };
        const owner = new Map();
        for (const a of window.__xenonet.scene.actors()) { if (a.label) owner.set(a.label, a.id); }
        const boxes = [];
        g.scene().traverse((o) => {
          if (!o.isSprite || typeof o.text !== "string" || !o.text.trim() || !shown(o) || (o.material?.opacity ?? 1) < 0.15) return;
          const c = new V(); o.getWorldPosition(c);
          // A sprite's z scale is 0, so its own world scale doesn't
          // decompose: its parent's, times its own.
          const ps = new V(1, 1, 1); if (o.parent) o.parent.getWorldScale(ps);
          const m = screen(c);
          if (m.z > 1) return;
          const t = screen(c.clone().addScaledVector(up, (o.scale.y * ps.y) / 2));
          const r = screen(c.clone().addScaledVector(right, (o.scale.x * ps.x) / 2));
          const w = 2 * Math.abs(r.x - m.x), h = 2 * Math.abs(m.y - t.y);
          const x0 = m.x - w * o.center.x, y0 = m.y - h * (1 - o.center.y);
          boxes.push([owner.get(o) ?? o.uuid, x0, y0, x0 + w, y0 + h, o.textHeight ?? 0, o.text.slice(0, 24)]);
        });
        if (${orbs}) for (const a of window.__xenonet.scene.actors()) {
          if (!a.body || !shown(a.body)) continue;
          const c = new V(); a.body.getWorldPosition(c);
          const m = screen(c);
          if (m.z > 1) continue;
          // The orb's reach: its meshes' bounding spheres, in the map's space.
          let reach = 0;
          a.body.traverse((mesh) => {
            if (!(mesh.isMesh || mesh.isLine) || !mesh.geometry) return;
            if (!mesh.geometry.boundingSphere) mesh.geometry.computeBoundingSphere();
            const ws = new V(); mesh.getWorldScale(ws);
            reach = Math.max(reach, mesh.geometry.boundingSphere.radius * Math.max(ws.x, ws.y));
          });
          const rad = Math.abs(screen(c.clone().addScaledVector(up, reach)).y - m.y);
          boxes.push([a.id + "#orb", m.x - rad, m.y - rad, m.x + rad, m.y + rad, 99, a.id]);
        }
        return boxes;
      })()`),
    // How many of the map's labels are cut by the frame's edge anywhere above
    // the tab bar (the caption's shade lets them show through), or, above
    // the caption and below the alert card, overlap another where one of the
    // two is a place's or an actor's (or its orb): a project's gathered
    // sessions sit close by design, their small labels touching.
    async overlaps() {
      const all = (await x.labels()).filter(([, x0, y0, x1, y1]) => x1 > 0 && x0 < WINDOW[0] && y1 > 0 && y0 < WINDOW[1] - 60);
      const faults = [];
      let count = 0;
      for (const [o, x0, y0, x1, , , text] of all) {
        if (o.endsWith("#orb") || !(x0 < 0 || x1 > WINDOW[0] || y0 < 0)) continue;
        // Under the caption's shade a cut label shows dimly, so it counts
        // for less.
        count += y0 > 575 ? 0.5 : 1;
        faults.push(`${text} cut`);
      }
      const boxes = all.filter(([, , y0, , y1]) => y1 > 90 && y0 < 575);
      for (let i = 0; i < boxes.length; i++)
        for (let j = i + 1; j < boxes.length; j++) {
          const [p, ...a] = boxes[i];
          const [q, ...b] = boxes[j];
          if (Math.max(a[4], b[4]) < 3 || p.replace("#orb", "") === q.replace("#orb", "")) continue;
          // Two places' or actors' names need a line's room between them,
          // or they read as one stack.
          const gap = Math.min(a[4], b[4]) >= 3 && !p.endsWith("#orb") && !q.endsWith("#orb") ? 40 : 0;
          if (a[0] < b[2] && b[0] < a[2] && a[1] - gap < b[3] && b[1] - gap < a[3]) (count += 1), faults.push(`${a[5]} on ${b[5]}`);
        }
      if (process.env.XCUTS_DEBUG && faults.length) console.log("    ", faults.join(", "));
      return count;
    },
    // The actors at rest: yours glides to wherever the camera last stopped
    // a few seconds after it does, and the others ease to their places.
    async still() {
      await sleep(4000);
      let last = null;
      for (const end = Date.now() + 8000; Date.now() < end; await sleep(500)) {
        const orbs = (await x.labels()).filter(([o]) => o.endsWith("#orb")).map(([o, x0, y0, x1, y1]) => [o, (x0 + x1) / 2, (y0 + y1) / 2]);
        const moved = last ? Math.max(0, ...orbs.map(([o, cx, cy]) => { const was = last.find(([w]) => w === o); return was ? Math.hypot(cx - was[1], cy - was[2]) : 0; })) : Infinity;
        last = orbs;
        if (moved < 2) return;
      }
    },
    // The actors that float by pathfinder and ledger (you, Wren, Kestrel)
    // offline, so they drift away from the two sessions and their labels;
    // the next beat's clear brings them back.
    async aside(actors = ["actor:you", WREN, KESTREL]) {
      for (const a of actors) await s.simulate("actor-offline", { a }).catch(() => null);
      await sleep(2500);
    },
    // A project's id, by its name.
    async place(name) {
      const found = (await s.driver("find", { query: name, kind: "project", limit: 5 })).nodes.find((n) => n.title === name);
      if (!found) throw new Error(`the showcase has no project "${name}"`);
      return found.id;
    },
    // The camera a step round the map, or tilted, at rest after.
    async turn({ up = false } = {}) {
      if (up) await s.input({ drag: { from: { x: 60, y: 820 }, to: { x: 60, y: 760 }, steps: 14 }, settle: false });
      else await x.orbit({ from: 60, by: 90, steps: 18 });
      await x.park();
      await s.driver("settled");
    },
    // The pointer off the map, on the tabs under the caption's band, so no
    // node's hover card shows.
    park: () => s.input({ move: { x: WINDOW[0] - 30, y: WINDOW[1] - 10 }, settle: false }),
    // The sheet closes as the camera sets off, so it doesn't rise over the
    // map on the way.
    async look(id) {
      await s.driver("select", id, { settle: false });
      await s.driver("sheet", null, { settle: false });
      await s.driver("settled");
      await sleep(300);
    },
    // The inspector's sheet again, for what's selected.
    inspect: () => s.driver("sheet", "selected"),
    // The camera nearer the middle of the map (dy below 0) or further out.
    async zoom(dy) {
      await s.input({ scroll: { x: WINDOW[0] / 2, y: WINDOW[1] / 2, dy } });
      await x.park();
      await sleep(600);
    },
    closer: () => x.zoom(-300),
    // A slow drag across the bottom of the map, which turns it.
    async orbit({ from = 60, by = 240, steps = 100 } = {}) {
      await s.input({ drag: { from: { x: from, y: 870 }, to: { x: from + by, y: 870 }, steps }, settle: false });
      await x.park();
    },
    // The camera pushing in, a wheel step at a time for a second and a
    // half, so it's moving when the film starts; the beat goes on meanwhile.
    push() {
      x.pushing = (async () => {
        for (let i = 0; i < 12; i++) {
          await s.input({ scroll: { x: WINDOW[0] / 2, y: WINDOW[1] / 2, dy: -70 }, settle: false });
          await sleep(110);
        }
        await x.park();
      })();
    },
    async wake(actor) {
      await s.simulate("actor-tunnel", { a: actor });
      await s.simulate("actor-online", { a: actor });
    },
    async openDiff() {
      const p = await s.eval(`return (() => {
        const card = [...document.querySelectorAll('[data-al-collision]')].find((el) => el.textContent.includes('app.ps1'));
        const r = card?.querySelector('.al-btn')?.getBoundingClientRect();
        return r && r.width ? { x: r.x + r.width / 2, y: r.y + r.height / 2 } : null;
      })()`);
      if (!p) throw new Error("the collision's card isn't showing");
      await s.input({ click: p, settle: false });
    },
    // What the crops follow, in the film's pixels: the open sheet's top and
    // bottom, the beat's subject element, the nodes it watches (their mean
    // height on screen); and every clock or date showing, to check.
    async measure(beat) {
      const watched = beat.watch ? await beat.watch(x) : [];
      const at = await Promise.all(watched.map((id) => s.driver("screenPos", id).catch(() => null)));
      const ys = at.filter((p) => p?.onScreen).map((p) => p.y);
      const page = await s.eval(`return (() => {
        const px = 1080 / innerWidth;
        const box = (el) => { const r = el?.getBoundingClientRect(); return r && r.height ? { top: Math.round(r.top * px), bottom: Math.round(r.bottom * px) } : null; };
        const sheet = box(document.querySelector('.dock > .panel.sheet-on'));
        const shown = [...document.querySelectorAll('#alerts > *')].filter((el) => el.checkVisibility()).map((el) => box(el)?.bottom ?? 0);
        const subject = ${JSON.stringify(beat.subject ?? null)} ? box(document.querySelector(${JSON.stringify(beat.subject ?? "")})) : null;
        return { sheet: sheet?.top ?? null, sheetBottom: sheet?.bottom ?? null, subject, cards: shown.length ? Math.max(...shown) : 0 };
      })()`);
      const px = 1080 / WINDOW[0];
      return { ...page, labels: await x.filmLabels(), clocks: await x.clocks(), focus: ys.length ? Math.round((ys.reduce((a, b) => a + b, 0) / ys.length) * px) : null };
    },
    // The map's labels in the film's pixels, [left, top, right, bottom,
    // text], those at least partly in the frame and not wholly behind an
    // open panel, sheet or alert card.
    async filmLabels() {
      const px = 1080 / WINDOW[0];
      const covers = await s.eval(`return [...document.querySelectorAll('#modal:not([hidden]) .m-panel, .dock > .panel.sheet-on, #alerts > *')]
        .filter((el) => el.checkVisibility())
        .map((el) => { const r = el.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom]; })
        .filter(([l, t, r, b]) => r > l && b > t)`);
      const under = ([x0, y0, x1, y1]) => covers.some(([l, t, r, b]) => x0 >= l && x1 <= r && y0 >= t && y1 <= b);
      return (await x.labels(false))
        .filter(([, ...b]) => !under(b))
        .map(([, ...b]) => [...b.slice(0, 4).map((v) => Math.round(v * px)), b[5]])
        .filter(([x0, y0, x1, y1]) => x1 > 0 && x0 < 1080 && y1 > 0 && y0 < 1920);
    },
    // Any clock time or date showing in the page's text now, or the
    // projects folder in lower case.
    clocks: () =>
      s.eval(`return (() => {
        const found = [];
        const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
        const when = new RegExp(${JSON.stringify(CLOCK)}, "i");
        for (let n = walk.nextNode(); n; n = walk.nextNode()) {
          if (!when.test(n.textContent) && !n.textContent.includes("~/projects")) continue;
          const el = n.parentElement;
          const r = el.getBoundingClientRect();
          if (!r.width || !r.height || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
          if (el.closest('[hidden]') || !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
          found.push(n.textContent.trim().slice(0, 80));
        }
        return found;
      })()`),
  };

  // Every beat starts from the same console: the film's Eclipse, no
  // selection, nothing simulated, no alert cards, the whole map in view.
  async function scene() {
    await s.simulate("clear");
    let st = await s.driver("state");
    if (st.overlays.modal || st.overlays.preview) await s.input({ keys: "Escape" });
    if (st.overlays.gallery) await s.driver("themeGallery", false);
    if (st.overlays.wallpaper) await s.driver("command", "wallpaper");
    if (st.sheet?.current) await s.driver("sheet", null);
    if (st.selection) await s.driver("select", null);
    if (st.theme.inUse !== FILM_THEME || st.theme.id !== FILM_THEME) await s.driver("theme", FILM_THEME);
    if (st.environment.inUse !== "nebula") await s.driver("environment", "nebula");
    await x.alerts(null);
    await s.eval(`return ${SCRUB}`);
    await x.park();
    await s.driver("reset");
    // Cards and states that clear went with are on their way out.
    await sleep(900);
    st = await s.driver("state");
    if (st.overlays.modal) await s.input({ keys: "Escape" });
  }

  // ---------- drawing in the page ----------

  // A PNG drawn by `body` (a function of g, the 2D context, W and H) in the
  // page, where Chakra Petch is loaded.
  async function draw(file, [W, H], body) {
    const png = await s.eval(`return (async () => {
      await document.fonts.load('600 64px "Chakra Petch"');
      await document.fonts.load('500 40px "JetBrains Mono"');
      const c = document.createElement('canvas'); c.width = ${W}; c.height = ${H};
      const g = c.getContext('2d');
      const W = ${W}, H = ${H}, MARK = ${JSON.stringify(mark)};
      (${body})(g, W, H, MARK);
      return c.toDataURL('image/png').split(',')[1];
    })()`);
    fs.writeFileSync(file, Buffer.from(png, "base64"));
    return file;
  }

  // A caption low in the frame, over a dark band to the bottom: white Chakra
  // Petch, centered, one or two lines. The first caption of a video carries
  // the green accent.
  const caption = (file, fmt, text, accent) =>
    draw(file, FORMATS[fmt].size, `(g, W, H) => {
      const size = 66, lead = 80, pad = 72;
      g.font = '600 ' + size + 'px "Chakra Petch", sans-serif';
      // One line if it fits; else two, broken where the longer line is
      // shortest, after a sentence if that's nearly as good.
      const words = ${JSON.stringify(text)}.split(' ');
      const width = (ws) => g.measureText(ws.join(' ')).width;
      let lines = [words.join(' ')];
      if (width(words) > W - 2 * pad) {
        let best = null;
        for (let i = 1; i < words.length; i++) {
          const cost = Math.max(width(words.slice(0, i)), width(words.slice(i))) - (/[.,?:]$/.test(words[i - 1]) ? size * 2.5 : 0);
          if (!best || cost < best.cost) best = { i, cost };
        }
        lines = [words.slice(0, best.i).join(' '), words.slice(best.i).join(' ')];
      }
      // The vertical caption centers at 70% of the height, clear of the
      // buttons and text Shorts, TikTok and Reels lay over the bottom.
      const bottom = ${fmt === "vertical" ? "Math.round(H * 0.7 + lines.length * lead / 2 + 4)" : "H - 70"};
      const top = bottom - lines.length * lead - (${accent} ? 34 : 0);
      const band = g.createLinearGradient(0, top - 120, 0, top - 20);
      band.addColorStop(0, 'rgba(5,7,10,0)');
      band.addColorStop(1, 'rgba(5,7,10,0.86)');
      g.fillStyle = band; g.fillRect(0, top - 120, W, 100);
      g.fillStyle = 'rgba(5,7,10,0.86)'; g.fillRect(0, top - 20, W, H - top + 20);
      if (${accent}) { g.fillStyle = '${XENO}'; g.fillRect(W / 2 - 36, top, 72, 6); }
      g.fillStyle = '#FFFFFF'; g.textAlign = 'center'; g.textBaseline = 'alphabetic';
      lines.forEach((line, i) => g.fillText(line, W / 2, bottom - (lines.length - 1 - i) * lead - 14));
    }`);

  // The end card: the mark, the line and where to find XenoNet.
  const endCard = (file, fmt) =>
    draw(file, FORMATS[fmt].size, `(g, W, H, MARK) => {
      const cy = H / 2 - ${fmt === "vertical" ? 120 : 40};
      g.save(); g.translate(W / 2 - 96, cy - 250); g.scale(3, 3); g.fillStyle = '${XENO}'; g.fill(new Path2D(MARK)); g.restore();
      g.textAlign = 'center';
      g.fillStyle = '${TEXT}'; g.font = '600 84px "Chakra Petch", sans-serif';
      g.fillText('The network for SI.', W / 2, cy + 70);
      g.fillStyle = '#9FB0BD'; g.font = '500 40px "JetBrains Mono", monospace';
      g.fillText('xenonet.dev · open source', W / 2, cy + 150);
    }`);

  // ---------- recording a beat ----------

  async function record(video, index) {
    const [id, seconds] = VIDEOS[video].beats[index];
    const beat = BEATS[id];
    const { raw, audio, meta } = rawOf(video, index);
    fs.mkdirSync(path.dirname(raw), { recursive: true });
    console.log(`Recording ${video}/${id}`);
    await scene();
    await beat.prepare?.({ x });
    if (beat.prepare) await sleep(1200);
    // What the crops follow, before the beat acts and after.
    const before = await x.measure(beat);
    const marks = {};
    const audioStartedAt = await startAudio(s);
    const video0 = await s.record({ action: "start", file: raw, fps: FPS, width: 1080, size: windowPx, keep: true });
    const t0 = video0.startedAt;
    // The camera, every frame the page draws, to show a move is smooth: its
    // place and what it looks at, by the clock the recording keeps.
    await s.eval(`return (() => {
      const g = window.__xenonet.scene.graph;
      const camera = g.camera(), controls = g.controls();
      const log = (window.__xcutsCamera = []);
      const tick = () => {
        if (window.__xcutsCamera !== log) return;
        const p = camera.position, q = controls.target;
        log.push([performance.timeOrigin + performance.now(), p.x, p.y, p.z, q.x, q.y, q.z].map((v) => Math.round(v * 100) / 100));
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      return true;
    })()`);
    const at = (n) => (marks[n] = (Date.now() - t0) / 1000);
    const until = (sec) => sleep(t0 + sec * 1000 - Date.now());
    let captured;
    let after;
    let cameraLog = [];
    // The page's text is checked for clocks and dates all through the beat.
    const seenClocks = new Set();
    // And where the labels are, since actors drift and labels fade while the
    // camera holds: a square's top is set against all of them.
    const seenLabels = [];
    let sampling = true;
    const sampler = (async () => {
      for (let n = 0; sampling; n++) {
        for (const c of await x.clocks().catch(() => [])) seenClocks.add(c);
        if (n % 2 === 0) {
          const t = (Date.now() - t0) / 1000;
          const boxes = await x.filmLabels().catch(() => null);
          if (boxes) seenLabels.push([Math.round(t * 100) / 100, boxes]);
        }
        await sleep(300);
      }
    })();
    try {
      await beat.run?.({ x, at, until });
      await until(beat.record ?? seconds + 0.8);
      after = await x.measure(beat);
    } finally {
      sampling = false;
      await sampler;
      cameraLog = await s.eval(`return (() => { const log = window.__xcutsCamera ?? []; window.__xcutsCamera = null; return log; })()`).catch(() => []);
      try {
        captured = await s.record({ action: "stop" });
      } finally {
        await stopAudio(s, audio);
      }
    }
    const offset = (t0 - audioStartedAt) / 1000;
    if (!Number.isFinite(offset) || offset < 0) throw new Error("Missing audio/video timing.");
    const seen = { sheet: after.sheet ?? before.sheet ?? 1920, sheetBottom: after.sheetBottom ?? before.sheetBottom ?? 1920, subject: after.subject ?? before.subject ?? { top: 600, bottom: 1170 }, focus0: before.focus, focus: after.focus ?? before.focus, cards: Math.max(before.cards, after.cards), labels: after.labels };
    const clocks = [...new Set([...before.clocks, ...after.clocks, ...seenClocks])];
    const camera = cameraLog.map(([t, ...pose]) => [Math.round(t - t0) / 1000, ...pose]);
    fs.writeFileSync(meta, JSON.stringify({ video, id, marks, frames: captured.frames, offset, ...seen, clocks, seenLabels, camera }));
  }

  // A beat's recording, its sound and what's known of it.
  function rawOf(video, index) {
    const [id] = VIDEOS[video].beats[index];
    const base = path.join(work, "raw", video, `${String(index + 1).padStart(2, "0")}-${id}`);
    return { raw: `${base}.mp4`, audio: `${base}-audio.webm`, meta: `${base}.json` };
  }

  // A recorded beat, cut and captioned for each format, on the encode queue.
  async function cutBeat(video, index) {
    const [id, seconds, text] = VIDEOS[video].beats[index];
    const beat = BEATS[id];
    const { raw, audio, meta } = rawOf(video, index);
    const { marks, frames, offset, seenLabels = [], ...seen } = JSON.parse(fs.readFileSync(meta, "utf8"));
    const spans = cutOf(beat, marks, seconds, frames / FPS);
    // Every label shown while the kept spans run, and at the end.
    const kept = seenLabels.filter(([t]) => spans.some(([a, b]) => t >= a - 0.1 && t <= b + 0.1)).flatMap(([, boxes]) => boxes);
    seen.labels = [...(seen.labels ?? []), ...kept];
    for (const fmt of formats) {
      const dir = path.join(work, fmt, video);
      fs.mkdirSync(dir, { recursive: true });
      const name = path.basename(raw, ".mp4");
      const overlay = await caption(path.join(dir, `${name}-caption.png`), fmt, text, index === 0);
      // The square follows the watched nodes unless the beat says otherwise.
      // A top worked out from what was seen is settled clear of the labels;
      // one the beat gives as a number stays where it says.
      const place = (fmt === "square" ? beat.square ?? (({ focus }) => centered(focus)) : beat.vertical) ?? null;
      const top = typeof place === "function" ? settleTop(place(seen), seen, fmt, beat.subject ? seen.subject : null) : place;
      if (typeof top === "number" || fmt === "vertical") {
        const cut = cutLabels(seen.labels, fmt === "square" ? top : 0, fmt);
        console.log(`  ${video}/${id} ${fmt}: ${cut.length ? `labels cut by the frame's edge: ${cut.join(", ")}` : "no label cut by the frame's edge"}`);
      }
      encode(beatArgs({ raw, audio, overlay, file: path.join(dir, `${name}.mkv`), offset, seconds, spans, top, fade: index > 0, push: beat.push, size: FORMATS[fmt].size, free: beat.audio === "free" }))
        .then(() => fs.rmSync(overlay, { force: true }))
        // A failed encode is reported where the queue is awaited.
        .catch(() => {});
    }
  }

  // A square's top moved, as little as it can be, so that no label is cut by
  // an edge or sits in the band just above the caption, and an alert card at
  // the top is wholly in or wholly out. A beat's subject (an open panel)
  // stays whole above the caption. A moving crop is left as it is.
  function settleTop(top, { labels = [], cards = 0 }, fmt, subject = null) {
    if (fmt !== "square" || typeof top !== "number") return top;
    if (subject && subject.bottom - subject.top > 720) return top;
    const holds = (t) => !subject || (subject.top >= t && subject.bottom <= t + 720);
    const faults = (t) =>
      cutLabels(labels, t, fmt).length +
      labels.filter(([, y0, , y1]) => y1 > t + 720 && y0 < t + 880).length +
      (cards && t > 0 && t < cards + 16 ? 1 : 0);
    let best = { t: top, f: faults(top) };
    for (let d = 10; d <= 220 && best.f; d += 10)
      for (const t of [top - d, top + d])
        if (t >= 0 && t <= 840 && holds(t) && faults(t) < best.f) best = { t, f: faults(t) };
    if (best.f && cards && holds(0) && faults(0) < best.f) best = { t: 0, f: faults(0) };
    return best.t;
  }

  // The labels a frame's edges cut, by their text: the sides, a square's top
  // and bottom, and a vertical's top (its bottom is the tab bar's).
  function cutLabels(labels, top, fmt) {
    const bottom = fmt === "square" ? top + 1080 : 1800;
    const cut = new Set();
    for (const [x0, y0, x1, y1, text = "?"] of labels) {
      if (y1 <= top || y0 >= bottom) continue;
      if (x0 < 0 || x1 > 1080 || y0 < top || y1 > bottom) cut.add(text);
    }
    return [...cut];
  }

  // The spans kept, fitted to the beat's length: the last one runs on as far
  // as the length needs (the recording's last frame holds if it's short).
  function cutOf(beat, marks, seconds, length) {
    const spans = (beat.cut ? beat.cut(marks) : [[0.3, 0.3 + seconds]]).map(([a, b, speed = 1]) => [a, Math.min(b, length), speed]);
    let total = 0;
    const kept = [];
    for (const [a, b, speed] of spans) {
      const left = seconds - total;
      if (left <= 0.001) break;
      const end = Math.min(b, a + left * speed);
      kept.push([a, end, speed]);
      total += (end - a) / speed;
    }
    return kept;
  }

  // One beat, encoded once more only at the end: H.264 at high quality and
  // PCM sound, with TAIL seconds of sound past the picture for the crossfade.
  function beatArgs({ raw, audio, overlay, file, offset, seconds, spans, top, fade, push, size: [W, H], free }) {
    const v = spans.map(([a, b, speed], i) => `[0:v]trim=start=${a}:end=${b},setpts=(PTS-STARTPTS)/${speed}[v${i}]`);
    let chain = `${spans.map((_, i) => `[v${i}]`).join("")}concat=n=${spans.length}:v=1:a=0,${TO709},format=yuv420p,fps=${FPS},tpad=stop_mode=clone:stop_duration=3,trim=duration=${seconds},setpts=PTS-STARTPTS`;
    // The film moves over the recording, which runs on in the void below.
    if (top !== null) chain += `,pad=1080:3840:0:0:color=${VOID.replace("#", "0x")},crop=${W}:${H}:0:'${track(top)}'`;
    // A slow push in on part of the frame, from the beat's start.
    if (push) {
      const z = `(1+${push.to - 1}*min(1,on/${seconds * FPS}))`;
      chain += `,scale=${W * 2}:${H * 2}:flags=lanczos,zoompan=z='${z}':x='(iw-iw/zoom)*${push.at[0]}':y='(ih-ih/zoom)*${push.at[1]}':d=1:s=${W}x${H}:fps=${FPS}`;
    }
    const filter = [...v, `${chain}[pic]`, `[1:v]format=rgba${fade ? ",fade=t=in:st=0:d=0.18:alpha=1" : ""},${TO709},format=yuva420p[cap]`, `[pic][cap]overlay=0:0:format=yuv420,format=yuv420p,${AS709}[out]`];
    // The sound: each span's own, sped up with it, or (free) the beat's
    // sound from its first frame at normal speed.
    if (free) filter.push(`[2:a]atrim=start=${offset + spans[0][0]}:duration=${seconds + TAIL},asetpts=PTS-STARTPTS,apad,atrim=duration=${seconds + TAIL}[snd]`);
    else {
      spans.forEach(([a, b, speed], i) => {
        const end = i === spans.length - 1 ? b + TAIL * speed + 1 : b;
        filter.push(`[2:a]atrim=start=${offset + a}:end=${offset + end},asetpts=PTS-STARTPTS${speed !== 1 ? `,atempo=${speed}` : ""},afade=t=in:d=0.012[a${i}]`);
      });
      filter.push(`${spans.map((_, i) => `[a${i}]`).join("")}concat=n=${spans.length}:v=0:a=1,apad,atrim=duration=${seconds + TAIL}[snd]`);
    }
    return ["-i", raw, "-loop", "1", "-framerate", String(FPS), "-t", String(seconds), "-i", overlay, "-i", audio, "-filter_complex", filter.join(";"), "-map", "[out]", "-map", "[snd]", "-frames:v", String(Math.round(seconds * FPS)), "-c:v", "libx264", "-preset", "fast", "-crf", "12", "-pix_fmt", "yuv420p", ...TAGS, "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2", file];
  }

  // ---------- the end card's sound ----------

  async function endSound() {
    const file = path.join(work, "end-audio.webm");
    await scene();
    await startAudio(s);
    await sleep((END_SECONDS + 0.6) * 1000);
    await stopAudio(s, file);
    return file;
  }

  // ---------- putting a video together ----------

  async function assemble(fmt, video) {
    const { beats, poster } = VIDEOS[video];
    const [W, H] = FORMATS[fmt].size;
    const dir = path.join(work, fmt, video);
    const files = beats.map(([id], i) => path.join(dir, `${String(i + 1).padStart(2, "0")}-${id}.mkv`));
    for (const f of files) if (!fs.existsSync(f)) throw new Error(`${path.relative(root, f)} is missing; record that beat`);
    const card = await endCard(path.join(dir, "end.png"), fmt);
    const total = beats.reduce((t, [, sec]) => t + sec, 0) + END_SECONDS;
    const n = files.length;
    const audioEnd = path.join(work, "end-audio.webm");
    const filter = [
      `${files.map((_, i) => `[${i}:v]`).join("")}concat=n=${n}:v=1:a=0[body]`,
      `color=c=${VOID.replace("#", "0x")}:s=${W}x${H}:r=${FPS}:d=${END_SECONDS},format=rgb24[bg]`,
      `[${n}:v]format=rgba,fade=t=in:st=0:d=0.35:alpha=1[ct]`,
      `[bg][ct]overlay=0:0:format=rgb,${TO709},format=yuv420p,fps=${FPS}[card0]`,
      // The card's last half second becomes the video's first frame, so a
      // looping player comes round without a jump.
      `[0:v]trim=end_frame=1,loop=loop=${FPS / 2 - 1}:size=1:start=0,setpts=N/${FPS}/TB,format=yuv420p,fps=${FPS}[first]`,
      `[card0][first]xfade=transition=fade:duration=0.5:offset=${END_SECONDS - 0.5}[card]`,
      `[body][card]concat=n=2:v=1:a=0,format=yuv420p,${AS709}[v]`,
    ];
    let last = "[0:a]";
    for (let i = 1; i < n; i++) {
      filter.push(`${last}[${i}:a]acrossfade=d=${TAIL}:c1=tri:c2=tri[x${i}]`);
      last = `[x${i}]`;
    }
    filter.push(`[${n + 1}:a]aformat=sample_rates=48000:channel_layouts=stereo,atrim=start=0.3:duration=${END_SECONDS},asetpts=PTS-STARTPTS,apad,atrim=duration=${END_SECONDS}[end]`);
    filter.push(`${last}[end]acrossfade=d=${TAIL}:c1=tri:c2=tri,afade=t=in:d=0.12,afade=t=out:st=${total - 0.7}:d=0.7,atrim=duration=${total}[a]`);
    const output = path.join(out, `${video}-${fmt}.mp4`);
    const mixed = path.join(dir, "mixed.mkv");
    await run([
      ...files.flatMap((f) => ["-i", f]),
      "-loop", "1", "-framerate", String(FPS), "-t", String(END_SECONDS), "-i", card,
      "-i", audioEnd,
      "-filter_complex", filter.join(";"),
      "-map", "[v]", "-map", "[a]",
      "-r", String(FPS), "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-profile:v", "high", "-pix_fmt", "yuv420p", ...TAGS, "-g", String(FPS * 2),
      "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2", mixed,
    ]);
    // XenoNet's mix is quiet for a phone: brought to -16 LUFS, the usual
    // level for social video, by one steady gain measured first.
    const target = "I=-16:TP=-1.5:LRA=11";
    const measured = JSON.parse((await run(["-i", mixed, "-af", `loudnorm=${target}:print_format=json`, "-f", "null", "-"], { report: true })).match(/\{[^{}]*"input_i"[^{}]*\}/)[0]);
    const gain = `loudnorm=${target}:measured_I=${measured.input_i}:measured_TP=${measured.input_tp}:measured_LRA=${measured.input_lra}:measured_thresh=${measured.input_thresh}:offset=${measured.target_offset}:linear=true`;
    await run(["-i", mixed, "-map", "0:v", "-map", "0:a", "-c:v", "copy", "-af", gain, "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2", "-movflags", "+faststart", output]);
    fs.rmSync(mixed, { force: true });
    await run(["-ss", String(poster), "-i", output, "-frames:v", "1", "-q:v", "2", output.replace(/\.mp4$/, ".jpg")]);
    fs.rmSync(card, { force: true });
    console.log(`Saved ${output}`);
    return { video, format: fmt, file: path.basename(output), poster: path.basename(output).replace(/\.mp4$/, ".jpg"), seconds: total, captions: beats.map(([, , text]) => text) };
  }

  // ---------- the run ----------

  const made = [];
  // The window takes the phone's shape, in points, for the page's density.
  const { viewport } = await s.window({ size: null });
  const windowPx = WINDOW.map((p) => p * viewport.dpr);
  try {
    const warm = path.join(work, "warm.mp4");
    fs.mkdirSync(work, { recursive: true });
    await s.record({ action: "start", file: warm, fps: FPS, width: 1080, size: windowPx, keep: true });
    await sleep(500);
    await s.record({ action: "stop" });
    fs.rmSync(warm, { force: true });
    const warmUp = Number(process.env.XCUTS_WARM) || 0;
    if (warmUp > 0) {
      console.log(`Letting the made-up sessions run ${warmUp} s first`);
      await sleep(warmUp * 1000);
    }
    // XCUTS_RECUT=1 cuts the beats again from their recordings, as when a
    // crop or a caption changes, recording only those that have none.
    const recut = process.env.XCUTS_RECUT === "1";
    for (const video of videos) {
      for (let i = 0; i < VIDEOS[video].beats.length; i++) {
        const [id] = VIDEOS[video].beats[i];
        const chosen = !only || only.includes(`${video}/${id}`);
        if (chosen && !(recut && fs.existsSync(rawOf(video, i).meta))) await record(video, i);
        if (chosen || recut) await cutBeat(video, i);
      }
    }
    if (!only || !fs.existsSync(path.join(work, "end-audio.webm"))) await endSound();
    await queue;
    for (const fmt of formats) for (const video of videos) made.push(await assemble(fmt, video));
    await scene();

    // What was made, for review.html.
    const manifest = path.join(out, "x-cuts.json");
    const before = fs.existsSync(manifest) ? JSON.parse(fs.readFileSync(manifest, "utf8")) : [];
    const all = [...before.filter((m) => !made.some((n) => n.file === m.file)), ...made].sort((a, b) => a.file.localeCompare(b.file));
    fs.writeFileSync(manifest, JSON.stringify(all, null, 2));
    fs.writeFileSync(path.join(out, "review.html"), review(all));
    if (process.env.XCUTS_CLEAN === "1") fs.rmSync(work, { recursive: true, force: true });
  } finally {
    // The showcase goes back to how it was: Eclipse, its alerts showing.
    await s.driver("theme", "eclipse").catch(() => {});
    fs.rmSync(themeFile, { force: true });
    await s.eval("return document.getElementById('x-cuts-alerts')?.remove() ?? true").catch(() => {});
  }

  // Every clock or date on screen in the beats made, to check: the page
  // runs in UTC, so none can tell where the films were recorded.
  const clocks = new Map();
  for (const video of videos)
    for (let i = 0; i < VIDEOS[video].beats.length; i++) {
      const { meta } = rawOf(video, i);
      if (fs.existsSync(meta)) for (const c of JSON.parse(fs.readFileSync(meta, "utf8")).clocks ?? []) clocks.set(c, [...(clocks.get(c) ?? []), `${video}/${VIDEOS[video].beats[i][0]}`]);
    }
  console.log(clocks.size ? `Clocks and dates on screen (UTC):\n${[...clocks].map(([c, where]) => `  ${c}  (${[...new Set(where)].join(", ")})`).join("\n")}` : "No clocks or dates on screen.");

  const logs = await s.logs({ level: "error" });
  if (logs.entries.length) throw new Error(`The page reported errors: ${logs.entries.map((e) => e.text).join("; ")}`);
}

// A page that plays every video, square and vertical side by side, with its
// captions and length.
function review(all) {
  const h = (t) => String(t).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const names = [...new Set(all.map((m) => m.video))].sort((a, b) => (a === "x-hero" ? -1 : b === "x-hero" ? 1 : a.localeCompare(b)));
  const cell = (m) => `<figure><video src="${h(m.file)}" poster="${h(m.poster)}" controls loop muted playsinline preload="metadata"></video><figcaption>${h(m.format)} · ${m.seconds.toFixed(1)} s · <a href="${h(m.file)}">${h(m.file)}</a></figcaption></figure>`;
  const rows = names.map((name) => {
    const items = all.filter((m) => m.video === name).sort((a, b) => a.format.localeCompare(b.format));
    return `<section><h2>${h(name)}</h2><ol>${items[0].captions.map((c) => `<li>${h(c)}</li>`).join("")}<li class="end">End card: The network for SI. · xenonet.dev · open source</li></ol><div class="row">${items.map(cell).join("")}</div></section>`;
  });
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>XenoNet X cuts</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; padding: 24px 16px 64px; background: #05070a; color: #dce6ee; font: 15px/1.5 -apple-system, "Segoe UI", sans-serif; }
  h1 { font-size: 22px; margin: 0 0 4px; } p.lead { color: #9fb0bd; margin: 0 0 24px; }
  section { border-top: 1px solid #1c242c; padding: 20px 0; }
  h2 { font-size: 17px; margin: 0 0 8px; color: #fff; }
  ol { margin: 0 0 16px; padding-left: 20px; color: #c7d2db; } li.end { color: #9fb0bd; }
  .row { display: flex; flex-wrap: wrap; gap: 20px; align-items: flex-start; }
  figure { margin: 0; } figcaption { color: #9fb0bd; font-size: 13px; margin-top: 6px; } a { color: #9fb0bd; }
  video { display: block; background: #000; border: 1px solid #1c242c; height: 540px; max-width: calc(100vw - 32px); object-fit: contain; }
  @media (max-width: 700px) { video { height: auto; width: calc(100vw - 32px); } }
</style></head><body>
<h1>XenoNet cuts for X</h1>
<p class="lead">Square for the X feed, vertical for Shorts, TikTok and Reels. Each plays muted and loops; unmute in the player to hear XenoNet's own sound.</p>
${rows.join("\n")}
</body></html>
`;
}
