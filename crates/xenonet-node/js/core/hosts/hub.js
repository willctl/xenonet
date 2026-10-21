// core/hosts/hub.js in Rust (crates/xenonet-hosts/src/hub.rs): the
// console's side of hosts, a link to each host in hosts.json, each host's
// last snapshot kept in the cache folder, and the merged view the server
// sends in place of this machine's own. How a host stands, what the page
// hears of it, a host's own doctor and the waits are the Rust's, and so
// are the links, the merging and the cache (the twins beside this). The
// emitter, the timers and the hosts' runtimes are here.
import { EventEmitter } from "node:events";
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { hostEntry } from "./config.js";
import { HostLink } from "./link.js";
import { mergeGraph, mergeLive, mergeSummary, qualifyEvent, qualifyEngine, eventNote, cleanOnce, DEFAULT_WINDOWS } from "./merge.js";
import { readCache, writeCache, removeCache } from "./cache.js";

const { systemClock } = await import(peer("blocks/clock.js"));
const { tilde } = await import(peer("paths.js"));

const C = native.hostsHubConstants();
const fail = (message, status) => Object.assign(new Error(message), { status });

export function createHub({ hosts, cacheDir, root, home = null, demo = false, windows = DEFAULT_WINDOWS, local, clock = systemClock, linkOptions = {}, log = null }) {
  const hub = new EventEmitter();
  const runtimes = new Map();
  const timers = { graph: null, live: null };
  const last = { graph: 0, live: 0 };
  const shown = (p) => tilde(p, home);

  const infoOf = (rt) => native.hostsInfoOf(rt.link.info(), rt.cached, rt.cachedHello, rt.skewMs || 0);
  const forMerge = () => [...runtimes.values()].map((rt) => ({ info: infoOf(rt), snapshot: rt.snapshot, clean: rt.clean }));

  // ---------- what goes out ----------

  function graph(localGraph) {
    if (!runtimes.size) return localGraph;
    const ids = local.identities?.() ?? {};
    return mergeGraph(localGraph, forMerge(), { layout: local.layout?.() ?? {}, now: clock.now(), windows, self: hosts.self().id, names: ids.names ?? {}, hidden: ids.hidden ?? {}, aliases: local.aliases?.() ?? new Map() });
  }

  const live = (localLive) => (runtimes.size ? mergeLive(localLive, forMerge()) : localLive);
  const summary = (localSummary) => (runtimes.size ? mergeSummary(localSummary, forMerge()) : localSummary);

  function soon(kind, every, send) {
    if (timers[kind]) return;
    timers[kind] = clock.setTimeout(
      () => {
        timers[kind] = null;
        last[kind] = clock.now();
        send();
      },
      native.hostsSoonWait(last[kind], every, clock.now()),
    );
    timers[kind]?.unref?.();
  }
  const graphSoon = () =>
    soon("graph", C.graphEveryMs, () => {
      const g = local.graph();
      if (g) hub.emit("graph", graph(g));
    });
  const liveSoon = () =>
    soon("live", C.liveEveryMs, () => {
      const l = local.live();
      if (l) hub.emit("live", live(l));
    });
  const forPage = () => list().map((v) => native.hostsForPage(v));
  const hostsChanged = () => {
    hub.emit("hosts", forPage());
    graphSoon();
    liveSoon();
  };

  // ---------- the cache ----------

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
  function forgetCache(rt) {
    if (rt.simulated) return;
    for (const key of new Set([rt.link.entry.key, rt.cacheKey])) dropCache(rt, key);
    rt.cacheKey = null;
  }
  // A file another host's link uses too stays for that one.
  function dropCache(rt, key) {
    if (!key || [...runtimes.values()].some((o) => o !== rt && !o.simulated && (o.link.entry.key === key || o.cacheKey === key))) return;
    removeCache(cacheDir, key);
  }
  function saveSoon(rt) {
    if (rt.cacheTimer || rt.simulated) return;
    rt.cacheTimer = clock.setTimeout(() => save(rt), native.hostsCacheWait(rt.lastCacheAt ?? null, clock.now()));
    rt.cacheTimer?.unref?.();
  }

  // ---------- each host ----------

  function start(entry, { simulated = false } = {}) {
    const cached = entry.key ? readCache(cacheDir, entry.key) : null;
    const link = new HostLink({ entry, sshConfig: hosts.sshConfig, root, demoOnly: demo, clock, tilde: shown, ...linkOptions });
    const rt = { link, simulated, clean: cleanOnce(), snapshot: null, cached: false, cachedHello: null, doctor: null, engine: null, lastCacheAt: null, cacheTimer: null, cacheKey: null, skewMs: 0 };
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
        rt.doctor = native.hostsDoctorOf(data);
        return hub.emit("hosts", forPage());
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

  function stopOne(rt, { keep = true } = {}) {
    rt.link.removeAllListeners();
    rt.link.stop();
    if (keep && rt.cacheTimer) save(rt);
    clock.clearTimeout(rt.cacheTimer);
    rt.cacheTimer = null;
  }

  // Only the map's nodes count toward how a host stands.
  const view = (rt) => native.hostsView(infoOf(rt), rt.link.entry, rt.snapshot?.graph ? { graph: { nodes: rt.snapshot.graph.nodes } } : null, rt.cached, rt.simulated, rt.doctor);

  function list() {
    return [...runtimes.values()].map(view);
  }

  function need(id) {
    const rt = runtimes.get(id);
    if (!rt) throw fail(`there's no host called ${id}`, 404);
    return rt;
  }

  // ---------- what you can do ----------

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

  // One try, apart from the host's own link, that says how far it got.
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
        got = native.hostsProbeSnapshot(s.graph, s.live);
        // The clock's answer is on its way; a moment more for it.
        if (probe.info().offsetMs != null) finish(true);
        else timer = clock.setTimeout(() => finish(true), 2000);
      });
      probe.on("offset", () => got && finish(true));
      probe.on("state", (i) => (i.state === "offline" || i.state === "error") && finish(false));
      clock.clearTimeout(timer);
      timer = clock.setTimeout(() => finish(false), C.testMs);
      probe.start();
    });
  }

  function detail(id) {
    const rt = need(id);
    return { ...view(rt), engine: rt.snapshot?.engine ? qualifyEngine(rt.snapshot.engine, infoOf(rt)) : null };
  }

  function get(id, route, opts) {
    const rt = runtimes.get(id);
    if (!rt) return Promise.reject(fail(`there's no host called ${id}`, 404));
    return rt.link.get(route, opts);
  }

  // XenoNet Dev's simulations (POST /api/dev/simulate).
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
      rt.skewMs = native.hostsSkewOf(ms, b);
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
