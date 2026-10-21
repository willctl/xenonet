// core/hosts/merge.js in Rust (crates/xenonet-hosts/src/merge.rs): other
// machines in this XenoNet's view, every id from a host qualified as
// @<host>:<its id>, its nodes hung from its host node, its times moved by
// its clock's offset, and nothing it sent passed on as it came. The merging
// is the Rust's. The schemas are zod's and graph.js's activityOf is the
// graph's, so the Rust asks for those through the two callbacks below.
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { declared } from "./declared.js";

const { activityOf } = await import(peer("graph.js"));
const { GRAPH_NODES } = await import(peer("schema/graph.js"));
const { Identity } = await import(peer("schema/identity.js"));
const { LiveSession, LiveActivity } = await import(peer("schema/live.js"));
const { EngineAgent, EngineMessage } = await import(peer("schema/engine.js"));

export const MAX_NODES = native.hostsMergeConstants().maxNodes;
export const DEFAULT_WINDOWS = { activeWindowMs: 10 * 60e3, recentWindowMs: 24 * 3600e3, projectActiveMs: 3600e3 };

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);
const own = (o, k) => (typeof k === "string" && Object.hasOwn(o, k) ? o[k] : undefined);
const SCHEMAS = { identity: Identity, "live-session": LiveSession, "live-activity": LiveActivity, "engine-agent": EngineAgent, "engine-message": EngineMessage };

// The Rust's ask: the value checked against the schema of that kind and
// copied with what it declares, or null.
const clean = (kind, value) => {
  const schema = kind.startsWith("node:") ? own(GRAPH_NODES, kind.slice(5)) : SCHEMAS[kind];
  return schema?.safeParse(value).success ? declared(schema, value) : null;
};
const activity = (updatedAt, now, windows) => activityOf(updatedAt, now, windows);

export const remoteId = (host, id) => native.hostsRemoteId(String(host), id) ?? null;

export const identityId = (host, id) => native.hostsIdentityId(String(host), id) ?? null;

export function cleanNode(n) {
  const schema = isObject(n) ? own(GRAPH_NODES, n.kind) : undefined;
  return schema?.safeParse(n).success ? declared(schema, n) : null;
}

// cleanNode remembered for one host, by each node's text. The memo is the
// Rust's, so a merge there uses it as it is.
const MEMO = Symbol("memo");
export function cleanOnce() {
  const memo = new native.HostsCleanOnce();
  const once = (n) => memo.clean(n, clean, activity);
  once.done = () => memo.done();
  once[MEMO] = memo;
  return once;
}

const memoOf = (c) => c?.[MEMO] ?? null;

// The Rust's answers are copies. What the JS kept of this machine's own by
// spreading it is put back as it was, so a caller holding one of its
// values finds that same one in the merge: every field but those `made`,
// and the first of each list, which are this machine's.
function keep(out, local, made) {
  for (const k of Object.keys(out)) if (!made.includes(k) && Object.hasOwn(local, k)) out[k] = local[k];
  return out;
}
function keepFirst(list, mine) {
  if (!Array.isArray(list) || !Array.isArray(mine)) return;
  for (let i = 0; i < mine.length && i < list.length; i++) list[i] = mine[i];
}
function keepLimits(limits, mine, replaced) {
  if (!mine || typeof mine !== "object") return limits;
  for (const k of Object.keys(limits)) if (k !== "others" && !replaced.includes(k) && Object.hasOwn(mine, k)) limits[k] = mine[k];
  return limits;
}

export function qualifyGraph(graph, host, { layout = {}, now = Date.now(), windows = DEFAULT_WINDOWS, clean: given = cleanNode } = {}) {
  const part = native.hostsQualifyGraph(graph, host, layout, Number(now), windows, memoOf(given), clean, activity);
  part.hostNode = part.nodes[0];
  return part;
}

export const sameRepo = (nodes, { cap = 8 } = {}) => native.hostsSameRepo(nodes, Number(cap));

export const mergeIdentities = (local, remotes, { names = {}, hidden = {} } = {}) => native.hostsMergeIdentities(local, remotes.map(({ host, identities }) => ({ host, identities })), names, hidden);

export const hostSummary = (host, part = null) => native.hostsHostSummary(host, part ? { nodes: part.nodes, stats: part.stats, truncated: part.truncated } : null);

export function mergeGraph(local, hosts, { layout = {}, now = Date.now(), windows = DEFAULT_WINDOWS, self = null, names = {}, hidden = {}, aliases = new Map() } = {}) {
  if (!hosts.length) return local;
  const out = native.hostsMergeGraph(
    local,
    hosts.map((h) => ({ info: h.info, snapshot: h.snapshot ?? null })),
    hosts.map((h) => memoOf(h.clean)),
    { layout, now: Number(now), windows, self, names, hidden, aliases: aliases instanceof Map ? [...aliases] : aliases },
    clean,
    activity,
  );
  keepFirst(out.links, local.links);
  keepFirst(out.relations, local.relations);
  return keep(out, local, ["nodes", "links", "relations", "stats", "identities", "hosts", "self"]);
}

export const qualifyLive = (live, host) => native.hostsQualifyLive(live, host, clean, activity);

export function mergeLimits(local, readings) {
  const r = native.hostsMergeLimits(local, readings);
  return keepLimits(r.limits, local, r.replaced);
}

const pairs = (hosts) => hosts.map((h) => ({ info: h.info, snapshot: h.snapshot ?? null }));

export function mergeLive(local, hosts) {
  if (!hosts.length) return local;
  const { live: out, replaced } = native.hostsMergeLive(local, pairs(hosts), clean, activity);
  for (const k of ["sessions", "collisions", "collab"]) keepFirst(out[k], local[k]);
  out.limits = keepLimits(out.limits, local.limits ?? {}, replaced);
  return keep(out, local, ["sessions", "collisions", "collab", "series", "tpm", "today", "total24h", "limits"]);
}

export function mergeSummary(local, hosts) {
  if (!hosts.length) return local;
  const out = native.hostsMergeSummary(local, pairs(hosts), clean, activity);
  for (const k of ["needsYou", "stuck"]) keepFirst(out[k], local[k]);
  return keep(out, local, ["needsYou", "stuck", "working", "review", "collisions", "tpm"]);
}

export const qualifyEvent = (name, data, host) => native.hostsQualifyEvent(String(name), data, host, clean, activity);
export const eventNote = (name, data, host) => native.hostsEventNote(String(name), data, host) ?? null;

export const qualifyEngine = (engine, host) => native.hostsQualifyEngine(engine, host, clean, activity);
