// What every observer of an actor product shares (Hermes Agent's, Grok
// Bot's; docs/design/actors/interface.md, sections 1 and 7). An observer is
// a building block (core/blocks) whose health /api/health lists, and it
// keeps its product's actors in the registry (index.js) the way section 1
// has it:
//
//   keep(id, …)       the actor itself, made once and kept current; a
//                     registered actor's name, heartbeat and capabilities
//                     are what you allowed, so the observer only adds how it
//                     knows the actor; and whether its product is installed,
//                     which keeps it listed however long it hasn't run
//   say(id, …)        what it sees of it (presence and status), said when it
//                     changes, and every 30 s besides while it's there, so
//                     it stays heard from (lastSeen is when it last was); a
//                     registered actor says its own status
//   gather(id, …)     what it did, gathered per minute, each minute recorded
//                     once (happened), even across a restart; once the actor
//                     is registered, events are left to its own reports and
//                     the observer keeps presence and attribution ("nothing
//                     counts twice")
//   touch(id, …)      the places it works in, at most once a minute
//
// An observer looks every few seconds while its product's observer is on
// and the map is there, and reads nothing while it's off.
import crypto from "node:crypto";
import { Block } from "../blocks/lifecycle.js";
import { CAPABILITIES } from "./registration.js";

export const LOOK_MS = 5000;
export const KEEPALIVE_MS = 30e3;
const TOUCH_MS = 60e3;
const MINUTE_MS = 60e3;
// A minute's events wait this long past its end for rows written late.
const GRACE_MS = 15e3;
const WEEK_MS = 7 * 24 * 3600e3;
const RECORDED_MAX = 4096;
// The presences an actor is heard from in; offline and unknown are said
// once.
const THERE = new Set(["working", "idle", "away"]);

// A short, stable name for a place in an event's ref.
const tag = (s) => crypto.createHash("sha256").update(String(s ?? "")).digest("hex").slice(0, 8);

export class Observer extends Block {
  // `product`: its entry in products.js. `observing()`: whether its
  // observer is on (registration.observing). `every`: how often it looks
  // by itself once started; null when something else drives its looks
  // (observers.js, so the observers look together and share one read of
  // the process table).
  constructor({ name, product, actors, observing = () => true, every = LOOK_MS, clock }) {
    super({ name, kind: "observer", clock });
    this.product = product;
    this.actors = actors;
    this.observing = observing;
    this.every = every;
    this.said = new Map();
    this.touched = new Map();
    this.recorded = new Set();
    this.buckets = new Map();
    this.timer = null;
    this.looking = null;
  }

  // ---------- the actor ----------

  keep(id, { name, version = null, via, heartbeatMs, capabilities = [], installed, at = this.clock.now() }) {
    const caps = CAPABILITIES.filter((c) => capabilities.includes(c));
    const info = this.actors.info(id);
    if (!info) {
      // Made again (you removed it, say): nothing said of the old one counts.
      this.said.delete(id);
      for (const key of this.touched.keys()) if (key.startsWith(`${id}|`)) this.touched.delete(key);
      const product = { id: this.product.id, vendor: this.product.vendor, version };
      this.actors.upsert({ id, kind: "agent", name, product, via, heartbeatMs, capabilities: caps, identity: null, enabled: this.observing(), at });
      if (installed !== undefined) this.actors.setInstalled(id, installed);
      return;
    }
    if (installed !== undefined) this.actors.setInstalled(id, installed);
    const registered = this.actors.isRegistered(id);
    const was = { name: info.name, product: info.product, via: info.via, heartbeatMs: info.heartbeatMs, capabilities: info.capabilities, identity: info.identity };
    const next = {
      name: registered ? info.name : name,
      // A version XenoNet can't read just now (an update under way) isn't
      // a version gone.
      product: { id: this.product.id, vendor: this.product.vendor, version: version ?? info.product?.version ?? null },
      via: [...info.via, ...via.filter((v) => !info.via.includes(v))],
      heartbeatMs: registered ? info.heartbeatMs : heartbeatMs,
      capabilities: registered ? info.capabilities : caps,
      identity: info.identity,
    };
    if (JSON.stringify(next) !== JSON.stringify(was)) this.actors.upsert({ id, kind: "agent", ...next, at });
  }

  // ---------- what it sees ----------

  // Presence (any of the contract's: an observer can say away and unknown)
  // and a status in XenoNet's words, or null for none. Whether it said it.
  // An actor that isn't there, of a product that isn't on this Mac, has
  // nothing to say: saying it's still gone would count as hearing from it,
  // which lists it for a week (the placeholder of a product whose observer
  // you turned on before it was installed, or a product long gone). Once
  // its product is seen, it's listed.
  say(id, { presence, status = null, at = this.clock.now() }) {
    if (!this.actors.has(id)) return false;
    if (!THERE.has(presence) && !THERE.has(this.actors.presence(id)) && this.actors.installed(id) === false) return false;
    const line = this.actors.isRegistered(id) ? undefined : status;
    const last = this.said.get(id);
    if (last && last.presence === presence && last.status === line && (!THERE.has(presence) || at - last.at < KEEPALIVE_MS)) return false;
    this.actors.report(id, { instance: "observed", presence, ...(line !== undefined ? { status: line } : {}), via: "observed", at });
    this.said.set(id, { presence, status: line, at });
    return true;
  }

  // ---------- what it did ----------

  // An event, recorded once by its `ref`, which survives a restart: an
  // observer that reads the last minutes again after one finds them
  // recorded. Attributed events (spawned) are the observer's to make even
  // for a registered actor; the rest are left to the actor's reports.
  happened(id, { ref, kind, target = null, via = "observed", at = this.clock.now(), ...facts }) {
    // One removed meanwhile (you removed it, say) is left alone until
    // the observer makes it again.
    if (!this.actors.has(id)) return null;
    if (via !== "attributed" && this.actors.isRegistered(id)) return null;
    const key = `${id}|${ref}`;
    if (this.recorded.has(key)) return null;
    this.recorded.add(key);
    if (this.recorded.size > RECORDED_MAX) this.recorded = new Set([...this.recorded].slice(-RECORDED_MAX / 2));
    if (this.actors.store.hasActorRef(id, ref, this.clock.now() - WEEK_MS)) return null;
    return this.actors.record(id, { kind, target, via, at, ref, ...facts });
  }

  // Something it did, gathered with the rest of its minute of the same kind
  // in the same place: one event with the count (and up to 20 files),
  // recorded once the minute is over (flush).
  gather(id, { kind, target = null, at, count = 1, paths = [], platform, subject }) {
    const minute = Math.floor(at / MINUTE_MS);
    const key = `${id}|${kind}|${target ?? ""}|${platform ?? ""}|${minute}`;
    const b = this.buckets.get(key) ?? { id, kind, target, platform, subject, minute, count: 0, paths: [], at };
    b.count += count;
    b.at = Math.max(b.at, at);
    for (const p of paths) if (b.paths.length < 20 && !b.paths.includes(p)) b.paths.push(p);
    this.buckets.set(key, b);
  }

  // The minutes that are over, recorded.
  flush(now = this.clock.now()) {
    for (const [key, b] of this.buckets) {
      if (now < (b.minute + 1) * MINUTE_MS + GRACE_MS) continue;
      this.buckets.delete(key);
      this.happened(b.id, {
        ref: `o:${b.kind}:${b.minute}:${tag(`${b.target ?? ""}|${b.platform ?? ""}`)}`,
        kind: b.kind,
        target: b.target,
        at: b.at,
        count: b.count,
        ...(b.paths.length ? { paths: b.paths } : {}),
        ...(b.platform ? { platform: b.platform } : {}),
        ...(b.subject ? { subject: b.subject } : {}),
      });
    }
  }

  // A place or session it acts on, touched at most once a minute.
  touch(id, nodeId, relation, at = this.clock.now()) {
    if (!nodeId || !this.actors.has(id)) return;
    const key = `${id}|${relation}|${nodeId}`;
    if (at - (this.touched.get(key) ?? -Infinity) < TOUCH_MS) return;
    this.touched.set(key, at);
    if (this.touched.size > RECORDED_MAX) this.touched.clear();
    this.actors.target(id, nodeId, relation, at);
  }

  // ---------- looking ----------

  // One look (look(), each kind's own), unless its observer is off, when
  // nothing is read. A look that throws leaves the block degraded until
  // the next one that doesn't.
  async tick(at = this.clock.now()) {
    if (!this.observing()) return false;
    // Before XenoNet's first look at the map, at start, no place could be
    // named, so what an actor did would land nowhere: it waits for it.
    if (this.actors.map?.ready?.() === false) return false;
    if (this.looking) return this.looking;
    this.looking = (async () => {
      try {
        await this.look(at);
        this.flush(at);
        this.ok(at);
      } catch (err) {
        this.problem(err, at);
      }
      return true;
    })().finally(() => (this.looking = null));
    return this.looking;
  }

  onStart() {
    if (!this.every) return;
    const run = async () => {
      if (!this.started) return;
      await this.tick();
      if (!this.started) return;
      this.timer = this.clock.setTimeout(run, this.every);
      this.timer?.unref?.();
    };
    this.timer = this.clock.setTimeout(run, 0);
    this.timer?.unref?.();
  }

  onStop() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
