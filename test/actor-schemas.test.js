// The actor schemas the interface adds (docs/design/actors/interface.md,
// and contracts section 8): its examples fit them strictly, as definitions
// and as the files in schemas/, and what they leave out doesn't. Made-up
// actors and paths throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { check, explain } from "../core/schema/index.js";
import { fromFile } from "./fixtures/schema-files.js";

const T = 1791234567000;
const SESSION = "claude:6c1f0000-0000-4000-8000-000000000001";
const PROJECT = "project:/users/nova/projects/atlas-api";

function fits(id, data, what = id) {
  const r = check(id, data, { strict: true });
  assert.ok(r.ok, `${what} doesn't fit ${id}:\n${explain(r)}`);
  assert.ok(fromFile(id).safeParse(data).success, `${what} doesn't fit schemas/${id}.json`);
}
const misfits = (id, data) => !check(id, data).ok && !fromFile(id).safeParse(data).success;

test("registering: an optional slug, and the use-computer capability", () => {
  fits("actor/registration/v1", { schema: "actor/registration/v1", name: "Wren", slug: "wren", product: null, capabilities: ["run-commands", "edit-repos", "message-agents", "use-computer"], heartbeatMs: 60000, pid: 48213 });
  fits("actor/registration/v1", { name: "Wren" }, "just a name");
  assert.ok(misfits("actor/registration/v1", { name: "Wren", slug: "Wren!" }), "a slug follows the slug rule");
  assert.ok(misfits("actor/registration/v1", { name: "Wren", slug: "w".repeat(33) }), "32 characters at most");
});

test("credentials: waiting with a time it lapses, or allowed without one; the token's form", () => {
  const token = `xenonet_actor_${"0f".repeat(32)}`;
  fits("actor/credentials/v1", { schema: "actor/credentials/v1", id: "actor:wren", token, state: "waiting", expiresAt: T + 600e3, ignored: [] });
  fits("actor/credentials/v1", { schema: "actor/credentials/v1", id: "actor:wren", token, state: "allowed", ignored: ["fly"] }, "from the window");
  assert.ok(misfits("actor/credentials/v1", { schema: "actor/credentials/v1", id: "actor:wren", token: "xenonet_actor_short", state: "allowed", ignored: [] }));
  assert.ok(misfits("actor/credentials/v1", { schema: "actor/credentials/v1", id: "wren", token, state: "allowed", ignored: [] }), "an actor id");
});

test("reports: a heartbeat, what it did as facts, and what it's acting on", () => {
  fits("actor/report/v1", { schema: "actor/report/v1", instance: "gateway", presence: "working", status: "Triaging issues in atlas-api", cwd: "/Users/nova/Projects/atlas-api", pid: 48213 }, "a heartbeat");
  fits(
    "actor/report/v1",
    {
      schema: "actor/report/v1",
      events: [
        { kind: "ran", path: "/Users/nova/Projects/atlas-api", count: 3, ref: "run-118" },
        { kind: "edited", path: "/Users/nova/Projects/atlas-api", paths: ["/Users/nova/Projects/atlas-api/src/app.ts"] },
        { kind: "spawned", target: SESSION },
        { kind: "spawned", app: "claude", path: "/Users/nova/Projects/atlas-api" },
        { kind: "messaged", platform: "telegram" },
        { kind: "finished", subject: "job", at: T },
      ],
    },
    "events",
  );
  fits("actor/report/v1", { schema: "actor/report/v1", targets: [{ path: "/Users/nova/Projects/atlas-api", relation: "works-on" }, { id: SESSION, relation: "spawned" }], remove: [{ id: "project:/users/nova/projects/ledger", relation: "works-on" }], replace: false }, "targets");
  assert.ok(misfits("actor/report/v1", { presence: "away" }), "only XenoNet decides away");
  assert.ok(misfits("actor/report/v1", { instance: "Gateway 2" }), "an instance follows the slug rule");
  assert.ok(misfits("actor/report/v1", { events: Array.from({ length: 51 }, () => ({ kind: "ran" })) }), "50 events a call");
  assert.ok(misfits("actor/report/v1", { events: [{ kind: "ran", count: 0 }] }), "a count is 1 to 10,000");
  assert.ok(misfits("actor/report/v1", { events: [{ kind: "ran", ref: "has spaces" }] }), "a ref's characters");
});

test("me: whether it's allowed, its presence and unread messages, and what XenoNet took", () => {
  fits("actor/me/v1", { schema: "actor/me/v1", id: "actor:wren", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 1 });
  fits("actor/me/v1", { schema: "actor/me/v1", id: "actor:wren", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 0, accepted: 4, dropped: [{ index: 2, reason: "XenoNet doesn't know that session" }] }, "an answer to events");
  fits("actor/me/v1", { schema: "actor/me/v1", id: "actor:kestrel", state: "waiting", presence: "unknown", heartbeatMs: null, unread: 0 }, "waiting, without a heartbeat");
});

test("events since a time, each an actor/event/v1", () => {
  const event = { schema: "actor/event/v1", id: "wren:41", actor: "actor:wren", at: T, kind: "spawned", target: SESSION, detail: "Started a Claude Code session in atlas-api", via: "attributed" };
  fits("actor/events/v1", { schema: "actor/events/v1", at: T, events: [event] });
  fits("actor/events/v1", { schema: "actor/events/v1", at: T, events: [] }, "none");
  assert.ok(misfits("actor/events/v1", { schema: "actor/events/v1", at: T, events: [{ ...event, detail: "x".repeat(201) }] }), "details are 200 characters at most");
});

test("the person's reports: a presence and what you did, as node ids and actions", () => {
  fits("actor/console/v1", { schema: "actor/console/v1", presence: "working", did: [{ what: "looked", node: PROJECT, at: T }] });
  fits("actor/console/v1", { did: [{ what: "handed-off", node: SESSION, at: T, to: "codex" }, { what: "started", node: PROJECT, app: "claude" }, { what: "terminal", node: PROJECT }] }, "without a presence");
  assert.ok(misfits("actor/console/v1", { presence: "offline" }), "the desktop app says offline, not the page");
  assert.ok(misfits("actor/console/v1", { did: Array.from({ length: 21 }, () => ({ what: "looked", node: PROJECT })) }), "20 a report");
  assert.ok(misfits("actor/console/v1", { did: [{ what: "looked" }] }), "a node");
});

test("a snapshot may say it needs you, and lists use-computer", () => {
  const actor = {
    schema: "actor/snapshot/v1",
    id: "actor:wren",
    kind: "agent",
    name: "Wren",
    product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" },
    host: null,
    via: ["observed", "attributed"],
    presence: "working",
    presenceSince: T - 60e3,
    lastSeen: T,
    heartbeatMs: 60000,
    status: "Triaging issues in atlas-api",
    needsYou: { kind: "question", text: "Wren is waiting for an answer", since: T },
    capabilities: ["spawn-sessions", "use-computer"],
    targets: [{ id: PROJECT, relation: "works-on", since: T - 3600e3, lastAt: T }],
    recent: [],
    counts: { events24h: 1, spawned24h: 0, messages24h: 0 },
    identity: null,
  };
  fits("actor/snapshot/v1", actor);
  fits("actor/snapshot/v1", { ...actor, needsYou: null }, "needing nothing");
  const { needsYou, ...without } = actor;
  fits("actor/snapshot/v1", without, "without needsYou");
  fits("actor/list/v1", { schema: "actor/list/v1", at: T, actors: [actor] });
});
