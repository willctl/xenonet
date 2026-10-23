import { escapeHtml as h, timeAgo, stamp, shortPath, setHome } from "./format.js";
import { APP_LABEL, APP_DESKTOP, THEME, onTheme, colorVar, categoryVar, nodeColorVar } from "./theme.js";
import { colorOf } from "./actors-model.js";
import { compact } from "./telemetry.js";
import { fileManager } from "./platform.js";
import { whereHtml, hostRows, tipText, hostTag, sameHost, alertText } from "./hosts.js";
import { stoppedWords, turnEndWords } from "./live-words.js";
import { cleanTitle, renamable, MAX_TITLE } from "../core/rename.js";

const $ = (sel) => document.querySelector(sel);

const KIND_LABEL = { core: "Root", host: "Host", project: "Project", native: "Codex project", session: "Session", resource: "Shared system", wiki: "Xenolith", page: "Wiki page" };
// The projects folder's name (see graph.js), which the page shows in a few
// places; it changes with the folder.
let rootName = "~/Projects";
const kindLabel = (n) => (n.kind === "project" && n.group ? "Group" : n.kind === "zone" ? `Outside ${rootName}` : KIND_LABEL[n.kind]);
const ACTIVITY_LABEL = { active: "active", recent: "recent", idle: "idle" };
// Why the scan made a folder a project or a group (see scanProjects).
const VCS_NAME = { ".git": "git repo", ".hg": "Mercurial repo", ".jj": "Jujutsu repo", ".sl": "Sapling repo", ".svn": "Subversion checkout" };
// Where a Grok Build session came from: a subagent's, or the kind of
// session Grok says it is (a fork, a headless run), else Grok Build.
function grokOrigin(n) {
  if (n.parentKind === "subagent") return "Grok Build subagent";
  return n.source ? `Grok Build · ${n.source}` : "Grok Build";
}

function foundAs(n) {
  if (n.rule?.by === "codex") return `Codex's pick: ${n.rule.reason || n.rule.as}`;
  if (n.rule?.by === "you") return { project: "your rule: one project", group: "your rule: a group" }[n.rule.as] ?? "your rule";
  if (n.git?.worktreeOf) return "a worktree of another repo";
  return {
    repo: VCS_NAME[n.marker] ?? "its own repo",
    workspace: `a workspace (${n.marker})`,
    submodule: "a submodule",
    sessions: "sessions ran here",
    folder: "a folder of its own",
    contains: "a folder of projects",
  }[n.detected] ?? null;
}
// Codex reports these for every thread our private app-server hasn't loaded.
const QUIET_STATUS = new Set(["notLoaded", "idle"]);
const LIVE_LABEL = { tool: "running a tool", thinking: "thinking", idle: "idle" };
const GLYPH = {
  shell: "›_", edit: "✎", read: "◎", web: "⌁", mcp: "⬡", agent: "◈", code: "{}", other: "•",
  prompt: "▲", message: "◆", "agent-message": "✉", "turn-start": "▶", "turn-end": "■", "subagent-start": "◈", "subagent-stop": "◇", "tool-error": "✕", hook: "⚑", updated: "↻",
  "needs-you": "◉", "ship-it": "⇪", stuck: "⟳", refusal: "⊘",
};
// What actors do (actor/event/v1), by kind; kinds are an open set.
const ACTOR_GLYPH = { presence: "○", status: "◇", spawned: "◈", edited: "✎", ran: "›_", messaged: "✉", said: "◆", finished: "■", failed: "✕", "agent-message": "✉" };
const FEED_MAX = 60;
const SEED_MS = 6 * 3600e3;

// `actorOf(id)` is an actor as the actors layer has it now, and
// `actorBeams(id)` the nodes it beams into (see actors.js).
// `onRename(id, title)` saves a session's new name and answers { ok, title,
// error }; without it, sessions can't be renamed here.
export function createHud({ onSelect, onBack, onStep, onFilters, onLayout, onPreview, onStats, onReset, onHandoff, onTerminal, onAck, onOpenApp, onOpened = null, onRename = null, onMessageActor = null, onMessageSession = null, identities = null, actorOf = () => null, actorBeams = () => [], sfx = () => {}, mcp = null }) {
  const filters = { claude: true, codex: true, grok: true, archived: true, external: true };
  let data = null;
  let index = new Map();
  let children = new Map();
  let parent = new Map();
  let current = null;
  // The session being renamed in the inspector: { id, from, saving }.
  let renaming = null;
  let live = null;
  let liveById = new Map();
  let seeded = false;

  // ---------- data ----------

  function setData(graph) {
    data = graph;
    index = new Map(graph.nodes.map((n) => [n.id, n]));
    children = new Map();
    parent = new Map();
    for (const l of graph.links) {
      const s = typeof l.source === "object" ? l.source.id : l.source;
      const t = typeof l.target === "object" ? l.target.id : l.target;
      if (l.kind === "lineage-cross") continue;
      if (!children.has(s)) children.set(s, []);
      children.get(s).push(t);
      parent.set(t, s);
    }
    setHome(index.get("core")?.home);
    nameRoot(index.get("core")?.name ?? "~/Projects");
    renderCounts();
    identities?.renderFilters(null, { nodes: graph.nodes, parent });
    renderSources();
    if (current && !renaming) renderInspector(fresh(current), { quiet: true });
  }

  // What the inspector shows, as it is now: a node from the map, or an actor.
  const fresh = (x) => (x.kind === "actor" ? actorOf(x.id) : (index.get(x.id) ?? null));

  // The folder's name in the brand, and in the labels marked for it in
  // index.html: the legend, and the filter for what's outside it.
  function nameRoot(name) {
    $("#root-name").textContent = name;
    if (name === rootName) return;
    rootName = name;
    for (const el of document.querySelectorAll("[data-root-label]")) {
      const text = [...el.childNodes].findLast((c) => c.nodeType === Node.TEXT_NODE && c.nodeValue.trim());
      if (text) text.nodeValue = el.dataset.rootLabel === "outside" ? `Outside ${name}` : name;
    }
  }

  function setLive(snap) {
    live = snap;
    liveById = new Map(snap.sessions.map((s) => [s.id, s]));
    if (!seeded && data) seedFeed();
    if (current?.kind === "session") renderLiveSection();
  }

  // How many sessions on the map each filter toggle covers. The other
  // headline numbers live where they're explained: working in Active
  // sessions, tokens in Token use, totals under the scope buttons.
  function renderCounts() {
    const sessions = data.nodes.filter((n) => n.kind === "session");
    const counts = {
      claude: sessions.filter((n) => n.app === "claude").length,
      codex: sessions.filter((n) => n.app === "codex").length,
      grok: sessions.filter((n) => n.app === "grok").length,
      archived: sessions.filter((n) => n.archived).length,
    };
    for (const el of document.querySelectorAll("[data-count]")) el.textContent = counts[el.dataset.count];
    // Grok Build's toggle shows once there's a Grok Build session, or while
    // it hides them.
    const grok = document.querySelector('[data-filter="grok"]');
    if (grok) grok.hidden = !counts.grok && filters.grok;
  }

  function renderSources() {
    const sources = data.sources ?? [];
    const sub = $("#link-sub");
    if (sub) sub.textContent = sources.length ? `${sources.filter((src) => src.ok).length}/${sources.length} sources reading` : "";
    $("#sources").innerHTML = sources
      .map((src) => `<span class="src ${src.ok ? "ok" : "bad"}" title="${h(src.ok ? `${src.name}: ${src.note ?? src.count ?? ""}${src.ms != null ? ` in ${src.ms}ms` : ""}` : `${src.name}: ${src.error}`)}">${h(src.name)}</span>`)
      .join("");
  }

  // ---------- signal feed ----------

  // A sign-in that changed (see state.js). Its names are written from the
  // identities' ids, and again whenever identities are hidden or shown.
  const SIGNED_IN = { claude: "Claude Code", chatgpt: "Codex", gh: "gh" };
  function identityText(li) {
    const { app, now, before } = li.dataset;
    const who = (id) => identities?.label(id) ?? "another account";
    li.querySelector(".ev-title").textContent = now ? (app === "gh" ? `gh pushes as ${who(now)}` : `${SIGNED_IN[app]} signed in as ${who(now)}`) : `${SIGNED_IN[app]} signed out`;
    li.querySelector(".ev-detail").textContent = before ? `was ${who(before)}` : "";
  }
  function identityItem(ev) {
    const li = document.createElement("li");
    li.className = "ev identity";
    li.dataset.ts = ev.ts;
    Object.assign(li.dataset, { app: ev.app, now: ev.identity ?? "", before: ev.before ?? "" });
    li.style.setProperty("--c", "var(--ring-0)");
    li.innerHTML = `<span class="ev-glyph">◈</span><span class="ev-body"><span class="ev-title"></span><span class="ev-detail"></span></span><span class="feed-time">${timeAgo(ev.ts)}</span>`;
    identityText(li);
    return li;
  }

  // Something an actor did ({ actor, kind, detail, ts }): its name in its
  // own color, marked as an actor where a session has its app's swatch,
  // and XenoNet's words for it, never a session's title; or a message it sent
  // agents (agent-message), with its text as an agent's has. Failures in
  // the theme's danger color.
  const actorColor = (rec, kind) => (kind === "failed" ? colorVar("danger") : colorOf(rec.snapshot, THEME));
  function actorItem(ev) {
    const rec = actorOf(ev.actor);
    if (!rec) return null;
    const s = rec.snapshot;
    const li = document.createElement("li");
    li.className = "ev actor-ev";
    li.dataset.actor = rec.id;
    li.dataset.kind = ev.kind;
    li.dataset.ts = ev.ts;
    li.style.setProperty("--c", actorColor(rec, ev.kind));
    li.innerHTML = `<span class="ev-glyph">${ACTOR_GLYPH[ev.kind] ?? "◇"}</span>
      <span class="ev-body"><span class="ev-title"><span class="ev-actor" title="An actor">◇</span>${h(s.kind === "human" ? "You" : s.name)}</span><span class="ev-detail">${h(ev.detail ?? "")}</span></span>
      <span class="feed-time">${timeAgo(ev.ts)}</span>`;
    return li;
  }
  // Actors' lines take their colors in a new theme, as the rest of the
  // feed does through var().
  onTheme(() => {
    for (const li of document.querySelectorAll("#feed li[data-actor]")) {
      const rec = actorOf(li.dataset.actor);
      if (rec) li.style.setProperty("--c", actorColor(rec, li.dataset.kind));
    }
  });

  function feedItem(ev) {
    if (ev.actor) return actorItem(ev);
    if (ev.kind === "identity") return identityItem(ev);
    const n = index.get(ev.sessionId);
    if (!n) return null;
    const key = ev.kind === "tool" ? ev.category : ev.kind;
    // Colors that follow the theme (var(--…)): the feed stays up while it changes.
    const color = ev.kind === "tool" ? categoryVar(ev.category) : { "tool-error": colorVar("danger"), "needs-you": colorVar("attention"), "ship-it": colorVar("pressure"), stuck: colorVar("stuck"), refusal: colorVar("danger") }[ev.kind] ?? nodeColorVar(n);
    let detail = ev.detail ?? "";
    if (ev.kind === "tool") detail = `${ev.tool.replace(/^mcp__/, "").replace(/__/g, ".")}${ev.detail ? ` · ${ev.detail}` : ""}`;
    if (ev.kind === "subagent-start") detail = `started subagent · ${ev.label ?? ""}`;
    if (ev.kind === "subagent-stop") detail = `subagent finished · ${ev.label ?? ""}`;
    if (ev.kind === "turn-end") detail = turnEndWords(ev);
    if (ev.kind === "turn-start") detail = "turn started";
    if (ev.kind === "tool-error") detail = "a tool call failed";
    if (ev.kind === "updated") detail = "session updated";
    if (ev.kind === "needs-you") detail = `waiting for you · ${ev.detail ?? ""}`;
    if (ev.kind === "ship-it") detail = `turn ended with ${ev.detail ?? "uncommitted changes"}`;
    if (ev.kind === "stuck") detail = `may be stuck · ${ev.detail ?? ""}`;
    if (ev.kind === "refusal") detail = `refused · ${ev.detail ?? ""}`;
    const li = document.createElement("li");
    li.className = `ev ${ev.kind}`;
    li.dataset.id = n.id;
    li.dataset.ts = ev.ts;
    li.style.setProperty("--c", color);
    li.innerHTML = `<span class="ev-glyph">${GLYPH[key] ?? "•"}</span>
      <span class="ev-body"><span class="ev-title"><i class="sw ${h(n.app)}"></i>${hostTag(n)}${h(n.title)}</span><span class="ev-detail">${h(detail)}</span></span>
      <span class="feed-time">${timeAgo(ev.ts)}</span>`;
    return li;
  }

  // Keep a row under the pointer still while someone reads or clicks it.
  // New activity is bounded and joins the feed when the pointer leaves.
  const activityFeed = $("#feed");
  let feedPaused = false;
  const pendingFeed = [];
  function insertActivity(li, ordered = false) {
    if (feedPaused) {
      pendingFeed.push({ li, ordered });
      if (pendingFeed.length > FEED_MAX) pendingFeed.shift();
      return;
    }
    const after = ordered ? [...activityFeed.children].find((c) => Number(c.dataset.ts) < Number(li.dataset.ts)) : activityFeed.firstChild;
    activityFeed.insertBefore(li, after ?? null);
    while (activityFeed.children.length > FEED_MAX) activityFeed.lastElementChild.remove();
  }
  activityFeed.addEventListener("pointerenter", () => { feedPaused = true; });
  activityFeed.addEventListener("pointerleave", () => {
    feedPaused = false;
    for (const { li, ordered } of pendingFeed.splice(0)) insertActivity(li, ordered);
  });

  function pushActivity(ev, { animate = true } = {}) {
    if (ev.kind === "message" && ev.detail?.length < 3) return;
    const li = feedItem(ev);
    if (!li) return;
    if (animate) li.classList.add("fresh");
    insertActivity(li);
  }

  // A line put where its time puts it, for what happened before the page
  // was told; live lines go on top as they come.
  function placeActivity(ev) {
    const li = feedItem(ev);
    if (!li) return;
    insertActivity(li, true);
  }

  // The first time the actors are known, each one's last word, as each
  // session's last tool call starts the feed.
  let actorsSeeded = false;
  function seedActors(list) {
    if (actorsSeeded || !list?.actors) return;
    actorsSeeded = true;
    for (const a of list.actors) {
      const e = Array.isArray(a.recent) ? a.recent.at(-1) : null;
      if (e && Number.isFinite(e.at) && Date.now() - e.at < SEED_MS) placeActivity({ actor: a.id, kind: e.kind, detail: e.detail, ts: e.at });
    }
  }

  // Before live events arrive, show each session's most recent tool call.
  function seedFeed() {
    seeded = true;
    const items = live.sessions
      .filter((s) => s.lastTool && Date.now() - s.lastTool.ts < 6 * 3600e3)
      .map((s) => ({ sessionId: s.id, kind: "tool", tool: s.lastTool.name, category: s.lastTool.category, detail: s.lastTool.detail, ts: s.lastTool.ts }))
      .sort((a, b) => a.ts - b.ts);
    for (const ev of items) pushActivity(ev, { animate: false });
  }

  setInterval(() => {
    for (const li of document.querySelectorAll("#feed li[data-ts]")) li.querySelector(".feed-time").textContent = timeAgo(Number(li.dataset.ts));
  }, 10000);

  // ---------- visibility ----------

  function computeVisible() {
    const vis = new Set();
    // An identity filter (see identities.js) lets through the sessions and
    // projects linked to the identities picked; null while it's off.
    const pass = identities?.filterPass(data.nodes, parent) ?? null;
    const sessionOk = (n) => filters[n.app] && (filters.archived || !n.archived) && (!pass || pass.has(n.id));
    for (const n of data.nodes) if (n.kind === "session" && sessionOk(n)) vis.add(n.id);
    // A session hidden by a filter hides its spawned children too.
    for (const n of data.nodes) {
      let p = parent.get(n.id);
      while (p && index.get(p)?.kind === "session") {
        if (!vis.has(p)) vis.delete(n.id);
        p = parent.get(p);
      }
    }
    // Under an identity filter, a project or Codex project shows while it's
    // linked itself or something in it shows, and so do the groups it's in.
    const shows = (id) => (children.get(id) ?? []).some((c) => vis.has(c) || (index.get(c)?.kind !== "session" && shows(c)));
    for (const n of data.nodes) {
      if (n.kind === "core" || n.kind === "host" || n.kind === "resource" || n.kind === "wiki" || n.kind === "page") vis.add(n.id);
      if (n.kind === "project" && (!pass || pass.has(n.id) || shows(n.id))) vis.add(n.id);
      if (n.kind === "native" && filters.codex && (!pass || shows(n.id))) vis.add(n.id);
      if (n.kind === "zone" && filters.external && (children.get(n.id) ?? []).some((c) => vis.has(c))) vis.add(n.id);
    }
    if (pass) for (const id of [...vis]) for (let p = parent.get(id); p && index.get(p)?.kind === "project" && !vis.has(p); p = parent.get(p)) vis.add(p);
    return vis;
  }

  // ---------- inspector ----------

  const row = (k, v, cls = "") => (v == null || v === "" ? "" : `<div class="kv ${cls}"><span class="k">${h(k)}</span><span class="v">${v}</span></div>`);
  const mono = (v) => (v == null ? null : `<code>${h(v)}</code>`);
  // The group (or project) a project sits in.
  const groupLink = (n) => {
    const up = n.parent && index.get(sameHost(n, `project:${n.parent}`));
    return up ? `<a href="#" data-goto="${h(up.id)}">${h(up.name)}</a>` : null;
  };

  function sessionsUnder(id) {
    const out = [];
    const walk = (x) => {
      for (const c of children.get(x) ?? []) {
        const n = index.get(c);
        if (!n) continue;
        if (n.kind === "session") out.push(n);
        walk(c);
      }
    };
    walk(id);
    return out.sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
  }

  // Another machine's session in a network, as its presence says
  // (network-map.js): its state and what it's doing, what waits on you or
  // why it seems stuck, and what it said last, each escaped. Its transcript
  // and stats stay on its machine.
  function presenceHtml(node) {
    const p = node.presence ?? {};
    const as = node.stale ? ` · as of ${timeAgo(index.get(`@${node.host}:core`)?.lastHeard)}` : "";
    const need = p.waits
      ? `<div class="need-box ${h(p.kind ?? "question")}"><div class="need-k">${p.kind === "permission" ? "◉ needs permission" : "◉ waiting for you"}${p.since ? ` · ${timeAgo(p.since)}` : ""}</div><div class="need-text">${h(p.question ?? `The question shows in ${node.host}'s console.`)}</div></div>`
      : "";
    const stuck = p.stuck?.length ? `<div class="stuck-box">⟳ may be stuck · ${p.stuck.map(h).join(" · ")}</div>` : "";
    const state = p.state === "working" ? "working" : p.state === "waiting" ? "waiting for you" : p.state === "ended" ? "ended" : "idle";
    // Its state, unless the box above says it waits.
    const head = p.waits && !as ? "" : `<div class="live-head"><span class="live-status ${p.state === "working" ? "thinking" : "idle"}">${h(state)}${h(as)}</span></div>`;
    return `${need}${stuck}
      ${head}
      ${p.focus ? `<div class="sub-k">doing</div><div class="net-said">${h(p.focus)}</div>` : ""}
      ${p.lastMessage ? `<div class="sub-k">said last</div><div class="net-said">${h(p.lastMessage)}</div>` : ""}
      <div class="net-from">From ${h(node.host)}'s presence. Its transcript and stats stay on ${h(node.host)}.</div>`;
  }

  function liveSectionHtml(node) {
    // Drawn from presence (network-map.js); one drawn from its host's relay
    // snapshot (network: "relay") has the live state, as an SSH host's does.
    if (node.network === true) return presenceHtml(node);
    const s = liveById.get(node.id);
    if (!s) return `<div class="live-none">No activity in the last 24 hours</div>`;
    const pct = s.context.window ? Math.min(100, (s.context.used / s.context.window) * 100) : 0;
    // Counts go into an inline style, where escaping wouldn't stop a
    // declaration, so only numbers do.
    const count = (n) => (Number.isFinite(n) ? n : 0);
    const calls = Object.values(s.toolCounts).reduce((a, b) => a + count(b), 0);
    const total = calls || 1;
    const mix = Object.entries(s.toolCounts)
      .sort((a, b) => b[1] - a[1])
      .map(([cat, n]) => `<b style="flex:${count(n)};--c:${categoryVar(cat)}" title="${h(cat)} · ${count(n)}"></b>`)
      .join("");
    const mixLegend = Object.entries(s.toolCounts)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([cat, n]) => `<span><i style="--c:${categoryVar(cat)}"></i>${h(cat)} ${Math.round((count(n) / total) * 100)}%</span>`)
      .join("");
    const subs = s.subagents
      .slice(0, 6)
      .map((a) => `<li class="${a.active ? "on" : ""}"><span class="sa-dot"></span>${h(a.label)}<span class="feed-time">${a.active ? "running" : timeAgo(a.lastAt)}</span></li>`)
      .join("");
    const files = s.files.slice(0, 6).map((f) => `<li title="${h(f.path)}"><code>${h(shortPath(f.path, 44))}</code><span class="feed-time">${timeAgo(f.ts)}</span></li>`).join("");
    const mcp = s.mcp.map((m) => `<span class="tag res">${h(m.server)}</span>`).join(" ");
    const collab = (live.collab ?? []).filter((e) => e.a === node.id || e.b === node.id);
    const collabHtml = collab
      .map((e) => {
        const other = index.get(e.a === node.id ? e.b : e.a);
        return other ? `<li><a href="#" data-goto="${h(other.id)}">${h(other.title)}</a><span class="feed-time">${h(e.files.slice(0, 2).join(", "))}</span></li>` : "";
      })
      .join("");
    const need = s.needsYou;
    // A turn you stopped says so, while the session's idle after it.
    const stopped = stoppedWords(s);
    const f = s.forecast;
    const forecast =
      f?.turnsLeft != null
        ? `<div class="forecast ${f.turnsLeft <= 3 ? "crit" : f.turnsLeft <= 8 ? "warn" : ""}">${f.turnsLeft <= 0 ? "compaction due next turn" : `≈ ${h(f.turnsLeft)} turn${f.turnsLeft === 1 ? "" : "s"} until compaction`} <span class="dim">· +${compact(f.growthPerTurn)}/turn</span></div>`
        : `<div class="forecast dim">context isn't growing</div>`;
    return `
      ${need ? `<div class="need-box ${h(need.kind)}"><div class="need-k">${need.kind === "review" ? "▣ ready for review" : need.kind === "permission" ? "◉ needs permission" : "◉ waiting for you"} · ${timeAgo(need.since)}</div><div class="need-text">${h(alertText({ host: node.host, text: need.text }))}</div>${node.host ? "" : `<div class="need-actions">${APP_DESKTOP[node.app] ? `<button class="act primary ${h(node.app)}" data-need-open>Answer in ${h(APP_DESKTOP[node.app])}</button>` : ""}<button class="act" data-need-ack>Dismiss</button></div>`}</div>` : ""}
      ${s.stuck ? `<div class="stuck-box">⟳ may be stuck · ${s.stuck.reasons.map(h).join(" · ")}</div>` : ""}
      <div class="live-head">
        <span class="live-status ${h(s.status)}">${LIVE_LABEL[s.status]}${s.status === "tool" && s.lastTool ? ` · ${h(s.lastTool.name.replace(/^mcp__/, "").replace(/__/g, "."))}` : ""}${stopped ? ` · ${h(stopped)}` : ""}</span>
        <span class="live-tpm">${compact(s.tpm)}<small>/min</small></span>
      </div>
      <div class="meter">
        <div class="meter-k"><span>context</span><span>${compact(s.context.used)} / ${compact(s.context.window)} · ${pct.toFixed(0)}%</span></div>
        <div class="ctx-bar big"><b style="width:${pct.toFixed(1)}%" class="${pct > 90 ? "crit" : pct > 70 ? "warn" : ""}"></b></div>
        ${forecast}
        ${pct >= 85 && !node.host ? `<button class="act fresh" data-fresh>⇄ Start a fresh session from a handoff brief</button>` : ""}
      </div>
      <div class="live-nums">
        <div><span>${compact(s.tokens.processed)}</span><small>tokens processed</small></div>
        <div><span>${compact(s.tokens.output)}</span><small>output</small></div>
        <div><span>${calls}</span><small>tool calls</small></div>
      </div>
      ${mix ? `<div class="mix">${mix}</div><div class="mix-legend">${mixLegend}</div>` : ""}
      ${subs ? `<div class="sub-k">subagents</div><ul class="subagents">${subs}</ul>` : ""}
      ${collabHtml ? `<div class="sub-k collab-k">editing the same files</div><ul class="files">${collabHtml}</ul>` : ""}
      ${files ? `<div class="sub-k">files edited · last 20 min</div><ul class="files">${files}</ul>` : ""}
      ${mcp ? `<div class="sub-k">shared systems</div><div class="res-list">${mcp}</div>` : ""}`;
  }

  function renderLiveSection() {
    const el = document.getElementById("insp-live");
    if (el && current) el.innerHTML = liveSectionHtml(current);
  }

  // How an actor shows in the inspector and the tooltip (actor-inspector.js),
  // loaded the first time it's needed or warmed once there are actors, so
  // the page starts without it.
  let actorViews = null;
  let loadingViews = null;
  const loadActorViews = () => (loadingViews ??= import("./actor-inspector.js").then((m) => (actorViews = m)));

  // An actor, in its own color. Drawn again only when something it shows
  // has changed, so reading it isn't interrupted.
  let actorShown = "";
  function renderActor(rec, el, quiet) {
    if (!actorViews) return void loadActorViews().then(() => current?.id === rec.id && renderInspector(actorOf(rec.id), { quiet }));
    const { actorInspectorHtml } = actorViews;
    const s = rec.snapshot;
    const beams = actorBeams(rec.id);
    const key = [rec.id, s.presence, s.presenceSince, s.status, s.needsYou?.text, (s.targets ?? []).map((t) => `${t.id}@${t.lastAt}`).join(), s.recent?.at(-1)?.id, beams.join(), identities?.masked].join("|");
    if (quiet && key === actorShown && !el.hidden) return;
    actorShown = key;
    // Never a name for you: the person at the console stays "You".
    const identity = s.kind !== "human" && s.identity && identities ? identities.label(s.identity) : null;
    el.style.setProperty("--accent", `#${rec.color.getHexString()}`);
    el.innerHTML = actorInspectorHtml(rec, { index, beams, identity });
    el.hidden = false;
    if (!quiet) {
      el.classList.remove("enter");
      void el.offsetWidth;
      el.classList.add("enter");
    }
  }

  function renderInspector(node, { quiet = false } = {}) {
    // Another node in the inspector ends a rename that hasn't been sent.
    if (renaming && !renaming.saving && renaming.id !== node?.id) renaming = null;
    current = node;
    const el = $("#inspector");
    if (!node) {
      el.hidden = true;
      return;
    }
    if (node.kind === "actor") return renderActor(node, el, quiet);
    actorShown = "";
    const color = nodeColorVar(node);
    const isSession = node.kind === "session";
    const chip = isSession ? `${KIND_LABEL.session} · ${APP_LABEL[node.app] ?? node.app}` : kindLabel(node);
    const title = isSession ? node.title : node.name;
    const sub = isSession ? shortPath(node.cwd, 56) : node.kind === "resource" ? "MCP shared system" : node.kind === "host" && node.network ? "a machine in your network" : shortPath(node.path, 56);

    let grid = "";
    if (isSession) {
      const p = node.parentId && index.get(node.parentId);
      grid =
        row("status", `<span class="activity ${h(node.activity)}">${ACTIVITY_LABEL[node.activity]}</span>${node.archived ? ` <span class="tag">archived</span>` : ""}${node.starred ? ` <span class="tag star">starred</span>` : ""}${node.status && !QUIET_STATUS.has(node.status) ? ` <span class="tag">${h(node.status)}</span>` : ""}`) +
        row("updated", `${timeAgo(node.updatedAt)} <span class="dim">· ${stamp(node.updatedAt)}</span>`) +
        row("created", stamp(node.createdAt)) +
        row("model", mono(node.model && `${node.model}${node.effort ? ` · ${node.effort}` : ""}`)) +
        row("branch", mono(node.branch)) +
        row("folder", mono(shortPath(node.cwd, 40))) +
        row("worktree", mono(node.worktree && shortPath(node.worktree, 40))) +
        row("origin", mono(node.originator ?? (node.observed || node.reader || node.network ? null : node.inDesktop ? "Claude desktop" : node.app === "claude" ? "CLI / SDK" : node.app === "grok" ? grokOrigin(node) : null))) +
        row("seen", node.reader === "experimental" ? `read from ${h(APP_LABEL[node.app] ?? node.app)}'s own records (experimental)` : node.observed === "numbat" ? "observed through numbat's records" : node.observed === "gateway" ? `reported through the gateway${node.reportedBy ? ` by ${h(node.reportedBy)}` : ""}` : null) +
        row("parent", p ? `<a href="#" data-goto="${h(p.id)}">${h(p.title)}</a> <span class="dim">${h(node.parentKind ?? "")}</span>` : null) +
        row("id", `<code class="copy" data-copy="${h(node.rawId)}" title="copy">${h(node.rawId)}</code>`);
    } else if (node.kind === "host") {
      grid = hostRows(node, { row, mono });
    } else if (node.kind === "project" && node.group) {
      const inside = (children.get(node.id) ?? []).map((id) => index.get(id)).filter((n) => n?.kind === "project");
      grid =
        row("sessions", `${h(node.sessionCount)}${node.activeCount ? ` <span class="activity active">${h(node.activeCount)} live</span>` : ""}`) +
        row("last activity", node.lastActivity ? `${timeAgo(node.lastActivity)} <span class="dim">· ${stamp(node.lastActivity)}</span>` : "—") +
        row("holds", inside.length ? inside.map((p) => `<a href="#" data-goto="${h(p.id)}">${h(p.name)}</a>`).join(" · ") : `<span class="dim">no projects drawn now</span>`, "wrap") +
        row("in", groupLink(node)) +
        row("found as", h(foundAs(node))) +
        row("stack", node.stack?.length ? node.stack.map((s) => `<span class="tag">${h(s)}</span>`).join(" ") : null) +
        row("about", node.description ? h(node.description) : null, "wrap");
    } else if (node.kind === "project") {
      grid =
        row("sessions", `${h(node.sessionCount)}${node.activeCount ? ` <span class="activity active">${h(node.activeCount)} live</span>` : ""}`) +
        row("last activity", node.lastActivity ? `${timeAgo(node.lastActivity)} <span class="dim">· ${stamp(node.lastActivity)}</span>` : "—") +
        row("git", mono(node.repo?.branch ?? (node.git ? node.git.branch ?? "detached" : "not a repo"))) +
        row(
          "working tree",
          node.repo
            ? node.repo.dirty
              ? `<span class="pressure">Δ${h(node.repo.dirty)} uncommitted</span> <span class="dim">${h(node.repo.staged)} staged · ${h(node.repo.modified)} modified · ${h(node.repo.untracked)} new</span>`
              : `<span class="dim">clean</span>`
            : null,
        ) +
        row("sync", node.repo && (node.repo.ahead || node.repo.behind) ? `${node.repo.ahead ? `↑${h(node.repo.ahead)} to push` : ""} ${node.repo.behind ? `↓${h(node.repo.behind)} to pull` : ""}` : null) +
        row("remote", mono(node.git?.origin)) +
        row("stack", node.stack?.length ? node.stack.map((s) => `<span class="tag">${h(s)}</span>`).join(" ") : null) +
        row("grouped with", node.satelliteOf ? `<a href="#" data-goto="${h(sameHost(node, `project:${node.satelliteOf}`))}">${h(index.get(sameHost(node, `project:${node.satelliteOf}`))?.name ?? "")}</a>` : null) +
        row("in", groupLink(node)) +
        row("found as", h(foundAs(node))) +
        row("about", node.description ? h(node.description) : null, "wrap");
    } else if (node.kind === "wiki") {
      const pages = (children.get(node.id) ?? []).map((id) => index.get(id)).filter(Boolean);
      grid =
        row("pages", h(node.pageCount ?? pages.length)) +
        row("last write", node.lastActivity ? `${timeAgo(node.lastActivity)} <span class="dim">· ${stamp(node.lastActivity)}</span>` : "—") +
        row("inbox", node.inbox?.length ? `<span class="pressure">${node.inbox.length} unfiled</span><br>${node.inbox.slice(0, 6).map((n) => `<span class="dim">· ${h(n.title)}</span>`).join("<br>")}` : `<span class="dim">empty</span>`) +
        row("folder", mono(shortPath(node.path, 40)));
    } else if (node.kind === "page") {
      grid =
        row("type", `<span class="tag">${h(node.type)}</span>`) +
        row("space", node.space ? h(node.space) : null) +
        row("about", node.summary ? h(node.summary) : null, "wrap") +
        row("updated", node.updated ? `${timeAgo(node.updated)} <span class="dim">by ${h(node.updatedBy ?? "?")}</span>` : "—") +
        row("recent writes", node.writes?.length ? node.writes.slice().reverse().map((w) => `<span class="dim">${new Date(w.ts).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} · ${h(w.app)}</span> ${h(w.title)}`).join("<br>") : null, "wrap") +
        row("tags", node.tags?.length ? node.tags.map((t) => `<span class="tag">${h(t)}</span>`).join(" ") : null) +
        row("also called", node.aliases?.length ? node.aliases.map((a) => h(a)).join(", ") : null, "wrap");
    } else if (node.kind === "resource") {
      const users = live ? live.sessions.filter((s) => (s.host ?? null) === (node.host ?? null) && s.mcp.some((m) => m.server === node.name)) : [];
      grid =
        row("last call", node.lastActivity ? timeAgo(node.lastActivity) : "—") +
        row("used by", users.length ? users.map((s) => (index.get(s.id) ? `<a href="#" data-goto="${h(s.id)}">${h(index.get(s.id).title)}</a>` : "")).filter(Boolean).join("<br>") : "—");
    } else {
      grid =
        row("sessions", h(node.sessionCount ?? 0)) +
        row("last activity", node.lastActivity ? timeAgo(node.lastActivity) : "—") +
        row("folders", node.roots ? node.roots.map((r) => mono(shortPath(r, 40))).join("<br>") : null);
    }

    const canTerminal = onTerminal && !node.host && (node.kind === "project" || node.kind === "zone" || node.kind === "wiki" || (isSession && node.cwd));
    // A session of this machine's takes a message from you, from here, and
    // so does one on another machine in your network.
    const canMessage = onMessageSession && isSession && (!node.host || node.network);
    const actions = [...(node.actions ?? []), ...(canMessage ? [{ id: "message", label: "✉ Message", client: true }] : []), ...(canTerminal ? [{ id: "terminal", label: "›_ Open terminal here", client: true }] : [])]
      .map((a, i) => `<button class="act ${i === 0 && isSession ? `primary ${h(node.app)}` : ""}" data-action="${h(a.id)}">${h(a.label)}</button>`)
      .join("");

    const list = isSession ? (children.get(node.id) ?? []).map((id) => index.get(id)).filter(Boolean) : sessionsUnder(node.id);
    const listTitle = isSession ? "sessions it started" : "sessions";
    const listHtml = list.length
      ? `<div class="panel-title sub"><span>${listTitle}</span><span class="tick">${list.length}</span></div>
         <ol class="sessions">${list
           .slice(0, 40)
           .map(
             (s) => `<li data-goto="${h(s.id)}" class="${h(s.activity)}${s.archived ? " archived" : ""}">
               <i class="sw ${h(s.app)}"></i><span class="feed-title">${h(s.title)}</span><span class="feed-time">${timeAgo(s.updatedAt)}</span>
             </li>`,
           )
           .join("")}</ol>`
      : "";

    // A session of this machine's has its name beside a pencil, to rename it.
    const titleHtml =
      onRename && renamable(node)
        ? `<div class="insp-title-row"><h2 class="insp-title" data-rename-title>${h(title)}</h2><button class="rename-btn" data-rename title="Rename · F2" aria-label="Rename this session">✎</button></div>`
        : `<h2 class="insp-title">${h(title)}</h2>`;

    el.style.setProperty("--accent", color);
    el.innerHTML = `
      <div class="panel-title"><span class="chip">${h(chip)}</span><button class="close" data-close title="close">✕</button></div>
      ${titleHtml}
      <div class="insp-sub">${h(sub)}</div>
      ${whereHtml(node, index)}
      ${actions ? `<div class="actions">${actions}</div>` : ""}
      ${isSession ? `<div class="panel-title sub"><span>${node.network === true ? "Presence" : "Live"}</span><span class="tick live-tick">● live</span></div><div id="insp-live" class="insp-live">${liveSectionHtml(node)}</div>` : ""}
      <div class="panel-title sub"><span>Details</span></div>
      <div class="grid">${grid}</div>
      ${mcp && ["resource", "wiki"].includes(node.kind) ? '<div class="panel-title sub"><span>MCP server</span></div><div id="insp-mcp"></div>' : ""}
      ${identities?.sectionHtml(node, { index, parent }) ?? ""}
      ${listHtml}`;
    el.hidden = false;
    if (mcp && ["resource", "wiki"].includes(node.kind)) mcp.inspect(node);
    if (!quiet) {
      el.classList.remove("enter");
      void el.offsetWidth;
      el.classList.add("enter");
    }
  }

  async function runAction(node, action) {
    sfx("select");
    if (action === "preview") return onPreview(node);
    if (action === "stats") return onStats?.(node);
    if (action === "handoff") return onHandoff?.(node);
    if (action === "terminal") return onTerminal?.(node);
    if (action === "message") return onMessageSession?.(node);
    try {
      const res = await fetch("/api/open", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ nodeId: node.id, action }) });
      const body = await res.json();
      if (!res.ok) throw new Error(body.error);
      onOpened?.(node, action);
      const where = action === "open-app" ? APP_DESKTOP[node.app] : action === "open-vscode" ? "VS Code" : action === "new-claude" ? "Claude desktop" : fileManager;
      toast(`Opening in ${where}`);
    } catch (err) {
      toast(`Couldn't open it: ${err.message}`, true);
    }
  }

  // ---------- renaming a session ----------

  // The inspector's title becomes a field: Enter or clicking away saves,
  // Esc puts the name back. While it's open (`renaming`) the inspector isn't
  // drawn again, so the map's updates don't take what you're typing.
  const renameWhere = (n) =>
    data?.demo ? "The demo keeps the new name in XenoNet" : n.app === "codex" ? "Codex gets the new name too" : n.inDesktop ? "Claude Code and Claude desktop get it too" : "Claude Code gets the new name too";

  // The field grows with the name, as the title it stands in for would (the
  // height takes in its bottom border).
  const fit = (input) => {
    input.style.height = "auto";
    input.style.height = `${input.scrollHeight + 1}px`;
  };

  function startRename() {
    if (renaming) return !renaming.saving && ($("#inspector .rename-input")?.focus(), true);
    const node = current && fresh(current);
    const row = $("#inspector .insp-title-row");
    if (!onRename || !renamable(node) || !row) return false;
    renaming = { id: node.id, from: node.title, saving: false };
    const form = document.createElement("form");
    form.className = "rename";
    form.dataset.renameForm = "";
    form.innerHTML = `<textarea class="insp-title rename-input" rows="1" maxlength="${MAX_TITLE}" spellcheck="false" aria-label="Session name">${h(node.title)}</textarea>
      <div class="rename-foot"><span class="rename-where">${h(renameWhere(node))}</span><span class="rename-count"></span><span class="rename-keys"><kbd>Enter</kbd> save <kbd>Esc</kbd> cancel</span></div>`;
    row.replaceWith(form);
    const input = form.querySelector("textarea");
    fit(input);
    input.focus();
    input.select();
    sfx("select");
    return true;
  }

  // Back to the title: as it was, or with the name just saved, which glows.
  function endRename(saved = null) {
    renaming = null;
    const node = current && fresh(current);
    if (node && saved && saved.id === node.id) node.title = saved.title;
    if (!node) return;
    renderInspector(node, { quiet: true });
    if (saved?.id === node.id) $("#inspector .insp-title")?.classList.add("renamed");
  }

  // A name the field can't take: it shakes and says why, and stays open.
  function refuse(form, why) {
    form.classList.remove("refused");
    void form.offsetWidth;
    form.classList.add("refused");
    form.querySelector(".rename-where").textContent = why;
    sfx("error");
  }

  async function commitRename() {
    const r = renaming;
    const form = $("#inspector [data-rename-form]");
    if (!r || r.saving || !form) return;
    const input = form.querySelector("textarea");
    const title = cleanTitle(input.value);
    if (!title) return refuse(form, "A session needs a name");
    if (title === r.from) return endRename();
    r.saving = true;
    input.readOnly = true;
    form.classList.add("saving");
    const res = await onRename(r.id, title);
    if (renaming !== r) return;
    if (res.ok) return endRename({ id: r.id, title: res.title ?? title });
    // Still showing: try again or press Esc. Gone with another selection: the
    // toast has said what went wrong.
    r.saving = false;
    if (!form.isConnected) return void (renaming = null);
    input.readOnly = false;
    form.classList.remove("saving");
    refuse(form, res.error ?? "That didn't work");
    input.focus();
  }

  document.addEventListener("keydown", (e) => {
    if (!e.target.matches?.(".rename-input")) return;
    if (e.key === "Enter") {
      e.preventDefault();
      commitRename();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (renaming && !renaming.saving) (endRename(), sfx("unfocus"));
    }
  });
  // A name is one line: a line break typed or pasted isn't taken.
  document.addEventListener("beforeinput", (e) => {
    if (e.target.matches?.(".rename-input") && /^insert(LineBreak|Paragraph)$/.test(e.inputType)) e.preventDefault();
  });
  document.addEventListener("input", (e) => {
    if (!e.target.matches?.(".rename-input")) return;
    const input = e.target;
    if (/[\r\n]/.test(input.value)) input.value = input.value.replace(/[\r\n]+/g, " ");
    fit(input);
    const form = input.closest("form");
    form.classList.remove("refused");
    const node = renaming && index.get(renaming.id);
    if (node) form.querySelector(".rename-where").textContent = renameWhere(node);
    const left = MAX_TITLE - input.value.length;
    form.querySelector(".rename-count").textContent = left <= 20 ? `${left} left` : "";
  });
  document.addEventListener("focusout", (e) => {
    if (e.target.matches?.(".rename-input") && renaming && !renaming.saving && document.hasFocus()) commitRename();
  });
  document.addEventListener("submit", (e) => {
    if (!e.target.matches?.("[data-rename-form]")) return;
    e.preventDefault();
    commitRename();
  });
  document.addEventListener("dblclick", (e) => {
    if (e.target.closest("[data-rename-title]")) startRename();
  });

  // ---------- tooltip / toast / clock ----------

  const tip = $("#tooltip");
  let mouse = null;
  addEventListener("pointermove", (e) => {
    mouse = { x: e.clientX, y: e.clientY };
    if (!tip.hidden) tip.style.transform = `translate(${mouse.x + 16}px, ${mouse.y + 14}px)`;
  });
  let hovering = null;
  function hover(node) {
    hovering = node;
    if (!node || !mouse) {
      tip.hidden = true;
      return;
    }
    if (node.kind === "actor") {
      if (!actorViews) return void loadActorViews().then(() => hovering === node && hover(node));
      tip.style.setProperty("--accent", `#${node.color.getHexString()}`);
      tip.innerHTML = actorViews.actorTipHtml(node);
      tip.style.transform = `translate(${mouse.x + 16}px, ${mouse.y + 14}px)`;
      tip.hidden = false;
      return;
    }
    const isSession = node.kind === "session";
    const s = isSession ? liveById.get(node.id) : null;
    const liveLine = s && s.status !== "idle" ? ` · ${LIVE_LABEL[s.status]}` : "";
    tip.style.setProperty("--accent", nodeColorVar(node));
    const where = tipText(node, index);
    tip.innerHTML = `<div class="tip-k">${h(isSession ? APP_LABEL[node.app] : kindLabel(node))}${isSession ? ` · ${ACTIVITY_LABEL[node.activity]}${liveLine}` : ""}${where ? ` · ${h(where)}` : ""}</div>
      <div class="tip-t">${h(isSession ? node.title : node.name)}</div>
      <div class="tip-m">${
        isSession
          ? `${timeAgo(node.updatedAt)}${s ? ` · ctx ${Math.round((s.context.used / s.context.window) * 100)}% · ${compact(s.tpm)}/min` : ""}${s?.needsYou && s.needsYou.kind !== "review" ? " · waiting for you" : ""}`
          : node.kind === "page"
            ? `${h(node.type)} · updated ${timeAgo(node.updated)}`
            : node.kind === "wiki"
              ? `${h(node.pageCount)} pages · ${h(node.inbox?.length ?? 0)} unfiled`
              : `${h(node.sessionCount ?? 0)} sessions${node.repo?.dirty ? ` · Δ${h(node.repo.dirty)} uncommitted` : ""}`
      }</div>`;
    tip.style.transform = `translate(${mouse.x + 16}px, ${mouse.y + 14}px)`;
    tip.hidden = false;
  }

  let toastTimer;
  // One toast at a time: a new message replays the entrance, and the old one
  // fades out rather than vanishing.
  function toast(msg, bad = false) {
    const t = $("#toast");
    t.textContent = msg;
    t.classList.toggle("bad", bad);
    t.classList.remove("out");
    t.hidden = false;
    t.style.animation = "none";
    void t.offsetWidth;
    t.style.animation = "";
    clearTimeout(toastTimer);
    // Long enough to read: a longer message stays longer.
    toastTimer = setTimeout(() => {
      t.classList.add("out");
      toastTimer = setTimeout(() => (t.hidden = true), 200);
    }, Math.max(2600, String(msg).length * 45));
  }

  let clockOverride = null;
  const dateEl = $("#clock-date");
  const tickClock = () => {
    $("#clock").textContent = clockOverride ?? new Date().toLocaleTimeString(undefined, { hour12: false });
    // The date under it, except while a film or a replay holds the clock.
    if (dateEl) dateEl.textContent = clockOverride ? "" : new Date().toLocaleDateString(undefined, { weekday: "short", day: "2-digit", month: "short" });
  };
  setInterval(tickClock, 1000);

  function setLink(state) {
    $("#link-led").className = `led ${state}`;
    $("#link-text").textContent = { live: "connected", down: "reconnecting", connecting: "connecting" }[state];
  }

  // ---------- search ----------

  const search = $("#search");
  const results = $("#search-results");
  let matches = [];
  function runSearch() {
    const q = search.value.trim().toLowerCase();
    if (!q) {
      matches = [];
      results.innerHTML = "";
      onFilters({ visible: computeVisible(), dim: null });
      return;
    }
    matches = data.nodes
      .filter((n) => n.kind !== "core" && `${n.title ?? ""} ${n.name ?? ""} ${n.cwd ?? ""} ${n.branch ?? ""}`.toLowerCase().includes(q))
      .sort((a, b) => (b.updatedAt ?? b.lastActivity ?? 0) - (a.updatedAt ?? a.lastActivity ?? 0));
    results.innerHTML =
      matches
        .slice(0, 8)
        .map((n) => `<div class="hit" data-goto="${h(n.id)}"><i class="sw ${h(n.app ?? n.kind)}"></i>${hostTag(n)}<span>${h(n.title ?? n.name)}</span></div>`)
        .join("") || `<div class="hit none">No matches</div>`;
    onFilters({ visible: computeVisible(), dim: new Set(matches.map((n) => n.id)) });
  }
  search.addEventListener("input", runSearch);
  search.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && matches[0]) onSelect(matches[0]);
    if (e.key === "Escape") {
      search.value = "";
      runSearch();
      search.blur();
    }
  });

  // ---------- legend ----------

  const legend = $("#legend");
  function foldLegend(folded) {
    legend.classList.toggle("folded", folded);
    legend.querySelector("[data-legend-toggle]").setAttribute("aria-expanded", String(!folded));
  }
  try {
    foldLegend(localStorage.getItem("xenonet.legend") === "folded");
  } catch {}

  // ---------- events ----------

  document.addEventListener("click", (e) => {
    const goto = e.target.closest("[data-goto]");
    if (goto) {
      e.preventDefault();
      const n = index.get(goto.dataset.goto);
      if (n) onSelect(n);
      return;
    }
    const feedItem = e.target.closest("#feed li[data-id]");
    if (feedItem) {
      const n = index.get(feedItem.dataset.id);
      if (n) onSelect(n);
      return;
    }
    const actorLine = e.target.closest("#feed li[data-actor]");
    if (actorLine) {
      const rec = actorOf(actorLine.dataset.actor);
      if (rec) onSelect(rec);
      return;
    }
    if (current && e.target.closest("[data-need-open]")) return onOpenApp?.(current);
    if (current && e.target.closest("[data-need-ack]")) return onAck?.([current.id]);
    if (current && e.target.closest("[data-fresh]")) return onHandoff?.(current, current.app);
    if (e.target.closest("[data-rename]")) return void startRename();
    const write = e.target.closest("[data-message-actor]");
    if (write) return void onMessageActor?.(write.dataset.messageActor);
    const act = e.target.closest("[data-action]");
    if (act && current) return runAction(current, act.dataset.action);
    if (e.target.closest("[data-close]")) return onSelect(null);
    const copy = e.target.closest("[data-copy]");
    if (copy) {
      navigator.clipboard?.writeText(copy.dataset.copy).then(() => toast("ID copied"));
      return;
    }
    const toggle = e.target.closest("[data-filter]");
    if (toggle) {
      setFilter(toggle.dataset.filter, !filters[toggle.dataset.filter]);
      sfx("toggle");
      return;
    }
    if (e.target.closest("[data-legend-toggle]")) {
      const folded = !legend.classList.contains("folded");
      foldLegend(folded);
      try {
        localStorage.setItem("xenonet.legend", folded ? "folded" : "open");
      } catch {}
      sfx("toggle");
      return;
    }
    const layout = e.target.closest("[data-layout]");
    if (layout) {
      document.querySelectorAll("[data-layout]").forEach((b) => b.classList.toggle("on", b === layout));
      sfx("toggle");
      onLayout(layout.dataset.layout);
    }
  });

  // A toggle under Filters, on or off: an app's sessions (claude, codex,
  // grok), archived ones, or the folders outside the projects folder.
  function setFilter(name, on) {
    if (!Object.hasOwn(filters, name)) throw new Error(`no filter called ${name}; there are ${Object.keys(filters).join(", ")}`);
    filters[name] = !!on;
    const toggle = document.querySelector(`[data-filter="${name}"]`);
    toggle?.classList.toggle("on", filters[name]);
    // Grok Build's toggle stays while it hides them, to bring them back.
    if (name === "grok" && toggle && !on) toggle.hidden = false;
    runSearch();
  }

  addEventListener("keydown", (e) => {
    if (e.target.matches?.("input, textarea")) return;
    if (e.key === "/") {
      e.preventDefault();
      search.focus();
    } else if (e.key === "r" || e.key === "R") onReset();
    // F2 renames, as it does a file in Explorer.
    else if (e.key === "F2") startRename() && e.preventDefault();
    else if (e.key === "Escape") (onBack ?? (() => onSelect(null)))();
    else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && onStep) {
      e.preventDefault();
      onStep(e.key === "ArrowDown" ? 1 : -1);
    }
  });

  return {
    mcp,
    setData,
    setLive,
    computeVisible,
    refreshFilters: runSearch,
    // Identities masked or shown: the chips and the inspector again.
    refreshIdentities() {
      identities?.renderFilters();
      for (const li of document.querySelectorAll("#feed li.identity")) identityText(li);
      if (current && !renaming) renderInspector(fresh(current), { quiet: true });
    },
    // The actors changed: an actor in the inspector shows what's new, and
    // the actors' views load, ready for the first one shown.
    refreshActor() {
      loadActorViews();
      if (current?.kind === "actor") renderInspector(actorOf(current.id), { quiet: true });
    },
    inspect: (node) => renderInspector(node),
    // Opens the selected session's name for renaming (F2, the pencil, the
    // palette); false when there's nothing here to rename.
    rename: startRename,
    get renaming() {
      return renaming ? { id: renaming.id, saving: renaming.saving } : null;
    },
    hover,
    toast,
    setLink,
    activity: pushActivity,
    feedState: () => ({ paused: feedPaused, pending: pendingFeed.length }),
    // The actors' list (actor/list/v1), drawn: each one's last word starts
    // their lines in the feed, once.
    seedActors,
    setClock(text) {
      clockOverride = text;
      tickClock();
    },
    filters,
    setFilter,
    get current() {
      return current;
    },
    get index() {
      return index;
    },
  };
}
