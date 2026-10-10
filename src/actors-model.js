// What the actors layer (actors.js) draws, worked out from the actors' list
// (an actor/list/v1, contracts section 3) without drawing anything: each
// actor's color in the theme, how present it looks, which of its targets
// get a beam and how bright, and where it floats. Plain functions of plain
// data, so the rules can be tested without a page.

import { isRemote } from "./hosts.js";

export const BEAMS_MAX = 24;
// Beams one actor may have at once, so a busy one can't crowd out the rest.
export const BEAMS_PER_ACTOR = 8;

// How present an actor looks. Enums are open sets: a presence the console
// doesn't know is drawn as unknown.
//   light    how bright it is
//   breathe  how much of the theme's breathing it shows
//   beams    how bright its beams are; 0 draws none
//   scale    its size
//   lift     how far above the others' height it floats
export const PRESENCE = {
  working: { light: 1, breathe: 1, beams: 1, scale: 1, lift: 0 },
  idle: { light: 0.72, breathe: 0.65, beams: 0.62, scale: 0.94, lift: -4 },
  away: { light: 0.46, breathe: 0.35, beams: 0.38, scale: 0.88, lift: -8 },
  offline: { light: 0.2, breathe: 0, beams: 0, scale: 0.78, lift: -20 },
  unknown: { light: 0.3, breathe: 0.2, beams: 0, scale: 0.82, lift: -14 },
};
export const presenceOf = (actor) => PRESENCE[actor?.presence] ?? PRESENCE.unknown;

// How long after it was last touched a target still gets a beam: a while
// for what an actor did (spawned, edited, messaged, reviewed), longer for
// where it works and what it watches, which stand while it's there.
export const LIVE_MS = { spawned: 30 * 60e3, edited: 30 * 60e3, messaged: 20 * 60e3, reviews: 30 * 60e3, "works-on": 2 * 3600e3, watches: 2 * 3600e3 };
const LIVE_DEFAULT = 20 * 60e3;

// A number from a string that stays the same for the same string, so an
// actor keeps its color and place from one start to the next: FNV-1a, its
// bits then mixed by MurmurHash3's finalizer, since FNV alone tells short,
// similar names apart badly in its low bits (two of the demo's three
// agents took the same color).
export function hashOf(text) {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 16777619);
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return h >>> 0;
}
// The same as a fraction, 0 to 1, with a salt for each use.
export const fractionOf = (text, salt = "") => hashOf(`${salt}${text}`) / 4294967296;

// An actor's color in a theme: the person at the console takes
// colors.human, every other actor one of actors.palette by a stable hash of
// its id (contracts section 4).
export function colorOf(actor, theme) {
  if (actor.kind === "human") return theme.colors.human;
  const palette = theme.actors?.palette ?? [];
  return palette.length ? palette[hashOf(actor.id) % palette.length] : theme.colors.actor;
}

// The form it's drawn in: the theme's for agents, the human form for you.
export const formOf = (actor, theme) => (actor.kind === "human" ? theme.actors?.human?.form : theme.actors?.form) ?? "orb";

// Its beams at `now`: one per node it acts on, while it's there (working,
// idle or away) and the node was touched lately enough for the relation
// (LIVE_MS). Several relations to one node make one beam, as bright as
// the freshest. `drawn(id)` says whether a node is on the map to land on.
// Each: { node, relations, relation (the latest), lastAt, strength 0 to 1 }.
export function beamsOf(actor, now, drawn = () => true) {
  const look = presenceOf(actor);
  if (!look.beams) return [];
  const byNode = new Map();
  for (const t of actor.targets ?? []) {
    if (!t?.id || !drawn(t.id)) continue;
    const window = LIVE_MS[t.relation] ?? LIVE_DEFAULT;
    // Not yet, when a replay's clock is before it began; between its
    // first and last touch, as fresh as can be.
    if ((t.since ?? t.lastAt ?? 0) > now + 60e3) continue;
    const age = now - Math.min(t.lastAt ?? t.since ?? 0, now);
    if (age > window) continue;
    // Fresh is bright, fading to a third by the end of its window.
    const fresh = 1 - Math.max(0, age) / window;
    const strength = look.beams * (0.34 + 0.66 * fresh * fresh);
    const had = byNode.get(t.id);
    if (!had) byNode.set(t.id, { node: t.id, relations: [t.relation], relation: t.relation, lastAt: t.lastAt ?? 0, strength });
    else {
      had.relations.push(t.relation);
      if ((t.lastAt ?? 0) > had.lastAt) [had.relation, had.lastAt] = [t.relation, t.lastAt ?? 0];
      had.strength = Math.max(had.strength, strength);
    }
  }
  return [...byNode.values()].sort((a, b) => b.strength - a.strength || b.lastAt - a.lastAt).slice(0, BEAMS_PER_ACTOR);
}

// Every actor's beams, the brightest BEAMS_MAX of them, as
// { actor, key, ...beam }.
export function allBeams(actors, now, drawn) {
  const out = [];
  for (const a of actors) for (const b of beamsOf(a, now, drawn)) out.push({ actor: a.id, key: `${a.id}>${b.node}`, ...b });
  return out.sort((x, y) => y.strength - x.strength || y.lastAt - x.lastAt).slice(0, BEAMS_MAX);
}

// Whether an actor reaches XenoNet through a tunnel, which the map draws as a
// gate between it and what it reads (looks/gate.js), and how open: 1 while
// an adapter is connected through one of its connections, STANDBY (a
// sliver) while none is, and null for no gate, with no active connection
// through a tunnel.
export const TUNNELS = new Set(["openai-secure-mcp-tunnel"]);
export const STANDBY = 0.16;
export function gateOf(actor) {
  const r = actor?.remote;
  if (!r?.active || !(r.transports ?? []).some((x) => TUNNELS.has(x))) return null;
  return r.bridge === "connected" ? 1 : STANDBY;
}

// Where an actor's gate sits: between it and what it beams into, `gap`
// along the way to their weighted middle but never more than `share` of
// the way, or toward the map's middle with no beams. `from` is the actor
// ([x, y, z]), `targets` [[x, y, z, weight]…], `center` the map's middle.
// Returns { at: [x, y, z], dir: [x, y, z] } with dir a unit vector.
export function gateSpot(from, targets, center, { gap = 30, share = 0.45 } = {}) {
  let [x, y, z] = center;
  const pull = targets.reduce((w, t) => w + t[3], 0);
  if (pull > 0) {
    (x = 0), (y = 0), (z = 0);
    for (const [tx, ty, tz, k] of targets) (x += tx * k), (y += ty * k), (z += tz * k);
    (x /= pull), (y /= pull), (z /= pull);
  }
  let dx = x - from[0], dy = y - from[1], dz = z - from[2];
  let d = Math.hypot(dx, dy, dz);
  // Right over its middle: straight down.
  if (d < 1e-3) (dx = 0), (dy = -1), (dz = 0), (d = 1);
  const step = Math.min(gap, d * share);
  (dx /= d), (dy /= d), (dz /= d);
  return { at: [from[0] + dx * step, from[1] + dy * step, from[2] + dz * step], dir: [dx, dy, dz] };
}

// The drawn map's middle and reach, which actors float around: this
// machine's nodes only, since actors are this machine's, and another
// machine's constellation sits on an orbit of its own (hosts.js). `at(n)`
// is where a node is (a gathered one, where it lives). Null with none.
export function boundsOf(nodes, at = (n) => n) {
  const all = nodes.filter((n) => !isRemote(n)).map(at);
  if (!all.length) return null;
  let x = 0, y = 0, z = 0;
  for (const p of all) (x += p.x), (y += p.y), (z += p.z);
  (x /= all.length), (y /= all.length), (z /= all.length);
  let r = 0;
  for (const p of all) r = Math.max(r, Math.hypot(p.x - x, p.y - y, p.z - z));
  return { center: [x, y, z], radius: r };
}

// Where each actor floats: on a layer of their own over the map, hovering
// above what it acts on. `bounds` is the drawn map's { center: [x, y, z],
// radius }; each actor comes with the positions of the nodes it beams into
// and how strongly ([[x, y, z, weight]…]). Returns id → [x, y, z].
//
// - An actor with beams hovers HOVER above their weighted middle, drawn a
//   little in toward the map's middle so it stays over the map.
// - One without waits on a ring round the map, a little above its middle,
//   at a place its id picks; you wait nearer the middle.
// - One that isn't there hangs lower (PRESENCE's lift), and each has a
//   little height of its own, so they don't line up.
// - Then they're eased apart until none is closer than GAP to another.
export const HOVER = 50, GAP = 44;
export function placeActors(actors, bounds) {
  const [cx, cy, cz] = bounds.center;
  const r = Math.max(60, bounds.radius);
  const spots = actors.map((a) => {
    const turn = fractionOf(a.id, "place") * Math.PI * 2;
    const own = fractionOf(a.id, "height") * 16 + presenceOf(a).lift;
    const pull = a.targets?.reduce((w, [, , , k]) => w + k, 0) ?? 0;
    if (pull > 0) {
      let x = 0, y = 0, z = 0;
      for (const [tx, ty, tz, k] of a.targets) (x += tx * k), (y += ty * k), (z += tz * k);
      (x /= pull), (y /= pull), (z /= pull);
      // In toward the middle a little, and a nudge of its own so two
      // actors on the same project don't sit on top of each other.
      x += (cx - x) * 0.12 + Math.cos(turn) * 8;
      z += (cz - z) * 0.12 + Math.sin(turn) * 8;
      return { id: a.id, x, y: y + HOVER + own, z };
    }
    const ring = a.kind === "human" ? r * 0.3 : r * 0.82;
    return { id: a.id, x: cx + Math.cos(turn) * ring, y: cy + r * 0.3 + own, z: cz + Math.sin(turn) * ring };
  });
  for (let pass = 0; pass < 8; pass++) {
    let moved = false;
    for (let i = 0; i < spots.length; i++)
      for (let j = i + 1; j < spots.length; j++) {
        const a = spots[i], b = spots[j];
        let dx = b.x - a.x, dy = b.y - a.y, dz = b.z - a.z;
        let d = Math.hypot(dx, dy, dz);
        if (d >= GAP) continue;
        // Two on the same spot part sideways, along a direction their ids
        // pick.
        if (d < 1e-3) {
          const turn = fractionOf(`${a.id}|${b.id}`, "part") * Math.PI * 2;
          (dx = Math.cos(turn)), (dy = 0), (dz = Math.sin(turn)), (d = 1);
        }
        const push = (GAP - d) / 2 / d;
        (a.x -= dx * push), (a.y -= dy * push), (a.z -= dz * push);
        (b.x += dx * push), (b.y += dy * push), (b.z += dz * push);
        moved = true;
      }
    if (!moved) break;
  }
  return new Map(spots.map((s) => [s.id, [s.x, s.y, s.z]]));
}

// What the pointer takes where an actor and a node may both be under it:
// whichever is drawn in front, the nearer along the pointer's ray. `actor`
// is how far along the ray the actor is hit, and `body` where the node's
// own body starts, each Infinity when the ray misses it; a node's glow
// alone never stands in front of an actor. "actor", "node" or null.
export function pointerTakes(actor, body) {
  if (actor === Infinity && body === Infinity) return null;
  return actor < body ? "actor" : "node";
}

// An actor's message to agents comes over the event stream twice: as the
// message itself (agent-message), with its text and every agent it
// reached, and then as the said event XenoNet writes of it (actor-event).
// The page draws and lists it once, from the message: spoke(actor) notes a
// message as it comes, and echo(event) says whether a said event is the one
// a message just drew.
export function createSaidEcho({ windowMs = 3000, now = Date.now } = {}) {
  const spoke = new Map();
  return {
    spoke(actor) {
      spoke.delete(actor);
      spoke.set(actor, now());
      if (spoke.size > 64) spoke.delete(spoke.keys().next().value);
    },
    echo: (e) => e?.kind === "said" && now() - (spoke.get(e.actor) ?? -Infinity) < windowMs,
  };
}

// Whether the actors' list means drawing the map again (main.js): an actor
// acts on a place that's on the map but not drawn, which the map then
// draws whatever the scope. Some never are drawn that way: a folder or a
// Codex project shows only while something in it does (scope.js). So a
// place the last redraw left undrawn doesn't ask again with every list;
// only one newly undrawn does. `known(id)` says whether a place is on the
// map at all, `drawn(id)` whether it's drawn; drew() goes after a redraw.
export function createTargetWatch() {
  let tried = new Set();
  const undrawn = (targets, { known, drawn }) => targets.filter((id) => known(id) && !drawn(id));
  return {
    wants: (targets, map) => undrawn(targets, map).some((id) => !tried.has(id)),
    drew: (targets, map) => void (tried = new Set(undrawn(targets, map))),
  };
}

// You, as the page sees you (docs/design/actors/interface.md, section 6).
// Outside the demo, the list has you as a placeholder, not heard from,
// and the page draws you from what it sees itself: working with input in
// the last two minutes, idle up to `awayMs` (ten minutes), away after
// that; with the window hidden (`hiddenMs` is how long), away, and offline
// once it's been hidden `awayMs`, as XenoNet has you then. What you've
// looked at and done comes from your record (`record`, which only this
// window reads), so your beams show, after a reload too; who you are stays
// the placeholder's, never named. The list as it was otherwise.
const YOURS = ["status", "presenceSince", "lastSeen", "targets", "recent", "counts"];
export function youHere(list, { quietMs, hiddenMs = null, awayMs = 10 * 60e3, record = null }) {
  const you = list?.actors?.find((a) => a.id === "actor:you");
  if (!you || you.presence !== "unknown" || you.via?.includes("demo")) return list;
  const presence = hiddenMs != null ? (hiddenMs >= awayMs ? "offline" : "away") : quietMs >= awayMs ? "away" : quietMs < 2 * 60e3 ? "working" : "idle";
  const mine = record?.id === "actor:you" ? Object.fromEntries(YOURS.filter((k) => record[k] !== undefined).map((k) => [k, record[k]])) : {};
  if (mine.targets && !Array.isArray(mine.targets)) delete mine.targets;
  if (mine.recent && !Array.isArray(mine.recent)) delete mine.recent;
  return { ...list, actors: list.actors.map((a) => (a === you ? { ...a, ...mine, presence } : a)) };
}

// Words for what an actor's relation to a node is, how XenoNet knows of it,
// how present it is and what it may do (after "can" or "asks to"), for the
// inspector, the tooltip, the Actors panel and the request card.
export const RELATION_WORDS = { "works-on": "works on", spawned: "started", edited: "edited", messaged: "messaged", watches: "watches", reviews: "reads" };
export const VIA_WORDS = { observed: "XenoNet reads its state", attributed: "XenoNet traces work here back to it", registered: "it reports itself", remote: "it reads through a remote connection you paired", demo: "made up, for the demo" };
// The routes a remote actor's connections take (core/actors/adapters.js).
export const TRANSPORT_WORDS = { "openai-secure-mcp-tunnel": "OpenAI Secure MCP Tunnel", "local-stdio": "a local MCP client" };
export const PRODUCT_NAMES = { "hermes-agent": "Hermes Agent", "grok-bot": "Grok Bot", muse: "Muse", dots: "dots" };
export const PRESENCE_WORDS = { working: "working", idle: "idle", away: "away", offline: "offline", unknown: "not heard from" };
// An actor's presence in a word. Unknown is "not heard from" until XenoNet
// has heard from it once, and "out of sight" after: Grok Bot with its app
// closed, which nothing here can say has stopped.
export const presenceWord = (s) => (s?.presence === "unknown" || !PRESENCE_WORDS[s?.presence] ? (s?.lastSeen ? "out of sight" : PRESENCE_WORDS.unknown) : PRESENCE_WORDS[s.presence]);
export const CAPABILITY_WORDS = { "spawn-sessions": "start sessions", "edit-repos": "edit repos", "run-commands": "run commands", "message-people": "message people", "message-agents": "message SI", schedule: "run on a schedule", browse: "browse the web", "use-computer": "use apps" };

// What the Actors panel offers for an actor (an actor/snapshot/v1), in a
// window that can use its own routes (`canDecide`: it holds the window key)
// or can't (a browser tab). `observed` is the products XenoNet has an observer
// for.
//   you       the person at the console, who has Show me on the map instead
//   token     New token: a registered actor's
//   remove    Remove: a registered actor, or one XenoNet knows no other way.
//             An observed one would be back at its observer's next look,
//             so its observer's switch is how to keep it away
//   observer  the product whose observer it falls under, or null
// The products whose agents work in their makers' clouds, which reach XenoNet
// through its actor gateway (server/actor-gateway.js), even before they
// report themselves: a gateway token registers one XenoNet only observes.
export const GATEWAY_PRODUCTS = new Set(["grok-bot", "muse"]);

export function actorActions(s, { canDecide = false, observed = new Set() } = {}) {
  const you = s?.kind === "human" || s?.id === "actor:you";
  const via = s?.via ?? [];
  const registered = via.includes("registered");
  const product = s?.product?.id ?? null;
  const observer = !you && product && observed.has(product) && (via.includes("observed") || via.includes("attributed")) ? product : null;
  const demo = via.includes("demo");
  return { you, token: canDecide && !you && registered, remove: canDecide && !you && (registered || !observer), observer, gateway: canDecide && !you && !demo && (registered || GATEWAY_PRODUCTS.has(product)) };
}

// Actors waiting on you (a snapshot's needsYou, contracts section 3),
// which reach the attention strip as sessions waiting on you do: each with
// its name, how it waits (its kind, an open set: question, permission,
// review), its words and since when, the latest first. Its words are the
// actor's own, checked by the server like a status.
const NEED_WORDS = { question: "asks", permission: "needs permission", review: "is ready for review" };
export function actorNeeds(list) {
  return (list?.actors ?? [])
    .filter((a) => a?.needsYou && a.kind !== "human")
    .map((a) => ({ kind: "actor-need", id: a.id, name: a.name, sub: NEED_WORDS[a.needsYou.kind] ?? "waits for you", text: a.needsYou.text || "Waiting for you", since: a.needsYou.since ?? null }))
    .sort((x, y) => (y.since ?? 0) - (x.since ?? 0));
}

// The actors as the panel lists them: you first, then by name, so a row
// never moves under the pointer when an actor's presence changes.
export const actorOrder = (a, b) => (b.kind === "human") - (a.kind === "human") || String(a.name).localeCompare(String(b.name)) || String(a.id).localeCompare(String(b.id));
