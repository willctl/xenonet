// What the page writes as markup from the map's nodes, live sessions,
// actors and other machines (docs/design/hosts.md: what hosts send is
// untrusted input, and the page escapes it). The page's own renderers run
// here on a stand-in for the browser that records every piece of markup
// written, fed made-up data whose fields carry markup of their own. None of
// it may come out as markup: no element, no attribute, and no declaration
// added to an inline style.
import { test } from "node:test";
import assert from "node:assert/strict";
import { register } from "node:module";

// The page's modules import their stylesheets, which only Vite loads: here
// they're empty.
register("data:text/javascript," + encodeURIComponent(`export async function load(url, context, next) { return url.endsWith(".css") ? { format: "module", source: "", shortCircuit: true } : next(url, context); }`));

// ---------- a stand-in for the browser ----------

const written = [];
const listeners = {};
class El {
  constructor(tag = "div") {
    this.tagName = tag.toUpperCase();
    this._html = "";
    this.textContent = "";
    this.className = "";
    this.value = "";
    this.hidden = false;
    this.dataset = {};
    this.style = { setProperty() {}, removeProperty() {} };
    this.classList = { add() {}, remove() {}, toggle() {}, contains: () => false };
    this.children = [];
    this.found = new Map();
    this.isConnected = true;
  }
  getContext() {
    return {};
  }
  set innerHTML(v) {
    this._html = String(v);
    written.push(this._html);
  }
  get innerHTML() {
    return this._html;
  }
  // A template's content: one element holding what was written into it.
  get content() {
    const el = new El();
    el._html = this._html;
    return { firstElementChild: el };
  }
  get firstElementChild() {
    return this.children[0] ?? null;
  }
  get lastElementChild() {
    return this.children.at(-1) ?? null;
  }
  querySelector(sel) {
    if (!this.found.has(sel)) this.found.set(sel, new El());
    return this.found.get(sel);
  }
  querySelectorAll() {
    return [];
  }
  prepend(c) {
    this.children.unshift(c);
    c.remove = () => (this.children = this.children.filter((x) => x !== c));
  }
  append(c) {
    this.children.push(c);
  }
  appendChild(c) {
    this.children.push(c);
    return c;
  }
  insertBefore(c) {
    if (!this.children.includes(c)) this.children.push(c);
    return c;
  }
  remove() {}
  replaceWith() {}
  replaceChildren() {}
  addEventListener() {}
  setAttribute() {}
  getAttribute() {
    return null;
  }
  closest() {
    return null;
  }
  matches() {
    return false;
  }
  focus() {}
  getBoundingClientRect() {
    return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
  }
}
const page = new Map();
globalThis.document = {
  body: new El("body"),
  documentElement: new El("html"),
  title: "",
  querySelector(sel) {
    if (!page.has(sel)) page.set(sel, new El());
    return page.get(sel);
  },
  querySelectorAll: () => [],
  getElementById(id) {
    return this.querySelector(`#${id}`);
  },
  createElement: (tag) => new El(tag),
  addEventListener() {},
  hasFocus: () => true,
};
globalThis.window = globalThis;
globalThis.Node = { TEXT_NODE: 3 };
globalThis.addEventListener = (type, fn) => (listeners[type] ??= []).push(fn);
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.devicePixelRatio = 1;
globalThis.requestAnimationFrame = () => 0;
globalThis.cancelAnimationFrame = () => {};
// What the server answers, by path; set by each test.
const answers = new Map();
globalThis.fetch = async (url) => {
  const body = answers.get(String(url).split("?")[0]) ?? {};
  return { ok: true, status: 200, statusText: "OK", json: async () => body };
};
// The page's timers mustn't keep the test running.
const { setInterval: every, setTimeout: later } = globalThis;
globalThis.setInterval = (fn, ms) => every(fn, ms).unref();
globalThis.setTimeout = (fn, ms) => later(fn, ms).unref();

// ---------- what comes out ----------

// Markup in a field: closing its attribute, adding one, and an element.
const MARK = `x" onmouseover="pwned()"><img src=x onerror=pwned()>`;
const fire = (type, e) => listeners[type]?.forEach((fn) => fn(e));

// The markup written while `run` runs.
function writes(run) {
  written.length = 0;
  run();
  return written.join("\n");
}
// None of the made-up markup came out as markup: no element or attribute
// of its own, and nothing of it inside an inline style, where escaping
// alone wouldn't stop a declaration.
function clean(html, what) {
  assert.doesNotMatch(html, /<img|onmouseover="/, `${what}: a field's markup came out as markup`);
  for (const [, style] of html.matchAll(/style="([^"]*)"/g)) assert.doesNotMatch(style, /pwned/, `${what}: a field reached an inline style`);
}

const NOW = Date.now();
const MIN = 60e3;

// ---------- the map: the feed, the search, the inspector, the tooltip ----------

const { createHud } = await import("../src/hud.js");
const noop = () => {};
const hud = createHud({ onSelect: noop, onBack: noop, onStep: noop, onFilters: noop, onLayout: noop, onPreview: noop, onStats: noop, onReset: noop, onHandoff: noop, onTerminal: noop, onAck: noop, onOpenApp: noop });

// This machine's core, and a host whose project carries a field graph/v1
// doesn't have, as the review found one could: kept by the loose schemas
// and the merge's spread, and named by an activity event.
const ATLAS = "@vm1:project:~/projects/atlas";
const graph = {
  nodes: [
    { id: "core", kind: "core", name: "~/Projects", path: "/p", home: null },
    { id: "@vm1:core", kind: "host", host: "vm1", name: "vm1", state: "live", since: NOW, lastHeard: NOW, sessionCount: 1, activeCount: 0 },
    { id: ATLAS, kind: "project", host: "vm1", name: "atlas", path: "~/projects/atlas", group: false, sessionCount: 1, activeCount: 0, lastActivity: NOW, actions: [], app: MARK },
  ],
  links: [
    { source: "core", target: "@vm1:core", kind: "host" },
    { source: "@vm1:core", target: ATLAS, kind: "contains" },
  ],
  relations: [],
  stats: {},
  identities: [],
};

test("a host's node can't write markup into the activity feed or the map search", () => {
  hud.setData(graph);
  const feed = writes(() => hud.activity({ sessionId: ATLAS, app: "claude", kind: "turn-start", ts: NOW, host: "vm1" }));
  assert.match(feed, /turn started/, "the feed has its line");
  clean(feed, "the feed");
  // An agent message names its sender the same way.
  clean(writes(() => hud.activity({ sessionId: ATLAS, kind: "agent-message", detail: "to everyone · hi", ts: NOW })), "an agent message");
  document.querySelector("#search").value = "atlas";
  const search = writes(() => hud.refreshFilters());
  assert.match(search, /class="hit"/, "the search found it");
  clean(search, "the search");
});

test("an actor's lines in the activity feed escape its name and XenoNet's words, and come in order", () => {
  const recs = { "actor:wren": { id: "actor:wren", kind: "actor", snapshot: { id: "actor:wren", kind: "agent", name: `Wren${MARK}` } }, "actor:you": { id: "actor:you", kind: "actor", snapshot: { id: "actor:you", kind: "human", name: "Someone" } } };
  const feed = new El("ol");
  page.set("#feed", feed);
  const actorHud = createHud({ onSelect: noop, onBack: noop, onStep: noop, onFilters: noop, onLayout: noop, onPreview: noop, onStats: noop, onReset: noop, onHandoff: noop, onTerminal: noop, onAck: noop, onOpenApp: noop, actorOf: (id) => recs[id] ?? null });
  actorHud.setData(graph);
  const html = writes(() => actorHud.activity({ actor: "actor:wren", kind: "edited", detail: `Edited app.ts in atlas${MARK}`, ts: NOW }));
  assert.match(html, /Wren/, "the actor's line is in the feed");
  clean(html, "an actor's line");
  assert.equal(feed.children[0].dataset.actor, "actor:wren");
  // A message it sent agents carries its own text, escaped like an agent's.
  const said = writes(() => actorHud.activity({ actor: "actor:wren", kind: "agent-message", detail: `to everyone · ${MARK}`, ts: NOW }));
  assert.match(said, /✉/, "the message's line");
  clean(said, "an actor's message");
  // You're never named, whatever the record says.
  assert.match(writes(() => actorHud.activity({ actor: "actor:you", kind: "status", detail: "Looking at atlas", ts: NOW })), />You</);
  assert.equal(writes(() => actorHud.activity({ actor: "actor:gone", kind: "ran", detail: "x", ts: NOW })), "", "an actor the layer doesn't have has no line");
  // Each actor's last word starts the feed once, placed by its time.
  const before = feed.children.length;
  actorHud.seedActors({ actors: [{ id: "actor:wren", recent: [{ kind: "ran", detail: "Ran 3 commands", at: NOW - MIN }] }, { id: "actor:you", recent: [] }, { id: "actor:gone", recent: [{ kind: "ran", detail: "x", at: NOW }] }] });
  actorHud.seedActors({ actors: [{ id: "actor:wren", recent: [{ kind: "ran", detail: "again", at: NOW }] }] });
  assert.equal(feed.children.length, before + 1, "one line, once");
  page.delete("#feed");
});

// This machine's nodes and a live session, with markup in fields whose
// values are numbers and words from a set: whatever arrives is escaped.
const P = "project:/p/atlas", G = "project:/p/labs", W = "resource:agent-wiki", Z = "zone:tmp", S = "claude:0190d3e0-0000-7000-8000-00000000aa01", C = "claude:0190d3e0-0000-7000-8000-00000000aa02";
const hostile = {
  ...graph,
  nodes: [
    ...graph.nodes,
    { id: G, kind: "project", group: true, name: "labs", path: "/p/labs", sessionCount: MARK, activeCount: MARK, lastActivity: NOW, actions: [] },
    { id: P, kind: "project", group: false, name: "atlas", path: "/p/atlas", parent: "/p/labs", sessionCount: MARK, activeCount: MARK, lastActivity: NOW, actions: [], repo: { branch: "main", dirty: MARK, staged: MARK, modified: MARK, untracked: MARK, ahead: MARK, behind: MARK } },
    { id: W, kind: "wiki", name: "Agent Wiki", path: "/w", pageCount: MARK, inbox: [], lastActivity: NOW },
    { id: Z, kind: "zone", name: "tmp", path: "/tmp", sessionCount: MARK, lastActivity: NOW },
    { id: S, kind: "session", app: MARK, activity: MARK, title: "parser", cwd: "/p/atlas", rawId: "aa01", updatedAt: NOW, createdAt: NOW, actions: [{ id: "open-app", label: "Open" }] },
    { id: C, kind: "session", app: MARK, activity: MARK, title: "child", cwd: "/p/atlas", rawId: "aa02", updatedAt: NOW, createdAt: NOW, parentId: S, actions: [] },
  ],
  links: [
    ...graph.links,
    { source: "core", target: G, kind: "contains" },
    { source: G, target: P, kind: "nested" },
    { source: "core", target: W, kind: "wiki" },
    { source: "core", target: Z, kind: "zone" },
    { source: P, target: S, kind: "session" },
    { source: S, target: C, kind: "lineage" },
  ],
};
const liveSession = {
  id: S,
  app: MARK,
  status: MARK,
  tokens: { processed: 10, output: 2 },
  tokensToday: 10,
  tpm: 5,
  context: { used: 50, window: 100 },
  forecast: { pct: 0.5, growthPerTurn: 10, turnsLeft: MARK },
  model: "m",
  turnStartedAt: null,
  lastTool: null,
  toolCounts: { edit: MARK, [MARK]: 2, read: "1;background:url(//pwned.invalid/x)" },
  subagents: [],
  files: [],
  mcp: [],
  needsYou: { kind: MARK, text: "which one?", since: MARK },
  stuck: { reasons: [MARK] },
  lastMessage: null,
  lastTurnEndAt: null,
  lastEventAt: NOW,
};
const live = { at: NOW, sessions: [liveSession], series: { claude: [], codex: [] }, tpm: 5, total24h: 10, today: 10, limits: {}, limitForecast: {}, collab: [], collisions: [], resources: [], status: {} };
const byId = (id) => hostile.nodes.find((n) => n.id === id);

test("the inspector, its live section and the tooltip escape every field of a node and a live session", () => {
  hud.setData(hostile);
  hud.setLive(live);
  for (const id of [G, P, W, Z, S, "@vm1:core", ATLAS]) {
    const html = writes(() => hud.inspect(byId(id)));
    assert.ok(html.length, `${id} was drawn`);
    clean(html, `the inspector for ${id}`);
  }
  assert.match(writes(() => hud.inspect(byId(S))), /need-box/, "the session's live section and what it waits for are drawn");
  fire("pointermove", { clientX: 10, clientY: 10 });
  for (const id of [P, W, Z, S]) clean(writes(() => hud.hover(byId(id))), `the tooltip for ${id}`);
});

test("the active sessions list escapes a live session's fields", async () => {
  const { createTelemetry } = await import("../src/telemetry.js");
  const telemetry = createTelemetry({ onSelect: noop });
  const html = writes(() => telemetry.update(live, hud.index));
  assert.match(html, /data-goto=/, "the session is listed");
  clean(html, "the active sessions");
});

test("the alert strip escapes a session's app and its times", async () => {
  const { createAlerts } = await import("../src/alerts.js");
  const alerts = createAlerts({ sfx: noop, select: noop, ack: noop, openApp: noop, collision: noop });
  const html = writes(() => alerts.update(live, hud.index));
  assert.match(html, /al need/, "the question is on a card");
  assert.match(html, /al stuck/, "and so is the stuck session");
  clean(html, "the alert strip");
});

test("the Actors panel escapes an actor's and a request's times", async () => {
  const { createActorsPanel } = await import("../src/actors-panel.js");
  const panel = new El("section");
  const view = { canDecide: true, waiting: 1, off: false, requests: [{ id: "req-1", name: "Kite", actor: "actor:kite", capabilities: ["run-commands"], heartbeatMs: 60000, expiresAt: MARK, asker: null }] };
  answers.set("/api/actors/observers", { observers: [] });
  const actors = createActorsPanel({
    modals: { open: () => panel, close: noop },
    sfx: noop,
    toast: noop,
    calls: { ready: async () => true, call: async () => ({ ok: true, data: {} }) },
    requests: { refresh: async () => view, view: () => view, armed: () => false, armsIn: () => 1000 },
    decide: noop,
    you: { shown: () => true, setShown: noop },
    list: () => ({ actors: [{ id: "actor:wren", kind: "agent", name: "Wren", presence: "working", lastSeen: MARK, status: "tidying", via: ["registered"], targets: [] }] }),
  });
  written.length = 0;
  await actors.open();
  const html = written.join("\n");
  assert.match(html, /Wren/, "the actor is listed");
  assert.match(html, /Kite/, "and so is the request");
  clean(html, "the Actors panel");
});

// The Actors panel as main.js makes it, for `actors`, the requests in
// `view` and how `tokens` stand, opened: the markup written as it opens.
async function actorsPanelWrites({ actors = [], view = { canDecide: true, waiting: 0, off: false, requests: [] }, tokens = null } = {}) {
  const { createActorsPanel } = await import("../src/actors-panel.js");
  const panel = new El("section");
  answers.set("/api/actors/observers", { observers: [] });
  const made = createActorsPanel({
    modals: { open: () => panel, close: noop },
    sfx: noop,
    toast: noop,
    calls: { ready: async () => true, call: async () => ({ ok: true, data: {} }) },
    requests: { refresh: async () => view, view: () => view, armed: () => false, armsIn: () => 1000 },
    decide: noop,
    you: { shown: () => true, setShown: noop },
    list: () => ({ actors }),
    tokens: tokens && { refresh: async () => ({ canSee: true, off: false, at: NOW, tokens }), of: (id) => tokens.find((t) => t.actor === id) ?? null },
  });
  written.length = 0;
  await made.open();
  return written.slice();
}

test("an actor's product in the Actors panel: a custom one's maker and version are its own words, said once", async () => {
  const acme = { id: "actor:acme", kind: "agent", name: "Acme", presence: "idle", via: ["registered"], product: { id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" }, targets: [] };
  const wren = { id: "actor:wren", kind: "agent", name: "Wren", presence: "working", via: ["observed"], product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }, targets: [] };
  const html = (await actorsPanelWrites({ actors: [acme, wren] })).join("\n");
  assert.match(html, /<span class="ac-what">“acme-sync” · “Acme Tools” · v2\.1\.0, its own words<\/span>/);
  assert.doesNotMatch(html, /its own word 2\.1\.0/, "no version tacked onto its own word");
  assert.match(html, /<span class="ac-what">Hermes Agent · Nous Research · v0\.21\.5<\/span>/);
  clean(html, "the Actors panel's products");
});

test("a request that joins an actor XenoNet knows says Allow replaces that actor's token and capabilities, in the strip and the panel, whose requests wait in the attention color", async () => {
  const hermes = { id: "req-2", name: "Wren", actor: "actor:wren", product: { id: "hermes-agent", vendor: "Nous Research", version: "0.22.0" }, capabilities: ["run-commands"], heartbeatMs: 60000, asker: null, joins: true, at: NOW, expiresAt: NOW + 5 * MIN };
  const kite = { ...hermes, id: "req-3", name: "Kite", actor: "actor:kite", product: null, joins: false };
  const view = { canDecide: true, waiting: 2, off: false, requests: [hermes, kite] };
  const panel = await actorsPanelWrites({ view });
  const row = (id) => panel.find((w) => w.includes(`data-ac-allow="${id}"`)) ?? "";
  assert.match(row("req-2"), /it joins <code>actor:wren<\/code>, which XenoNet knows: Allow replaces its token and capabilities with this program&#39;s/);
  assert.doesNotMatch(row("req-3"), /Allow replaces/, "a new actor replaces nothing");
  assert.match(row("req-3"), /it would be <code>actor:kite<\/code>/);
  for (const id of ["req-2", "req-3"]) assert.match(row(id), /^<li class="ac-row ac-request" style="--c:var\(--attention\)">/, `${id} waits in the attention color`);

  const { createAlerts } = await import("../src/alerts.js");
  const alerts = createAlerts({ sfx: noop, select: noop, ack: noop, openApp: noop, collision: noop, requests: { allow: noop, deny: noop, open: noop, armed: () => false, armsIn: () => 1000 } });
  alerts.update(live, hud.index);
  written.length = 0;
  alerts.setRequests(view);
  const strip = written.slice();
  const card = (id) => strip.find((w) => w.includes(`data-al-request="${id}"`)) ?? "";
  assert.match(card("req-2"), /<div class="al-text">joins actor:wren, which XenoNet knows: Allow replaces its token and capabilities with this program&#39;s<\/div>/, "on a line of its own");
  assert.doesNotMatch(card("req-3"), /Allow replaces|which XenoNet knows/);
  clean(strip.join("\n"), "the request cards");
});

test("each registered actor's row in the Actors panel says how its token stands, escaped", async () => {
  const kite = { id: "actor:kite", kind: "agent", name: "Kite", presence: "idle", via: ["registered"], targets: [] };
  const moth = { id: "actor:moth", kind: "agent", name: "Moth", presence: "idle", via: ["demo", "registered"], targets: [] };
  const wren = { id: "actor:wren", kind: "agent", name: "Wren", presence: "working", via: ["observed"], targets: [] };
  const tokens = [
    { actor: "actor:kite", state: "works", expiresAt: NOW + 30 * 86400e3, usedAt: null },
    { actor: "actor:moth", state: MARK, expiresAt: MARK, usedAt: MARK },
  ];
  const rows = await actorsPanelWrites({ actors: [kite, moth, wren], tokens });
  const row = (id) => rows.find((w) => w.includes(`data-ac-show="${id}"`)) ?? "";
  assert.match(row("actor:kite"), /<div class="ac-token-state works" data-ac-token-state="works">token works · expires in 30 days · never used<\/div>/);
  assert.doesNotMatch(row("actor:wren"), /data-ac-token-state/, "an actor XenoNet only observes has no token");
  assert.match(row("actor:moth"), /data-ac-token-state/, "a state the page doesn't know is still said");
  clean(rows.join("\n"), "the tokens' lines");
});

test("a transcript's header and a digest escape a session's fields", async () => {
  const { createPreview } = await import("../src/preview.js");
  answers.set(`/api/session/${encodeURIComponent(S)}/transcript`, { entries: [], cursor: null });
  const preview = createPreview({ onOpenApp: noop });
  clean(writes(() => preview.open(byId(S))), "the transcript's header");
  const { createModals } = await import("../src/modals.js");
  const modals = createModals({ sfx: noop, toast: noop, select: noop });
  const row = { id: S, app: MARK, title: "parser", turns: MARK, toolCalls: MARK, errors: MARK, files: [], tools: { edit: MARK }, needsYou: null };
  answers.set("/api/digest", { totals: { sessions: MARK, turns: MARK, toolCalls: 3, files: MARK, errors: MARK }, sessions: [row] });
  written.length = 0;
  await modals.away(NOW - 60 * MIN);
  clean(written.join("\n"), "while you were away");
  answers.set("/api/daylog", { date: "Oct 5", hours: [MARK, 2], busiestHour: 1, firstAt: NOW, lastAt: NOW, tokens: 5, totals: { sessions: 1, turns: 1, toolCalls: 1, files: 0, errors: 0 }, topProjects: [{ id: P, name: "atlas", toolCalls: MARK }], uncommitted: [{ name: "atlas", dirty: MARK, ahead: MARK }], waiting: MARK, sessions: [row] });
  written.length = 0;
  await modals.dayLog();
  clean(written.join("\n"), "the day's log");
});

test("a remote session's stats, which its host's relay sends as they came, are escaped in every chart", async () => {
  const { createStatsPanel } = await import("../src/stats.js");
  // The panel's charts are drawn into the elements its body names.
  const body = new El();
  const panel = new El("section");
  panel.querySelector = (sel) => (sel === ".m-body" ? body : El.prototype.querySelector.call(panel, sel));
  panel.querySelectorAll = (sel) =>
    sel === "[data-chart]"
      ? [...body.innerHTML.matchAll(/data-chart="(\w+)"/g)].map(([, id]) => Object.assign(new El(), { dataset: { chart: id }, clientWidth: 600 }))
      : [];
  const id = "@vm1:claude:0190d3e0-0000-7000-8000-00000000aa09";
  answers.set(`/api/session/${encodeURIComponent(id)}/stats`, {
    cwd: "~/projects/atlas",
    branch: "main",
    from: NOW - 10 * MIN,
    to: NOW,
    activeMs: MIN,
    totals: { cost: 1, tokens: 100, cacheRead: 10, calls: 2 },
    kinds: { cacheRead: { tokens: 10, cost: 0.1 }, input: { tokens: 50, cost: 0.5 }, cacheWrite: { tokens: 20, cost: 0.2 }, output: { tokens: 20, cost: 0.2 } },
    models: [
      { model: MARK, calls: 1, tokens: 50, cost: 0.5, from: NOW - 10 * MIN, to: NOW, efforts: [] },
      { model: "m2", calls: 1, tokens: 50, cost: 0.5, from: NOW - 5 * MIN, to: NOW, efforts: [] },
    ],
    turns: [
      { n: MARK, start: NOW - 10 * MIN, end: NOW - 9 * MIN, cost: 0.5, tokens: 50, tools: 1, errors: 0, byModel: { [MARK]: { cost: 0.5 } } },
      { n: 2, start: NOW - 5 * MIN, end: NOW - 4 * MIN, cost: 0.5, tokens: 50, tools: 1, errors: 0, byModel: { m2: { cost: 0.5 } } },
    ],
    context: [{ t: MARK, used: 10, window: 100 }, { t: NOW, used: 20, window: 100 }],
    tools: [{ name: "Bash", calls: MARK, errors: MARK, ms: null }],
    subagents: [],
    compactions: [],
    pricing: { checked: "today", unpriced: [] },
  });
  const stats = createStatsPanel({ modals: { open: () => panel }, sfx: noop, toast: noop });
  written.length = 0;
  await stats.open({ id, app: "claude", title: "parser", activity: "idle", host: "vm1" });
  const html = written.join("\n");
  for (const chart of ["Cost per turn, by model", "Where the tokens went", "Cost by model", "Turn length", "Context over the session", "Tool calls"]) assert.ok(html.includes(`role="img" aria-label="${chart}"`), `${chart} is drawn`);
  clean(html, "the stats panel");
});

// The stats panel opened for remote session `id` whose stats are `body`:
// the markup written, its charts drawn into the elements its body names.
async function statsWrites(id, body) {
  const { createStatsPanel } = await import("../src/stats.js");
  const box = new El();
  const panel = new El("section");
  panel.querySelector = (sel) => (sel === ".m-body" ? box : El.prototype.querySelector.call(panel, sel));
  panel.querySelectorAll = (sel) =>
    sel === "[data-chart]"
      ? [...box.innerHTML.matchAll(/data-chart="(\w+)"/g)].map(([, chart]) => Object.assign(new El(), { dataset: { chart }, clientWidth: 600 }))
      : [];
  answers.set(`/api/session/${encodeURIComponent(id)}/stats`, body);
  const stats = createStatsPanel({ modals: { open: () => panel }, sfx: noop, toast: noop });
  written.length = 0;
  await stats.open({ id, app: "claude", title: "parser", activity: "idle", host: "vm1" });
  return written.join("\n");
}
const CHARTS = ["Cost per turn, by model", "Where the tokens went", "Cost by model", "Turn length", "Context over the session", "Tool calls"];

test("a remote session's stats with text where numbers go: every field comes out escaped or as a number", async () => {
  const T0 = NOW - 10 * MIN;
  const html = await statsWrites("@vm1:claude:0190d3e0-0000-7000-8000-00000000aa0a", {
    cwd: MARK,
    branch: MARK,
    from: MARK,
    to: MARK,
    activeMs: MARK,
    totals: { cost: MARK, tokens: MARK, cacheRead: MARK, calls: MARK },
    kinds: Object.fromEntries(["cacheRead", "input", "cacheWrite", "output"].map((k) => [k, { tokens: MARK, cost: MARK }])),
    models: [
      { model: MARK, calls: MARK, tokens: MARK, cost: MARK, from: MARK, to: MARK, efforts: [MARK] },
      { model: "m2", calls: 1, tokens: 50, cost: 0.5, from: T0, to: NOW, efforts: [] },
    ],
    turns: [
      { n: MARK, start: MARK, end: MARK, cost: MARK, tokens: MARK, tools: MARK, errors: MARK, prompt: MARK, byModel: { [MARK]: { cost: MARK } } },
      { n: 2, start: T0, end: NOW, cost: 0.5, tokens: 50, tools: 1, errors: 0, prompt: "tidy up", byModel: { m2: { cost: 0.5 } } },
    ],
    context: [{ t: MARK, used: MARK, window: MARK }, { t: NOW, used: 20, window: 100 }],
    tools: [{ name: MARK, calls: MARK, errors: MARK, ms: MARK }, { name: "Bash", calls: 3, errors: 1, ms: 1200 }],
    subagents: [{ id: MARK, label: MARK, model: MARK, calls: MARK, tokens: MARK, cost: MARK, from: MARK, to: MARK }],
    compactions: [MARK],
    pricing: { checked: MARK, unpriced: [MARK] },
  });
  for (const chart of CHARTS) assert.ok(html.includes(`role="img" aria-label="${chart}"`), `${chart} is drawn`);
  assert.match(html, /data-card="subagents"/, "and the subagents' table");
  clean(html, "the stats panel");
});

test("a remote session's stats with numbers too small or too large to chart draw every chart on an axis of nothing, rather than never finishing", async () => {
  const T0 = NOW - 10 * MIN;
  // The smallest number JSON can carry, a quarter of which is 0: a tick step
  // of 0 never reached the top of an axis.
  const tiny = 5e-324;
  const html = await statsWrites("@vm1:claude:0190d3e0-0000-7000-8000-00000000aa0b", {
    cwd: "~/projects/atlas",
    branch: "main",
    from: T0,
    to: NOW,
    activeMs: MIN,
    totals: { cost: tiny, tokens: 100, cacheRead: 10, calls: 2 },
    kinds: Object.fromEntries(["cacheRead", "input", "cacheWrite", "output"].map((k) => [k, { tokens: 25, cost: tiny }])),
    models: [
      { model: "m1", calls: 1, tokens: 50, cost: tiny, from: T0, to: NOW, efforts: [] },
      { model: "m2", calls: 1, tokens: 50, cost: Infinity, from: T0, to: NOW, efforts: [] },
    ],
    turns: [
      { n: 1, start: T0, end: T0 + MIN, cost: tiny, tokens: 50, tools: 1, errors: 0, byModel: { m1: { cost: tiny } } },
      { n: 2, start: T0 + MIN, end: NOW, cost: tiny, tokens: 50, tools: 1, errors: 0, byModel: { m2: { cost: tiny } } },
    ],
    context: [{ t: T0, used: tiny, window: tiny }, { t: NOW, used: tiny, window: tiny }],
    tools: [{ name: "Bash", calls: 2, errors: 0, ms: 1000 }],
    subagents: [],
    compactions: [],
    pricing: { checked: "today", unpriced: [] },
  });
  for (const chart of CHARTS) assert.ok(html.includes(`role="img" aria-label="${chart}"`), `${chart} is drawn`);
  // The cost axis is the ticks of nothing: $0.00 and $1.00.
  assert.match(html, /text-anchor="end">\$1\.00<\/text>/);
  assert.ok(html.match(/class="st-gl"/g).length <= 3 * 7, "a few gridlines a chart");
});

test("a turn you stopped reads as stopped in the inspector's live status, the active sessions and the feed", async () => {
  const stopped = { ...liveSession, app: "claude", status: "idle", needsYou: null, stuck: null, toolCounts: {}, forecast: { pct: 0.5, growthPerTurn: null, turnsLeft: null }, interruptedAt: NOW - 2 * MIN, lastEventAt: NOW - 2 * MIN };
  const snap = { ...live, sessions: [stopped] };
  hud.setData(hostile);
  hud.setLive(snap);
  assert.match(writes(() => hud.inspect(byId(S))), /<span class="live-status idle">idle · stopped 2m ago<\/span>/);
  const { createTelemetry } = await import("../src/telemetry.js");
  const telemetry = createTelemetry({ onSelect: noop });
  assert.match(writes(() => telemetry.update(snap, hud.index)), /<span class="a-state idle">idle · stopped 2m ago<\/span>/);
  assert.match(writes(() => hud.activity({ sessionId: S, kind: "turn-end", duration: 12000, interrupted: true, ts: NOW })), /<span class="ev-detail">turn stopped after 12s<\/span>/);
  // A time that isn't one, from a hostile host, says nothing.
  const odd = { ...snap, sessions: [{ ...stopped, interruptedAt: MARK }] };
  hud.setLive(odd);
  const html = writes(() => hud.inspect(byId(S)));
  assert.match(html, /<span class="live-status idle">idle<\/span>/);
  clean(html + writes(() => telemetry.update(odd, hud.index)), "a stopped turn's time");
});
