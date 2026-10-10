// What a registered actor reports, with its token (docs/design/actors/
// interface.md, section 2: Heartbeat, Events, Targets), taken into the
// registry (index.js) a part at a time. A part that can't be taken is
// dropped with a reason and doesn't fail the rest.
//
// An actor reports facts, never text: XenoNet writes every event's detail
// from them. Its paths land on the project or folder holding them, and a
// path outside every place XenoNet knows is dropped, so an actor can't add
// places to the map. Each kind and relation needs the capability you
// allowed it, and on macOS a claim to have spawned a session held by a
// process outside the actor's own is dropped, while a spawned report that
// names no session waits two minutes for one to start under the actor's
// own processes (Actors#awaitSession).
import path from "node:path";
import { pathStyle } from "../paths.js";
import { statusLine } from "./detail.js";
import { nodeKind } from "./index.js";
import { ActorError } from "./registration.js";

const DAY_MS = 24 * 3600e3;
const AHEAD_MS = 60e3;
const KINDS = new Set(["spawned", "edited", "ran", "messaged", "finished", "failed"]);
// The capability each kind of event, and each relation, needs.
const NEEDS = { spawned: "spawn-sessions", edited: "edit-repos", ran: "run-commands", messaged: "message-people" };
const PLATFORMS = new Set(["telegram", "discord", "slack", "whatsapp", "signal", "matrix", "mattermost", "email", "sms", "imessage", "teams", "other"]);
const SUBJECTS = new Set(["session", "task", "job", "turn", "command"]);
const APPS = new Set(["claude", "codex", "other"]);
// The relations an actor may set (messaged is the engine's), and what each
// points at (the interface's table of relations).
const POINTS_AT = {
  "works-on": ["project", "native", "zone"],
  spawned: ["session"],
  edited: ["project", "zone"],
  watches: ["project", "native", "zone", "session"],
  reviews: ["session", "project"],
};
const WORDS = { project: "a project", native: "a project", zone: "a folder", session: "a session" };
// A session's id, as the engine has them (isAgentId in core/engine/engine.js).
const SESSION = /^(claude|codex):[0-9A-Za-z][0-9A-Za-z-]{6,}$/;

// A node of the map from a report's `target` (a node id) or `path` (an
// absolute path, landing on the project or folder holding it): { id }, or
// { why } it can't be. A session not on the map yet is given two minutes
// (the registry keeps it that long), in case its transcript is on the way.
function nodeOf(actors, { target, path: p }) {
  if (target != null) {
    const kind = nodeKind(target);
    if (!kind || kind === "actor") return { why: "a target is a project, folder or session on XenoNet's map, never an actor" };
    if (kind === "session" && !SESSION.test(target)) return { why: "that isn't a session's id" };
    // Before XenoNet's first look at the map, at start, every node is taken;
    // the registry lets go of the ones that never turn up.
    const ready = actors.map?.has && actors.map.ready?.() !== false;
    if (kind !== "session" && ready && !actors.map.has(target)) return { why: "XenoNet doesn't know that node" };
    return { id: target };
  }
  if (p != null) {
    if (!path.isAbsolute(p)) return { why: "a path is absolute" };
    const id = actors.placeOf(p);
    return id ? { id } : { why: "that path isn't in a project or folder XenoNet knows" };
  }
  return { id: null };
}

// Whether a session the actor says it spawned runs under its own
// processes (`own`, a set of pids): the session's pins (the processes XenoNet
// saw holding it) against their lineage. Null when XenoNet can't tell: not
// macOS, no pins, or the actor's processes unknown.
async function holdsSession(actors, session, { own, lineage }) {
  if (!own?.size || !lineage) return null;
  let seen = false;
  for (const pin of actors.store.pins(session)) {
    const line = await lineage(pin.pid);
    if (line == null) return null;
    if (!line.length) continue;
    seen = true;
    if (line.some((pid) => own.has(pid))) return true;
  }
  return seen ? false : null;
}

// A heartbeat (POST /api/actors/heartbeat): presence, a status, the folder
// it works in and its process, for one of its instances. A status that
// looks like it holds a key is refused (400); a folder outside every place
// XenoNet knows is left out. Paths are kept as they resolve (no .. in them),
// the form placeOf judged.
export function takeHeartbeat(actors, id, body, { now = actors.now() } = {}) {
  if (body.status !== undefined) {
    const checked = statusLine(body.status);
    if (!checked.ok) throw new ActorError(400, "bad-request", checked.reason);
  }
  if (body.cwd !== undefined && !path.isAbsolute(body.cwd)) throw new ActorError(400, "bad-request", "cwd is an absolute path");
  const cwd = body.cwd === undefined ? undefined : actors.placeOf(body.cwd) ? pathStyle(body.cwd).resolve(body.cwd) : null;
  actors.report(id, { instance: body.instance ?? "main", presence: body.presence, status: body.status, pid: body.pid, cwd, via: "registered", at: now });
}

// What it did (POST /api/actors/events), each as facts: { accepted,
// dropped: [{ index, reason }] }. `own` and `lineage`: its processes and how
// to read a process's lineage, for the sessions it says it spawned.
// `skip`: the indexes already dropped (ones that don't fit the schema).
export async function takeEvents(actors, id, events, { now = actors.now(), own = null, lineage = null, skip = null } = {}) {
  const dropped = [];
  const refs = new Set();
  let accepted = 0;
  for (const [index, e] of events.entries()) {
    if (e == null || skip?.has(index)) continue;
    const drop = (reason) => dropped.push({ index, reason });
    if (!KINDS.has(e.kind)) {
      drop("XenoNet doesn't take that kind in a report: presence and status come from heartbeats, and said from the engine");
      continue;
    }
    const need = NEEDS[e.kind];
    if (need && !actors.can(id, need)) {
      // A message to someone outside XenoNet is easily mistaken for one to an
      // agent, which is the engine's, so the refusal says which is which.
      const aside = e.kind === "messaged" ? " (messaged is a message to someone outside XenoNet, by email or a chat app; to write to a session or another actor, use xenonet actor send)" : "";
      drop(`${e.kind} needs the ${need} capability, which it wasn't allowed${aside}`);
      continue;
    }
    let at = e.at ?? now;
    if (now - at > DAY_MS) {
      drop("that was more than a day ago");
      continue;
    }
    if (at - now > AHEAD_MS) at = now;
    if (e.ref != null && (refs.has(e.ref) || actors.store.hasActorRef(id, e.ref, now - DAY_MS))) {
      drop("an event with that ref was reported already");
      continue;
    }
    // Edited files: each in a place XenoNet knows.
    let paths;
    if (e.kind === "edited" && e.paths?.length) {
      if (e.paths.some((p) => typeof p !== "string" || !path.isAbsolute(p))) {
        drop("an edited path is absolute");
        continue;
      }
      if (e.paths.some((p) => !actors.placeOf(p))) {
        drop("an edited path isn't in a project or folder XenoNet knows");
        continue;
      }
      paths = e.paths.map((p) => pathStyle(p).resolve(p));
    }
    const node = nodeOf(actors, e);
    if (node.why) {
      drop(node.why);
      continue;
    }
    // Without a place of its own, an edit lands where its first file is.
    const target = node.id ?? (paths ? actors.placeOf(paths[0]) : null);
    if (e.kind === "spawned" && nodeKind(target) === "session" && (await holdsSession(actors, target, { own, lineage })) === false) {
      drop("that session runs under a process outside this actor's own");
      continue;
    }
    if (e.ref != null) refs.add(e.ref);
    const app = e.kind === "spawned" && e.app != null ? (APPS.has(e.app) ? e.app : "other") : undefined;
    actors.record(id, {
      kind: e.kind,
      target,
      via: "registered",
      at,
      ref: e.ref ?? null,
      count: e.count,
      paths,
      platform: e.kind === "messaged" ? (PLATFORMS.has(e.platform) ? e.platform : e.platform == null ? undefined : "other") : undefined,
      subject: SUBJECTS.has(e.subject) ? e.subject : undefined,
      app,
    });
    // A session it started that it didn't name: one that starts under its
    // own processes in that place in the next two minutes is the one.
    if (e.kind === "spawned" && nodeKind(target) !== "session" && app !== "other") actors.awaitSession(id, { app: app ?? null, place: target, own, at });
    accepted++;
  }
  return { accepted, dropped };
}

// What it acts on (POST /api/actors/targets): `targets` merged, or with
// `replace` the whole list of what an actor may set, and `remove` taken
// away. { accepted, dropped: [{ index, reason }] }, the indexes into
// `targets`; `skip`, the ones already dropped.
export async function takeTargets(actors, id, { targets = [], remove = [], replace = false } = {}, { now = actors.now(), own = null, lineage = null, skip = null } = {}) {
  const dropped = [];
  const keep = [];
  for (const [index, t] of targets.entries()) {
    if (t == null || skip?.has(index)) continue;
    const drop = (reason) => dropped.push({ index, reason });
    const relation = t.relation ?? "works-on";
    if (!Object.hasOwn(POINTS_AT, relation)) {
      drop(relation === "messaged" ? "messaged is the engine's to set, when the actor sends a message" : "an actor sets works-on, spawned, edited, watches or reviews");
      continue;
    }
    const need = NEEDS[relation];
    if (need && !actors.can(id, need)) {
      drop(`${relation} needs the ${need} capability, which it wasn't allowed`);
      continue;
    }
    const node = nodeOf(actors, { target: t.id, path: t.path });
    if (node.why || !node.id) {
      drop(node.why ?? "a target names a node (id) or a path");
      continue;
    }
    const kind = nodeKind(node.id);
    if (!POINTS_AT[relation].includes(kind)) {
      drop(`${relation} points at ${POINTS_AT[relation].map((k) => WORDS[k]).filter((w, i, all) => all.indexOf(w) === i).join(" or ")}, not ${WORDS[kind]}`);
      continue;
    }
    if (relation === "spawned" && (await holdsSession(actors, node.id, { own, lineage })) === false) {
      drop("that session runs under a process outside this actor's own");
      continue;
    }
    keep.push({ id: node.id, relation });
  }
  const mine = (relation) => Object.hasOwn(POINTS_AT, relation);
  if (replace) {
    const wanted = new Set(keep.map((t) => `${t.relation}|${t.id}`));
    for (const t of actors.targetsOf(id)) if (mine(t.relation) && !wanted.has(`${t.relation}|${t.id}`)) actors.untarget(id, t.id, t.relation);
  }
  for (const r of remove) {
    const node = nodeOf(actors, { target: r.id, path: r.path });
    if (!node.id) continue;
    for (const t of actors.targetsOf(id)) if (t.id === node.id && mine(t.relation) && (r.relation == null || r.relation === t.relation)) actors.untarget(id, t.id, t.relation);
  }
  for (const t of keep) actors.target(id, t.id, t.relation, now);
  return { accepted: keep.length, dropped };
}
