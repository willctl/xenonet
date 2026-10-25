// The assistant's reads (core/assistant/service.js, projection.js): each of
// the six operations against a made-up map, what a grant sees and never
// sees (places it doesn't name, archived and old sessions, text that
// isn't a label, labels unless they're on), its pages and their tokens,
// freshness and partial sources, its limits, and the check just before an
// answer goes.
import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";
import { world, ask, view, T, MIN, DAY, ATLAS, LEDGER, SECRET, S, CANARIES } from "./fixtures/assistant.js";
import { readCredential } from "../core/assistant/credentials.js";
import { createAssistantService, createFixtureService, LIMITS } from "../core/assistant/service.js";
import { createAudit } from "../core/assistant/audit.js";
import { createRemotePresence } from "../core/actors/remote-presence.js";
import { createCursors } from "../core/assistant/cursors.js";
import { sessionStatus, cleanLabel } from "../core/assistant/projection.js";
import { RESULTS, OPERATIONS } from "../core/schema/assistant.js";
import { AssistantError } from "../core/schema/assistant.js";

const rid = () => `r_${crypto.randomUUID()}`;

// A world with one grant and a service reading `v` (a view, or a function
// of the time), with what it was asked for stats.
function setup({ over = {}, views = (t) => view(t), stats = async () => ({ totals: { input: 100, cacheWrite: 20, cacheRead: 300, output: 50 }, tools: [{ calls: 3 }, { calls: 4 }], turns: [{}, {}] }), limits } = {}) {
  const w = world();
  const v = w.connections.pair(ask(over), { places: w.places, identities: w.identities });
  const token = readCredential(w.dir, v.grant_id).token;
  const s = w.connections.authenticate({ grant: v.grant_id, token });
  const session = { grantId: v.grant_id, actorId: s.actorId, connectionId: s.connection.connection_id };
  const asked = [];
  const source = { view: () => views(w.clock.now()), stats: (id) => (asked.push(id), stats(id)), actor: (id) => w.actors.has(id) ? { id, kind: "agent", productId: "dots", name: "dots", presence: "working", via: ["remote"], remote: { bridge: "connected", accountLink: "unverified" }, reachable: false, targets: [] } : null };
  const audit = createAudit({ store: w.store, now: w.clock.now });
  const service = createAssistantService({ connections: w.connections, source, remote: createRemotePresence({ actors: w.actors, now: w.clock.now }), audit, version: "0.7.0-test", now: w.clock.now, ...(limits ? { limits: { ...LIMITS, ...limits } } : {}) });
  const read = async (operation, args = {}) => service.read(session, { request_id: rid(), operation, arguments: args });
  const ok = async (operation, args) => {
    const r = await read(operation, args);
    assert.ok(r.ok, `${operation} ${JSON.stringify(args)}: ${JSON.stringify(r.error)}`);
    const tool = Object.keys(OPERATIONS).find((k) => OPERATIONS[k] === operation);
    assert.ok(RESULTS[tool].safeParse(r.result).success, `${operation} fits its contract`);
    return r.result;
  };
  const refused = async (operation, args, code) => {
    const r = await read(operation, args);
    assert.equal(r.ok, false, `${operation} ${JSON.stringify(args)} should be refused`);
    assert.ok(AssistantError.safeParse(r.error).success);
    assert.equal(r.error.code, code);
    return r.error;
  };
  const projectOf = (place) => v.projects.find((p) => p.place === place)?.project_id ?? w.store.projectIdFor(place);
  return { ...w, v, token, session, service, read, ok, refused, asked, projectOf, audit };
}

test("xenonet_status says who, where, what and how fresh, and nothing about the rest of the machine", async () => {
  const t = setup();
  try {
    const r = await t.ok("status");
    assert.deepEqual([r.freshness, r.partial, r.grant_revision, r.device_id], ["live", false, 1, t.connections.device()]);
    assert.deepEqual(r.data.actor, { actor_id: "actor:dots", product_id: "dots", account_link_state: "unverified", participation: "read_only", messaging: "disabled" });
    assert.deepEqual([r.data.mode, r.data.connection, r.data.assistant_api, r.data.authorized_project_count, r.data.labels_enabled, r.data.data_mode, r.data.transport_scope, r.data.audience_label], ["read_only", "connected", 1, 1, false, "real", "owner_endpoint", "My dot"]);
    assert.equal(r.data.capabilities.length, 6);
    assert.deepEqual(r.data.sources.map((s) => [s.source, s.state, s.coverage]), [["claude", "ready", "complete"], ["codex", "ready", "complete"], ["grok", "ready", "complete"]]);
    assert.equal(r.served_at, new Date(T).toISOString());
    assert.equal(r.observed_at, new Date(T - 1000).toISOString());
  } finally {
    t.done();
  }
});

test("xenonet_projects and xenonet_sessions see only the places the grant names, counted the same way", async () => {
  const t = setup({ over: { projects: [ATLAS, LEDGER] } });
  try {
    const atlas = t.projectOf(ATLAS);
    const ledger = t.projectOf(LEDGER);
    const p = await t.ok("projects");
    assert.deepEqual(p.data.projects.map((x) => x.project_id).sort(), [atlas, ledger].sort());
    const counts = Object.fromEntries(p.data.projects.map((x) => [x.project_id, x.session_counts]));
    assert.deepEqual(counts[atlas], { working: 1, waiting: 1, idle: 0, completed: 1, stuck: 1, unknown: 0 }, "archived and 40-day-old sessions don't count");
    assert.deepEqual(counts[ledger], { working: 0, waiting: 0, idle: 1, completed: 0, stuck: 0, unknown: 0 });
    assert.equal(p.data.window_start, new Date(T - 7 * DAY).toISOString());
    assert.ok(p.data.projects.every((x) => x.label === null), "labels off: null");

    const s = await t.ok("sessions");
    assert.deepEqual(s.data.sessions.map((x) => x.session_id), [S.working, S.waiting, S.stuck, S.done, S.ledger], "newest first");
    assert.deepEqual(s.data.sessions.map((x) => [x.status, x.status_basis]), [["working", "observed_turn_active"], ["waiting", "observed_wait"], ["stuck", "inferred_stall"], ["completed", "observed_turn_end"], ["idle", "observed_inactive"]]);
    const waiting = await t.ok("sessions", { statuses: ["waiting", "stuck"], project_id: atlas });
    assert.deepEqual(waiting.data.sessions.map((x) => x.session_id), [S.waiting, S.stuck]);
    const all = JSON.stringify([p, s]);
    for (const c of CANARIES) assert.ok(!all.includes(c), `no ${c}`);
    assert.ok(!all.includes(S.hidden) && !all.includes(S.archived) && !all.includes(S.old));
  } finally {
    t.done();
  }
});

test("a Grok Build session is listed like the others, and only it is unknown when Grok's source is down", async () => {
  const id = "grok:019a3b5c-0000-7000-8000-00000000ab09";
  const withGrok = (at, o) => {
    const v = view(at, o);
    v.sessions.push({ id, app: "grok", place: LEDGER, title: "Read the logs", updatedAt: at - 30e3, archived: false, live: { status: "tool", needsYou: null, stuck: null, turnStartedAt: at - MIN, lastTurnEndAt: null, interruptedAt: null, lastEventAt: at - 30e3 } });
    return v;
  };
  const t = setup({ over: { projects: [LEDGER] }, views: (at) => withGrok(at) });
  try {
    const s = await t.ok("sessions");
    assert.deepEqual(s.data.sessions.map((x) => [x.session_id, x.app, x.status]), [[id, "grok", "working"], [S.ledger, "codex", "idle"]]);
  } finally {
    t.done();
  }
  const down = setup({ over: { projects: [LEDGER] }, views: (at) => withGrok(at, { grok: "unavailable" }) });
  try {
    const r = await down.ok("sessions");
    assert.deepEqual([r.partial, r.warnings], [true, [{ code: "SOURCE_UNAVAILABLE", source: "grok" }]]);
    assert.deepEqual(r.data.sessions.map((x) => [x.app, x.status]), [["grok", "unknown"], ["codex", "idle"]]);
  } finally {
    down.done();
  }
});

test("a place or session the grant doesn't name is NOT_FOUND, exactly as one that doesn't exist", async () => {
  const t = setup();
  try {
    const secret = t.store.projectIdOf(SECRET, () => `prj_${crypto.randomUUID()}`);
    const nowhere = `prj_${crypto.randomUUID()}`;
    const a = await t.refused("sessions", { project_id: secret }, "NOT_FOUND");
    const b = await t.refused("sessions", { project_id: nowhere }, "NOT_FOUND");
    assert.deepEqual({ ...a, request_id: null }, { ...b, request_id: null });
    const c = await t.refused("session", { session_id: S.hidden }, "NOT_FOUND");
    const d = await t.refused("session", { session_id: "claude:0190d3e0-0000-7000-8000-0000000000ff" }, "NOT_FOUND");
    assert.deepEqual({ ...c, request_id: null }, { ...d, request_id: null });
    await t.refused("session", { session_id: S.archived }, "NOT_FOUND");
    await t.refused("attention", { project_id: secret }, "NOT_FOUND");
    await t.refused("actors", { project_id: secret }, "NOT_FOUND");
  } finally {
    t.done();
  }
});

test("arguments are strict: unknown fields, statuses, paths, future or too-old times are refused", async () => {
  const t = setup();
  try {
    await t.refused("sessions", { path: "/etc" }, "INVALID_ARGUMENT");
    await t.refused("sessions", { statuses: ["sleeping"] }, "INVALID_ARGUMENT");
    await t.refused("sessions", { updated_since: new Date(T + DAY).toISOString() }, "INVALID_ARGUMENT");
    await t.refused("sessions", { updated_since: new Date(T - 31 * DAY).toISOString() }, "INVALID_ARGUMENT");
    await t.refused("sessions", { limit: 51 }, "INVALID_ARGUMENT");
    await t.refused("session", { session_id: "claude:../../etc/passwd" }, "INVALID_ARGUMENT");
    await t.refused("status", { principal_id: "actor:you" }, "INVALID_ARGUMENT");
    const r = await t.service.read(t.session, { request_id: rid(), operation: "terminal.write", arguments: {} });
    assert.equal(r.error.code, "INVALID_ARGUMENT");
    const extra = await t.service.read(t.session, { request_id: rid(), operation: "status", arguments: {}, thread_id: "codex:x" });
    assert.equal(extra.error.code, "INVALID_ARGUMENT");
    const big = await t.read("sessions", { project_id: "x".repeat(LIMITS.inputBytes + 1) });
    assert.equal(big.error.code, "INVALID_ARGUMENT");
    // A grant's lookback can be less than 30 days.
    const short = setup({ over: { lookbackDays: 3 } });
    try {
      await short.refused("sessions", { updated_since: new Date(T - 4 * DAY).toISOString() }, "INVALID_ARGUMENT");
      await short.refused("session", { session_id: S.ledger }, "NOT_FOUND");
    } finally {
      short.done();
    }
  } finally {
    t.done();
  }
});

test("pages: tokens go on from where the last page ended, and refuse tampering, other filters, other grants and age", async () => {
  const t = setup({ over: { projects: [ATLAS, LEDGER] } });
  try {
    const first = await t.ok("sessions", { limit: 2 });
    assert.equal(first.data.sessions.length, 2);
    const second = await t.ok("sessions", { limit: 2, cursor: first.data.next_cursor });
    const third = await t.ok("sessions", { limit: 2, cursor: second.data.next_cursor });
    assert.deepEqual([...first.data.sessions, ...second.data.sessions, ...third.data.sessions].map((s) => s.session_id), [S.working, S.waiting, S.stuck, S.done, S.ledger]);
    assert.equal(third.data.next_cursor, null);
    const cursor = first.data.next_cursor;
    const [payload, mac] = cursor.split(".");
    const forged = Buffer.from(JSON.stringify({ ...JSON.parse(Buffer.from(payload, "base64url")), a: [0, "a"] })).toString("base64url");
    await t.refused("sessions", { limit: 2, cursor: `${forged}.${mac}` }, "INVALID_ARGUMENT");
    await t.refused("sessions", { limit: 2, cursor, statuses: ["idle"] }, "INVALID_ARGUMENT");
    await t.refused("projects", { cursor }, "INVALID_ARGUMENT");
    t.clock.add(5 * MIN + 1);
    await t.refused("sessions", { limit: 2, cursor }, "CURSOR_EXPIRED");
    const fresh = await t.ok("sessions", { limit: 2 });
    t.connections.update(t.v.grant_id, { labels: true }, { places: t.places });
    await t.refused("sessions", { limit: 2, cursor: fresh.data.next_cursor }, "CURSOR_EXPIRED");
    // Another grant's token, with the same key.
    const cursors = createCursors({ now: t.clock.now, key: Buffer.alloc(32) });
    const other = cursors.make({ grant: "gr_" + "9".repeat(24), revision: 1, op: "sessions", filters: {}, after: [0, "x"] });
    assert.equal(cursors.read(other, { grant: t.v.grant_id, revision: 1, op: "sessions", filters: {} }).code, "INVALID_ARGUMENT");
  } finally {
    t.done();
  }
});

test("xenonet_attention: waiting, stalled, finished turns and collisions within the grant, as codes", async () => {
  const t = setup();
  try {
    const r = await t.ok("attention");
    const items = r.data.items;
    assert.deepEqual(items.map((i) => [i.kind, i.reason_code, i.session_ids.join(",")]), [
      ["waiting", "input_requested", S.waiting],
      ["stalled", "repeated_failure", S.stuck],
      ["collision", "recent_overlap", [S.waiting, S.working].sort().join(",")],
      ["turn_completed", "turn_ended", S.done],
    ]);
    assert.equal(items.find((i) => i.kind === "collision").file_count, 3);
    assert.ok(items.every((i) => i.resolved === false && /^att_[0-9a-f]{24}$/.test(i.attention_id)));
    assert.ok(!JSON.stringify(r).includes(S.hidden), "a collision with a session it doesn't see isn't hinted at");
    assert.ok(items.every((i) => i.file_count !== 7), "nor its file count");
    const again = await t.ok("attention", { kinds: ["collision"] });
    assert.deepEqual(again.data.items.map((i) => i.attention_id), [items[2].attention_id], "stable ids");
    for (const c of CANARIES) assert.ok(!JSON.stringify(r).includes(c));
  } finally {
    t.done();
  }
});

test("xenonet_session: one session; its stats only when asked, and a warning when they can't be read", async () => {
  let fail = false;
  const t = setup({ stats: async () => (fail ? Promise.reject(new Error("C:\\Users\\nova\\secret ENOENT")) : { totals: { input: 100, cacheWrite: 20, cacheRead: 300, output: 50 }, tools: [{ calls: 3 }, { calls: 4 }], turns: [{}, {}] }) });
  try {
    const plain = await t.ok("session", { session_id: S.waiting });
    assert.equal(plain.data.stats, null);
    assert.deepEqual(t.asked, [], "no transcript work without include_stats");
    const withStats = await t.ok("session", { session_id: S.waiting, include_stats: true });
    assert.deepEqual(withStats.data.stats, { input_tokens: 120, output_tokens: 50, cached_input_tokens: 300, tool_call_count: 7, turn_count: 2, stats_observed_at: new Date(T).toISOString(), complete: true });
    fail = true;
    const failed = await t.ok("session", { session_id: S.waiting, include_stats: true });
    assert.equal(failed.data.stats, null);
    assert.deepEqual(failed.warnings, [{ code: "STATS_UNAVAILABLE" }]);
    assert.ok(!JSON.stringify(failed).includes("ENOENT") && !JSON.stringify(failed).includes("nova"));
  } finally {
    t.done();
  }
});

test("xenonet_actors: itself, and others only through what they act on in the grant's places", async () => {
  const t = setup();
  try {
    const r = await t.ok("actors");
    assert.equal(r.data.self_actor_id, "actor:dots");
    assert.deepEqual(r.data.actors.map((a) => a.actor_id), ["actor:dots", "actor:kestrel"]);
    const kestrel = r.data.actors[1];
    assert.deepEqual(kestrel.relations, [{ target_id: t.projectOf(ATLAS), relation: "watches" }], "its work elsewhere isn't shown");
    assert.deepEqual([kestrel.product_id, kestrel.presence_basis, kestrel.connection_state, kestrel.reachable, kestrel.label], ["grok-bot", "observed", "none", false, null]);
    assert.equal(r.data.actors[0].reachable, false);
    assert.ok(!JSON.stringify(r).includes("actor:you") && !JSON.stringify(r).includes("actor:wren"));
  } finally {
    t.done();
  }
});

test("labels, when on, are cleaned of keys, emails, folders and control characters, and cut to 240", async () => {
  const t = setup({ over: { labels: true } });
  try {
    const r = await t.ok("sessions");
    const label = r.data.sessions.find((s) => s.session_id === S.waiting).label;
    assert.equal(label, String.raw`Refunds for [email] ~\secret password: [withheld]`, "a person's folder as ~, as XenoNet shows it");
    assert.equal((await t.ok("projects")).data.projects[0].label, "atlas-api [withheld]");
    assert.equal(cleanLabel("x".repeat(500)).length, 240);
    assert.equal(cleanLabel("\u202eevil\u0000 title\u2028next"), "evil title next");
    assert.equal(cleanLabel("   "), null);
    assert.equal(cleanLabel(String.raw`C:\Users\Jane Doe\clients\acme\deploy.ps1`), String.raw`~\clients\acme\deploy.ps1`, "a name with a space goes whole");
    assert.equal(cleanLabel(String.raw`see D:\Clients\Acme\plan.md now`), "see [path] now", "other absolute paths go whole");
    assert.equal(cleanLabel("edit /srv/app/config.yml"), "edit [path]");
    assert.equal(cleanLabel("Fix the /home page, 1/2 done, src/app.js"), "Fix the /home page, 1/2 done, src/app.js", "words with slashes stay");
    const all = JSON.stringify([r, await t.ok("projects"), await t.ok("attention"), await t.ok("actors")]);
    for (const c of CANARIES.filter((c) => c !== "secret-plans")) assert.ok(!all.includes(c), `no ${c}`);
  } finally {
    t.done();
  }
});

test("freshness: stale after 30 seconds, unknown with no observation, partial with a source down", async () => {
  const stale = setup({ views: (t) => view(t, { observedAgo: 31e3 }) });
  try {
    assert.equal((await stale.ok("status")).freshness, "stale");
  } finally {
    stale.done();
  }
  const down = setup({ views: (t) => view(t, { claude: "unavailable", codex: "degraded" }) });
  try {
    const r = await down.ok("sessions");
    assert.deepEqual([r.freshness, r.partial, r.observed_at], ["unknown", true, null]);
    assert.deepEqual(r.warnings, [{ code: "SOURCE_UNAVAILABLE", source: "claude" }, { code: "SOURCE_DEGRADED", source: "codex" }]);
    assert.ok(r.data.sessions.filter((s) => s.app === "claude").every((s) => s.status === "unknown" && s.status_basis === "source_unavailable"), "unknown stays unknown");
    const a = await down.ok("attention");
    assert.ok(a.data.items.every((i) => !i.session_ids.some((id) => id.startsWith("claude:"))), "nothing claimed from a source that's down");
  } finally {
    down.done();
  }
  assert.deepEqual(sessionStatus(null, "ready"), { status: "idle", basis: "observed_inactive" });
  assert.deepEqual(sessionStatus({ status: "idle", interruptedAt: 1 }, "ready"), { status: "idle", basis: "observed_interrupt" });
});

test("limits: ten at once and one a second after, four at a time, ten seconds each", async () => {
  const t = setup();
  try {
    for (let i = 0; i < 10; i++) await t.ok("status");
    const limited = await t.refused("status", {}, "RATE_LIMITED");
    assert.ok(limited.retryable && limited.retry_after_ms > 0);
    t.clock.add(2000);
    await t.ok("status");
  } finally {
    t.done();
  }
  let release;
  const slow = setup({ stats: () => new Promise((r) => (release = r)), limits: { timeoutMs: 200 } });
  try {
    const pending = [1, 2, 3, 4].map(() => slow.read("session", { session_id: S.waiting, include_stats: true }));
    await slow.refused("session", { session_id: S.waiting, include_stats: true }, "RATE_LIMITED");
    const out = await Promise.all(pending);
    assert.deepEqual(out.map((r) => r.error?.code), ["TIMEOUT", "TIMEOUT", "TIMEOUT", "TIMEOUT"]);
    release?.(null);
    slow.clock.add(1000);
    await new Promise((r) => setTimeout(r, 10));
    await slow.ok("status");
  } finally {
    slow.done();
  }
  // A read that timed out still holds its place until its work ends, so
  // asking again can't pile up work under the limit.
  let finish;
  const one = setup({ stats: () => new Promise((r) => (finish = r)), limits: { timeoutMs: 50, concurrent: 1 } });
  try {
    await one.refused("session", { session_id: S.waiting, include_stats: true }, "TIMEOUT");
    await one.refused("session", { session_id: S.waiting, include_stats: true }, "RATE_LIMITED");
    finish(null);
    await new Promise((r) => setTimeout(r, 10));
    await one.ok("status");
  } finally {
    one.done();
  }
});

test("revoked, turned off or changed while a read is under way: nothing from before goes out", async () => {
  let during = () => {};
  const t = setup({ stats: async () => (during(), { totals: {}, tools: [], turns: [] }) });
  try {
    let once = true;
    during = () => once && ((once = false), t.connections.update(t.v.grant_id, { labels: true }, { places: t.places }));
    const changed = await t.ok("session", { session_id: S.waiting, include_stats: true });
    assert.equal(changed.grant_revision, 2, "read again under the new scope");
    during = () => t.connections.revoke(t.v.grant_id);
    const revoked = await t.refused("session", { session_id: S.waiting, include_stats: true }, "GRANT_REVOKED");
    assert.equal(revoked.retryable, false);
    await t.refused("status", {}, "GRANT_REVOKED");
  } finally {
    t.done();
  }
  const off = setup();
  try {
    off.connections.disable();
    await off.refused("status", {}, "DISABLED");
  } finally {
    off.done();
  }
});

test("a grant whose audience isn't verified reads made-up data only, said so on every answer", async () => {
  const t = setup({ over: { audience: { description: "My workspace", verification: "unverified", scope: "authenticated_user" }, projects: [ATLAS, SECRET] } });
  try {
    const status = await t.ok("status");
    assert.equal(status.data.data_mode, "fixture");
    const all = [status, await t.ok("projects"), await t.ok("sessions"), await t.ok("attention"), await t.ok("actors")];
    assert.ok(all.every((r) => r.warnings.some((w) => w.code === "FIXTURE_DATA")));
    const text = JSON.stringify(all);
    for (const id of Object.values(S)) assert.ok(!text.includes(id), "no real session");
    assert.ok(!text.includes(t.projectOf(ATLAS)), "no real project id");
  } finally {
    t.done();
  }
});

test("reads change no work: only the log, the actor's own presence and its watches", async () => {
  const t = setup({ over: { projects: [ATLAS] } });
  try {
    t.store.send({ sender: "claude:x", target: "codex:y", body: "hello", recipients: ["codex:y"] });
    t.store.claim("claude:x", "src/**", null, T + DAY);
    const tables = ["messages", "deliveries", "claims", "focus", "contacts", "pins", "actor_requests", "actor_revoked", "actor_grants", "actor_connections", "actor_account_links", "actor_external_refs"];
    const snapshot = () => Object.fromEntries(tables.map((x) => [x, JSON.stringify(t.store.db.prepare(`SELECT * FROM ${x}`).all().map(({ last_used_at, ...r }) => r))]));
    const before = snapshot();
    for (const op of ["status", "projects", "sessions", "attention", "actors"]) await t.ok(op);
    await t.ok("sessions", { project_id: t.projectOf(ATLAS) });
    await t.ok("session", { session_id: S.waiting, include_stats: true });
    assert.deepEqual(snapshot(), before);
    const audit = t.store.audit({ limit: 100 }).filter((r) => r.kind === "access");
    assert.equal(audit.length, 7);
    assert.ok(audit.every((r) => r.result === "ok" && r.grant_id === t.v.grant_id && r.actor === "actor:dots" && Number.isInteger(r.bytes)));
    for (const c of CANARIES) assert.ok(!JSON.stringify(t.store.audit({ limit: 1000 })).includes(c));
    assert.ok(!JSON.stringify(t.store.audit({ limit: 1000 })).includes(t.token));
    assert.deepEqual(t.actors.targetsOf("actor:dots").map((x) => x.relation).sort(), ["watches", "watches"]);
    assert.ok(t.actors.targetsOf("actor:dots").every((x) => x.id === ATLAS || x.id === S.waiting));
    assert.equal(t.actors.snapshot("actor:dots").presence, "working");
  } finally {
    t.done();
  }
});

test("the fixture service needs nothing of this computer's and answers every tool", async () => {
  const svc = createFixtureService({ version: "0.7.0", now: () => T });
  const session = { grantId: "gr_000000000000000000000000", actorId: "actor:fixture-client", connectionId: "cn_000000000000000000000000" };
  for (const [tool, op] of Object.entries(OPERATIONS)) {
    const args = op === "session" ? { session_id: "claude:0190fa00-0000-7000-8000-000000000001", include_stats: true } : {};
    const r = await svc.read(session, { request_id: rid(), operation: op, arguments: args });
    assert.ok(r.ok, `${op}: ${JSON.stringify(r.error)}`);
    assert.ok(RESULTS[tool].safeParse(r.result).success);
    assert.ok(r.result.warnings.some((w) => w.code === "FIXTURE_DATA"));
  }
  // A second read a phase later sees new states with new times.
  const later = createFixtureService({ version: "0.7.0", now: () => T + 2 * MIN });
  const a = await svc.read(session, { request_id: rid(), operation: "sessions", arguments: {} });
  const b = await later.read(session, { request_id: rid(), operation: "sessions", arguments: {} });
  assert.notDeepEqual(a.result.data.sessions.map((s) => s.status), b.result.data.sessions.map((s) => s.status));
  assert.ok(Date.parse(b.result.observed_at) > Date.parse(a.result.observed_at));
});
