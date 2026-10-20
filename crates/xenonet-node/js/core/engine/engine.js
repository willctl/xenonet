// core/engine/engine.js in Rust (crates/xenonet-engine/src/engine.rs):
// what the agents on this machine are doing, and the means for them to
// work together: views of them, peers, edit warnings, messages, claims,
// focus and hooks. Every decision and every line agents read are Rust's.
// The twin keeps the events, the promises inboxes and listening hooks wait
// on, the git reads behind Repos (a cache of promises), and narrow readers
// of the map's state, which belongs to the state's own layer.
import { EventEmitter } from "node:events";
import { execFile } from "node:child_process";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { limiter } from "../../../../../core/projects.js";

const ACTIVE_MS = 2 * 3600e3;
const CLAIM_MINUTES = 60;

export const isAgentId = (id) => native.engineIsAgentId(id);
export const isActorAgent = (id) => native.engineIsActorAgent(id);
export const matches = (pattern, file) => native.engineMatches(pattern, file);
export const editPaths = (toolName, input, cwd) => native.engineEditPaths(toolName, input, cwd);

// The git repository a folder belongs to: its working tree's top and the
// shared git dir, which is the same for every worktree of a repo. Each
// folder's answer is kept for a minute, and at most `max` folders are
// kept: past that the stale ones go, then those read longest ago. `read`
// reads one (git, unless a test says otherwise). What's kept are the
// reads' promises, so this stays JS.
export class Repos {
  constructor({ ttlMs = 60e3, max = 512, read = readRepo } = {}) {
    this.cache = new Map();
    this.ttlMs = ttlMs;
    this.max = max;
    this.read = read;
  }
  of(cwd) {
    if (!cwd) return Promise.resolve(null);
    const hit = this.cache.get(cwd);
    if (hit && Date.now() - hit.at < this.ttlMs) return hit.value;
    const value = this.read(cwd);
    // Read again, it goes to the end, the last to be let go.
    this.cache.delete(cwd);
    this.cache.set(cwd, { at: Date.now(), value });
    if (this.cache.size > this.max) {
      const now = Date.now();
      for (const [k, e] of this.cache) if (now - e.at >= this.ttlMs) this.cache.delete(k);
      for (const k of this.cache.keys()) {
        if (this.cache.size <= this.max) break;
        this.cache.delete(k);
      }
    }
    return value;
  }
}

// Once about fifty run together, git on Windows answers nothing and still
// exits 0, so only a few run at a time.
const gitSlot = limiter(8);

export const parseRepo = (stdout) => native.engineParseRepo(String(stdout));

function readRepo(cwd) {
  return gitSlot(
    () =>
      new Promise((resolve) =>
        execFile("git", ["-C", cwd, "rev-parse", "--path-format=absolute", "--show-toplevel", "--git-common-dir", "--abbrev-ref", "HEAD"], { timeout: 3000, windowsHide: true }, (err, stdout) => {
          resolve(err ? null : parseRepo(stdout));
        }),
      ),
  );
}

// Whether a process is still running (signal 0 only checks).
function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
}

// A node of the map as the engine reads it.
const nodeOf = (n) =>
  n
    ? {
        id: n.id,
        kind: n.kind,
        title: n.title,
        name: n.name,
        worktree: n.worktree,
        cwd: n.cwd,
        updatedAt: n.updatedAt,
        branch: n.branch,
        reader: n.reader,
        observed: n.observed,
        path: n.path,
        archived: n.archived,
        identities: [...(n.identities ?? [])].map((l) => ({ id: l?.id })),
      }
    : null;

const unwrap = (out) => {
  if (out?.refused) throw Object.assign(new Error(out.refused.message), { status: out.refused.status, code: out.refused.code });
  return out?.done;
};

export class Engine extends EventEmitter {
  #core = new native.EngineCore();

  // `actors`: the actor registry (core/actors). `policy`: who may see and
  // message whom (core/delivery/policy.js). `delivery`: the router
  // (core/delivery/router.js), set once it's made.
  constructor({ state, store, repos = new Repos(), now = () => Date.now(), actors = null, policy = null }) {
    super();
    this.policy = policy;
    this.delivery = null;
    this.state = state;
    this.store = store;
    this.repos = repos;
    this.now = now;
    this.actors = actors;
    this.waiters = new Map(); // agent → Set(resolve), inboxes waiting for mail
    this.listening = new Map(); // agent → until when its Stop hook listens
  }

  // What the engine reads of the map, the events it tells, and what it
  // waits on. `signal`: a listening hook's request.
  #deps(signal = null) {
    const wait = (me, ms, extra) =>
      new Promise((resolve) => {
        const set = this.waiters.get(me) ?? this.waiters.set(me, new Set()).get(me);
        const off = extra?.(() => done()) ?? (() => {});
        const done = () => (clearTimeout(timer), set.delete(done), off(), resolve());
        const timer = setTimeout(done, ms);
        set.add(done);
      });
    return {
      engine: this,
      signal,
      liveIds: () => [...(this.state.live?.sessions ?? [])].map(([id, s]) => [id, s.lastEventAt]),
      index: () => [...(this.state.index ?? [])].map(([id, n]) => [id, { id: n.id, kind: n.kind, archived: n.archived, updatedAt: n.updatedAt, name: n.name, path: n.path }]),
      node: (id) => nodeOf(this.state.index?.get?.(id)),
      live: (id, now) => {
        const l = this.state.live?.sessions?.get(id) ?? this.state.observed?.live?.(id) ?? null;
        if (!l) return null;
        const edits = [...(l.edits?.values() ?? [])].map((list) => list[list.length - 1]);
        return { observed: l.observed, lastEventAt: l.lastEventAt, endedAt: l.endedAt, status: l.status(now), needsYou: l.needsYou?.(now)?.kind, edits: edits.map((e) => ({ path: e.path, ts: e.ts })) };
      },
      observedLive: (id) => !!this.state.observed?.live?.(id),
      homeOf: (id) => this.state.homeOf?.(id) ?? null,
      identities: () => [...(this.state.graph?.identities ?? [])].map((i) => ({ id: i?.id, handle: i?.handle })),
      alive: (pid) => alive(pid),
      emit: (name, payload) => this.emit(name, payload),
      wakeWaiters: (id) => {
        for (const wake of this.waiters.get(id) ?? []) wake();
      },
      wait: (me, ms) => wait(me, ms),
      // Until a message comes, the main Stop hook takes one, the request
      // goes, or the window ends.
      listenWait: (me, ms) =>
        wait(me, ms, (done) => {
          const seen = ({ agent }) => agent === me && done();
          signal?.addEventListener("abort", done);
          this.on("seen", seen);
          return () => (signal?.removeEventListener("abort", done), this.off("seen", seen));
        }),
      wall: () => Date.now(),
      // A turn, where the JS awaited one of its own async functions.
      tick: () => Promise.resolve(),
      timeOf: (ms) => new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }),
    };
  }

  #go(machine, signal = null) {
    return drive(machine, calls(this.#deps(signal)), null, unwrap);
  }

  agents(withinMs = ACTIVE_MS) {
    return this.#go(this.#core.agents(withinMs));
  }
  view(id) {
    return this.#go(this.#core.view(id));
  }
  static together(a, b) {
    return native.engineTogether(a, b);
  }
  whoami(me) {
    return this.#go(this.#core.whoami(me));
  }
  peers(me, { scope = "repo", withinMs = ACTIVE_MS } = {}) {
    return this.#go(this.#core.peers(me, scope, withinMs));
  }
  conflicts(me, files) {
    return this.#go(this.#core.conflicts(me, [...files]));
  }
  whoHas(me, file) {
    return this.#go(this.#core.whoHas(me, file));
  }
  send(me, { to, text, replyTo = null }, { envelope = null } = {}) {
    return this.#go(this.#core.send(me, { to, text, replyTo }, envelope));
  }
  static replyTarget(me, m) {
    return native.engineReplyTarget(me, m ?? null);
  }
  inbox(me, { waitMs = 0 } = {}) {
    return this.#go(this.#core.inbox(me, waitMs));
  }
  listen(me, { ms, signal = null } = {}) {
    return this.#go(this.#core.listen(me, ms), signal);
  }
  claim(me, { paths, note = null, minutes = CLAIM_MINUTES }) {
    return this.#go(this.#core.claim(me, { paths, note, minutes }));
  }
  release(me, { paths = null } = {}) {
    return this.#core.release(this.#deps(), me, paths);
  }
  setFocus(me, text) {
    return this.#core.setFocus(this.#deps(), me, text);
  }
  touch(me, { app, cwd, via, pid = null }) {
    this.#core.touch(this.#deps(), me, { app, cwd, via, pid });
  }
  facts(a) {
    return this.#core.facts(this.#deps(), a);
  }
  visibleTo(me, a) {
    return this.#core.visibleTo(this.#deps(), me, a ?? null);
  }
  hook(app, event, input = {}) {
    return this.#go(this.#core.hook(app, event, input));
  }
  briefing(me) {
    return this.#go(this.#core.briefing(me));
  }
  snapshot() {
    return this.#go(this.#core.snapshot());
  }
}
