// core/actors/index.js in Rust (crates/xenonet-actors/src/registry.rs, with
// you.rs and demo.rs): the registry of actors. Their records, presence,
// events, targets and lists are Rust's, kept in the engine's store. The
// twin keeps the class's API: the EventEmitter it is, the list's throttle
// and the clock's timers, and the host Rust asks of the world (the time,
// the map, whether a process runs). mapOf, the map's adapter over XenoNet's
// state, is still the JS: it reads the state's own nodes.
import { EventEmitter } from "node:events";
import { native } from "../../index.js";
import { EngineStore, STORE } from "../engine/store.js";
import { resolveDemo } from "./demo.js";
export { mapOf } from "../../../../../core/actors/index.js";

const C = native.actorConstants();
export const LIST_THROTTLE_MS = C.listThrottleMs;
export const CLOCK_MS = C.clockMs;
export const PRUNE_MS = C.pruneMs;
export const REMOTE_LEASE_MS = C.remoteLeaseMs;

export const nodeKind = (id) => native.nodeKind(id) ?? null;

// Whether a process still runs: signal 0 only checks, and a process of
// another user's (EPERM) is still there.
const running = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
};

// The demo's scripts, for addDemoActors.
export const DEMO = Symbol.for("xenonet.actors.demo");

export class Actors extends EventEmitter {
  #core;
  #host;

  constructor({ store = new EngineStore(":memory:"), now = Date.now, map = null, demo = false, alive = running } = {}) {
    super();
    this.store = store;
    this.now = now;
    this.map = map;
    this.demo = demo;
    this.alive = alive;
    this.drivers = [];
    this.timers = [];
    this.listTimer = null;
    // What runs before an actor's record goes (remove): its remote
    // connections are revoked first, while their grants can still be named.
    this.beforeRemove = [];
    const core = store?.[STORE];
    if (!core) throw new Error("the actors keep their records in an EngineStore");
    this.#host = (op, arg) => this.#answer(op, arg);
    this.#core = new native.ActorsCore(core, !!demo, this.#host);
    const h = this.#host;
    this.you = {
      report: (body = {}, { at = this.now() } = {}) => this.#core.youReport(h, body, at),
      window: (state, at = this.now()) => void this.#core.youWindow(h, state, at),
      forget: () => void this.#core.youForget(h),
      off: () => this.#core.youOff(),
      presence: () => this.#core.youPresence() ?? null,
    };
  }

  // What Rust asks of the world, and what it tells.
  #answer(op, arg) {
    switch (op) {
      case "now":
        return this.now();
      case "describe":
        return this.map?.describe?.(arg) ?? null;
      case "hasMap":
        return !!this.map?.has;
      case "has":
        return this.map?.has ? !!this.map.has(arg) : undefined;
      case "ready":
        return this.map?.ready?.() === false ? false : undefined;
      case "placeOf":
        return this.map?.placeOf?.(arg) ?? null;
      case "recentSession":
        return this.map?.recentSession?.() ?? null;
      case "alive":
        return !!this.alive(arg);
      case "resolve":
        return resolveDemo(arg[0], arg[1]);
      case "emit":
        this.emit(arg[0], arg[1]);
        return undefined;
      case "changed":
        this.#changed();
        return undefined;
      default:
        return undefined;
    }
  }

  // The records, as the JS kept them, for the tests that look in.
  get actors() {
    return new Map(this.#core.records().map((r) => [r.id, { ...r, instances: new Map(r.instances), targets: new Map(r.targets) }]));
  }
  // Spawned reports waiting for their session (awaitSession).
  get waiting() {
    return this.#core.waiting().map((w) => ({ ...w, own: new Set(w.own) }));
  }

  // The waiting reports the attribution matches (attribution.js): those
  // still waiting at `at`, and one let go once its session is named.
  expireWaiting(at) {
    this.#core.expireWaiting(at);
    return this.waiting;
  }
  dropWaitingAt(i) {
    this.#core.dropWaitingAt(i);
  }

  has = (id) => this.#core.has(id);
  ids = () => this.#core.ids();
  presence = (id) => this.#core.presence(id) ?? null;
  presenceSince = (id) => this.#core.presenceSince(id);
  isPublic = (id) => this.#core.isPublic(id);
  can = (id, capability) => this.#core.can(id, capability);
  enabled = (id) => this.#core.enabled(id);
  installed = (id) => this.#core.installed(id) ?? undefined;
  isRegistered = (id) => this.#core.isRegistered(id);
  onlyObserved = (id) => this.#core.onlyObserved(id);
  targetsOf = (id) => this.#core.targetsOf(id);
  pidsOf = (id) => new Set(this.#core.pidsOf(id));
  info = (id) => this.#core.info(id);
  registeredCount = () => this.#core.registeredCount();
  ofProduct = (product) => this.#core.ofProduct(product);
  isRemote = (id) => this.#core.isRemote(id);
  remoteOf = (id) => this.#core.remoteOf(id);

  #record = (id) => this.actors.get(id);

  upsert(def) {
    this.#core.upsert(this.#host, def);
    return this.#record(def.id);
  }
  addVia(id, v) {
    this.#core.addVia(this.#host, id, v);
  }
  dropVia(id, v) {
    this.#core.dropVia(this.#host, id, v);
  }
  setRemote(id, remote) {
    this.#core.setRemote(this.#host, id, remote ?? null);
  }
  remove(id) {
    const slug = this.#core.slugOf(id);
    for (const fn of this.beforeRemove) fn(slug);
    this.#core.remove(this.#host, id);
  }
  forgetYou() {
    this.#core.forgetYou(this.#host);
  }
  unregister(id, { keepRemote = true } = {}) {
    const revokes = this.#core.unregisterRevokes(id, !!keepRemote);
    if (revokes) {
      const slug = this.#core.slugOf(id);
      for (const fn of this.beforeRemove) fn(slug);
    }
    if (this.#core.unregister(this.#host, id, revokes)) return this.remove(id);
  }
  allow(id, at = this.now()) {
    this.#core.allow(this.#host, id, at);
  }
  setEnabled(id, on) {
    this.#core.setEnabled(this.#host, id, !!on);
  }
  setInstalled(id, installed) {
    this.#core.setInstalled(this.#host, id, !!installed);
  }
  awaitSession(id, { app = null, place = null, own = null, at = this.now() } = {}) {
    return this.#core.awaitSession(id, app, place, own ? [...own] : [], at, this.now());
  }
  watchPid(id, pid) {
    this.#core.watchPid(id, pid);
  }
  report(id, options = {}) {
    this.#core.report(this.#host, id, options);
    return this.#record(id);
  }
  presenceOf(a, now = this.now()) {
    return this.#core.presenceAt(a?.id, now) ?? "unknown";
  }
  record(id, options) {
    return this.#core.record(this.#host, id, options);
  }
  where(target) {
    return this.#core.whereOf(this.#host, target);
  }
  target(id, nodeId, relation, at = this.now()) {
    this.#core.target(this.#host, id, nodeId, relation, at);
  }
  untarget(id, nodeId, relation) {
    this.#core.untarget(this.#host, id, nodeId, relation);
  }
  snapshot(id, { view = "public", now = this.now() } = {}) {
    return this.#core.snapshot(this.#host, id, String(view), now);
  }
  sessionsStarted({ now = this.now() } = {}) {
    const { names, starts } = this.#core.sessionsStarted(now);
    return { names: new Map(names), starts };
  }
  placeOf(p) {
    return this.#core.placeOf(this.#host, p);
  }
  engineView(id, { now = this.now(), since } = {}) {
    return this.#core.engineView(this.#host, id, now, since ?? undefined);
  }
  setStatus(id, text, { via = "registered", at = this.now() } = {}) {
    return this.#core.setStatus(this.#host, id, text, via, at);
  }
  said({ from, to = [], ts } = {}) {
    return this.#core.said(this.#host, from, [...to].map(String), ts ?? undefined);
  }
  engineIds(since, now = this.now()) {
    return this.#core.engineIds(since, now);
  }
  tokens(now = this.now()) {
    return this.#core.tokens(now);
  }
  list({ view = "public", now = this.now() } = {}) {
    return this.#core.list(this.#host, String(view), now);
  }
  events({ since, limit = 1000, view = "public", only = null, now = this.now() } = {}) {
    return this.#core.events({ since, limit, view, only }, now);
  }

  // The list goes out (`actors`) at most every LIST_THROTTLE_MS after a
  // change, and only when what it says has changed.
  #changed() {
    if (this.listTimer) return;
    this.listTimer = setTimeout(() => {
      this.listTimer = null;
      const list = this.#core.listIfChanged(this.#host);
      if (list) this.emit("actors", list);
    }, LIST_THROTTLE_MS);
    this.listTimer.unref?.();
  }

  refresh() {
    this.#changed();
  }

  tick(now = this.now()) {
    this.#core.tick(this.#host, now);
  }
  prune(now = this.now()) {
    this.#core.prune(this.#host, now);
  }

  drive(ms, fn) {
    this.drivers.push([ms, fn]);
    return this;
  }
  start() {
    const every = (ms, fn) => {
      const t = setInterval(fn, ms);
      t.unref?.();
      this.timers.push(t);
    };
    for (const [ms, fn] of this.drivers) every(ms, fn);
    every(CLOCK_MS, () => this.tick());
    every(PRUNE_MS, () => this.prune());
    return this;
  }
  stop() {
    for (const t of this.timers) clearInterval(t);
    this.timers = [];
    clearTimeout(this.listTimer);
    this.listTimer = null;
    this.#core.stop(this.#host);
  }

  simulationHeld(id) {
    return this.#core.simulationHeld(id);
  }
  simulate(kind, { a, b } = {}) {
    return this.#core.simulate(this.#host, kind, a, b);
  }

  // The demo's actors and their scripts (demo.js): the step the driver
  // plays.
  [DEMO](history) {
    this.#core.addDemo(this.#host, history);
    const step = () => this.#core.demoStep(this.#host);
    this.drive(C.stepMs, step);
    return step;
  }
}

export function createActors({ store, demo = false, now = Date.now, map = null } = {}) {
  const actors = new Actors({ store, now, map, demo });
  if (demo) actors[DEMO](3 * 3600e3);
  return actors;
}
