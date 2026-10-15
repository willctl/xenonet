// Drives XenoNet by meaning rather than by pixel, for scripts, tests and
// agents. This is the page's half of the control channel: the desktop app's
// main process calls it (electron/control.js) on behalf of
// core/control.js, the console MCP server (mcp/console.js) and
// the end-to-end tests. Available in installed Desktop and development builds.
//
// Actions wait for the view to settle (the layout cooled, camera moves and
// any gathering at rest) and return the state that follows, so a caller never
// sleeps for a fixed time. Everything returned is plain JSON.

import { themeState, THEME } from "./theme.js";
import { isActorId } from "./actors.js";
import { formOf } from "./actors-model.js";
import { tokenWords } from "./actor-requests.js";
import { environmentState } from "./environment.js";

export const VERSION = 1;

// A frame, or a tenth of a second if frames have stopped (a hidden tab).
const frame = () => new Promise((resolve) => (requestAnimationFrame(() => resolve()), setTimeout(resolve, 100)));

// An on-or-off argument, which a script step passes as text.
const yes = (v) => v !== false && !["false", "off", "no", "0"].includes(String(v).toLowerCase());

const PLACES = new Set(["project", "native", "zone"]);

// The panels a column shows, by their sheet's id (the inspector's is
// "selected"): those laid out now, not hidden.
const shownPanels = (column) => [...document.querySelectorAll(`${column} > .panel[data-sheet]`)].filter((p) => !p.hidden && getComputedStyle(p).display !== "none").map((p) => p.dataset.sheet);

// What's still moving: the scene (layout, camera, gathering, a change of
// theme; see motion in scene.js) and CSS transitions in flight, such as a
// panel opening or closing. Decorative CSS animations don't count, nor do
// live readouts gliding to each new reading (data-live), such as this
// computer's load: they never stop.
function motion(scene) {
  const m = scene.motion();
  const transitions = document.getAnimations().filter((a) => a instanceof CSSTransition && a.playState === "running" && !a.effect?.target?.closest?.("[data-live]")).length;
  // A change of theme counts from when it's asked for (see themeState).
  const theme = m.theme || themeState().pending;
  return { ...m, theme, transitions, settled: m.settled && !transitions && !theme };
}

export function createDriver(app) {
  const { scene, alerts, palette, modals, preview, replay } = app;

  const nodeOf = (id) => (isActorId(id) ? scene.actor(id) : null) ?? scene.getNode(id) ?? app.graph()?.nodes.find((n) => n.id === id) ?? null;
  const liveOf = (id) => app.live()?.sessions.find((s) => s.id === id) ?? null;

  // An actor (actors.js) as the layer draws it: who, whether it's there,
  // what it's doing and how XenoNet knows, its form and color in the theme,
  // and the nodes it beams into now.
  function actorBrief(rec) {
    const s = rec.snapshot;
    return { id: rec.id, kind: "actor", title: s.kind === "human" ? "You" : s.name, drawn: true, actorKind: s.kind, presence: s.presence, status: s.status ?? null, via: s.via ?? [], product: s.product?.id ?? null, form: formOf(s, THEME), color: `#${rec.color.getHexString()}`, beams: scene.actorLayer.targetsOf(rec.id) };
  }

  function brief(n) {
    if (!n) return null;
    if (n.kind === "actor") return actorBrief(n);
    const out = { id: n.id, kind: n.kind, title: n.title ?? n.name ?? n.id, drawn: !!scene.getNode(n.id) };
    if (n.kind === "session") {
      const home = nodeOf(scene.homeOf(n.id) ?? "");
      const live = liveOf(n.id);
      Object.assign(out, { app: n.app, project: home ? (home.name ?? home.title) : null, activity: n.activity, status: live?.status ?? "idle" });
      if (n.archived) out.archived = true;
      if (live?.needsYou) out.needsYou = live.needsYou.kind;
      if (n.ranAs && app.identities) out.ranAs = app.identities.label(n.ranAs.id);
      if (live?.stuck) out.stuck = live.stuck.reasons;
    } else if (PLACES.has(n.kind)) {
      out.active = !!n.active;
      if (n.repo?.dirty) out.uncommitted = n.repo.dirty;
    } else if (n.kind === "host") out.state = n.state;
    // From another machine (hosts.js): which, and whether it's live.
    if (n.host) Object.assign(out, { host: n.host, ...(n.stale ? { stale: true } : {}) });
    return out;
  }

  function alertBrief(it) {
    if (it.kind === "review") return { kind: "review", sessions: it.ids };
    if (it.kind === "collision") return { kind: "collision", sessions: [it.a, it.b], files: it.files };
    // A program asking to be an actor (actorRequests has the rest), or how
    // many ask, in a window without the key; an actor waiting on you.
    if (it.kind === "request") return { kind: "request", request: it.id, actor: it.request.actor, name: it.request.name, ...(it.request.gateway ? { gatewayToken: true } : {}) };
    if (it.kind === "requests") return { kind: "requests", count: it.count };
    if (it.kind === "actor-need") return { kind: "actor-need", actor: it.id, waits: it.sub, text: it.text };
    return { kind: it.kind, session: it.id, text: it.text };
  }

  function state() {
    const graph = app.graph();
    const live = app.live();
    const selected = app.selectedId();
    const count = (kinds) => graph?.nodes.filter((n) => kinds.includes(n.kind)).length ?? 0;
    return {
      version: VERSION,
      selection: selected ? brief(nodeOf(selected)) : null,
      gathered: scene.gathering(),
      identities: { filter: app.identities?.selected ?? [], masked: !!app.identities?.masked },
      overlays: { palette: palette.isOpen, modal: modals.isOpen, preview: preview.isOpen, rename: !!app.hud?.renaming, terminal: !!app.terminal()?.isOpen, wallpaper: app.wallpaper(), replay: replay.active, film: app.film && app.film.state !== "idle" ? app.film.state : false, switcher: !!app.themes?.switcher.isOpen, gallery: !!app.themes?.gallery.isOpen },
      alerts: alerts.items.map(alertBrief),
      counts: {
        nodes: graph?.nodes.length ?? 0,
        drawn: scene.nodes().length,
        projects: count(["project", "native", "zone"]),
        sessions: count(["session"]),
        working: live?.sessions.filter((s) => s.status !== "idle").length ?? 0,
        actors: scene.actors().length,
        beams: scene.actorLayer.beams().length,
        requests: app.actorRequests?.view().waiting ?? 0,
      },
      motion: motion(scene),
      still: scene.still,
      view: scene.view(),
      viewport: { width: innerWidth, height: innerHeight, dpr: devicePixelRatio },
      // The narrow screen's sheet of panels: which is open, if the screen is narrow.
      sheet: app.sheet?.() ?? null,
      // The panels each column shows now, by their sheet's id: on a wide
      // screen, the right column is the inspector alone while something is
      // selected.
      panels: { left: shownPanels(".hud-left"), right: shownPanels(".hud-right") },
      // The theme drawn, the one in use (they differ while previewing), and
      // how many changes of theme have been drawn.
      theme: themeState(),
      environment: app.environments?.state() ?? environmentState(),
      mcp: app.hud?.mcp?.state() ?? null,
      // The switcher's list and the theme highlighted in it, and the
      // gallery's cards, the one looked at and a theme just made; null
      // while closed.
      themes: { switcher: app.themes?.switcher.state() ?? null, gallery: app.themes?.gallery.state() ?? null },
      errors: app.errors.length,
      // Other machines on the map and how each stands (hosts.js).
      hosts: (graph?.hosts ?? []).map((x) => ({ id: x.id, state: x.state })),
    };
  }

  // Resolves once the view has stayed settled for a few frames.
  async function settled({ timeout = 15000 } = {}) {
    const t0 = performance.now();
    for (let calm = 0; ; ) {
      await frame();
      const m = motion(scene);
      calm = m.settled ? calm + 1 : 0;
      if (calm >= 3) return { waitedMs: Math.round(performance.now() - t0) };
      if (performance.now() - t0 > timeout) {
        const what = [m.layout && "the layout", m.camera && "the camera", m.moves && `${m.moves} gathering nodes`, m.theme && "a change of theme", m.transitions && `${m.transitions} CSS transitions`].filter(Boolean).join(", ");
        throw new Error(`still moving after ${timeout} ms: ${what}`);
      }
    }
  }

  // Runs an action, waits for the view to settle unless asked not to, and
  // returns the state that follows.
  async function act(run, { settle = true, timeout } = {}) {
    await run();
    if (settle) await settled({ timeout });
    else await frame();
    return state();
  }

  function known(id) {
    if (id != null && !nodeOf(id)) throw new Error(`XenoNet has no node "${id}"; find lists the ids`);
    return id;
  }

  return {
    version: VERSION,
    async terminal(action = "list", options = {}) {
      const terminal = app.terminal();
      if (!terminal) throw new Error("Terminals need XenoNet Desktop.");
      if (action === "open") await terminal.show({ ...options, fresh: true });
      else if (action === "read") return { text: terminal.bufferText(options.lines ?? 100), tabs: terminal.debug };
      else if (action === "write") terminal.write(options.id, options.text);
      else if (action === "close") terminal.close(options.id);
      else if (action !== "list") throw new Error("Use list, open, read, write or close.");
      return { tabs: terminal.debug };
    },
    state,
    settled,

    // Nodes whose id, title or name contain every word of `query`, drawn
    // ones first, then the most recent. `kind` narrows it: session, project,
    // native (a Codex project), zone (a folder outside the projects folder),
    // resource, wiki, page, core, host (another machine's XenoNet, with
    // XENONET_HOSTS=1) or actor (actors beside the map, by id, name and
    // product, after the nodes when not narrowed, so words that found a node
    // still find it first). A node from another machine has an id that
    // starts with @<host>: and says which host it's on.
    find({ query = "", kind = null, limit = 20 } = {}) {
      const words = String(query).toLowerCase().split(/\s+/).filter(Boolean);
      const when = (n) => n.updatedAt ?? n.lastActivity ?? n.snapshot?.lastSeen ?? 0;
      const actors = !kind || kind === "actor" ? scene.actors().filter((a) => words.every((w) => `${a.id} ${a.snapshot.kind === "human" ? "you" : a.snapshot.name} ${a.snapshot.product?.id ?? ""}`.toLowerCase().includes(w))) : [];
      const all = kind === "actor" ? [] : (app.graph()?.nodes ?? []).filter((n) => {
        if (kind && n.kind !== kind) return false;
        const hay = `${n.id} ${n.title ?? ""} ${n.name ?? ""}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      });
      all.sort((a, b) => !!scene.getNode(b.id) - !!scene.getNode(a.id) || when(b) - when(a));
      const found = [...all, ...actors.sort((a, b) => when(b) - when(a))];
      return { total: found.length, nodes: found.slice(0, limit).map((n) => ({ ...brief(n), screen: scene.screenPos(n.id) })) };
    },

    // The actors beside the map (actors.js), each as find has it, with
    // the beams drawn now and the layer's own time for its last frame.
    // Tunnels: the gates drawn for remote actors that reach XenoNet through
    // one, how open each is and where.
    actors() {
      const list = scene.actors().map((a) => ({ ...actorBrief(a), screen: scene.screenPos(a.id), selected: app.selectedId() === a.id }));
      return { count: list.length, beams: scene.actorLayer.beams(), tunnels: scene.actorLayer.tunnels(), layer: scene.actorLayer.stats(), actors: list };
    },

    // The programs waiting for you to allow them as actors, as the
    // attention strip shows them (actor-requests.js): what each asks, who
    // asked, whether its card is up, and whether its Allow is ready for a
    // click yet (it waits a moment after the card shows). Read-only:
    // allowing is yours, so there's no driver method for it, and nothing
    // here points at its button. `canDecide` is false in a window without
    // the key, which knows only how many wait.
    actorRequests() {
      const v = app.actorRequests?.view() ?? { requests: [], gatewayAsks: [], waiting: 0, canDecide: false, off: false };
      const requests = v.requests.map((r) => ({
        id: r.id,
        actor: r.actor,
        name: r.name,
        product: r.product?.id ?? null,
        capabilities: r.capabilities ?? [],
        heartbeatMs: r.heartbeatMs ?? null,
        asker: r.asker ?? null,
        joins: !!r.joins,
        at: r.at,
        expiresAt: r.expiresAt,
        card: !!document.querySelector(`#alerts [data-al-request="${CSS.escape(r.id)}"]`),
        armed: app.actorRequests.armed(r.id),
      }));
      // Sessions' asks for an actor's gateway token, which Allow makes: who
      // asked and when it lapses, never a token, and no method to allow one.
      const gatewayAsks = v.gatewayAsks.map((g) => ({
        id: g.id,
        actor: g.actor,
        name: g.name,
        replaces: !!g.replaces,
        asker: g.asker ?? null,
        at: g.at,
        expiresAt: g.expiresAt,
        card: !!document.querySelector(`#alerts [data-al-request="${CSS.escape(g.id)}"]`),
        armed: app.actorRequests.armed(g.id),
      }));
      return { canDecide: v.canDecide, off: v.off, waiting: v.waiting, requests, gatewayAsks };
    },

    // How each registered actor's token stands, as the Actors panel says it
    // (actor-requests.js), read fresh with the window's key: whether it
    // works, expired or there's none, when it expires, when it was last
    // used (null if never), the words for it, and what the open panel's
    // row says (null while it isn't showing). Never a token: the page has
    // none. `canSee` is false in a window without the key.
    async actorTokens() {
      const v = (await app.actorTokens?.refresh()) ?? { canSee: false, off: false, tokens: [] };
      const now = Date.now();
      const rowOf = (id) => document.querySelector(`[data-ac-actors] > li[data-key="${CSS.escape(id)}"] [data-ac-token-state]`)?.textContent ?? null;
      const tokens = v.tokens.map((t) => ({ actor: t.actor, state: t.state, expiresAt: t.expiresAt ?? null, usedAt: t.usedAt ?? null, words: tokenWords(t, now), row: rowOf(t.actor) }));
      return { canSee: v.canSee, off: v.off, tokens };
    },

    // Remote actors' connections, as the Actors panel shows them
    // (remote-panel.js), read fresh with the window's key: whether cloud
    // access is on, and each connection's actor, state, projects (how
    // many), whether labels go, its data mode, expiry and whether an
    // adapter is connected, with what the open panel's row says. Read-only:
    // pairing, widening, revoking and turning access on or off are yours,
    // so there's no driver method for any of them. `canSee` is false in a
    // window without the key. `tunnels`: whether tunnel-client is here, and
    // each tunnel XenoNet can run, with its state; never a key.
    async remoteConnections() {
      const v = (await app.remoteConnections?.refresh()) ?? { canSee: false, off: false, disabled: false, connections: [] };
      const rowOf = (grant) => document.querySelector(`[data-ac-remote] > li[data-key="${CSS.escape(grant)}"] .ac-live`)?.textContent.trim() ?? null;
      return {
        canSee: v.canSee,
        off: v.off,
        disabled: v.disabled,
        connections: v.connections.map((x) => ({ grant: x.grant_id, actor: x.actor_id, label: x.label, state: x.state, projects: x.projects.length, labels: x.labels, dataMode: x.data_mode, expiresAt: x.expires_at, bridge: x.bridge, transport: x.transport_kind, row: rowOf(x.grant_id) })),
        tunnels: v.tunnels ? { client: v.tunnels.client, store: v.tunnels.store, tunnels: v.tunnels.tunnels.map((t) => ({ grant: t.grant, tunnelId: t.tunnel_id, key: t.key, autostart: t.autostart, state: t.state, why: t.why, profile: t.profile?.name ?? null })) } : null,
      };
    },

    // About what today's work would cost, and what OpenRouter's key has
    // spent, as Token use shows them (telemetry.js). Read-only.
    async spend(action = "read") {
      if (action === "refresh") await app.refreshSpend?.();
      else if (action !== "read") throw new Error(`Unknown spend action: ${action}`);
      return app.spend?.() ?? null;
    },

    // The narrow screen's sheet of panels (sheets.js): with no argument,
    // which is open and whether the screen is narrow; with a tab's id
    // (sessions, activity, filters, load, tokens, link, legend), opens it;
    // with "" or null, closes it.
    sheet: (id, opts) => (id === undefined ? app.sheet?.() : act(() => app.sheet?.(id ?? ""), opts)),

    // This computer's load as the system panel shows it (system-panel.js):
    // whether it shows, the CPU, memory and GPU, where the GPU's figures
    // come from, how much history it has, and its text. Read-only.
    system() {
      return app.system?.() ?? null;
    },

    // Spotify as the chip shows it (spotify-ui.js): whether it's connected,
    // what's playing, whether XenoNet's music has stepped aside, and the
    // chip's text. Read-only: connecting and playing are yours.
    spotify() {
      return app.spotify?.() ?? null;
    },

    // Sounds placed on the map lately (ambient.js, sound-space.js): each
    // one's kind, where (by id), how loud and from which side, and whether
    // it played, noted while sound waits for a click too; and the mode.
    sound() {
      return app.sound?.() ?? null;
    },

    // The activity feed as it shows now, newest first: each line's kind, who
    // it's about (a session's id, or an actor's), its title and its words.
    feed({ limit = 20 } = {}) {
      const lines = [...document.querySelectorAll("#feed > li")].slice(0, Math.max(1, Number(limit) || 20));
      // The title's words, without the mark an actor's line carries.
      const title = (el) => [...(el?.childNodes ?? [])].filter((n) => !n.classList?.contains("ev-actor")).map((n) => n.textContent).join("").trim();
      return {
        ...app.hud?.feedState(),
        lines: lines.map((li) => ({
          kind: li.dataset.kind ?? [...li.classList].find((c) => c !== "ev" && c !== "fresh") ?? null,
          ...(li.dataset.actor ? { actor: li.dataset.actor } : li.dataset.id ? { session: li.dataset.id } : {}),
          title: title(li.querySelector(".ev-title")),
          detail: li.querySelector(".ev-detail")?.textContent.trim() ?? "",
          at: Number(li.dataset.ts) || null,
        })),
      };
    },

    // Select a node by id (null clears the selection). A project, Codex
    // project or folder gathers its sessions around it.
    select: (id, opts) => act(() => app.select(known(id) ?? null), opts),
    // Esc: from a gathered session to its project, then out.
    back: (opts) => act(() => app.back(), opts),
    // ↓ / ↑: the next or previous gathered node.
    step: (by = 1, opts) => act(() => app.step(by), opts),
    // R: frame the whole map.
    reset: (opts) => act(() => scene.reset(), opts),
    // The hover highlight and label, as if the pointer were over the node.
    hover: (id, opts) => act(() => scene.hover(id ? nodeOf(known(id)) : null), { settle: false, ...opts }),

    async updates(action = "status", value) {
      await app.updates.open();
      return action === "status" ? app.updates.state : app.updates.action(action, value);
    },
    commands: () => app.commands.map(({ id, label, key }) => ({ id, label, ...(key ? { key } : {}) })),
    async refusals(action = "open", value, opts) {
      const st = await act(() => app.refusals.action(action, value), opts);
      return { ...st, refusals: app.refusals.state };
    },
    async integrations(action = "open", value, opts) {
      // Review and file only browse cached text; retrieval and export stay
      // in the owner's window like sandbox launch and deletion.
      const st = await act(() => app.integrations.action(action, value), opts);
      return { ...st, integrations: app.integrations.state };
    },
    // Runs a command from the palette (⌘K / Ctrl+K) by id; commands() lists them.
    command(id, opts) {
      const cmd = app.commands.find((c) => c.id === id);
      if (!cmd) throw new Error(`no command "${id}"; commands lists them`);
      return act(() => cmd.run(), opts);
    },
    // A session's stats (stats.js): opens the panel, waits for its numbers
    // and returns them in brief with the state.
    async stats(id, opts) {
      const n = nodeOf(known(id));
      if (n.kind !== "session") throw new Error(`"${id}" isn't a session; stats are a session's`);
      const st = await act(() => app.stats.open(n), opts);
      const d = app.stats.data;
      return { ...st, stats: d ? { cost: d.totals.cost, tokens: d.totals.tokens, calls: d.totals.calls, turns: d.turns.length, models: d.models.map((m) => m.model), tools: d.tools.length } : null };
    },
    // Renames a session, as typing a name over its title in the inspector
    // would: the name is saved in its own app (core/rename.js), so Claude
    // Code, Claude desktop or Codex shows it too. Returns the state, with
    // `renamed`: the name as saved, where it went and anything to know.
    async rename(id, title, opts) {
      const n = nodeOf(known(id));
      if (n.kind !== "session") throw new Error(`"${id}" isn't a session; only sessions are renamed`);
      let done = null;
      const st = await act(async () => (done = await app.rename(n.id, String(title ?? ""))), opts);
      if (!done.ok) throw new Error(done.error);
      return { ...st, renamed: { title: done.title, saved: done.saved, note: done.note, demo: done.demo } };
    },
    // What you've done in the console, as steps (oplog.js), oldest first;
    // xenonet script record saves them, xenonet script run plays them again.
    ops: () => app.ops(),
    clearOps: () => (app.clearOps(), { cleared: true }),
    // A film of the day (film.js): starts it and returns at once; state()
    // says how far it has got (overlays.film: title, playing, end, or false).
    film({ seconds = 45 } = {}) {
      if (app.film.state !== "idle") throw new Error("a film is already playing");
      app.film.play({ seconds }).catch((err) => app.errors.push({ message: `film: ${err.message}`, source: "film", ts: Date.now() }));
      return { started: true, seconds };
    },
    stopFilm: (opts) => act(() => app.film.stop(), opts),
    // Opens the palette with `text` typed in, or closes it with null.
    palette: (text = "", opts) => act(() => (text == null ? palette.close() : palette.open(text)), { settle: false, ...opts }),

    // The identity chips under Filters, as the page shows them (masked while
    // identities are hidden), keyed for filterIdentities.
    identities() {
      const ids = app.identities;
      if (!ids) return { masked: false, filter: [], chips: [] };
      const chips = ids.groups.filter((g) => !g.hidden).map((g) => ({ key: g.key, label: ids.groupLabel(g), kinds: [...g.kinds], named: !!g.name, ...ids.reachOf(g.key) }));
      return { masked: ids.masked, filter: ids.selected, shownSessions: ids.shownSessions, chips };
    },
    // Shows only these identities' sessions and projects; [] shows all.
    filterIdentities: (keys = [], opts) => act(() => app.identities.setSelected(Array.isArray(keys) ? keys : [keys]), opts),
    // Hides identities ("GitHub A", "Claude B"…) or shows them.
    maskIdentities: (on = true, opts) => act(() => app.identities.setMasked(!!on), opts),

    // The projects folder XenoNet reads (core/workspace.js): its path, where
    // that comes from (source: env for XENONET_PROJECTS_DIR, chosen or
    // default) and its last scan.
    root: () => app.root.get(),
    // What XenoNet would find in a folder (a full path, or one starting with
    // ~), without reading it: why it can't, or how many projects and groups.
    checkRoot: (path) => app.root.check(path),
    // Reads another folder, as Use this folder does (null: back to the
    // default), and returns once the map shows it and the view has settled,
    // with the folder as `root`. The layout, rules and Codex's picks stay
    // with each folder, so going back brings them back.
    async setRoot(path, opts) {
      let root = null;
      const st = await act(async () => {
        root = await app.root.use(path ?? null);
        // The new map comes over the event stream, a moment after the answer.
        const shows = () => app.graph()?.nodes.find((n) => n.id === "core")?.path === root.path;
        for (const end = performance.now() + 15000; !shows(); await frame()) if (performance.now() > end) throw new Error(`the map didn't show ${root.path} within 15 s`);
      }, opts);
      return { ...st, root };
    },

    // Holds every ambient animation still, CSS included, for screenshots.
    async still(on = true) {
      scene.setStill(on);
      document.body.classList.toggle("still", !!on);
      await frame();
      await frame();
      return scene.still;
    },

    // The themes (themes-ui.js): each one's id, name and whether it's
    // built in, and the one in use, fresh from the server.
    async themes() {
      await app.themes.load();
      return { current: app.themes.current, drawn: themeState().id, preview: themeState().preview, themes: app.themes.list.map(({ id, name, about, builtIn, replacesBuiltIn }) => ({ id, name, about, builtIn, ...(replacesBuiltIn ? { replacesBuiltIn } : {}) })) };
    },
    async mcp(id, opts) {
      const node = nodeOf(known(id));
      if (!node || !["resource", "wiki"].includes(node.kind)) throw new Error("choose an MCP shared system or Agent Wiki");
      return act(async () => { await app.select(node.id); await app.hud.mcp.open(node); }, opts);
    },
    mcpTab: (tab, opts) => act(() => app.hud.mcp.tab(tab), opts),
    mcpSearch: (query, opts) => act(() => app.hud.mcp.search(query), opts),
    environments: async () => { await app.environments.load(); return { current: environmentState().inUse, environments: app.environments.list }; },
    environment: (id, opts) => act(() => app.environments.set(id), opts),
    previewEnvironment: (id, opts) => act(() => app.environments.preview(id), opts),
    keepEnvironment: (opts) => act(() => app.environments.keep(), opts),
    revertEnvironment: (opts) => act(() => app.environments.revert(), opts),
    environmentPicker: (opts) => act(() => app.environments.open(), opts),
    // Uses a theme in every window, as Theme: in the palette does, and
    // returns once it's drawn and the view has settled. No reload.
    theme: (id, opts) => act(() => app.themes.set(id), opts),
    // Shows a theme in this window alone, to look at; keepTheme makes it
    // the one in use, revertTheme goes back.
    previewTheme: (id, opts) => act(() => app.themes.preview(id), opts),
    keepTheme: (opts) => act(() => app.themes.keep(), opts),
    revertTheme: (opts) => act(() => app.themes.revert(), opts),
    // "Theme…" in the palette: opens the switcher, or with false closes it
    // as Esc does, putting the theme in use back.
    themeSwitcher: (open = true, opts) => act(() => (yes(open) ? app.themes.switcher.show() : app.themes.switcher.cancel()), opts),
    // ↓ (by 1) or ↑ (by -1) in the switcher, opening it first: shows the
    // theme it lands on in this window alone, and returns once it's drawn.
    // keepTheme then keeps it, as Enter does; revertTheme puts the old one
    // back, as Esc does.
    async stepTheme(by = 1, opts) {
      if (!app.themes.switcher.isOpen) await app.themes.switcher.show();
      return act(() => app.themes.switcher.step(Number(by) || 1), opts);
    },
    // The gallery: opens it, looking at theme `id` if given, or with false
    // closes it, putting the theme in use back.
    themeGallery: (open = true, opts) => act(() => (!yes(open) ? app.themes.gallery.hide() : app.themes.gallery.show(typeof open === "string" && open !== "true" && open !== "on" ? open : undefined)), opts),
    // Looks at a card in the gallery (opening it), as pointing at it does:
    // the theme is drawn in this window alone.
    focusTheme(id, opts) {
      if (!app.themes.list.some((t) => t.id === id)) throw new Error(`there's no theme called ${id}; themes lists them`);
      return act(() => (app.themes.gallery.isOpen ? app.themes.gallery.focus(id) : app.themes.gallery.show(id)), opts);
    },
    // "Make your own" in the gallery (opening it): a copy of the theme
    // looked at, saved as a file of yours, which is then the one in use.
    async makeTheme(opts) {
      if (!app.themes.gallery.isOpen) await app.themes.gallery.show();
      let made = null;
      const st = await act(async () => (made = await app.themes.gallery.make()), opts);
      if (!made) throw new Error("couldn't make a theme of your own; logs says why");
      return { ...st, made };
    },

    // Other machines on the map (hosts.js, with XENONET_HOSTS=1): each host
    // with how it stands, its node and where that is on screen, its
    // constellation, and the machines' chips under Filters with the ones
    // picked; the bridges between checkouts of one repo, drawn or not.
    hosts() {
      const graph = app.graph();
      const drawn = scene.hosts();
      const node = (id) => nodeOf(`@${id}:core`);
      const list = (graph?.hosts ?? []).map((x) => {
        const n = node(x.id);
        const c = drawn.constellations.find((k) => k.host === x.id);
        return { ...x, node: n ? n.id : null, drawn: !!scene.getNode(`@${x.id}:core`), screen: n ? scene.screenPos(n.id) : null, stale: !!n?.stale, constellation: c ? { at: c.at, radius: c.radius, nodes: c.nodes } : null };
      });
      return { self: graph?.self ?? null, hosts: list, bridges: drawn.bridges, filter: app.hosts?.selected ?? [], chips: app.hosts?.chips() ?? [] };
    },
    // Shows only these machines' work: host ids, and "@here" (or this
    // XenoNet's own id) for this machine; [] shows every machine's.
    filterHosts: (keys = [], opts) => act(() => app.hosts.setSelected(Array.isArray(keys) ? keys : [keys]), opts),

    // Where a node is on screen, in CSS pixels; null if it isn't drawn.
    screenPos: (id) => scene.screenPos(known(id)),
    errors: () => app.errors.slice(),
  };
}
