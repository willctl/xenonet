// What a remote actor's reads say of it (docs/design/cloud-assistants.md,
// section 3), put to the actors' registry the way its other sources are,
// without taking their place:
//
//   - a read that went out says the actor was at work then: its
//     connection's own instance (remote-<connection>) says working, for a
//     read's lease (REMOTE_LEASE_MS), then it's out of sight (unknown),
//     never offline. An observer that sees it at work, or a registration
//     that reports it, outranks a lapsed lease, as the most present
//     instance always wins (Actors#presenceOf);
//   - a read of one project, or one session, ties the actor to it as
//     watches, which the map draws as a beam;
//   - that's all: no event of its own, and nothing it did elsewhere.
//
// Polling shouldn't flood the feed or the database: an instance's word is
// taken at most every 15 seconds, and a watches target touched at most
// once a minute. A presence that changes is one event, as for any actor.
//
// The map shows the reads themselves too, light passing through the
// actor's tunnel to what it read: `onRead` hears of them, at most one an
// actor a second, and keeps nothing (server/index.js sends them to the
// page as remote-read).
import { REMOTE_LEASE_MS } from "./index.js";

const PRESENCE_MS = 15e3;
const TARGET_MS = 60e3;
const SHOWN_MS = 1000;
const REACH_MAX = 8;

export const remoteInstance = (connectionId) => `remote-${String(connectionId).replace(/^cn_/, "").slice(0, 12)}`;

export function createRemotePresence({ actors, now = () => actors.now(), onRead = null }) {
  const said = new Map();
  const watched = new Map();
  const shown = new Map();
  return {
    // A read through `connectionId` went out for `actorId`, about
    // `targets` (node ids on the map, or none), having read `reach` (the
    // places it covered, for the map's light only).
    read(actorId, connectionId, { targets = [], reach = targets, at = now() } = {}) {
      if (!actors.has(actorId)) return;
      if (onRead && at - (shown.get(actorId) ?? -Infinity) >= SHOWN_MS) {
        shown.set(actorId, at);
        try {
          onRead({ actor: actorId, connection: connectionId, reach: [...new Set(reach)].slice(0, REACH_MAX), at });
        } catch {}
      }
      const key = `${actorId}|${connectionId}`;
      if (at - (said.get(key) ?? -Infinity) >= PRESENCE_MS) {
        said.set(key, at);
        actors.report(actorId, { instance: remoteInstance(connectionId), presence: "working", heartbeatMs: null, leaseMs: REMOTE_LEASE_MS, via: "remote", at });
      }
      for (const target of targets.slice(0, 4)) {
        const k = `${actorId}|${target}`;
        if (at - (watched.get(k) ?? -Infinity) < TARGET_MS) continue;
        watched.set(k, at);
        try {
          actors.target(actorId, target, "watches", at);
        } catch {}
      }
      if (said.size > 1024) said.clear();
      if (shown.size > 1024) shown.clear();
      if (watched.size > 4096) watched.clear();
    },
  };
}
