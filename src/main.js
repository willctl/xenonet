import { createUpdatesPanel } from "./updates.js";
import "@fontsource/chakra-petch/400.css";
import "@fontsource/chakra-petch/500.css";
import "@fontsource/chakra-petch/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "./style.css";
import { createScene } from "./scene.js";
import { createHud } from "./hud.js";
import { createPreview } from "./preview.js";
import { createAmbient } from "./ambient.js";
import { createTelemetry } from "./telemetry.js";
import { createModals } from "./modals.js";
import { createConnectPanel } from "./connect.js";
import { createDoctorPanel } from "./doctor.js";
import { createStatsPanel } from "./stats.js";
import { createRootPanel } from "./root.js";
import { createAlerts } from "./alerts.js";
import { createPalette } from "./palette.js";
import { createReplay } from "./replay.js";
import { createFilm } from "./film.js";
import { note, ops, clearOps } from "./oplog.js";
import { makePoster } from "./poster.js";
import { createIdentities } from "./identities.js";
import { keyLabel } from "./platform.js";
import { WINDOWS, loadScope, saveScope, scopeGraph, scopeCounts } from "./scope.js";
import { syncTheme, themeFonts } from "./theme.js";
import { createThemes } from "./themes-ui.js";
import { createMcpUi } from "./mcp-ui.js";
import { createEnvironments } from "./environments-ui.js";
import { syncEnvironment } from "./environment.js";
import { tildePath, escapeHtml } from "./format.js";
import { createSpotifyChip } from "./spotify-ui.js";
import { createSystemPanel } from "./system-panel.js";
import { createSheets } from "./sheets.js";
import { createYou } from "./you.js";
import { isActorId } from "./actors.js";
import { youHere, createTargetWatch, createSaidEcho } from "./actors-model.js";
import { createWindowCalls, createActorRequests, createActorTokens } from "./actor-requests.js";
import { createRemoteConnections } from "./remote-panel.js";
import { createHostFilters, decorate as decorateHosts } from "./hosts.js";
import { chimes } from "./live-words.js";
import { renamable } from "../core/rename.js";
import "./hosts.css";

const params = new URLSearchParams(location.search);
const desktop = window.xenonetDesktop ?? null;

// Dev builds keep the page's uncaught errors for scripts and tests (see
// driver.js), from the start, so ones raised while booting count too.
const pageErrors = [];
if (import.meta.env.DEV || desktop) {
  const keep = (message, source) => {
    pageErrors.push({ message: String(message).slice(0, 2000), source, ts: Date.now() });
    if (pageErrors.length > 100) pageErrors.shift();
  };
  addEventListener("error", (e) => keep(e.error?.stack ?? e.message, e.filename ? `${e.filename}:${e.lineno}` : null));
  addEventListener("unhandledrejection", (e) => keep(e.reason?.stack ?? e.reason, null));
}
// The page's uncaught errors go in XenoNet's error log too (POST
// /api/logs/page), a few a minute at most, so xenonet logs and the doctor
// see what went wrong here.
{
  let sent = 0, since = Date.now();
  const report = (message, stack) => {
    if (Date.now() - since > 60e3) (sent = 0), (since = Date.now());
    if (++sent > 5) return;
    fetch("/api/logs/page", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ message: String(message ?? "").slice(0, 1000), stack: String(stack ?? "").slice(0, 3000) }) }).catch(() => {});
  };
  addEventListener("error", (e) => report(e.message, e.error?.stack));
  addEventListener("unhandledrejection", (e) => report(e.reason?.message ?? e.reason, e.reason?.stack));
}
if (desktop) document.body.classList.add("desktop");
// Shortcut hints in the page are written for Windows; show this platform's.
for (const el of document.querySelectorAll("[data-key]")) el.textContent = keyLabel(el.dataset.key);
for (const el of document.querySelectorAll("[data-key-title]")) el.title = el.title.replace(/·.*$/, `· ${keyLabel(el.dataset.keyTitle)}`);

// Hidden or background tabs pause requestAnimationFrame. `?raf-shim` keeps the
// scene rendering there (dev only, for automated screenshots).
if (import.meta.env.DEV && params.has("raf-shim")) {
  window.requestAnimationFrame = (cb) => setTimeout(() => cb(performance.now()), 16);
}

// Sprite labels are drawn onto canvases, so the theme's fonts must be ready first.
await themeFonts();

const ambient = createAmbient();
// A sound effect; one of the map's own (an alert) from where it happened,
// given its id (ambient.js).
const sfx = (name, at) => ambient.sfx(name, at);
const audioButton = document.getElementById("audio");
audioButton.addEventListener("click", () => ambient.toggle());
ambient.onChange((state) => {
  audioButton.dataset.state = state;
  audioButton.querySelector(".audio-label").textContent = { playing: "sound on", muted: "muted", armed: "click for sound" }[state];
  audioButton.title = state === "muted" ? "Turn sound on · M" : "Mute · M";
});

let lastWorkAt = Date.now();
let searchDim = null;
let identityLens = null;
let searchDimTimer = null;
let wallpaperOn = false;
let terminal = null;
// The actors' list as it last came (see applyActors), up here since your
// record can come before the rest of the page is set up.
let lastActors = null;
const post = (url, body) => fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) });
// The window's own calls, with the key only this window holds
// (actor-requests.js).
const windowCalls = createWindowCalls({ desktop });
// You at the console (you.js): what you look at and open here, told to
// XenoNet as node ids and actions, with that key. Your record, read back
// with it, draws your beams (see asYou).
const you = createYou({ desktop, calls: windowCalls, onRecord: () => lastActors && applyActors(lastActors) });
// The programs waiting for you to allow them as actors: a card each in the
// attention strip, and the Actors panel.
// The panel (actors-panel.js) loads the first time it opens; see openActors.
let actorsPanel = null;
let loadingActorsPanel = null;
const actorRequests = createActorRequests({
  calls: windowCalls,
  onChange: (view) => {
    alerts.setRequests(view);
    actorsPanel?.refresh();
  },
});
// How each registered actor's token stands, read with the window's key
// (actor-requests.js), for the Actors panel and the driver.
const actorTokens = createActorTokens({ calls: windowCalls });
// Remote actors' connections (remote-panel.js), read with the window's key,
// for the Actors panel and, read-only, the driver.
const remoteConnections = createRemoteConnections({ calls: windowCalls });
const preview = createPreview({ onOpenApp: (node) => openInApp(node) });
// A preview, wherever it's opened from, is reading the session.
const showPreview = preview.open;
preview.open = (node, ...rest) => (node?.kind === "session" && you.did("read", node.id), showPreview(node, ...rest));

// ---------- selection, with sound ----------

let selectedId = null;
function select(nodeOrId) {
  const id = typeof nodeOrId === "string" ? nodeOrId : (nodeOrId?.id ?? null);
  // An actor (actors.js) is selected like a node, from the layer.
  const actor = id && isActorId(id) ? scene.actor(id) : null;
  // Opened from the palette or an alert while outside the session scope: draw it first.
  if (id && !actor && !scene.getNode(id) && fullIndex.has(id)) reveal(id);
  const node = actor ?? (id ? (scene.getNode(id) ?? null) : null);
  if (node && wallpaperOn) setWallpaper(false);
  sfx(node ? (node.id === selectedId ? "select" : "focus") : selectedId ? "unfocus" : "select");
  selectedId = node?.id ?? null;
  // A host you look at is drawn in full (hosts.js), and that's said before
  // a remote project gathers its sessions, so those the cap hid are drawn
  // by the time they gather.
  hostFilters.look(node);
  scene.select(node ?? null);
  hud.inspect(node ?? null);
  note("select", selectedId);
  // You look at places and sessions; an actor isn't a node of the map, so
  // selecting one only ends a look in progress.
  you.looked(actor ? null : selectedId);
}

// Esc steps back: from a gathered session to its project, then out.
function back() {
  const root = scene.gatheredRoot();
  select(root && selectedId !== root.id ? root : null);
}
function step(by) {
  const next = scene.stepGathered(scene.getNode(selectedId), by);
  if (next) select(next);
}

// While a project's sessions are gathered, a bar says so and puts them back.
const gatherBar = document.getElementById("gather");
gatherBar.querySelector("[data-gather-done]").addEventListener("click", () => select(null));
function showGather(g) {
  document.body.classList.toggle("gathered", !!g);
  gatherBar.hidden = !g;
  if (!g) return;
  const what = g.sessions < g.total ? `the ${g.sessions} most recent of ${g.total} sessions` : `${g.total} session${g.total === 1 ? "" : "s"}`;
  gatherBar.querySelector("[data-gather-text]").textContent = `${what} around ${g.root.name ?? g.root.title}${g.root.host ? ` on ${g.root.host}` : ""}`;
}

async function openInApp(nodeOrId) {
  const node = typeof nodeOrId === "string" ? scene.getNode(nodeOrId) : nodeOrId;
  if (!node) return;
  const res = await post("/api/open", { nodeId: node.id, action: "open-app" });
  if (res.ok) you.did("opened", node.id);
  sfx(res.ok ? "success" : "error");
  hud.toast(res.ok ? `Opening in ${node.app === "codex" ? "Codex" : "Claude"}` : "Couldn't open the desktop app", !res.ok);
}

// A session's new name, saved in its own app (core/rename.js), so Claude
// Code, Claude desktop or Codex shows it too. Answers { ok, title, saved,
// note, error } and says how it went.
async function renameSession(id, title) {
  const res = await post("/api/session/rename", { nodeId: id, title });
  const body = await res.json().catch(() => ({}));
  sfx(res.ok ? "success" : "error");
  if (!res.ok) {
    hud.toast(`Couldn't rename it: ${body.error ?? res.statusText}`, true);
    return { ok: false, error: body.error ?? res.statusText };
  }
  const where = body.demo ? "Renamed in the demo" : `Renamed in ${body.saved.join(" and ")}`;
  hud.toast(body.note ? `${where}. ${body.note}` : where, !!body.note && !body.saved.includes("Claude desktop"));
  return { ok: true, ...body };
}

async function ack(ids) {
  for (const id of ids) await post("/api/ack", { sessionId: id });
  hud.toast(ids.length > 1 ? `Dismissed ${ids.length} alerts` : "Dismissed");
}

// Who sessions ran as (see identities.js): the chips under Filters, the
// inspector's section, the map's rings, and masking.
const identities = createIdentities({
  sfx,
  toast: (m, b) => hud.toast(m, b),
  onFilter: () => hud.refreshFilters(),
  // Hovering a chip lights its work, over any other dimming, until the
  // pointer leaves.
  onLens: (ids) => {
    identityLens = ids;
    hud.refreshFilters();
  },
  onChange: () => {
    hud.refreshIdentities();
    modals.refreshIdentities();
  },
});
identities.wire(document.getElementById("id-filters"));
// Other machines (hosts.js): a chip per machine under Filters, there once a
// host is on the map.
const hostFilters = createHostFilters({ sfx, onFilter: () => hud.refreshFilters() });

const modals = createModals({ sfx, toast: (m, b) => hud.toast(m, b), select, onPoster: () => poster(), identities, onChangeRoot: () => rootPanel.open(), onHandoff: (node, target) => you.did("handed-off", node.id, { to: target }) });
const connectPanel = createConnectPanel({ modals, sfx, toast: (m, b) => hud.toast(m, b) });
// XenoNet's doctor (core/doctor): its panel, and its problems in the strip.
const doctorPanel = createDoctorPanel({
  modals,
  sfx,
  toast: (m, b) => hud.toast(m, b),
  onResult: (r) => alerts.setDoctor(r),
  // Ask an agent runs in a terminal tab here, where Desktop has them.
  terminal: desktop?.terminal
    ? async (dir, command) => {
        await terminal?.show({ cwd: dir, fresh: true });
        terminal?.write(null, `${command}\r`);
      }
    : null,
});
// A session's models, tokens, costs and timing (core/stats.js).
const statsPanel = createStatsPanel({ modals, sfx, toast: (m, b) => hud.toast(m, b) });
const openStats = (node) => (note("stats", node.id), you.did("read", node.id), statsPanel.open(node));
// The OpenRouter key for decision models: into your credential store, by
// the window's own route; the field is cleared as it's sent.
async function decisionKey() {
  const now = await fetch("/api/decide").then((r) => (r.ok ? r.json() : null)).catch(() => null);
  const panel = modals.open(`<header class="m-head"><div><div class="m-chip">⚖ decisions</div><h2>Decision models</h2><div class="m-sub">${now ? `${escapeHtml(now.model)} · ${now.key === "stored" ? "a key is kept" : "no key yet"}` : ""}</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"><p class="pj-note">XenoNet can ask a decision model on OpenRouter some of what it now asks Codex, such as which folders are projects. It sends only what it shows you first, asks for zero data retention, and keeps only the answers it's sure of. With the key, Token use also shows what it has spent. Paste an OpenRouter key; XenoNet keeps it in ${escapeHtml(now?.store ?? "your credential store")} and never shows it again. Give it a credit limit on OpenRouter.</p><input class="msg-text" type="password" data-dk-key data-yours placeholder="sk-or-…" autocomplete="off" spellcheck="false" style="min-height:0" /><div class="m-foot">${now?.key === "stored" ? `<button class="act" data-dk-forget>Forget the key</button>` : ""}<span class="grow"></span><button class="act primary" data-dk-keep data-yours>Keep it</button></div></div>`, { accent: "var(--accent)" });
  panel.querySelector("[data-dk-keep]").addEventListener("click", async (e) => {
    const input = panel.querySelector("[data-dk-key]");
    const key = input.value.trim();
    input.value = "";
    if (!key) return input.focus();
    e.currentTarget.disabled = true;
    const r = await windowCalls.call("POST", "/api/decide/key", { key });
    if (!r.ok) {
      e.currentTarget.disabled = false;
      sfx("error");
      return hud.toast(r.error ?? "It wasn't kept", true);
    }
    sfx("success");
    modals.close();
    hud.toast("The key is kept; XenoNet won't show it again");
  });
  panel.querySelector("[data-dk-forget]")?.addEventListener("click", async () => {
    await fetch("/api/decide/key", { method: "DELETE" });
    modals.close();
    hud.toast("XenoNet forgot the key");
  });
}

// A message from you to an actor, from XenoNet's own window, which it reads
// in its inbox marked as from you (core/engine/attest.js).
function messageActor(id) {
  const name = scene.actor(id)?.snapshot.name ?? id;
  const panel = modals.open(`<header class="m-head"><div><div class="m-chip">✉ message</div><h2>To ${escapeHtml(name)}</h2><div class="m-sub">It reads it in its inbox, marked as from you at XenoNet's window</div></div><button class="act" data-mclose>Close ✕</button></header><div class="m-body"><textarea class="msg-text" data-msg-text data-yours maxlength="4000" rows="6" placeholder="What to tell ${escapeHtml(name)}"></textarea><p class="pj-note">An actor reads it as from you. It's still its own to act on: XenoNet passes it on and approves nothing.</p><div class="m-foot"><span class="grow"></span><button class="act primary" data-msg-send data-yours>Send</button></div></div>`, { accent: "var(--actor)" });
  const box = panel.querySelector("[data-msg-text]");
  box.focus();
  panel.querySelector("[data-msg-send]").addEventListener("click", async (e) => {
    const text = box.value.trim();
    if (!text) return box.focus();
    e.currentTarget.disabled = true;
    const r = await windowCalls.call("POST", "/api/actors/you/message", { to: id, text });
    if (!r.ok) {
      e.currentTarget.disabled = false;
      sfx("error");
      return hud.toast(r.error ?? "It didn't go", true);
    }
    sfx("success");
    modals.close();
    hud.toast(`Sent to ${name}`);
  });
}
// A terminal opened in a node's folder (`node`, or what's selected).
const terminalAt = (node = scene.getNode(selectedId)) => (you.did("terminal", node?.id), terminal?.show({ cwd: folderOf(node), fresh: true }));
// The projects folder XenoNet reads, and changing it (root.js).
const rootPanel = createRootPanel({ modals, sfx, toast: (m, b) => hud.toast(m, b), desktop });

const hud = createHud({
  mcp: createMcpUi({ modals, toast: (m, b) => hud.toast(m, b) }),
  onSelect: select,
  onBack: back,
  onStep: step,
  onFilters: ({ visible, dim }) => {
    if (replay?.active) return;
    scene.setVisible(hostFilters.restrict(visible));
    scene.dim(identityLens ?? dim ?? searchDim);
  },
  onLayout: (mode) => scene.setLayout(mode),
  onPreview: (node) => preview.open(node),
  onStats: (node) => openStats(node),
  onReset: () => scene.reset(),
  onHandoff: (node, target) => modals.handoff(node, target),
  onTerminal: desktop ? (node) => terminalAt(node) : null,
  onAck: ack,
  onOpenApp: openInApp,
  // The inspector's own buttons: a session opened in its app, or a new one.
  onOpened: (node, action) => (action === "open-app" ? you.did("opened", node.id) : action === "new-claude" ? you.did("started", node.id, { app: "claude" }) : null),
  onRename: (id, title) => (note("rename", id, title), renameSession(id, title)),
  // Writing to an actor from here (its inspector): yours alone, so the
  // box and Send are marked data-yours, which XenoNet's control channel
  // never presses outside the demo.
  onMessageActor: (id) => messageActor(id),
  identities,
  actorOf: (id) => scene.actor(id),
  actorBeams: (id) => scene.actorLayer.targetsOf(id),
  sfx,
});

const telemetry = createTelemetry({ onSelect: select });

// On a narrow screen the panels are one sheet at the bottom, picked by
// tabs (sheets.js).
// A panel shown in the sheet measures itself again, as on a resize (Token
// use's chart sizes its canvas then).
const sheets = createSheets({ tabs: document.getElementById("sheet-tabs"), inspector: document.getElementById("inspector"), onChange: () => requestAnimationFrame(() => dispatchEvent(new Event("resize"))) });
const scene = createScene(document.getElementById("scene"), {
  onSelect: select,
  onHover: (node) => {
    hud.hover(node);
    if (node) sfx("hover");
  },
  onPreview: (node) => {
    hud.inspect(node);
    preview.open(node);
  },
  onSonar: () => ambient.ping(),
  onGather: showGather,
  // What the panels, the alert strip and the terminal cover, so the map, a
  // gathering or an actor is framed in the space between. A film of the
  // day hides them all; wallpaper mode fades the panels out, leaving only
  // the strip's waiting cards.
  insets: () => {
    const rect = (sel) => {
      const r = document.querySelector(sel)?.getBoundingClientRect();
      return r?.width ? r : null;
    };
    if (document.body.classList.contains("filming")) return { left: 0, right: 0, top: 0, bottom: 0 };
    const alerts = rect("#alerts");
    if (wallpaperOn) return { left: 0, right: 0, top: alerts?.bottom ?? 0, bottom: 0 };
    const left = rect(".hud-left"), right = rect(".hud-right"), drawer = rect("#terminal");
    // Narrow, the panels are a sheet over the bottom of the map, above the
    // tabs; nothing covers its sides.
    if (sheets.narrow) {
      const tops = [left, right, rect("#sheet-tabs"), drawer].filter(Boolean).map((r) => r.top);
      return { left: 0, right: 0, top: alerts?.bottom ?? 0, bottom: tops.length ? innerHeight - Math.min(...tops) : 0 };
    }
    return { left: left?.right ?? 0, right: right ? innerWidth - right.left : 0, top: alerts?.bottom ?? 0, bottom: drawer ? innerHeight - drawer.top : 56 };
  },
  onPin: (node) => {
    post("/api/layout", { positions: { [node.id]: { x: node.x, y: node.y, z: node.z } } });
    sfx("pin");
    hud.toast(`Pinned ${node.title ?? node.name}. Undo with Reset layout in ${keyLabel("Mod+K")}.`);
  },
});
// Sounds come from where things happen on the map: the camera is the ear
// (ambient.js, sound-space.js).
ambient.attach({ ear: () => scene.ear(), where: (id) => scene.where(id) });
// What's playing in Spotify, under the status panel (spotify-ui.js); while
// it plays, XenoNet's music steps aside.
const spotifyChip = createSpotifyChip({ el: document.getElementById("spotify"), calls: windowCalls, modals, toast: (m, b) => hud.toast(m, b), sfx, ambient });
spotifyChip.refresh();
// This computer's CPU, GPU and memory, between the status and Token use
// panels (system-panel.js).
const systemPanel = createSystemPanel({ el: document.getElementById("sysmon"), tick: document.getElementById("sysmon-tick") });

const alerts = createAlerts({
  sfx,
  select,
  ack,
  openApp: openInApp,
  collision: (a, b) => modals.collision(a, b),
  doctor: () => doctorPanel.open(),
  requests: {
    allow: (id) => decideRequest(id, "allow"),
    deny: (id) => decideRequest(id, "deny"),
    open: () => openActors(),
    armed: (id) => actorRequests.armed(id),
    armsIn: (id) => actorRequests.armsIn(id),
  },
  native: desktop ? { notify: (n) => desktop.notify(n), clear: (keys) => desktop.clearNotifications(keys) } : null,
});

const updatesPanel = createUpdatesPanel({ modals, desktop, toast: (message) => hud.toast(message) });

// ---------- folders for "terminal here" ----------

function folderOf(node = scene.getNode(selectedId)) {
  if (!node || node.host) return null;
  if (node.kind === "session") return node.worktree || node.cwd || null;
  if (node.kind === "page") return null;
  return node.path ?? null;
}

// ---------- terminal (desktop app) ----------

if (desktop?.terminal) {
  const { createTerminalDrawer } = await import("./terminal.js");
  terminal = createTerminalDrawer({ api: desktop.terminal, sfx, toast: (m) => hud.toast(m), folderOf: () => folderOf() });
  document.getElementById("terminal-toggle").hidden = false;
  document.getElementById("terminal-toggle").addEventListener("click", () => terminal.toggle());
  desktop.onSelect?.((id) => select(id));
  desktop.onCommand?.((cmd) => {
    // macOS menu bar: ⌘W closes the focused terminal tab, or else the window.
    if (cmd === "close") return terminal.closeFocusedTab() || desktop.hideWindow?.();
    if (cmd === "palette") return palette.open();
    commandsById[cmd]?.run();
  });
}

// ---------- wallpaper mode ----------

function setWallpaper(on) {
  wallpaperOn = on;
  document.body.classList.toggle("wallpaper", on);
  scene.setWallpaper(on);
  if (on) {
    select(null);
    terminal?.hide();
  }
  sfx(on ? "open" : "close");
  try {
    const u = new URL(location.href);
    on ? u.searchParams.set("wallpaper", "") : u.searchParams.delete("wallpaper");
    history.replaceState(null, "", u);
  } catch {}
}

// ---------- replay & poster ----------

const replay = createReplay({
  scene,
  hud,
  sfx,
  getCanvas: () => scene.graph.renderer().domElement,
  onEnter: () => {
    select(null);
    document.body.classList.add("replaying");
  },
  onExit: () => {
    document.body.classList.remove("replaying");
    hud.refreshFilters();
  },
});

// A film of the day (film.js): the replay, framed to share, recorded to an
// MP4 by the desktop app.
const film = createFilm({ scene, replay });
async function makeFilm() {
  note("film", { seconds: 45 });
  let recording = false;
  if (desktop?.record) {
    try {
      await desktop.record({ action: "start" });
      recording = true;
    } catch (err) {
      return hud.toast(`Couldn't record the film: ${err.message}`, true);
    }
  }
  try {
    await film.play({ seconds: 45 });
  } catch (err) {
    hud.toast(err.message, true);
  }
  if (!recording) return;
  try {
    const done = await desktop.record({ action: "stop" });
    sfx("success");
    hud.toast(`Saved the film: ${tildePath(done.file)}`);
  } catch (err) {
    hud.toast(`The film didn't save: ${err.message}`, true);
  }
}

async function poster() {
  hud.toast("Rendering the poster…");
  await new Promise((r) => setTimeout(r, 60));
  const p = await makePoster({ scene });
  sfx("shutter");
  hud.toast(`Saved a ${p.width}×${p.height} poster`);
  return p;
}

// ---------- commands (palette) ----------

const commandList = [
  { id: "wallpaper", label: "Wallpaper mode", key: "W", keywords: "ambient screensaver", run: () => setWallpaper(!wallpaperOn) },
  { id: "replay", label: "Replay the last 24 hours", keywords: "timelapse time-lapse history", run: () => replay.enter() },
  { id: "film", label: "Make a film of today", keywords: "video movie mp4 share recap day timelapse render record", logs: true, run: () => makeFilm() },
  { id: "poster", label: "Make a poster of today", keywords: "image png export constellation", run: () => poster() },
  { id: "daylog", label: "Show the day's log", keywords: "summary today end of day", run: () => modals.dayLog() },
  { id: "away2", label: "What happened in the last 2 hours", keywords: "away digest", run: () => modals.away(Date.now() - 2 * 3600e3) },
  { id: "away8", label: "What happened in the last 8 hours", keywords: "away digest", run: () => modals.away(Date.now() - 8 * 3600e3) },
  { id: "needs", label: "Open Claude's waiting sessions", keywords: "needs input", run: () => post("/api/open", { action: "needs-input" }) },
  { id: "ackall", label: "Dismiss all ready-for-review alerts", keywords: "acknowledge", run: () => ack(lastLive?.sessions.filter((s) => s.needsYou?.kind === "review" && !s.host).map((s) => s.id) ?? []) },
  // The desktop app notifies natively; a browser tab has to ask first.
  ...(desktop ? [] : [{ id: "notify", label: "Enable desktop notifications", keywords: "alerts", run: async () => hud.toast(`Desktop notifications: ${await alerts.enableNotifications()}`) }]),
  { id: "updates", label: "Check for XenoNet updates", keywords: "version release download install restart", run: () => updatesPanel.open() },
  { id: "doctor", label: "Check XenoNet's setup", keywords: "doctor health sign in login auth permissions problems diagnose", run: () => doctorPanel.open() },
  { id: "connect-agents", label: "Connect your SIs to XenoNet", keywords: "si super intelligence agents claude code codex mcp hooks engine coordinate messages work together", run: () => connectPanel.open() },
  { id: "actors", label: "Actors", keywords: "si agents hermes grok bot muse programs allow deny requests register tokens observers you show me on the map add remove", run: () => openActors() },
  { id: "install-command", label: "Install the xenonet command", keywords: "cli terminal shell path doctor", run: () => installCommand() },
  { id: "layout", label: "Reset layout and unpin dragged nodes", keywords: "positions", run: async () => (await fetch("/api/layout", { method: "DELETE" }), hud.toast("Layout reset")) },
  { id: "projects", label: "How XenoNet finds your projects", keywords: "projects folder groups nested rules detection scan", run: () => modals.projects() },
  { id: "projects-codex", label: "Ask Codex which folders are projects", keywords: "projects folder groups classify", run: () => modals.projects({ showOutline: true }) },
  { id: "projects-rescan", label: "Detect projects again", keywords: "projects folder groups nested scan rescan", run: () => rescanProjects() },
  { id: "root", label: "Change the projects folder…", keywords: "projects folder root directory workspace switch choose another", run: () => rootPanel.open() },
  { id: "identities", label: "Your identities", keywords: "accounts who github claude chatgpt codex git email names sign in", run: () => modals.identities() },
  { id: "identities-mask", label: "Hide or show identities", keywords: "privacy mask screenshot screen share accounts emails private", run: () => identities.toggleMask() },
  { id: "identities-all", label: "Show every identity's work", keywords: "identity account filter clear", run: () => identities.setSelected([]) },
  { id: "identities-rescan", label: "Detect identities again", keywords: "accounts sign in github gh git claude chatgpt", run: () => rescanIdentities() },
  { id: "mute", label: "Mute or unmute sound", key: "M", run: () => ambient.toggle() },
  // The decision model's OpenRouter key (core/decide.js): typed here once,
  // kept in your credential store, never shown again.
  { id: "system-panel", label: "Show or hide this computer's load", keywords: "system cpu gpu ram memory vram load usage hardware stats monitor panel", run: () => { systemPanel.toggle(); hud.toast(systemPanel.on ? "Showing this computer's load" : "Hid this computer's load. Bring it back from the palette."); } },
  { id: "spotify", label: "Connect Spotify, or change what it does", keywords: "spotify music player play pause skip now playing volume duck", run: () => spotifyChip.open() },
  { id: "decision-key", label: "Set the OpenRouter key for decision models", keywords: "openrouter jev decide decision model key router typesafe", run: () => decisionKey() },
  // Provider refusals XenoNet keeps (core/refusals): whether each new one
  // keeps the transcript up to it too, for a dataset. Off until you turn it
  // on; transcripts can hold secrets.
  {
    id: "refusal-context",
    label: "Keep or stop keeping full context with provider refusals",
    keywords: "refusals refused dataset transcript context record safety policy",
    run: async () => {
      const now = await fetch("/api/refusals").then((r) => (r.ok ? r.json() : null)).catch(() => null);
      if (!now) return hud.toast("Only you can change that, from XenoNet's own window", true);
      const r = await fetch("/api/refusals/context", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ on: !now.context }) }).then((x) => x.json()).catch(() => null);
      hud.toast(r?.context ? "XenoNet keeps each new refusal's context, the transcript up to it, in its state folder" : "XenoNet keeps only the refusal itself from now on");
    },
  },
  // Where sounds come from (sound-space.js): from their place on the map,
  // for speakers or in 3D for headphones, or flat, as before.
  ...[
    ["speakers", "Sound from where it happens, for speakers", "spatial stereo pan left right near far distance 3d"],
    ["headphones", "Sound from where it happens, in 3D for headphones", "spatial binaural hrtf 3d near far distance"],
    ["flat", "Sound without placing it on the map", "spatial flat mono stereo off"],
  ].map(([mode, label, keywords]) => ({ id: `sound-${mode}`, label, keywords: `audio sound ${keywords}`, run: () => (ambient.setMode(mode), hud.toast(mode === "headphones" ? "Sound comes from where it happens, in 3D: best with headphones" : mode === "speakers" ? "Sound comes from where it happens" : "Sound plays without a place on the map")) })),
  { id: "reset", label: "Reset view", key: "R", run: () => scene.reset() },
  { id: "archived", label: "Show or hide archived sessions", run: () => document.querySelector('[data-filter="archived"]').click() },
  ...WINDOWS.map((w) => ({
    id: `scope-${w.id}`,
    label: w.id === "all" ? "Show every session and project on the map" : `Show sessions and projects from the last ${{ "24h": "24 hours", "7d": "7 days", "30d": "30 days" }[w.id]}`,
    keywords: "scope history older recent filter projects",
    run: () => setScope({ window: w.id }),
  })),
  { id: "focus-lighting", label: "Turn focus lighting on or off", keywords: "glare glow bloom dim zoom distance", run: () => setFocusLighting(!focusLighting) },
  { id: "scope-background", label: "Show or hide background SIs", keywords: "codex guardian review compaction exec subagents agents scope", run: () => setScope({ background: !scope.background }) },
  ...(desktop?.terminal
    ? [
        { id: "terminal", label: "Show or hide the terminal", key: "Ctrl+`", keywords: "shell powershell zsh bash console", run: () => terminal.toggle() },
        { id: "terminal-here", label: "New terminal in the selected folder", keywords: "shell powershell zsh bash", run: () => terminalAt() },
      ]
    : []),
];
// Each command logs itself as a step when it runs (see oplog.js), once:
// one that logs a step of its own (`logs`, such as film) isn't logged
// again as a command.
const logged = (c) => (c.logs ? c : { ...c, run: (...a) => (note("command", c.id), c.run(...a)) });
commandList.splice(0, commandList.length, ...commandList.map(logged));
const commandsById = Object.fromEntries(commandList.map((c) => [c.id, c]));
// Commands that come and go as a set, by the start of their ids (a
// command per theme): in the palette, and for the tray and the menu bar.
function setCommands(prefix, list) {
  for (let i = commandList.length - 1; i >= 0; i--) if (commandList[i].id.startsWith(prefix)) delete commandsById[commandList.splice(i, 1)[0].id];
  for (const c of list.map(logged)) commandList.push((commandsById[c.id] = c));
}

// Themes (themes-ui.js): a command for each, switching live in every window.
const themesUi = createThemes({ post, sfx, toast: (m, b) => hud.toast(m, b), note, desktop, onCommands: (list) => setCommands("theme-", list) });
const environmentsUi = createEnvironments({ modals, post, sfx, note, toast: (m, b) => hud.toast(m, b), onCommands: (list) => setCommands("environment-", list) });
document.getElementById("environment-open").onclick = () => environmentsUi.open();

// The xenonet command for terminals (core/command.js), run by this app.
async function installCommand() {
  const res = await fetch("/api/command", { method: "POST" });
  const st = await res.json().catch(() => ({}));
  sfx(res.ok ? "success" : "error");
  if (!res.ok) return hud.toast(`Couldn't install the xenonet command: ${st.error ?? res.statusText}`, true);
  hud.toast(st.onPath ? "Installed. Try xenonet doctor in a terminal." : `Installed ${st.file}, but its folder isn't on your PATH`, !st.onPath);
}

async function rescanProjects() {
  const res = await fetch("/api/projects/rescan", { method: "POST" }).then((r) => r.json()).catch(() => ({}));
  sfx(res.scan ? "success" : "error");
  hud.toast(res.scan ? `Found ${res.scan.projects} projects in ${res.scan.groups} group${res.scan.groups === 1 ? "" : "s"}` : "Couldn't read the projects folder", !res.scan);
}

// Rules for how the projects folder is read (see scanProjects), offered for
// whatever is selected: make a group one project, look inside a project,
// leave a folder out, or make a session's folder a project.
const keyOf = (p) => String(p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
const baseOf = (p) => String(p).split(/[\\/]/).filter(Boolean).pop();
function projectCommands(n) {
  const root = keyOf(fullIndex.get("core")?.path);
  if (!n || !root || n.host) return [];
  const rule = (dir, as, done) => async () => {
    const res = await post("/api/projects/rule", { path: dir, as }).then(async (r) => ({ ok: r.ok, ...(await r.json().catch(() => ({}))) }));
    sfx(res.ok ? "success" : "error");
    hud.toast(res.ok ? done : (res.error ?? "That didn't work"), !res.ok);
  };
  const out = [];
  if (n.kind === "project") {
    if (n.group) out.push({ id: "pj-one", label: `Treat ${n.name} as one project`, keywords: "group merge projects folder", run: rule(n.path, "project", `${n.name} is one project now`) });
    else out.push({ id: "pj-look", label: `Look inside ${n.name} for projects`, keywords: "group split nested projects folder", run: rule(n.path, "group", `XenoNet looks inside ${n.name} now`) });
    out.push({ id: "pj-ignore", label: n.group ? `Leave ${n.name} and its projects out` : `Stop treating ${n.name} as a project`, keywords: "ignore hide projects folder", run: rule(n.path, "ignore", `${n.name} is left out. Undo it in How XenoNet finds your projects.`) });
    if (n.rule?.by === "you") out.push({ id: "pj-undo", label: `Undo your rule for ${n.name}`, keywords: "projects folder", run: rule(n.path, null, "Rule removed") });
  }
  const cwd = n.kind === "session" ? keyOf(n.cwd) : null;
  if (cwd?.startsWith(root + "/") && !fullGraph.nodes.some((p) => p.kind === "project" && keyOf(p.path) === cwd)) {
    out.push({ id: "pj-pin", label: `Make ${baseOf(n.cwd)} a project`, keywords: "pin folder projects", run: rule(n.cwd, "project", `${baseOf(n.cwd)} is a project now`) });
  }
  return out;
}

async function rescanIdentities() {
  const res = await post("/api/identities/rescan").then((r) => r.json()).catch(() => ({}));
  const n = res.identities?.length;
  sfx(n != null ? "success" : "error");
  hud.toast(res.enabled === false ? "Identities are off (XENONET_IDENTITIES=off)" : n != null ? `Found ${n} identit${n === 1 ? "y" : "ies"}` : "Couldn't read the identities", n == null);
}

// Identities (see identities.js): show only one's work, and pin whatever is
// selected to an identity XenoNet couldn't place or placed wrong: a session to
// the account it ran as, a project to the GitHub account it belongs to.
function identityCommands(n) {
  const out = identities.groups
    .filter((g) => !g.hidden)
    .map((g) => ({ id: `identity-only:${g.key}`, label: `Show only ${identities.groupLabel(g)}'s work`, keywords: `identity account filter ${g.ids.map(identities.label).join(" ")}`, run: () => identities.only(g.key) }));
  if ((n?.kind !== "session" && n?.kind !== "project") || n.host) return out;
  const what = (n.title ?? n.name ?? "").length > 40 ? `${(n.title ?? n.name).slice(0, 39)}…` : (n.title ?? n.name);
  const pin = (identity, done) => async () => {
    const res = await post("/api/identities/pin", { nodeId: n.id, identity }).then(async (r) => ({ ok: r.ok, ...(await r.json().catch(() => ({}))) }));
    sfx(res.ok ? "success" : "error");
    hud.toast(res.ok ? done : (res.error ?? "That didn't work"), !res.ok);
  };
  const current = n.kind === "session" ? n.ranAs : n.identities?.find((l) => l.role === "owner");
  const github = identities.list.some((i) => i.kind === "github");
  const kinds = n.kind === "project" ? [github ? "github" : "git"] : n.app === "codex" ? ["chatgpt", "openai"] : ["claude"];
  for (const i of identities.list.filter((i) => kinds.includes(i.kind) && i.id !== current?.id)) {
    const who = identities.label(i.id);
    out.push({ id: `identity-pin:${i.id}`, label: n.kind === "session" ? `${what} ran as ${who}` : `${what} belongs to ${who}`, keywords: "identity account pin", run: pin(i.id, `${what} is pinned to ${who}`) });
  }
  if (current?.confidence === "yours") out.push({ id: "identity-unpin", label: `Undo your identity pin for ${what}`, keywords: "identity account pin", run: pin(null, "Pin removed") });
  return out;
}

// Renaming the selected session, in the inspector. It opens once the palette
// has closed, so the Enter that chose it doesn't land in the name.
function renameCommands(n) {
  if (!renamable(n)) return [];
  const what = n.title.length > 40 ? `${n.title.slice(0, 39)}…` : n.title;
  return [{ id: "rename", label: `Rename ${what}…`, key: "F2", keywords: "rename name title label session thread", run: () => requestAnimationFrame(() => hud.rename()) }];
}

const palette = createPalette({
  sfx,
  // The palette searches every session, drawn or not.
  nodes: () => fullGraph?.nodes ?? scene.nodes(),
  // A selected project's own commands come first; for a session, renaming
  // it comes first and its other commands last.
  commands: () => {
    const n = selectedId ? fullIndex.get(selectedId) : null;
    const own = projectCommands(n);
    return n?.kind === "project" ? [...own, ...commandList, ...identityCommands(n)] : [...renameCommands(n), ...commandList, ...own, ...identityCommands(n)];
  },
  // Transcript matches light up for 15 seconds, surviving graph refreshes.
  onSearchResults: (results) => {
    const ids = new Set(results.map((r) => r.sessionId));
    searchDim = ids.size ? ids : null;
    scene.dim(searchDim);
    clearTimeout(searchDimTimer);
    searchDimTimer = setTimeout(() => ((searchDim = null), hud.refreshFilters()), 15000);
  },
  actions: (n) => {
    const acts = [{ key: "Enter", label: "go to", mod: null, run: () => select(n) }];
    if (n.kind === "session") {
      if (!n.host) acts.push({ key: "Mod+O", label: "open in app", mod: "ctrl-o", run: () => openInApp(n) });
      acts.push({ key: "Mod+P", label: "preview", mod: "ctrl-p", run: () => (select(n), preview.open(n)) });
      acts.push({ key: "Mod+I", label: "stats", mod: "ctrl-i", run: () => (select(n), openStats(n)) });
      if (!n.host) acts.push({ key: "Mod+H", label: "hand off", mod: "ctrl-h", run: () => modals.handoff(n) });
    }
    if (n.kind === "project" && n.actions?.some((a) => a.id === "new-claude")) acts.push({ key: "Mod+N", label: "new Claude session here", mod: "ctrl-n", run: () => post("/api/open", { nodeId: n.id, action: "new-claude" }).then((r) => r.ok && you.did("started", n.id, { app: "claude" })) });
    if (terminal && folderOf(n)) acts.push({ key: "Mod+T", label: "open terminal here", mod: "ctrl-t", run: () => terminalAt(n) });
    acts.push({ key: "Mod+Enter", label: "go to", mod: "ctrl", run: () => select(n) });
    return acts;
  },
});

document.getElementById("palette-open").addEventListener("click", () => palette.open());

// ---------- data ----------

// ---------- focus lighting (see scene.js), remembered ----------

let focusLighting = true;
try {
  focusLighting = localStorage.getItem("xenonet.focusLighting") !== "off";
} catch {}
scene.setFocusLighting(focusLighting);
function setFocusLighting(on) {
  focusLighting = on;
  scene.setFocusLighting(on);
  try {
    localStorage.setItem("xenonet.focusLighting", on ? "on" : "off");
  } catch {}
  hud.toast(`Focus lighting ${on ? "on" : "off"}`);
}

// ---------- session scope (see scope.js) ----------

let fullGraph = null;
let fullIndex = new Map();
const scope = loadScope();
const revealed = new Set();
const scopeEl = document.getElementById("session-scope");
const scopeNote = document.getElementById("session-scope-note");
const backgroundToggle = document.querySelector("[data-scope-background]");

// Drawn whatever the scope: anything working, waiting on you or stuck, the
// selection, and what you've opened from search.
function keepIds() {
  const keep = new Set(revealed);
  if (selectedId) keep.add(selectedId);
  for (const s of lastLive?.sessions ?? []) if (s.status !== "idle" || s.needsYou || s.stuck) keep.add(s.id);
  for (const id of actorTargets()) keep.add(id);
  return keep;
}

function applyGraph(graph = fullGraph) {
  fullGraph = graph;
  fullIndex = new Map(graph.nodes.map((n) => [n.id, n]));
  decorateHosts(graph);
  hostFilters.setData(graph);
  identities.setData(graph);
  identities.decorate(graph.nodes);
  modals.refreshIdentities();
  const scoped = scopeGraph(graph, scope, { keep: keepIds() });
  hud.setData(scene.setData(scoped));
  renderScope(scoped.scope, scopeCounts(graph, scope));
  if (!replay.active) hud.refreshFilters();
}

function reveal(id) {
  revealed.add(id);
  applyGraph();
}

function setScope(patch) {
  Object.assign(scope, patch);
  saveScope(scope);
  revealed.clear();
  sfx("toggle");
  applyGraph();
}

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
function renderScope({ sessions, projects }, counts) {
  for (const b of scopeEl.querySelectorAll("[data-scope]")) {
    const c = counts[b.dataset.scope];
    b.classList.toggle("on", b.dataset.scope === scope.window);
    b.title = `${plural(c.projects, "project")} · ${plural(c.sessions, "session")}`;
  }
  backgroundToggle.classList.toggle("on", scope.background);
  const part = ({ shown, total }) => (shown < total ? `${shown} of ${total}` : `all ${total}`);
  scopeNote.textContent = `${part(projects)} projects · ${part(sessions)} sessions on the map`;
}

scopeEl.addEventListener("click", (e) => {
  const b = e.target.closest("[data-scope]");
  if (b && b.dataset.scope !== scope.window) setScope({ window: b.dataset.scope });
});
backgroundToggle.addEventListener("click", () => setScope({ background: !scope.background }));

// ---------- actors (actors.js) ----------
//
// The actors' list at the start and whenever it changes, and each thing an
// actor does, which sets off a reaction and runs light down its beam. What
// an actor acts on lately is drawn whatever the scope, so its beam lands.

const ACTOR_TARGET_MS = 2 * 3600e3;
function actorTargets() {
  const out = [];
  for (const a of lastActors ? asDrawn(lastActors).actors : []) for (const t of a.targets ?? []) if (Date.now() - (t.lastAt ?? 0) < ACTOR_TARGET_MS) out.push(t.id);
  return out;
}
// Which places actors act on the map couldn't draw (createTargetWatch).
const targetWatch = createTargetWatch();
const onMap = { known: (id) => fullIndex.has(id), drawn: (id) => !!scene.getNode(id) };
function applyActors(list) {
  if (!list?.actors) return;
  lastActors = list;
  // A place an actor just started acting on joins the map right away.
  if (targetWatch.wants(actorTargets(), onMap)) {
    applyGraph();
    targetWatch.drew(actorTargets(), onMap);
  }
  drawActors();
  hud.seedActors(asDrawn(list));
  alerts.setActors(list);
  actorsPanel?.refresh();
}

// You, as this page sees you (see youHere): drawn from your input here,
// and only drawn, never sent, with your record for where you've looked.
// Not drawn at all once you've turned Show me on the map off in the
// Actors panel.
const asYou = (list) => youHere(list, { quietMs: Date.now() - lastInputAt, hiddenMs: you.hiddenMs(), awayMs: AWAY_MS, record: you.record() });
const asDrawn = (list) => (you.shown() ? asYou(list) : { ...list, actors: list.actors.filter((a) => a.kind !== "human") });
function drawActors() {
  if (!lastActors) return;
  scene.setActors(asDrawn(lastActors));
  hud.refreshActor();
}
setInterval(drawActors, 15e3);
// The desktop window shown again, or hidden: you as you are now.
desktop?.onWindowState?.(() => drawActors());

// XENONET_ACTORS=off: the server keeps no actors and answers 503, so the
// layer draws nothing and costs nothing, and no request ever waits.
let actorsOff = false;
function actorsAreOff() {
  actorsOff = true;
  scene.actorLayer.setEnabled(false);
  actorRequests.setOff();
}

// The Actors panel (actors-panel.js), loaded the first time it opens.
async function openActors() {
  loadingActorsPanel ??= import("./actors-panel.js").then(({ createActorsPanel }) =>
    (actorsPanel = createActorsPanel({
      modals,
      sfx,
      toast: (m, b) => hud.toast(m, b),
      calls: windowCalls,
      requests: actorRequests,
      tokens: actorTokens,
      remote: remoteConnections,
      decide: decideRequest,
      you,
      desktop,
      list: () => (lastActors ? asYou(lastActors) : null),
      off: () => actorsOff,
      onShown: drawActors,
      select,
    })),
  );
  return (await loadingActorsPanel).open();
}

// Allow or Deny, from a request's card or the panel: the answer, said.
async function decideRequest(id, decision) {
  const asked = actorRequests.view().requests.find((r) => r.id === id);
  const res = await actorRequests.decide(id, decision);
  sfx(res.ok ? "success" : "error");
  const name = asked?.name ?? "It";
  hud.toast(res.ok ? (decision === "allow" ? `${name} is allowed. It shows as not heard from until it reports.` : `${name} is denied. Its token never works.`) : (res.error ?? "That didn't work"), !res.ok);
  return res;
}

let lastLive = null;
const stuckSeen = new Set();
function applyLive(snap) {
  lastLive = snap;
  // What waits on you on other machines is drawn whatever the hosts' cap;
  // the first live state switches the cap on, before anything is selected.
  hostFilters.setLive(snap);
  // A session that starts working or asks for you joins the map right away.
  if (snap.sessions.some((s) => (s.status !== "idle" || s.needsYou || s.stuck) && fullIndex.has(s.id) && !scene.getNode(s.id))) applyGraph();
  scene.setLive(snap);
  hud.setLive(snap);
  telemetry.update(snap, hud.index);
  alerts.update(snap, hud.index);
  const working = snap.sessions.filter((s) => s.status !== "idle").length;
  ambient.setIntensity(snap.tpm, working);
  for (const s of snap.sessions) {
    if (s.stuck && !stuckSeen.has(s.id)) hud.activity({ sessionId: s.id, kind: "stuck", detail: s.stuck.reasons.join(" · "), ts: Date.now() });
  }
  stuckSeen.clear();
  for (const s of snap.sessions) if (s.stuck) stuckSeen.add(s.id);
  if (working) {
    lastWorkAt = Date.now();
    scene.setRest(false);
  }
}

// The server may be restarting (in XenoNet Dev, an edit under server/ does
// that as the page reloads), so the first fetch tries again until it answers.
async function boot() {
  for (let wait = 500; ; wait = Math.min(wait * 2, 4000)) {
    try {
      return await Promise.all([fetch("/api/graph").then((r) => r.json()), fetch("/api/live").then((r) => r.json())]);
    } catch {
      await new Promise((r) => setTimeout(r, wait));
    }
  }
}
const [graph, live] = await boot();
applyGraph(graph);
applyLive(live);
fetch("/api/actors").then(async (r) => (r.status === 503 ? actorsAreOff() : applyActors(await r.json())), () => {});
actorRequests.refresh();
document.getElementById("boot").classList.add("done");
if (params.has("wallpaper")) setWallpaper(true);

const events = new EventSource("/api/events");
events.addEventListener("open", () => hud.setLink("live"));
events.addEventListener("error", () => hud.setLink("down"));
events.addEventListener("graph", (e) => applyGraph(JSON.parse(e.data)));
events.addEventListener("live", (e) => applyLive(JSON.parse(e.data)));
events.addEventListener("activity", (e) => {
  if (replay.active) return;
  const ev = JSON.parse(e.data);
  scene.activity(ev);
  hud.activity(ev);
  // From the session, where it is on the map.
  if (ev.kind === "tool") ambient.motif(scene.homeOf(ev.sessionId), { id: ev.sessionId });
  // A turn that ends by itself chimes; one you stopped doesn't.
  if (chimes(ev)) ambient.chime({ id: ev.sessionId });
});
events.addEventListener("tokens", (e) => !replay.active && scene.tokens(JSON.parse(e.data)));
events.addEventListener("actors", (e) => applyActors(JSON.parse(e.data)));
// What an actor does: it reacts on the layer, and it's a line in the feed.
// The said event XenoNet writes of an actor's message comes just after the
// message (see agent-message), which drew and listed it already: it only
// moves the actor's beams.
const saidEcho = createSaidEcho();
events.addEventListener("actor-event", (e) => {
  if (replay.active) return;
  const ev = JSON.parse(e.data);
  const echo = saidEcho.echo(ev);
  scene.actorEvent(ev, { quiet: echo });
  if (!echo && ev.kind !== "presence") ambient.cue("actor", { id: ev.actor });
  if (!echo) hud.activity({ actor: ev.actor, kind: ev.kind, detail: ev.detail, ts: ev.at });
});
// How many registrations wait for you; this window then reads who they
// are with its key.
events.addEventListener("actor-requests", (e) => actorRequests.heard(JSON.parse(e.data)));
// A remote actor read through its connection: light through its tunnel to
// what it read. Never in the feed; a read is not something it did here.
events.addEventListener("remote-read", (e) => {
  if (replay.active) return;
  const r = JSON.parse(e.data);
  scene.actorRead(r);
  ambient.cue("read", { id: r.actor });
});
// What's playing in Spotify changed.
events.addEventListener("spotify", (e) => spotifyChip.heard(JSON.parse(e.data)));
// A tunnel XenoNet runs changed; the panel reads how with the window's key.
events.addEventListener("assistant", () => actorsPanel?.tunnelsChanged());
events.addEventListener("pulse", (e) => !replay.active && scene.pulse(JSON.parse(e.data).nodeId));
events.addEventListener("doctor", (e) => alerts.setDoctor(JSON.parse(e.data)));
// How each other machine stands, between maps (hosts.js).
events.addEventListener("hosts", (e) => hostFilters.setHosts(JSON.parse(e.data)));
// The theme (core/themes.js): the server's choice wins, and a change to it
// (from the palette, another window, xenonet theme or an edit to a theme of
// your own) is drawn in place, without a reload.
events.addEventListener("theme", (e) => {
  syncTheme(JSON.parse(e.data));
  themesUi.load();
});
events.addEventListener("themes", () => themesUi.load());
fetch("/api/theme").then((r) => r.json()).then(syncTheme, () => {});
themesUi.load();
environmentsUi.load().catch(() => {});
events.addEventListener("environment", (e) => syncEnvironment(JSON.parse(e.data).id));
// The doctor's latest results (the server runs it a minute after it starts).
fetch("/api/doctor").then((r) => r.json()).then((r) => alerts.setDoctor(r), () => {});
// One agent messaging others through XenoNet's engine: a beam to each, and a
// line in the activity feed. An actor's message (from actor:<slug>) is
// drawn from the actor, as what else it does is, and listed as its own.
events.addEventListener("agent-message", (e) => {
  if (replay.active) return;
  const m = JSON.parse(e.data);
  const recipients = Array.isArray(m.to) ? m.to : [];
  // A host's message may come with no target (core/hosts/merge.js), which
  // reads as one to the agents it names.
  const target = typeof m.target === "string" ? m.target : "";
  const named = (id) => fullIndex.get(id)?.title ?? (isActorId(id) ? scene.actor(id)?.snapshot.name : null) ?? id;
  const to = target === "all" ? "everyone" : target.startsWith("repo:") ? `everyone in ${target.slice(5)}` : recipients[0] ? named(recipients[0]) : "another SI";
  const detail = `to ${to} · ${m.text ?? ""}`;
  ambient.cue("message", { id: m.from, to: recipients });
  if (isActorId(m.from)) {
    saidEcho.spoke(m.from);
    scene.actorMessage(m.from, recipients, m.ts);
    return hud.activity({ actor: m.from, kind: "agent-message", detail, ts: m.ts });
  }
  scene.message(m.from, recipients);
  hud.activity({ sessionId: m.from, kind: "agent-message", detail, ts: m.ts });
});

let hintTimer;
addEventListener("pointermove", () => {
  if (!wallpaperOn) return;
  document.body.classList.add("show-hint");
  clearTimeout(hintTimer);
  hintTimer = setTimeout(() => document.body.classList.remove("show-hint"), 2000);
});

// ---------- keys ----------

addEventListener("keydown", (e) => {
  if (e.target.matches?.("input, textarea, .xterm-helper-textarea") || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === "w" || e.key === "W") setWallpaper(!wallpaperOn);
  else if (e.key === "Escape" && wallpaperOn) setWallpaper(false);
});

// ---------- while you were away ----------
// Ten minutes without input (or with the window hidden) counts as away; the
// next interaction shows what happened, if anything did.

const AWAY_MS = 10 * 60e3;
let lastInputAt = Date.now();
let hiddenAt = null;
async function maybeWelcomeBack(since) {
  if (Date.now() - since < AWAY_MS || modals.isOpen || replay.active || wallpaperOn) return;
  const d = await fetch(`/api/digest?since=${since}`).then((r) => r.json());
  if (d.totals.toolCalls || d.totals.turns) modals.away(since);
}
for (const type of ["pointerdown", "keydown", "wheel"]) {
  addEventListener(
    type,
    () => {
      const idle = lastInputAt;
      lastInputAt = Date.now();
      if (Date.now() - idle >= AWAY_MS) maybeWelcomeBack(idle);
    },
    true,
  );
}
document.addEventListener("visibilitychange", () => {
  if (document.hidden) hiddenAt = Date.now();
  else if (hiddenAt) {
    const since = hiddenAt;
    hiddenAt = null;
    maybeWelcomeBack(since);
  }
});
if (params.has("away")) setTimeout(() => modals.away(Date.now() - Number(params.get("away") || 7200) * 1000), 800);

// ---------- day & night ----------

function hourNow() {
  if (params.has("hour")) return Number(params.get("hour"));
  const d = new Date();
  return d.getHours() + d.getMinutes() / 60;
}
function dayTick() {
  scene.setSky(hourNow());
  // End of day: after 6pm, once every agent has been idle 20 minutes, show the
  // day's log once and let the sky settle.
  const key = `xenonet.daylog.${new Date().toLocaleDateString("en-CA")}`;
  const idle = Date.now() - lastWorkAt > 20 * 60e3;
  if (hourNow() >= 18 && idle) {
    scene.setRest(true);
    let shown = false;
    try {
      shown = localStorage.getItem(key) === "1";
      if (!shown) localStorage.setItem(key, "1");
    } catch {}
    if (!shown && !modals.isOpen && !wallpaperOn) modals.dayLog();
  }
}
dayTick();
setInterval(dayTick, 60e3);
if (params.has("daylog")) setTimeout(() => modals.dayLog(), 800);

// ---------- dev hooks ----------

if (import.meta.env.DEV || desktop) {
  // How scripts, tests and AI agents drive XenoNet (see driver.js).
  const { createDriver } = await import("./driver.js");
  const driver = createDriver({
    scene, alerts, palette, modals, preview, replay, select, back, step, hud,
    commands: commandList,
    stats: { open: openStats, get data() { return statsPanel.data; } },
    rename: renameSession,
    film,
    ops,
    clearOps,
    identities,
    hosts: hostFilters,
    updates: updatesPanel,
    root: rootPanel,
    themes: themesUi,
    environments: environmentsUi,
    actorRequests,
    actorTokens,
    remoteConnections,
    sound: () => ({ state: ambient.state, mode: ambient.mode, heard: ambient.heard() }),
    spotify: () => spotifyChip.state,
    system: () => systemPanel.state,
    sheet: (id) => (id === undefined ? { current: sheets.current, narrow: sheets.narrow } : (sheets.show(id ?? ""), { current: sheets.current, narrow: sheets.narrow })),
    spend: () => ({ costs: telemetry.snapshot?.costs ?? null, openrouter: telemetry.spend?.openrouter ?? null, shown: !document.getElementById("spend").hidden, text: document.getElementById("spend").innerText.replace(/\s+/g, " ").trim(), gauge: document.querySelector("#gauges .gauge.or")?.innerText.replace(/\s+/g, " ").trim() ?? null }),
    graph: () => fullGraph,
    live: () => lastLive,
    selectedId: () => selectedId,
    wallpaper: () => wallpaperOn,
    terminal: () => terminal,
    errors: pageErrors,
  });
  window.__xenonet = {
    driver,
    ...(import.meta.env.DEV ? { scene, hud, preview, telemetry, ambient, modals, alerts, replay, terminal, select, setWallpaper, poster } : {}),
    get live() {
      return lastLive;
    },
    // Renders one frame and saves the canvas to .xenonet/<name>.png.
    async capture(name = "capture") {
      scene.graph.postProcessingComposer().render();
      const dataUrl = scene.graph.renderer().domElement.toDataURL("image/png");
      return (await post(`/api/dev/capture?name=${name}`, { dataUrl })).json();
    },
  };
}
