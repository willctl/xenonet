// What a remote actor may see of XenoNet's state (docs/design/cloud-assistants.md,
// section 5): pure functions from a view of the map (source.js, or the
// made-up one in fixtures.js) and a grant's policy (connections.js) to the
// six tools' data. Every object is made afresh from fields named here, and
// the service checks each against the strict schemas (core/schema/
// assistant.js) before it goes; nothing is copied from a state object and
// trimmed.
//
// A grant sees only the places it names, by their own ids (prj_<uuid>):
// a session counts when the place it lives in is one of them, and is
// never archived. Filtering, counting, sorting and paging all happen on
// that view, so a place the grant doesn't name never shows, not even as a
// count or a gap. Labels (a place's name, a session's title, an actor's
// name) go only when the grant says so, cleaned, and are null otherwise.
// Never a path, a branch, a remote, a prompt, a message, a question's text,
// a tool's arguments or output, an account or an error as a source wrote
// it.
import { SESSION_ID } from "../schema/assistant.js";

export const LABEL_MAX = 240;
export const LIVE_MS = 30e3;
export const WINDOW_MS = 7 * 24 * 3600e3;
export const LOOKBACK_MAX_MS = 30 * 24 * 3600e3;
export const SOURCE_NAMES = ["claude", "codex"];

const iso = (ms) => (Number.isFinite(ms) && ms > 0 ? new Date(ms).toISOString() : null);

// ---------- labels ----------

const CONTROL = /[\u0000-\u001f\u007f-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g;
// Keys and tokens by how they start, and runs of 32 or more letters and
// digits mixing both, as core/actors/detail.js finds them; emails; and
// folders named for a person, on any system.
const KEY = /(?<![A-Za-z0-9])(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)[A-Za-z0-9_-]*/g;
const RUN = /[A-Za-z0-9+/=_-]{32,}/g;
const EMAIL = /[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}/g;
// A person's folder runs to the next separator, spaces and all ("Jane
// Doe"), or to the end; any other absolute path goes whole.
const USER_FOLDER = /(?:[A-Za-z]:)?[\\/](?:Users|home)[\\/][^\\/"'`]+?(?=[\\/]|$)/gi;
const WINDOWS_PATH = /(?<![A-Za-z0-9])[A-Za-z]:[\\/][^\s"'`<>|]*/g;
const POSIX_PATH = /(?<![\w.~])\/(?:[\w.@+-]+\/)+[\w.@+-]*/g;
const BEARER = /\b(bearer|token|password|passwd|secret|api[_-]?key)\b\s*[:=]\s*\S+/gi;

// A name or a title as it may leave: one line, no control or direction
// characters, nothing that reads as a key, an email or a person's folder,
// at most LABEL_MAX characters; null for nothing left. Text can say
// anything, so this is protection on top of your choice to send labels,
// not a promise that what's left is harmless.
export function cleanLabel(text) {
  if (typeof text !== "string") return null;
  let s = text.normalize("NFC").replace(CONTROL, " ");
  s = s.replace(BEARER, "$1: [withheld]").replace(KEY, "[withheld]").replace(EMAIL, "[email]").replace(USER_FOLDER, "~").replace(WINDOWS_PATH, "[path]").replace(POSIX_PATH, "[path]");
  s = s.replace(RUN, (run) => (/[A-Za-z]/.test(run) && /\d/.test(run) ? "[withheld]" : run));
  s = s.replace(/\s+/g, " ").trim();
  if (!s) return null;
  const chars = [...s];
  return chars.length > LABEL_MAX ? `${chars.slice(0, LABEL_MAX - 1).join("")}…` : s;
}

// ---------- sessions ----------

// A session's status, from what the tailer saw (a live/session/v1, or null
// for one with nothing in its transcript lately) and how its app's source
// stands. A turn that ended is completed; that says nothing of whether the
// work it was part of is done.
export function sessionStatus(live, sourceState = "ready") {
  if (sourceState === "unavailable") return { status: "unknown", basis: "source_unavailable" };
  if (!live) return { status: "idle", basis: "observed_inactive" };
  const need = live.needsYou?.kind;
  if (need === "permission" || need === "question") return { status: "waiting", basis: "observed_wait" };
  if (live.stuck) return { status: "stuck", basis: "inferred_stall" };
  if (live.status === "tool" || live.status === "thinking") return { status: "working", basis: "observed_turn_active" };
  if (need === "review") return { status: "completed", basis: "observed_turn_end" };
  if (live.interruptedAt) return { status: "idle", basis: "observed_interrupt" };
  return { status: "idle", basis: "observed_idle" };
}

// Why a session looks stuck, as a code: never the reason's words, which
// name the command it repeats.
export function stallReason(reasons = []) {
  if (reasons.some((r) => /failed tool calls|times in a row/.test(r))) return "repeated_failure";
  if (reasons.some((r) => /^silent for/.test(r))) return "inactive_mid_turn";
  return "unknown";
}

// The places a grant names, both ways: its project ids to the map's node
// ids, and back.
function placesOf(policy) {
  const byPlace = new Map();
  for (const [project, place] of policy.places) byPlace.set(place, project);
  return byPlace;
}

// The sessions a grant may see, each with its project's id, status and
// times: in a place it names, not archived, with an id of the form the
// contract allows, and updated within the grant's lookback.
export function visibleSessions(view, policy, now) {
  const byPlace = placesOf(policy);
  const oldest = now - Math.min(policy.lookbackMs ?? LOOKBACK_MAX_MS, LOOKBACK_MAX_MS);
  const out = [];
  for (const s of view.sessions) {
    const project = byPlace.get(s.place);
    if (!project || s.archived || !SESSION_ID.test(s.id)) continue;
    const updated = Math.max(s.updatedAt ?? 0, s.live?.lastEventAt ?? 0);
    if (updated < oldest) continue;
    const source = view.sources[s.app]?.state ?? "unavailable";
    out.push({ s, project, updated, source, ...sessionStatus(s.live, source) });
  }
  return out;
}

export function sessionSummary(v, policy) {
  return {
    session_id: v.s.id,
    project_id: v.project,
    app: v.s.app,
    label: policy.labels ? cleanLabel(v.s.title) : null,
    status: v.status,
    status_basis: v.basis,
    updated_at: iso(v.s.updatedAt),
    last_activity_at: iso(v.updated),
    source_state: v.source,
  };
}

export const emptyCounts = () => ({ working: 0, waiting: 0, idle: 0, completed: 0, stuck: 0, unknown: 0 });

// ---------- freshness ----------

// How fresh a view is, from its sources (`names`, all by default): live
// when each was observed in the last 30 seconds, stale after, unknown when
// one never was; observed_at is the oldest. Partial when one isn't ready,
// with a warning for each.
export function freshnessOf(view, now, names = SOURCE_NAMES) {
  const list = names.map((n) => [n, view.sources[n]]);
  const warnings = [];
  let partial = false;
  for (const [name, s] of list) {
    if (!s || s.state === "unavailable") (partial = true), warnings.push({ code: "SOURCE_UNAVAILABLE", source: name });
    else if (s.state === "degraded") (partial = true), warnings.push({ code: "SOURCE_DEGRADED", source: name });
  }
  const times = list.map(([, s]) => s?.observedAt ?? null);
  if (times.some((t) => t == null)) return { freshness: "unknown", observedAt: null, partial, warnings };
  const oldest = Math.min(...times);
  return { freshness: now - oldest <= LIVE_MS ? "live" : "stale", observedAt: oldest, partial, warnings };
}

export function sourcesData(view) {
  return SOURCE_NAMES.map((name) => {
    const s = view.sources[name];
    return s
      ? { source: name, state: s.state, observed_at: iso(s.observedAt), coverage: s.coverage, reason: s.reason }
      : { source: name, state: "unavailable", observed_at: null, coverage: "unknown", reason: "not_started" };
  });
}

// ---------- attention ----------

// What needs someone's attention among the sessions a grant sees: one
// waiting on its user (a question or a permission, never its text), one
// that looks stuck, two that edited the same files lately (only when the
// grant sees both: one it doesn't is never hinted at), and a turn that
// ended. `idOf(parts)` makes an item's id. Nothing here acknowledges or
// clears anything.
export function attentionItems(view, policy, now, { idOf }) {
  const visible = visibleSessions(view, policy, now);
  const byId = new Map(visible.map((v) => [v.s.id, v]));
  const at = view.at;
  const items = [];
  for (const v of visible) {
    const live = v.s.live;
    if (!live || v.source === "unavailable") continue;
    const need = live.needsYou;
    if (need && (need.kind === "question" || need.kind === "permission")) items.push({ kind: "waiting", sessions: [v.s.id], project: v.project, first: need.since ?? null, last: at, reason: need.kind === "permission" ? "approval_requested" : "input_requested" });
    else if (live.stuck) items.push({ kind: "stalled", sessions: [v.s.id], project: v.project, first: live.turnStartedAt ?? live.lastEventAt ?? null, last: at, reason: stallReason(live.stuck.reasons) });
    else if (need?.kind === "review") items.push({ kind: "turn_completed", sessions: [v.s.id], project: v.project, first: live.lastTurnEndAt ?? need.since ?? null, last: at, reason: "turn_ended" });
  }
  for (const c of view.collisions) {
    const a = byId.get(c.a);
    const b = byId.get(c.b);
    if (!a || !b || a.source === "unavailable" || b.source === "unavailable") continue;
    // Two places can collide only when the grant sees both; the item goes
    // under the first session's project.
    items.push({ kind: "collision", sessions: [c.a, c.b].sort(), project: a.project, first: c.ts ?? null, last: at, reason: "recent_overlap", files: c.files });
  }
  return items.map((i) => ({
    attention_id: idOf([i.kind, ...i.sessions, i.first ?? ""].join("|")),
    kind: i.kind,
    session_ids: i.sessions,
    project_id: i.project,
    first_observed_at: iso(i.first),
    last_observed_at: iso(i.last),
    reason_code: i.reason,
    ...(i.kind === "collision" ? { file_count: Math.max(0, Math.floor(i.files ?? 0)) } : {}),
    resolved: false,
  }));
}

// ---------- actors ----------

// How XenoNet knows an actor's presence, as a code.
function presenceBasis(a) {
  if (a.presence === "unknown") return "unknown";
  if (a.via.includes("observed") || a.via.includes("attributed")) return "observed";
  if (a.via.includes("registered")) return "reported";
  if (a.via.includes("remote")) return "remote_request";
  return "unknown";
}

// The actors a grant may see: itself, and others whose targets fall in its
// places (the place itself, or a session it sees), with only those
// relations. Never the person at the console, and never an actor that
// acts only on places the grant doesn't name.
export function actorSummaries(view, policy, now, { projectId = null } = {}) {
  const byPlace = placesOf(policy);
  const sessions = new Map(visibleSessions(view, policy, now).map((v) => [v.s.id, v.project]));
  const out = [];
  for (const a of view.actors) {
    if (a.id === "actor:you" || a.kind === "human") continue;
    const self = a.id === policy.actorId;
    const relations = [];
    for (const t of a.targets ?? []) {
      const project = byPlace.get(t.id) ?? sessions.get(t.id) ?? null;
      if (!project || (projectId && project !== projectId)) continue;
      const target = byPlace.has(t.id) ? project : t.id;
      if (!relations.some((r) => r.target_id === target && r.relation === t.relation)) relations.push({ target_id: target, relation: t.relation });
    }
    if (!self && !relations.length) continue;
    out.push({
      actor_id: a.id,
      product_id: a.productId ?? null,
      label: policy.labels ? cleanLabel(a.name) : null,
      presence: ["working", "idle", "away", "offline", "unknown"].includes(a.presence) ? a.presence : "unknown",
      presence_basis: presenceBasis(a),
      connection_state: a.remote ? (a.remote.bridge === "connected" ? "connected" : "not_connected") : "none",
      account_link_state: a.remote?.accountLink ?? "unverified",
      relations: relations.slice(0, 16),
      reachable: !!a.reachable && !self,
    });
  }
  return out;
}

// ---------- stats ----------

// A session's counts from its stats (core/stats.js): fresh input (with
// what was written to a cache), output, cached input, tool calls and
// turns. Null for what it doesn't say, never 0 for unknown.
export function statsData(raw, now) {
  const n = (v) => (Number.isFinite(v) && v >= 0 ? Math.floor(v) : null);
  const totals = raw?.totals ?? {};
  const fresh = n(totals.input) != null ? n(totals.input) + (n(totals.cacheWrite) ?? 0) : null;
  return {
    input_tokens: fresh,
    output_tokens: n(totals.output),
    cached_input_tokens: n(totals.cacheRead),
    tool_call_count: Array.isArray(raw?.tools) ? raw.tools.reduce((a, t) => a + (n(t.calls) ?? 0), 0) : null,
    turn_count: Array.isArray(raw?.turns) ? raw.turns.length : null,
    stats_observed_at: new Date(now).toISOString(),
    complete: !!raw && Array.isArray(raw.turns),
  };
}
