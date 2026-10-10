// Actors: the persistent entities beside the session graph (contracts,
// section 3, and docs/design/actors/interface.md): the person at the
// console, always-on agents, and programs that register themselves.
//
// This is the registry. Everything XenoNet learns of an actor comes through
// it, whatever the source: an observer reading a product's state, the
// person's window (you.js), a registered actor's reports, or the demo's
// scripts (demo.js), which use the same calls:
//
//   upsert(def)          the actor itself: name, product, how XenoNet knows it
//   report(id, ...)      one of its instances says it's there: presence,
//                        status, whether it needs you
//   record(id, ...)      something it did, as facts; XenoNet writes the detail
//   target(id, ...)      a node of the map it acts on
//
// It keeps them in engine.sqlite (rows, events and targets; see
// core/engine/store.js), builds actor/snapshot/v1 and actor/list/v1 from
// them, and decides presence on a clock every 5 seconds: two missed
// heartbeats are away, ten are offline. The list goes out (`actors`) when
// it changes, at most every 500 ms, and each event as it happens (`event`).
//
// The person at the console is seen only by the window. Everywhere else
// (the list, the events, the event stream) actor:you is a placeholder, and
// the list isn't sent again when only you changed, so nothing can time its
// work to your absence. The demo's made-up you is shown like any actor.
import { EventEmitter } from "node:events";
import path from "node:path";
import { YOU, parseActorId } from "../schema/ids.js";
import { EngineStore } from "../engine/store.js";
import { pathStyle, pathKey } from "../paths.js";
import { writeDetail, presenceWords, statusLine, placeName, clip } from "./detail.js";
import { createYou, YOU_HEARTBEAT_MS } from "./you.js";
import { addDemoActors } from "./demo.js";
import { TOKEN_UNUSED_MS, tokenExpired } from "./registration.js";

export const LIST_THROTTLE_MS = 500;
export const CLOCK_MS = 5000;
export const PRUNE_MS = 3600e3;
const DAY_MS = 24 * 3600e3;
const WEEK_MS = 7 * DAY_MS;
const RECENT = 20; // events a snapshot shows
const TARGETS_SHOWN = 16;
const TARGETS_KEPT = 32;
const LIST_MAX = 32;
const INSTANCES_MAX = 8;
const GRACE_MS = 5000; // two heartbeats, and this, before away
const ACTED_MS = 90e3; // an action this recent makes it working
const SESSION_WAIT_MS = 2 * 60e3; // a session not on the map yet is kept this long
const EVENTS_MAX = 5000; // events an answer holds
const EDIT_HOLD_MS = 30 * 60e3; // a registered actor's edit holds its file this long, as an agent's does
const AWAIT_MS = 2 * 60e3; // a spawned report without a session waits this long for one
const AWAIT_MAX = 64;
// A remote actor's read is evidence it's at work then, and for this long
// after; then, unless another source says more, it's out of sight
// (unknown), never offline: a bridge that's gone says nothing of the actor
// itself (core/actors/remote-presence.js).
export const REMOTE_LEASE_MS = 90e3;
// Which presence wins when an actor's sources disagree: the most present.
const RANK = { working: 4, idle: 3, away: 2, unknown: 1, offline: 0 };
const ACTIONS = new Set(["spawned", "edited", "ran", "messaged", "said"]);
const FACTS = ["count", "paths", "platform", "subject", "app", "to"];

// What kind of node an id names, by its form.
export function nodeKind(id) {
  const s = String(id ?? "");
  if (parseActorId(s)) return "actor";
  if (/^(claude|codex):/.test(s)) return "session";
  const m = /^(project|zone|native):/.exec(s);
  return m ? m[1] : null;
}

// The relation an event gives its actor to its target: a session it
// started, files it changed, a session it said something to, and
// otherwise the place it works in. Presence and status give none of their
// own, and an actor is never a target.
function relationFor(kind, target) {
  const k = nodeKind(target);
  if (!k || k === "actor" || kind === "presence" || kind === "status") return null;
  if (kind === "spawned") return k === "session" ? "spawned" : "works-on";
  if (kind === "edited") return k === "session" ? null : "edited";
  if (kind === "said") return k === "session" ? "messaged" : null;
  return k === "session" ? null : "works-on";
}

// Whether a process still runs: signal 0 only checks, and a process of
// another user's (EPERM) is still there. Cheaper than reading the process
// table on every tick, and a pid is seldom reused within seconds.
const running = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
};

const json = (v) => (v == null ? null : JSON.stringify(v));
const parse = (s, fallback) => {
  try {
    return s == null ? fallback : JSON.parse(s);
  } catch {
    return fallback;
  }
};
const eventOf = (r) => ({ schema: "actor/event/v1", id: r.id, actor: r.actor, at: r.at, kind: r.kind, target: r.target ?? null, detail: r.detail ?? null, via: r.via });

// actor:you outside the window: there, and nothing more.
const placeholder = () => ({
  schema: "actor/snapshot/v1",
  id: YOU,
  kind: "human",
  name: "You",
  product: null,
  host: null,
  via: ["observed"],
  presence: "unknown",
  presenceSince: null,
  lastSeen: null,
  heartbeatMs: null,
  status: null,
  capabilities: [],
  targets: [],
  recent: [],
  counts: { events24h: 0, spawned24h: 0, messages24h: 0 },
  identity: null,
});

export class Actors extends EventEmitter {
  // `store`: the engine's (a store of its own in memory when none is
  // given, as tests use it). `map`: what the map knows (mapOf), to write
  // details and drop targets it doesn't have. `demo`: the demo world,
  // where the made-up you is shown like any actor. `alive`: whether a pid
  // still runs, for the processes actors name.
  constructor({ store = new EngineStore(":memory:"), now = Date.now, map = null, demo = false, alive = running } = {}) {
    super();
    this.store = store;
    this.now = now;
    this.map = map;
    this.demo = demo;
    this.alive = alive;
    this.actors = new Map();
    this.drivers = [];
    this.timers = [];
    this.listTimer = null;
    this.lastList = null;
    // Spawned reports waiting for their session (awaitSession).
    this.waiting = [];
    // What runs before an actor's record goes (remove): its remote
    // connections are revoked first, while their grants can still be named.
    this.beforeRemove = [];
    this.#load();
    // You are always here, if only as a placeholder.
    this.upsert({ id: YOU, kind: "human", name: "You", via: demo ? ["demo", "observed"] : ["observed"], heartbeatMs: YOU_HEARTBEAT_MS });
    this.you = createYou({ actors: this });
  }

  has = (id) => this.actors.has(id);
  ids = () => [...this.actors.keys()];
  // An actor's presence as last settled, without building its snapshot.
  presence = (id) => this.actors.get(id)?.presence ?? null;
  // Since when it has been that: since it was made, if it never changed.
  presenceSince = (id) => {
    const a = this.actors.get(id);
    return a ? (a.presenceSince ?? a.createdAt ?? null) : null;
  };
  // Whether an actor's record may be shown outside XenoNet's window.
  isPublic = (id) => this.demo || id !== YOU;
  // Whether an actor was allowed what it asks to do (interface section 4).
  can = (id, capability) => !!this.actors.get(id)?.capabilities.includes(capability);
  // Whether XenoNet looks at an observed actor: false once you've turned its
  // product's observer off.
  enabled = (id) => this.actors.get(id)?.enabled !== false;
  // Whether an observed actor's product is installed, as its observer last
  // said (setInstalled), or undefined before it has.
  installed = (id) => this.actors.get(id)?.installed;
  // Whether an actor reports itself because you allowed it: registered, and
  // allowed in the window (or the demo's own).
  isRegistered = (id) => {
    const a = this.actors.get(id);
    return !!a && a.via.includes("registered") && (a.allowedAt != null || a.via.includes("demo"));
  };
  // Whether XenoNet knows an actor only by observing it (Grok Bot, Hermes
  // before its plugin): it doesn't report itself, so it can't be removed;
  // its observer's switch takes it off the map instead.
  onlyObserved = (id) => {
    const a = this.actors.get(id);
    return !!a && id !== YOU && !this.isRegistered(id) && a.via.some((v) => v === "observed" || v === "attributed");
  };
  // What an actor acts on, as { id, relation }.
  targetsOf = (id) => [...(this.actors.get(id)?.targets.values() ?? [])].map((t) => ({ id: t.id, relation: t.relation }));
  // The processes an actor named: the one it registered as, and its
  // instances'.
  pidsOf = (id) => {
    const a = this.actors.get(id);
    return new Set([a?.pid, ...[...(a?.instances.values() ?? [])].map((i) => i.pid)].filter((pid) => Number.isInteger(pid)));
  };
  // An actor as registering sees it, or null.
  info = (id) => {
    const a = this.actors.get(id);
    return a ? { id, name: a.name, product: a.product, via: [...a.via], heartbeatMs: a.heartbeatMs, capabilities: [...a.capabilities], identity: a.identity, allowedAt: a.allowedAt, presence: a.presence, enabled: a.enabled } : null;
  };
  // The actors that registered and that you allowed, outside the demo:
  // what the limit of 16 counts.
  registeredCount = () => [...this.actors.values()].filter((a) => a.allowedAt != null && a.via.includes("registered") && !a.via.includes("demo")).length;
  // The actors of a product (by its id), observed or registered.
  ofProduct = (product) => [...this.actors.values()].filter((a) => a.product?.id === product).map((a) => a.id);
  // Whether an actor reaches XenoNet through a remote connection you approved
  // (core/actors/connections.js), active or not: it reads, and never
  // reports itself or receives messages that way.
  isRemote = (id) => !!this.actors.get(id)?.via.includes("remote");
  // What its remote connections come to, as connections.js keeps it: null
  // for an actor with none.
  remoteOf = (id) => this.actors.get(id)?.remote ?? null;

  #need(id) {
    const a = this.actors.get(id);
    if (!a) throw new Error(`no actor ${id}`);
    return a;
  }

  // ---------- what's kept ----------

  #load() {
    const targets = new Map();
    for (const t of this.store.actorTargets()) {
      if (!targets.has(t.actor)) targets.set(t.actor, new Map());
      targets.get(t.actor).set(`${t.relation}|${t.target}`, { id: t.target, relation: t.relation, since: t.since, lastAt: t.last_at });
    }
    for (const r of this.store.actorRows()) {
      const id = `actor:${r.slug}`;
      if (!parseActorId(id)) continue;
      const a = {
        id,
        slug: r.slug,
        kind: r.kind,
        name: r.name,
        product: parse(r.product, null),
        via: parse(r.via, []),
        capabilities: parse(r.capabilities, []),
        heartbeatMs: r.heartbeat_ms ?? null,
        identity: r.identity ?? null,
        enabled: r.enabled !== 0,
        createdAt: r.created_at,
        allowedAt: r.allowed_at ?? null,
        lastSeen: r.last_seen ?? null,
        presence: r.presence ?? "unknown",
        presenceSince: r.presence_at ?? null,
        status: r.status ?? null,
        statusAt: r.status_at ?? null,
        needsYou: null,
        seq: r.seq ?? 0,
        instances: new Map(),
        actedAt: null,
        cwd: null,
        pid: null,
        targets: targets.get(id) ?? new Map(),
        recent: this.store.lastActorEvents(id, RECENT).map(eventOf),
        dirty: false,
      };
      // After a restart an actor keeps its last presence while it's within
      // two heartbeats of when it was last heard from, and the rules take
      // over from there. (Yours came from the window.) It stands in only
      // until a source speaks again: an observer's first look says what's
      // true now, even for an actor with no heartbeat, whose presence would
      // otherwise never age.
      // One only a remote connection speaks for has no heartbeat to age by:
      // its last presence lasts a read's lease, as it would have.
      const lease = a.heartbeatMs == null && a.via.includes("remote") ? { leaseMs: REMOTE_LEASE_MS } : {};
      if (a.lastSeen != null) a.instances.set(id === YOU ? "window" : "main", { presence: a.presence, at: a.lastSeen, heartbeatMs: a.heartbeatMs, pid: null, restored: true, ...lease });
      this.actors.set(id, a);
    }
  }

  #row = (a) => ({
    slug: a.slug,
    kind: a.kind,
    name: a.name,
    product: json(a.product),
    via: JSON.stringify(a.via),
    capabilities: JSON.stringify(a.capabilities),
    heartbeat_ms: a.heartbeatMs,
    identity: a.identity,
    enabled: a.enabled ? 1 : 0,
    created_at: a.createdAt,
    allowed_at: a.allowedAt,
    last_seen: a.lastSeen,
    presence: a.presence,
    presence_at: a.presenceSince,
    status: a.status,
    status_at: a.statusAt,
    seq: a.seq,
  });

  #save(a) {
    this.store.saveActor(this.#row(a));
    a.dirty = false;
  }

  // Rows that only heard a heartbeat are written on the clock, not each time.
  #flush() {
    const dirty = [...this.actors.values()].filter((a) => a.dirty);
    if (dirty.length) this.store.transaction(() => dirty.forEach((a) => this.#save(a)));
  }

  // ---------- the actors themselves ----------

  // An actor XenoNet knows of, new or again: its id, kind, name, product (or
  // null), how XenoNet knows of it (via), heartbeat, capabilities and
  // identity, when you allowed it, and whether its product's observer is on
  // (enabled). Only you are human.
  upsert({ id, kind = "agent", name, product = null, via = [], heartbeatMs = null, capabilities = [], identity = null, allowedAt, enabled, at = this.now() }) {
    const parsed = parseActorId(id);
    if (!parsed || parsed.host) throw new Error(`not an actor id of this machine: ${id}`);
    if ((kind === "human") !== (id === YOU)) throw new Error(`only ${YOU} is human`);
    let a = this.actors.get(id);
    if (!a) {
      a = { id, slug: parsed.slug, enabled: true, createdAt: at, allowedAt: null, lastSeen: null, presence: "unknown", presenceSince: null, status: null, statusAt: null, needsYou: null, seq: 0, instances: new Map(), actedAt: null, cwd: null, pid: null, targets: new Map(), recent: [], dirty: false };
      this.actors.set(id, a);
    }
    Object.assign(a, { kind, name, product, via, heartbeatMs, capabilities, identity });
    if (allowedAt !== undefined) a.allowedAt = allowedAt;
    if (enabled !== undefined) a.enabled = !!enabled;
    this.#save(a);
    this.#changed();
    return a;
  }

  // How XenoNet knows an actor, one source added or taken away, and nothing
  // else of it changed: a remote connection joins an observed or
  // registered actor without touching its name, capabilities or identity.
  addVia(id, v) {
    const a = this.#need(id);
    if (a.via.includes(v)) return;
    a.via = [...a.via, v];
    this.#save(a);
    this.#changed();
  }
  dropVia(id, v) {
    const a = this.#need(id);
    if (!a.via.includes(v)) return;
    a.via = a.via.filter((x) => x !== v);
    this.#save(a);
    this.#changed();
  }

  // What an actor's remote connections come to (connections.js): whether
  // any is active, how many, whether an adapter is connected through one,
  // how sure XenoNet is of its account, what it may do, and the routes its
  // active ones take (a tunnel, which the map draws as a gate). In memory,
  // and in its snapshot; null takes it away.
  setRemote(id, remote) {
    const a = this.actors.get(id);
    if (!a) return;
    // A made-up tunnel (simulate actor-tunnel) stays until clear, whatever
    // the connections say of an actor that has none.
    if (a.simulatedRemote && !remote?.simulated) return;
    const next = remote ? { active: !!remote.active, connections: remote.connections ?? 0, bridge: remote.bridge ?? "not_connected", accountLink: remote.accountLink ?? "unverified", permissions: [...(remote.permissions ?? [])], transports: [...new Set(remote.transports ?? [])].sort() } : null;
    if (JSON.stringify(next) === JSON.stringify(a.remote ?? null)) return;
    a.remote = next;
    this.#changed();
  }

  // An actor gone, with its events and targets, and what it waits on.
  remove(id) {
    const a = this.#need(id);
    for (const fn of this.beforeRemove) fn(a.slug);
    this.store.removeActor(a.slug);
    this.actors.delete(id);
    this.waiting = this.waiting.filter((w) => w.actor !== id);
    this.#changed();
  }

  // Your record deleted at once, when you turn off "Show me on the map"
  // (the decision at merge point B): your events, targets, presence and
  // status. You stay, as the placeholder you are outside the window, and
  // your next events take new ids, never an old one's.
  forgetYou() {
    const a = this.#need(YOU);
    this.store.transaction(() => {
      this.store.forgetActor(a.slug);
      a.instances.clear();
      a.targets.clear();
      Object.assign(a, { presence: "unknown", presenceSince: null, lastSeen: null, status: null, statusAt: null, needsYou: null, actedAt: null, cwd: null, recent: [] });
      this.#save(a);
    });
    this.#changed();
  }

  // A registered actor that no longer reports itself (it left, or you
  // removed it). A product's actor that XenoNet also observes stays, as
  // observed, with its history. One with a remote connection stays too when
  // it left of its own accord, since its registration going doesn't end
  // the connection; when you removed it (`keepRemote` false), its
  // connections are revoked first, as removing it does. Any other goes,
  // with its events and targets.
  unregister(id, { keepRemote = true } = {}) {
    const a = this.#need(id);
    if (!keepRemote && a.via.includes("remote")) {
      for (const fn of this.beforeRemove) fn(a.slug);
      a.via = a.via.filter((v) => v !== "remote");
    }
    const observed = !a.via.includes("demo") && a.via.some((v) => v === "observed" || v === "attributed");
    if (!observed && !a.via.includes("remote")) return this.remove(id);
    Object.assign(a, { via: a.via.filter((v) => v !== "registered"), allowedAt: null, pid: null, cwd: null });
    this.#save(a);
    this.#changed();
  }

  // A new token for an actor, given from the window now (to one of the
  // demo's, too). Like Allow and adding an actor, it makes allowedAt the
  // token's own time, so the token counts as never used until a call with
  // it moves that on (tokens()).
  allow(id, at = this.now()) {
    const a = this.#need(id);
    if (a.allowedAt === at) return;
    a.allowedAt = at;
    this.#save(a);
    this.#changed();
  }

  // Turns a product's observer on or off for this actor of it: off, XenoNet
  // stops looking, and an actor known only that way leaves the list.
  setEnabled(id, on) {
    const a = this.#need(id);
    a.enabled = !!on;
    this.#save(a);
    this.#changed();
  }

  // Whether an observed actor's product is installed here, as its observer
  // saw at its last look: an installed product stays listed however long
  // it hasn't run (the decision at merge point B). Unsaid (undefined) until
  // its observer's first look since XenoNet started.
  setInstalled(id, installed) {
    const a = this.#need(id);
    if (a.installed === !!installed) return;
    a.installed = !!installed;
    this.#changed();
  }

  // A registered actor's spawned report that named no session, only an
  // app and a place (interface section 2): it waits up to two minutes for
  // a session to start under the actor's own processes (`own`, pids) in
  // that place, which the observers look for (ReportedSpawns in
  // attribution.js); when one does, a second event, attributed, names it,
  // and the actor's own stands as it was told. `app` is claude or codex,
  // or null for either; `place` a node id, or null for anywhere. In memory
  // only, at most 64 at once, the oldest going first. Whether it waits.
  awaitSession(id, { app = null, place = null, own = null, at = this.now() } = {}) {
    if (!own?.size || !this.actors.has(id)) return false;
    this.waiting.push({ actor: id, app, place, own: new Set(own), at, until: this.now() + AWAIT_MS });
    if (this.waiting.length > AWAIT_MAX) this.waiting.splice(0, this.waiting.length - AWAIT_MAX);
    return true;
  }

  // The process a registered actor said it runs as when it registered:
  // when it ends, the actor goes offline (see tick).
  watchPid(id, pid) {
    this.#need(id).pid = Number.isInteger(pid) && pid > 1 ? pid : null;
  }

  // ---------- presence ----------

  // One of an actor's instances says it's there (a heartbeat, an
  // observer's look, the window's report): its presence, which only goes
  // into effect through the rules (presenceOf), and maybe a status, and
  // whether it waits on you. Without a presence it's a heartbeat that
  // keeps the last one. `via`: the source, for the events this makes.
  // `pid`: the instance's process, which goes offline when it ends. `cwd`:
  // the folder it works in (inside a place XenoNet knows), the engine's place
  // for it. `leaseMs`: for an instance with no heartbeat that says it's
  // there only now and then (a remote actor's reads), how long its word
  // holds before it's out of sight.
  report(id, { instance = "main", presence, status, needsYou, heartbeatMs, leaseMs, pid, cwd, via, at = this.now() } = {}) {
    const a = this.#need(id);
    // A presence kept across a restart gives way to the first source that
    // speaks: the same instance carries on from it, and any other replaces it.
    for (const [name, i] of a.instances) if (i.restored) name === instance ? delete i.restored : a.instances.delete(name);
    let inst = a.instances.get(instance);
    if (!inst) {
      inst = { presence: presence ?? "idle", at, heartbeatMs: heartbeatMs !== undefined ? heartbeatMs : a.heartbeatMs, pid: pid ?? null };
      a.instances.set(instance, inst);
      // At most INSTANCES_MAX: the one silent longest goes.
      if (a.instances.size > INSTANCES_MAX) {
        const [oldest] = [...a.instances].filter(([name]) => name !== instance).sort((x, y) => x[1].at - y[1].at)[0];
        a.instances.delete(oldest);
      }
    }
    if (presence !== undefined) inst.presence = presence;
    if (heartbeatMs !== undefined) inst.heartbeatMs = heartbeatMs;
    if (leaseMs !== undefined) inst.leaseMs = leaseMs;
    if (pid !== undefined) inst.pid = pid;
    if (cwd !== undefined) a.cwd = cwd;
    inst.at = Math.max(inst.at, at);
    a.lastSeen = Math.max(a.lastSeen ?? 0, at);
    a.dirty = true;
    if (needsYou !== undefined) a.needsYou = needsYou ? { kind: String(needsYou.kind), text: statusLine(needsYou.text).status || "Waiting for you", since: needsYou.since ?? at } : null;
    this.store.transaction(() => {
      this.#settle(a, at, via);
      if (status !== undefined) this.#status(a, status, { via, at });
    });
    this.#changed();
    return a;
  }

  // What an actor's presence is at `now`, by section 1's rules: each
  // instance's word, unless it has missed two heartbeats (away) or ten
  // (offline), or its lease ran out (then it says nothing: a remote actor's
  // read says nothing of later); the most present instance wins; an action it reported in the
  // last 90 s is working; and with no word yet, unknown. Unknown isn't
  // aged: it's an observer saying it can't tell (Hermes's files can't be
  // read), said once, and silence says nothing more, so it stands until
  // the observer says otherwise, across a restart too.
  presenceOf(a, now = this.now()) {
    let best = null;
    for (const inst of a.instances.values()) {
      let p = inst.presence;
      // A lease that ran out says nothing at all, so it never hides what
      // another source says, offline included.
      if (inst.leaseMs != null && now - inst.at > inst.leaseMs) continue;
      if (p !== "offline" && p !== "unknown" && inst.heartbeatMs) {
        const silent = now - inst.at;
        if (silent > 10 * inst.heartbeatMs) p = "offline";
        else if (silent > 2 * inst.heartbeatMs + GRACE_MS) p = "away";
      }
      if (best == null || (RANK[p] ?? 0) > (RANK[best] ?? 0)) best = p;
    }
    best ??= "unknown";
    if (best !== "offline" && best !== "working" && a.actedAt != null && now - a.actedAt < ACTED_MS) best = "working";
    return best;
  }

  // A change of presence is an event, and its time is presenceSince. It's
  // settled at the time of what changed it, or at the last time it was
  // settled when that's later: something heard of late (an event a
  // registered actor kept for an hour, a scheduled job's last run) says
  // what happened then, not what's true now, so it can't take presence
  // back to that time. The demo's history, played in order, settles as it
  // goes.
  #settle(a, at, via) {
    const t = Math.max(at, a.settledAt ?? -Infinity);
    a.settledAt = t;
    const p = this.presenceOf(a, t);
    if (p === a.presence) return;
    const from = a.presence;
    [a.presence, a.presenceSince] = [p, t];
    this.#event(a, { kind: "presence", detail: presenceWords(from, p), via, at: t });
  }

  // A new status is an event; the same one again isn't, and one that
  // looks like it holds a key never gets in (statusLine).
  #status(a, text, { via, at, target = null } = {}) {
    const checked = statusLine(text);
    const line = checked.status;
    if (!checked.ok || line === a.status) return;
    [a.status, a.statusAt] = [line, at];
    // A status cleared isn't news.
    if (line) this.#event(a, { kind: "status", detail: line, target, via, at });
  }

  // ---------- events ----------

  // Something an actor did: an actor/event/v1 that XenoNet writes from
  // facts (`count`, `paths`, `platform`, `subject`, `app`, `to`; see
  // detail.js) and what the map says of `target`, kept and sent on. A
  // `status` event carries its line (`status`), and `relation` sets what
  // the target becomes, for the person's. Presence comes from reports.
  record(id, { kind, target = null, relation, status, via, at = this.now(), ref = null, ...rest }) {
    if (kind === "presence") throw new Error("presence comes from reports");
    const a = this.#need(id);
    const facts = Object.fromEntries(FACTS.filter((k) => rest[k] != null).map((k) => [k, rest[k]]));
    let event;
    this.store.transaction(() => {
      if (kind === "status") {
        const line = statusLine(status).status;
        if (!line) throw new Error("a status event needs a line of its own");
        [a.status, a.statusAt] = [line, at];
        event = this.#event(a, { kind, target, detail: line, via, at, facts, ref });
      } else event = this.#event(a, { kind, target, detail: writeDetail(kind, facts, this.where(target)), via, at, facts, ref });
      const rel = relation === undefined ? relationFor(kind, target) : relation;
      if (target && rel && nodeKind(target) !== "actor") this.#touch(a, target, rel, at);
      // A registered actor that reports an action is working for a while
      // after it (one reported a little ahead, from now); an observed one
      // is whatever its observer sees.
      if (ACTIONS.has(kind) && event.via === "registered") a.actedAt = Math.max(a.actedAt ?? 0, Math.min(at, this.now()));
      this.#settle(a, at, via);
    });
    this.#changed();
    return event;
  }

  #event(a, { kind, target = null, detail = null, via, at, facts = null, ref = null }) {
    const event = { schema: "actor/event/v1", id: `${a.slug}:${++a.seq}`, actor: a.id, at, kind, target, detail: clip(detail, 200), via: via ?? this.#source(a) };
    a.lastSeen = Math.max(a.lastSeen ?? 0, at);
    this.store.addActorEvent({ ...event, facts: facts && Object.keys(facts).length ? JSON.stringify(facts) : null, ref });
    this.#save(a);
    // Newest last, whatever order they came in.
    let i = a.recent.length;
    while (i > 0 && a.recent[i - 1].at > at) i--;
    a.recent.splice(i, 0, event);
    if (a.recent.length > RECENT) a.recent.splice(0, a.recent.length - RECENT);
    if (this.isPublic(a.id)) this.emit("event", event);
    return event;
  }

  // The source an event comes from when no one says: how XenoNet knows the
  // actor, the demo playing the part of a real source.
  #source = (a) => a.via.find((v) => v !== "demo") ?? "observed";

  // What the map says of a target, for details: a session's app and the
  // place it lives, or a place's name. Never a session's title.
  where(target) {
    const k = nodeKind(target);
    if (!k) return null;
    if (k === "actor") return { kind: "actor" };
    const d = this.map?.describe?.(target);
    if (!d) return { kind: k };
    if (d.kind === "session") return { kind: "session", app: d.app ?? null, place: placeName(d.place ?? null, "project") };
    const name = placeName(d.name ?? null, d.kind);
    return { kind: d.kind, name, place: name };
  }

  // ---------- targets ----------

  // Ties an actor to a node of the map it acts on: `since` is the first
  // time, `lastAt` the last. At most TARGETS_KEPT; the oldest goes.
  target(id, nodeId, relation, at = this.now()) {
    const a = this.#need(id);
    if (!nodeKind(nodeId) || nodeKind(nodeId) === "actor") throw new Error(`not a node of the map: ${nodeId}`);
    this.#touch(a, nodeId, relation, at);
    this.#changed();
  }

  untarget(id, nodeId, relation) {
    const a = this.#need(id);
    if (a.targets.delete(`${relation}|${nodeId}`)) this.store.dropActorTarget(id, nodeId, relation);
    this.#changed();
  }

  #touch(a, nodeId, relation, at) {
    const key = `${relation}|${nodeId}`;
    const t = a.targets.get(key) ?? { id: nodeId, relation, since: at, lastAt: at };
    t.since = Math.min(t.since, at);
    t.lastAt = Math.max(t.lastAt, at);
    a.targets.set(key, t);
    this.store.saveActorTarget({ actor: a.id, target: nodeId, relation, since: t.since, last_at: t.lastAt });
    if (a.targets.size > TARGETS_KEPT) {
      const [oldest] = [...a.targets].sort((x, y) => x[1].lastAt - y[1].lastAt)[0];
      const gone = a.targets.get(oldest);
      a.targets.delete(oldest);
      this.store.dropActorTarget(a.id, gone.id, gone.relation);
    }
  }

  // Whether the map can say what's on it: not before XenoNet's first look at
  // the projects and sessions, at start.
  #mapReady = () => !!this.map?.has && this.map.ready?.() !== false;

  // Whether a target is on the map: a session XenoNet doesn't know yet is
  // given two minutes, in case its transcript is on the way. Before the
  // map is there, everything is.
  #known(t, now) {
    if (!this.#mapReady()) return true;
    return this.map.has(t.id) || (nodeKind(t.id) === "session" && now - t.since < SESSION_WAIT_MS);
  }

  // ---------- what the console and the API see ----------

  #counts(now) {
    const out = new Map();
    for (const { actor, kind, n } of this.store.actorEventCounts(now - DAY_MS)) {
      const c = out.get(actor) ?? { events24h: 0, spawned24h: 0, messages24h: 0 };
      c.events24h += n;
      if (kind === "spawned") c.spawned24h += n;
      if (kind === "messaged" || kind === "said") c.messages24h += n;
      out.set(actor, c);
    }
    return out;
  }

  // One actor as an actor/snapshot/v1, or null. Outside the window (`view`
  // other than "window"), you are the placeholder.
  snapshot(id, { view = "public", now = this.now(), counts = null } = {}) {
    const a = this.actors.get(id);
    if (!a) return null;
    if (view !== "window" && !this.isPublic(id)) return placeholder();
    const targets = [...a.targets.values()]
      .filter((t) => now - t.lastAt < DAY_MS && this.#known(t, now))
      .sort((x, y) => y.lastAt - x.lastAt)
      .slice(0, TARGETS_SHOWN)
      .map((t) => ({ ...t }));
    return {
      schema: "actor/snapshot/v1",
      id: a.id,
      kind: a.kind,
      name: a.name,
      product: a.product,
      host: null,
      via: a.via,
      presence: a.presence,
      presenceSince: a.presenceSince,
      lastSeen: a.lastSeen,
      heartbeatMs: a.heartbeatMs,
      status: a.status,
      ...(a.needsYou ? { needsYou: { ...a.needsYou } } : {}),
      capabilities: a.capabilities,
      targets,
      recent: a.recent.map((e) => ({ ...e })),
      counts: (counts ?? this.#counts(now)).get(a.id) ?? { events24h: 0, spawned24h: 0, messages24h: 0 },
      identity: a.identity,
      // Its remote connections, apart from its capabilities: what it may do
      // through them is read, whatever it can do elsewhere.
      ...(a.remote ? { remote: { connections: a.remote.connections, active: a.remote.active, bridge: a.remote.bridge, accountLink: a.remote.accountLink, permissions: [...a.remote.permissions], transports: [...a.remote.transports] } } : {}),
    };
  }

  // Which actors the list shows: you, every registered one (offline too,
  // so you can see and remove them), every one with a remote connection
  // you approved that's still active, the demo's, observed ones whose
  // product their observer sees installed, however long since they ran,
  // and other observed ones heard of in the last 7 days. One known only
  // through an observer you turned off isn't.
  #listed(a, now) {
    if (a.id === YOU || a.allowedAt != null || a.remote?.active) return true;
    if (!a.enabled) return false;
    return a.via.includes("demo") || a.installed === true || (a.lastSeen != null && now - a.lastSeen < WEEK_MS);
  }

  // ---------- what the engine sees of actors ----------

  // The project or folder holding an absolute path, as a node id, or null:
  // where an actor's paths land. A path outside every place XenoNet knows
  // lands nowhere, so an actor can't add places to the map. The path is
  // taken as it resolves: atlas-api/../../../etc is /etc, not atlas-api.
  placeOf(p) {
    return typeof p === "string" && path.isAbsolute(p) ? (this.map?.placeOf?.(pathStyle(p).resolve(p)) ?? null) : null;
  }

  // What the engine needs of an actor (core/engine/engine.js), or null for
  // you and for one that isn't here: its name, how XenoNet knows it, its
  // product, presence and status, the folder its last heartbeat named, its
  // place (that folder's project or folder, else its latest works-on
  // target) and the files it edited since `since`, newest first, which hold
  // them as an agent's edits do. An edit it didn't report itself (its
  // observer saw it) is marked observed, so agents know XenoNet inferred it.
  engineView(id, { now = this.now(), since = now - EDIT_HOLD_MS } = {}) {
    const a = this.actors.get(id);
    if (!a || id === YOU) return null;
    const latest = [...a.targets.values()].filter((t) => t.relation === "works-on" && nodeKind(t.id) !== "session").sort((x, y) => y.lastAt - x.lastAt)[0];
    const placeId = (a.cwd && this.placeOf(a.cwd)) || latest?.id || null;
    const d = placeId ? this.map?.describe?.(placeId) : null;
    const edits = new Map();
    for (const row of this.store.actorFactsSince(id, "edited", since))
      for (const p of parse(row.facts, {})?.paths ?? []) if (typeof p === "string" && !edits.has(p)) edits.set(p, { at: row.at, observed: row.via !== "registered" });
    return {
      id,
      name: a.name,
      via: a.via,
      product: a.product,
      presence: a.presence,
      status: a.status,
      lastSeen: a.lastSeen,
      cwd: a.cwd,
      place: d ? { id: placeId, name: d.name ?? null, kind: d.kind, path: d.path ?? null } : null,
      capabilities: a.capabilities,
      edits: [...edits].map(([p, e]) => ({ path: p, at: e.at, ...(e.observed ? { observed: true } : {}) })),
    };
  }

  // An actor's own status line, set through the engine's focus (an actor's
  // focus is its status): { ok, status }, or { ok: false, reason } for one
  // that looks like it holds a key. An empty one clears it. It's a status
  // and not a heartbeat: no instance says it's there, so an actor that said
  // goodbye stays offline, and one whose heartbeats stopped stays away.
  setStatus(id, text, { via = "registered", at = this.now() } = {}) {
    const checked = statusLine(text ?? "");
    if (!checked.ok) return checked;
    const a = this.#need(id);
    this.store.transaction(() => this.#status(a, text ?? "", { via, at }));
    a.dirty = true;
    this.#changed();
    return { ok: true, status: a.status };
  }

  // A message an actor sent through the engine (its `message` event):
  // a said event in XenoNet's words, never the text, and a messaged target
  // for each session it reached. Null for a sender that isn't an actor, or
  // a message that reached no one yet (one to a repo waits for agents to
  // join it).
  said({ from, to = [], ts }) {
    if (from === YOU || !this.actors.has(from) || !to.length) return null;
    const at = ts ?? this.now();
    const sessions = to.filter((id) => nodeKind(id) === "session");
    const target = sessions.length === 1 ? sessions[0] : (this.engineView(from, { now: at })?.place?.id ?? null);
    let event;
    this.store.transaction(() => {
      event = this.record(from, { kind: "said", target, count: to.length, via: "registered", at });
      for (const s of sessions) this.#touch(this.actors.get(from), s, "messaged", at);
    });
    return event;
  }

  // The actors the engine counts among the agents at work: heard from since
  // `since` and there (working, idle or away), never you, and never one
  // whose observer you turned off.
  engineIds(since, now = this.now()) {
    return [...this.actors.values()].filter((a) => a.id !== YOU && a.lastSeen != null && a.lastSeen >= since && ["working", "idle", "away"].includes(a.presence) && this.#listed(a, now)).map((a) => a.id);
  }

  // Each registered actor's token, for XenoNet's window alone (GET
  // /api/actors/tokens), oldest actor first: whether it still works,
  // expired (30 days unused) or there's none (the demo's made-up one); when
  // it stops working unless it's used before then; and when it was last
  // used: exactly, the first time, and to within the hour after that
  // (core/actors/registration.js writes it no oftener), or null for a
  // token never used. A token's time is when it was made (allowedAt) until
  // its first use moves it on. Never the token, nor its hash.
  tokens(now = this.now()) {
    const rows = new Map(this.store.actorTokenTimes().map((r) => [r.slug, r]));
    return [...this.actors.values()]
      .filter((a) => a.id !== YOU && this.isRegistered(a.id))
      .sort((x, y) => x.createdAt - y.createdAt)
      .map((a) => {
        const r = rows.get(a.slug);
        const works = !!r?.hasToken && !tokenExpired(r.at, now);
        const used = !!r?.hasToken && r.at != null && a.allowedAt != null && r.at > a.allowedAt;
        // An actor that reports only through the gateway has no token of its
        // own, which is none, not expired.
        const state = works ? "works" : a.allowedAt != null && !r?.hasGateway ? "expired" : "none";
        const gate = r?.hasGateway && !tokenExpired(r.gatewayAt, now) ? { state: "works", expiresAt: r.gatewayAt + TOKEN_UNUSED_MS, usedAt: r.gatewayAt > (a.allowedAt ?? 0) ? r.gatewayAt : null } : null;
        return { actor: a.id, state, expiresAt: works ? r.at + TOKEN_UNUSED_MS : null, usedAt: used ? r.at : null, ...(gate ? { gateway: gate } : {}) };
      });
  }

  // The actors as an actor/list/v1: you first, then the oldest first, at
  // most 32 (those seen least lately go).
  list({ view = "public", now = this.now() } = {}) {
    const counts = this.#counts(now);
    let shown = [...this.actors.values()].filter((a) => this.#listed(a, now));
    if (shown.length > LIST_MAX) {
      const keep = new Set(
        [...shown]
          .sort((x, y) => (y.id === YOU) - (x.id === YOU) || (y.lastSeen ?? 0) - (x.lastSeen ?? 0))
          .slice(0, LIST_MAX)
          .map((a) => a.id),
      );
      shown = shown.filter((a) => keep.has(a.id));
    }
    shown.sort((x, y) => (y.id === YOU) - (x.id === YOU) || x.createdAt - y.createdAt);
    return { schema: "actor/list/v1", at: now, actors: shown.map((a) => this.snapshot(a.id, { view, now, counts })) };
  }

  // Events since `since` (the last 7 days at most), oldest first, as an
  // actor/events/v1: outside the window without yours, and for the window
  // (`only` you) just yours.
  events({ since, limit = 1000, view = "public", only = null, now = this.now() } = {}) {
    const from = Math.max(Number.isFinite(since) ? since : now - DAY_MS, now - WEEK_MS);
    const n = Math.min(Math.max(1, Math.floor(limit) || 1), EVENTS_MAX);
    if (only && view !== "window" && !this.isPublic(only)) return { schema: "actor/events/v1", at: now, events: [] };
    const rows = this.store.actorEventsSince(from, { limit: n, only, except: only || view === "window" || this.demo ? null : YOU });
    return { schema: "actor/events/v1", at: now, events: rows.map(eventOf) };
  }

  // The list goes out (`actors`) at most every LIST_THROTTLE_MS after a
  // change, and only when what it says has changed: a change only the
  // window may see (yours) sends nothing.
  #changed() {
    if (this.listTimer) return;
    this.listTimer = setTimeout(() => {
      this.listTimer = null;
      const list = this.list();
      const said = JSON.stringify(list.actors);
      if (said === this.lastList) return;
      this.lastList = said;
      this.emit("actors", list);
    }, LIST_THROTTLE_MS);
    this.listTimer.unref?.();
  }

  // Something outside changed what the list says (the map, say).
  refresh() {
    this.#changed();
  }

  // ---------- the clock ----------

  // Every CLOCK_MS: processes that ended, presence from silence, sessions
  // that never reached the map let go, heartbeats written down, and the
  // list sent if it changed.
  tick(now = this.now()) {
    this.store.transaction(() => {
      for (const a of this.actors.values()) {
        this.#ended(a, now);
        this.#settle(a, now);
        if (!this.#mapReady()) continue;
        for (const [key, t] of a.targets)
          if (nodeKind(t.id) === "session" && !this.map.has(t.id) && now - t.since >= SESSION_WAIT_MS) {
            a.targets.delete(key);
            this.store.dropActorTarget(a.id, t.id, t.relation);
          }
      }
      this.#flush();
    });
    this.#changed();
  }

  // The processes an actor named, watched: an instance whose process ended
  // goes offline, and when the process it registered as ends, all of it
  // does (interface section 2, `pid`).
  #ended(a, now) {
    for (const inst of a.instances.values())
      if (inst.pid && !this.alive(inst.pid)) [inst.presence, inst.pid] = ["offline", null];
    if (!a.pid || this.alive(a.pid)) return;
    a.pid = null;
    if (!a.instances.size) a.instances.set("main", { presence: "offline", at: now, heartbeatMs: a.heartbeatMs, pid: null });
    for (const inst of a.instances.values()) inst.presence = "offline";
  }

  // What's kept (core/engine/store.js prune), and what's in memory after it.
  prune(now = this.now()) {
    this.#flush();
    this.store.prune(now);
    const targets = new Map();
    for (const t of this.store.actorTargets()) targets.set(`${t.actor}|${t.relation}|${t.target}`, true);
    for (const a of this.actors.values()) {
      a.recent = this.store.lastActorEvents(a.id, RECENT).map(eventOf);
      for (const [key, t] of a.targets) if (!targets.has(`${a.id}|${key}`)) a.targets.delete(key);
      if (a.id === YOU && a.statusAt != null && now - a.statusAt >= DAY_MS) [a.status, a.statusAt] = [null, null];
    }
    this.#changed();
  }

  // `fn` every `ms` while started: the demo's scripts.
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
    this.#flush();
  }

  // ---------- XenoNet Dev's simulations ----------

  simulationHeld(id) { return this.demo && !!this.actors.get(id)?.simulatedOffline; }

  // actor-spawn, actor-edit, actor-message, actor-online and actor-offline:
  // actor `a` (by default the first agent) acting on node `b` (by default
  // the session busy most lately, or for an edit the place it lives). What
  // it did, or null for an unknown kind or actor. Only the demo's made-up
  // you can be made to act: outside it, only the window speaks for you.
  // actor-needs-you has an agent wait on you with a made-up question, in
  // the demo and the test instance only, whose actors are made up too; and
  // clear (the server's, which undoes every simulation) stops it waiting.
  // actor-tunnel gives an agent a made-up remote connection through a
  // tunnel, with an adapter connected, and actor-tunnel-down the same with
  // none, so the map draws its gate open or shut (demo and test only; the
  // server's actor-read sends a read through it); clear takes it away.
  simulate(kind, { a, b } = {}) {
    // A made-up wait on you, or its end, set without a report: a report
    // counts as a heartbeat and could change how present the actor is.
    const waits = (actor, needsYou) => {
      actor.needsYou = needsYou;
      actor.dirty = true;
      this.#changed();
    };
    if (kind === "clear") {
      if (!this.demo) return null;
      for (const actor of this.actors.values()) if (actor.simulatedOffline) {
        delete actor.simulatedOffline;
        this.report(actor.id, { instance: "main", presence: "working" });
      }
      for (const actor of this.actors.values()) if (actor.needsYou) waits(actor, null);
      for (const actor of this.actors.values()) if (actor.simulatedRemote) {
        delete actor.simulatedRemote;
        this.setRemote(actor.id, null);
      }
      return { ok: true, kind };
    }
    a ??= this.ids().find((id) => this.actors.get(id).kind === "agent") ?? (this.demo ? YOU : null);
    if (!a || !this.actors.has(a) || !this.isPublic(a)) return null;
    const session = b ?? this.map?.recentSession?.() ?? null;
    switch (kind) {
      case "actor-spawn":
        b = session;
        this.record(a, { kind: "spawned", target: b });
        return { ok: true, kind, a, b };
      case "actor-edit": {
        const d = session ? this.map?.describe?.(session) : null;
        b ??= d?.kind === "session" ? (d.placeId ?? session) : session;
        this.record(a, { kind: "edited", target: b, count: 1 });
        return { ok: true, kind, a, b };
      }
      case "actor-message":
        b = session;
        this.record(a, { kind: "said", target: b, count: 1 });
        return { ok: true, kind, a, b };
      case "actor-online":
        delete this.actors.get(a).simulatedOffline;
        this.report(a, { instance: "main", presence: "working" });
        return { ok: true, kind, a };
      case "actor-offline": {
        const actor = this.actors.get(a);
        if (this.demo) actor.simulatedOffline = true;
        if (!actor.instances.size) actor.instances.set("main", { presence: "offline", at: this.now(), heartbeatMs: actor.heartbeatMs, pid: null });
        for (const inst of actor.instances.values()) inst.presence = "offline";
        this.report(a, { instance: [...actor.instances.keys()][0] });
        return { ok: true, kind, a };
      }
      case "actor-tunnel":
      case "actor-tunnel-down": {
        const actor = this.actors.get(a);
        if (!this.demo || actor.kind !== "agent") return null;
        actor.simulatedRemote = true;
        this.setRemote(a, { simulated: true, active: true, connections: 1, bridge: kind === "actor-tunnel" ? "connected" : "not_connected", accountLink: "unverified", permissions: ["read"], transports: ["openai-secure-mcp-tunnel"] });
        return { ok: true, kind, a };
      }
      case "actor-needs-you": {
        const actor = this.actors.get(a);
        if (!this.demo || actor.kind !== "agent") return null;
        waits(actor, { kind: "question", text: "Should the refunds fix go out tonight, or wait for the review?", since: this.now() });
        return { ok: true, kind, a };
      }
      default:
        return null;
    }
  }
}

// What the map knows, from XenoNet's state (state.js): whether it's there
// yet, whether a node is on it, what a node is and where it lives (never a
// session's title), the project or folder holding a path, and the session
// busy most lately, for simulations.
export function mapOf(state) {
  // Places by their path key, and the sessions, made again when the graph is.
  let places = null;
  let placesOf;
  const byPath = () => {
    if (placesOf !== state.graph) {
      placesOf = state.graph;
      places = new Map();
      for (const n of state.index.values()) if ((n.kind === "project" || n.kind === "zone") && n.path) places.set(pathKey(n.path), n.id);
    }
    return places;
  };
  let sessions = [];
  let sessionsOf;
  return {
    ready: () => !!state.graph,
    has: (id) => state.index.has(id),
    // The sessions on the map as the observers match them to what started
    // them (core/actors/attribution.js): their app, folder and worktree,
    // when each began, and Codex's originator. Never a title.
    sessions() {
      if (sessionsOf !== state.graph) {
        sessionsOf = state.graph;
        sessions = [...state.index.values()].filter((n) => n.kind === "session").map((n) => ({ id: n.id, app: n.app ?? null, cwd: n.cwd ?? null, worktree: n.worktree ?? null, createdAt: n.createdAt ?? null, originator: n.originator ?? null }));
      }
      return sessions;
    },
    recentSession() {
      const live = [...(state.live?.sessions?.values() ?? [])].filter((s) => state.index.has(s.id)).sort((x, y) => (y.lastEventAt ?? 0) - (x.lastEventAt ?? 0));
      return live[0]?.id ?? [...state.index.values()].find((n) => n.kind === "session")?.id ?? null;
    },
    describe(id) {
      const n = state.index.get(id);
      if (!n) return null;
      if (n.kind !== "session") return { kind: n.kind, app: n.app ?? null, name: n.name ?? null, place: null, placeId: null, path: n.path ?? null };
      const placeId = state.homeOf(id);
      return { kind: "session", app: n.app ?? null, name: null, place: placeId ? (state.index.get(placeId)?.name ?? null) : null, placeId, path: null };
    },
    // The deepest project or folder holding an absolute path, as the engine
    // places a session (#placeOf in core/engine/engine.js): its id, or null.
    placeOf(p) {
      const map = byPath();
      for (let k = pathKey(p); k; k = k.slice(0, Math.max(0, k.lastIndexOf("/")))) {
        const id = map.get(k);
        if (id) return id;
      }
      return null;
    },
  };
}

// The actors for this XenoNet, kept in `store` (the engine's): you, and in
// the demo its made-up ones (demo.js).
export function createActors({ store, demo = false, now = Date.now, map = null } = {}) {
  const actors = new Actors({ store, now, map, demo });
  if (demo) addDemoActors(actors);
  return actors;
}
