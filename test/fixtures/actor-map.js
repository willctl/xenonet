// A made-up map for the actor tests, in the shape of mapOf (core/actors):
// two projects and a folder outside them in nova's home folder, and a
// session in each project. Nothing here is anyone's real data.
import { pathKey } from "../../core/paths.js";

export const HOME = "/Users/nova";
export const ATLAS = "project:/users/nova/projects/atlas-api";
export const LEDGER = "project:/users/nova/projects/ledger";
export const SCRATCH = "zone:/users/nova/scratch";
export const S1 = "claude:0190d3e0-0000-7000-8000-00000000aa01";
export const S2 = "codex:0190d3e0-0000-7000-8000-00000000aa02";
export const ATLAS_DIR = `${HOME}/Projects/atlas-api`;
export const LEDGER_DIR = `${HOME}/Projects/ledger`;
export const SCRATCH_DIR = `${HOME}/scratch`;

export const NODES = () => ({
  [ATLAS]: { kind: "project", name: "atlas-api", path: ATLAS_DIR },
  [LEDGER]: { kind: "project", name: "ledger", path: LEDGER_DIR },
  [SCRATCH]: { kind: "zone", name: "scratch", path: SCRATCH_DIR },
  [S1]: { kind: "session", app: "claude", home: ATLAS, title: "Add partial refunds" },
  [S2]: { kind: "session", app: "codex", home: LEDGER, title: "Close the month" },
});

export function fakeMap(nodes = NODES()) {
  return {
    nodes,
    ready: () => true,
    has: (id) => Object.hasOwn(nodes, id),
    describe(id) {
      const n = nodes[id];
      if (!n) return null;
      if (n.kind !== "session") return { kind: n.kind, app: null, name: n.name, place: null, placeId: null, path: n.path ?? null };
      return { kind: "session", app: n.app, name: null, place: nodes[n.home]?.name ?? null, placeId: n.home ?? null, path: null };
    },
    placeOf(p) {
      const k = pathKey(p);
      let best = null;
      for (const [id, n] of Object.entries(nodes)) {
        const nk = n.path ? pathKey(n.path) : null;
        if (nk && (k === nk || k.startsWith(`${nk}/`)) && (!best || nk.length > best.len)) best = { id, len: nk.length };
      }
      return best?.id ?? null;
    },
    recentSession: () => S1,
    // The sessions, as mapOf gives the observers them.
    sessions: () => Object.entries(nodes).filter(([, n]) => n.kind === "session").map(([id, n]) => ({ id, app: n.app ?? null, cwd: n.cwd ?? null, worktree: n.worktree ?? null, createdAt: n.createdAt ?? null, originator: n.originator ?? null })),
  };
}
