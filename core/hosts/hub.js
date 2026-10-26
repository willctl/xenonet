// The console's side of hosts (docs/design/hosts.md), with XENONET_HOSTS=1:
// a link to each host in hosts.json, each host's last snapshot (kept in
// the cache folder, so an offline host is still on the map after a
// restart), and the merged view the server sends in place of this
// machine's own: the map at most once a second, the live state at most
// every half second, and the hosts' events with their ids qualified.
//
// Nothing a host sends changes anything here. A link only reads, merge.js
// qualifies every id, and hosts stay read-only through Wave 2 (the
// owner's call): no dismissing, opening or messaging across machines.
import { EventEmitter } from "node:events";
import { systemClock } from "../blocks/clock.js";
import { tilde } from "../paths.js";
import { hostEntry } from "./config.js";
import { HostLink } from "./link.js";
import { mergeGraph, mergeLive, mergeSummary, qualifyEvent, qualifyEngine, eventNote, cleanOnce, DEFAULT_WINDOWS } from "./merge.js";
import { readCache, writeCache, removeCache } from "./cache.js";

const GRAPH_EVERY_MS = 1000;
const LIVE_EVERY_MS = 500;
const CACHE_EVERY_MS = 30e3;
const TEST_MS = 30e3;

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);
const fail = (message, status) => Object.assign(new Error(message), { status });

// `hosts`: the HostsConfig (hosts.json). `local`: this XenoNet's own map,
// live state, layout, identities (your names and hidden ones), SSH aliases,
// and a way to forget layout positions under a prefix. `demo`: a demo
// XenoNet shows demo hosts only, so nothing real reaches its screenshots.
// `log`: the service's (core/log.js), for what's left out of a host's events.
export function createHub({ hosts, cacheDir, root, home = null, demo = false, windows = DEFAULT_WINDOWS, local, clock = systemClock, linkOptions = {}, log = null }) {
  const hub = new EventEmitter();
  const runtimes = new Map();
  const timers = { graph: null, live: null };
  const last = { graph: 0, live: 0 };
  const shown = (p) => tilde(p, home);

  const infoOf = (rt) => {
    const info = { ...rt.link.info(), cached: rt.cached };
    if (rt.cached && info.state !== "live") info.hello ??= rt.cachedHello;
    if (rt.skewMs) info.offsetMs = (info.offsetMs ?? 0) + rt.skewMs;
    return info;
  };
  const forMerge = () => [...runtimes.values()].map((rt) => ({ info: infoOf(rt), snapshot: rt.snapshot, clean: rt.clean }));

  // ---------- what goes out ----------

  function graph(localGraph) {
    if (!runtimes.size) return localGraph;
    const ids = local.identities?.() ?? {};
    return mergeGraph(localGraph, forMerge(), { layout: local.layout?.() ?? {}, now: clock.now(), windows, self: hosts.self().id, names: ids.names ?? {}, hidden: ids.hidden ?? {}, aliases: local.aliases?.() ?? new Map() });
  }

  const live = (localLive) => (runtimes.size ? mergeLive(localLive, forMerge()) : localLive);
  // The tray's and the menu bar's summary, with live hosts' alerts in it.
  const summary = (localSummary) => (runtimes.size ? mergeSummary(localSummary, forMerge()) : localSummary);

  // At most once per `every`, the last change always going out.
  function soon(kind, every, send) {
    if (timers[kind]) return;
    const wait = Math.max(0, last[kind] + every - clock.now());
    timers[kind] = clock.setTimeout(() => {
      timers[kind] = null;
      last[kind] = clock.now();
      send();
    }, wait);
    timers[kind]?.unref?.();
  }
  const graphSoon = () =>
    soon("graph", GRAPH_EVERY_MS, () => {
      const g = local.graph();
      if (g) hub.emit("graph", graph(g));
    });
  const liveSoon = () =>
    soon("live", LIVE_EVERY_MS, () => {
      const l = local.live();
      if (l) hub.emit("live", live(l));
    });
  // The page hears how each host stands, without how it's reached: the ssh
  // target's user and machine names can hold a person's name, and the
  // page's screenshots shouldn't. GET /api/hosts has them, for the command.
  const forPage = () => list().map(({ target, key, command, counts, lastDrop, ...h }) => h);
  const hostsChanged = () => {
    hub.emit("hosts", forPage());
    graphSoon();
    liveSoon();
  };

  // ---------- the cache ----------

  // A host's last snapshot is kept under its key, in one file: rt.cacheKey
  // is the key it was last kept or found under. A host that comes back
  // with another key (a demo world made afresh, a machine set up again)
  // has its snapshot kept under the new one, and the old file goes, since
  // nothing reads it again.
  function save(rt) {
    clock.clearTimeout(rt.cacheTimer);
    rt.cacheTimer = null;
    const key = rt.link.entry.key;
    if (!rt.snapshot || rt.simulated || !key) return;
    rt.lastCacheAt = clock.now();
    if (!writeCache(cacheDir, key, { id: rt.link.entry.id, savedAt: rt.link.lastHeard ?? clock.now(), hello: rt.link.hello, ...rt.snapshot })) return;
    if (rt.cacheKey !== key) dropCache(rt, rt.cacheKey);
    rt.cacheKey = key;
  }
  // Forgetting a host takes every file it has: under the key it has now,
  // and the one it was kept under before, if it came back with another. A
  // simulated host keeps none, and a file another host's link uses too
  // (one XenoNet added twice, under two ids) stays for that one.
  function forgetCache(rt) {
    if (rt.simulated) return;
    for (const key of new Set([rt.link.entry.key, rt.cacheKey])) dropCache(rt, key);
    rt.cacheKey = null;
  }
  function dropCache(rt, key) {
    if (!key || [...runtimes.values()].some((o) => o !== rt && !o.simulated && (o.link.entry.key === key || o.cacheKey === key))) return;
    removeCache(cacheDir, key);
  }
  function saveSoon(rt) {
    if (rt.cacheTimer || rt.simulated) return;
    rt.cacheTimer = clock.setTimeout(() => save(rt), Math.max(0, (rt.lastCacheAt ?? -Infinity) + CACHE_EVERY_MS - clock.now()));
    rt.cacheTimer?.unref?.();
  }

  // ---------- each host ----------

  function start(entry, { simulated = false } = {}) {
    const cached = entry.key ? readCache(cacheDir, entry.key) : null;
    const link = new HostLink({ entry, sshConfig: hosts.sshConfig, root, demoOnly: demo, clock, tilde: shown, ...linkOptions });
    const rt = { link, simulated, clean: cleanOnce(), snapshot: null, cached: false, cachedHello: null, doctor: null, engine: null, system: null, lastCacheAt: null, cacheTimer: null, cacheKey: null, skewMs: 0 };
    if (cached) {
      Object.assign(rt, { snapshot: { graph: cached.graph, live: cached.live, engine: cached.engine ?? null }, cached: true, cachedHello: cached.hello ?? null, cacheKey: entry.key });
      link.lastHeard = cached.savedAt;
    }
    link.on("state", hostsChanged);
    link.on("learn", ({ key, name }) => {
      if (simulated) return;
      try {
        hosts.learn(entry.id, { key, name });
      } catch (err) {
        // hosts.json can't be written (a newer XenoNet's, say): the link
        // keeps what it learned until XenoNet quits.
        console.error(`[xenonet] couldn't keep ${entry.id}'s key: ${err.message}`);
      }
    });
    link.on("snapshot", (s) => {
      rt.snapshot = { graph: s.graph, live: s.live, engine: s.engine };
      rt.cached = false;
      saveSoon(rt);
      hostsChanged();
    });
    link.on("event", ({ name, data }) => {
      if (name === "graph" && Array.isArray(data.nodes)) {
        rt.snapshot = { ...rt.snapshot, graph: data };
        saveSoon(rt);
        return graphSoon();
      }
      if (name === "live") {
        rt.snapshot = { ...rt.snapshot, live: data };
        saveSoon(rt);
        return liveSoon();
      }
      // A host's own doctor feeds its state here, never this XenoNet's.
      if (name === "doctor") {
        const summary = isObject(data.summary) ? Object.fromEntries(["ok", "info", "warn", "fail", "skip"].map((k) => [k, Number.isInteger(data.summary[k]) && data.summary[k] >= 0 ? data.summary[k] : 0])) : null;
        rt.doctor = { at: Number.isFinite(data.at) ? data.at : null, summary, problems: (Array.isArray(data.problems) ? data.problems : []).filter(isObject).slice(0, 20).map((p) => ({ id: String(p.id ?? "").slice(0, 60), title: String(p.title ?? "").slice(0, 120), status: p.status === "fail" ? "fail" : "warn", detail: String(p.detail ?? "").slice(0, 300) })) };
        return hub.emit("hosts", forPage());
      }
      // A host's load, the latest kept for GET /api/system?hosts=1.
      if (name === "system") {
        const q = qualifyEvent(name, data, infoOf(rt));
        if (q) rt.system = q;
        return;
      }
      const q = qualifyEvent(name, data, infoOf(rt));
      if (q) return hub.emit(name, q);
      const note = eventNote(name, data, infoOf(rt));
      if (note) log?.info("hosts.unknown-app", note);
    });
    runtimes.set(entry.id, rt);
    link.start();
    return rt;
  }

  // Stops a host's link, with its last snapshot kept, unless `keep` is
  // false: it's being forgotten.
  function stopOne(rt, { keep = true } = {}) {
    rt.link.removeAllListeners();
    rt.link.stop();
    if (keep && rt.cacheTimer) save(rt);
    clock.clearTimeout(rt.cacheTimer);
    rt.cacheTimer = null;
  }

  // How a host stands, for GET /api/hosts and the doctor.
  function view(rt) {
    const info = infoOf(rt);
    const e = rt.link.entry;
    const nodes = Array.isArray(rt.snapshot?.graph?.nodes) ? rt.snapshot.graph.nodes : [];
    return {
      id: e.id,
      state: info.state,
      since: info.since,
      lastHeard: info.lastHeard,
      offsetMs: info.offsetMs,
      rttMs: info.rttMs,
      version: info.hello?.version ?? null,
      platform: info.hello?.platform ?? null,
      demo: info.hello?.demo ?? null,
      calls: info.hello?.id && info.hello.id !== e.id ? info.hello.id : (e.name ?? null),
      error: info.error,
      nextTryAt: info.nextTryAt,
      cached: rt.cached,
      simulated: rt.simulated,
      nodes: nodes.filter((n) => n?.id !== "core").length,
      sessions: nodes.filter((n) => n?.kind === "session").length,
      transport: e.transport,
      target: info.target,
      port: e.port,
      command: e.command,
      sessionsSince: e.since,
      detail: e.detail,
      key: e.key,
      addedAt: e.addedAt,
      counts: info.counts,
      lastDrop: info.lastDrop,
      doctor: rt.doctor,
    };
  }

  function list() {
    return [...runtimes.values()].map(view);
  }

  function need(id) {
    const rt = runtimes.get(id);
    if (!rt) throw fail(`there's no host called ${id}`, 404);
    return rt;
  }

  // ---------- what you can do ----------

  // Adds a host, or with `replace` one in place of the host of that id,
  // which is forgotten with its cache: the new entry learns its key again,
  // so nothing would read the old one's snapshot.
  function add(input, { replace = false } = {}) {
    const entry = hosts.add(input, { replace });
    const old = runtimes.get(entry.id);
    if (old) {
      stopOne(old, { keep: false });
      runtimes.delete(entry.id);
      forgetCache(old);
    }
    const rt = start(entry);
    hostsChanged();
    return view(rt);
  }

  // Forgets a host: its link, its entry, its cache, and with `forget`, where
  // you placed its nodes on the map.
  function remove(id, { forget = false } = {}) {
    const rt = need(id);
    stopOne(rt, { keep: false });
    runtimes.delete(id);
    if (!rt.simulated) hosts.remove(id);
    forgetCache(rt);
    const forgot = forget ? (local.forgetLayout?.(`@${id}:`) ?? 0) : 0;
    hostsChanged();
    return { id, removed: true, forgot };
  }

  function reconnect(id) {
    need(id).link.reconnect();
    return view(need(id));
  }

  // One try, apart from the host's own link, that says how far it got:
  // reached, a hello in a protocol this XenoNet speaks, the same key, a map,
  // the clock. For xenonet hosts test.
  function test(id) {
    const rt = need(id);
    const started = clock.now();
    const probe = new HostLink({ entry: rt.link.entry, sshConfig: hosts.sshConfig, root, demoOnly: demo, clock, tilde: shown, once: true, ...linkOptions });
    return new Promise((resolve) => {
      let done = false;
      let got = null;
      let timer = null;
      const finish = (ok) => {
        if (done) return;
        done = true;
        clock.clearTimeout(timer);
        const info = probe.info();
        probe.removeAllListeners();
        probe.stop();
        resolve({ id, ok, ms: clock.now() - started, state: info.state, error: info.error, hello: info.hello, offsetMs: info.offsetMs, rttMs: info.rttMs, target: info.target, snapshot: got, dropped: info.counts.dropped });
      };
      probe.on("snapshot", (s) => {
        got = { nodes: s.graph.nodes.filter((n) => n?.id !== "core").length, sessions: s.graph.nodes.filter((n) => n?.kind === "session").length, live: Array.isArray(s.live?.sessions) ? s.live.sessions.length : 0 };
        // The clock's answer is on its way; a moment more for it.
        if (probe.info().offsetMs != null) finish(true);
        else timer = clock.setTimeout(() => finish(true), 2000);
      });
      probe.on("offset", () => got && finish(true));
      probe.on("state", (i) => (i.state === "offline" || i.state === "error") && finish(false));
      clock.clearTimeout(timer);
      timer = clock.setTimeout(() => finish(false), TEST_MS);
      probe.start();
    });
  }

  // A host's detail: how it stands, and its engine (its agents and the
  // last day's messages) under qualified ids.
  function detail(id) {
    const rt = need(id);
    return { ...view(rt), engine: rt.snapshot?.engine ? qualifyEngine(rt.snapshot.engine, infoOf(rt)) : null };
  }

  // Reads through a host's relay, which answers from its allowlist.
  function get(id, route, opts) {
    const rt = runtimes.get(id);
    if (!rt) return Promise.reject(fail(`there's no host called ${id}`, 404));
    return rt.link.get(route, opts);
  }

  // XenoNet Dev's simulations (POST /api/dev/simulate): a host made of
  // another XenoNet on this machine, in memory only; one going offline and
  // coming back; and its clock three minutes off. Null for one it doesn't
  // know, or a host there isn't. What core/control.js sends is { a, b }, so
  // those carry them too: host-add's id and port, host-skew's milliseconds.
  function simulate(kind, { a = null, b = null, id = null, port = null, ms = null } = {}) {
    if (kind === "host-add") {
      let entry;
      try {
        entry = hostEntry({ id: id ?? a, local: true, port: port ?? b });
      } catch (err) {
        return { error: err.message };
      }
      if (runtimes.has(entry.id)) return { error: `there's already a host called ${entry.id}` };
      start(entry, { simulated: true });
      hostsChanged();
      return { ok: true, kind, id: entry.id };
    }
    const rt = runtimes.get(a ?? id) ?? (a || id ? null : runtimes.values().next().value);
    if (!rt) return null;
    const which = rt.link.entry.id;
    if (kind === "host-offline") rt.link.pause("taken offline by a simulation");
    else if (kind === "host-online") rt.link.resume();
    else if (kind === "host-skew") {
      const skew = Number(ms ?? b);
      rt.skewMs = ms == null && b == null ? 180e3 : Number.isFinite(skew) ? skew : 180e3;
      hostsChanged();
    } else return null;
    return { ok: true, kind, a: which };
  }

  return Object.assign(hub, {
    start() {
      try {
        hosts.writeSshConfig();
      } catch (err) {
        console.error(`[xenonet] couldn't write ${hosts.sshConfig}: ${err.message}`);
      }
      for (const entry of hosts.list()) start(entry);
      if (runtimes.size) hostsChanged();
      return hub;
    },
    stop() {
      for (const t of Object.values(timers)) clock.clearTimeout(t);
      for (const rt of runtimes.values()) stopOne(rt);
      runtimes.clear();
    },
    graph,
    live,
    summary,
    // The latest load each host sent (system/v1, under host), for GET
    // /api/system?hosts=1.
    systems: () => [...runtimes.values()].map((rt) => rt.system).filter(Boolean),
    list,
    detail,
    add,
    remove,
    reconnect,
    test,
    get,
    simulate,
    has: (id) => runtimes.has(id),
  });
}
