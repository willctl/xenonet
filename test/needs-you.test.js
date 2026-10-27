// What waits on you as one queue (core/needs-you.js): sessions waiting on a
// question or a permission, actors waiting on you, programs asking to be
// actors and asks for a gateway token, the longest wait first, each with
// what answers it; in a window without the key, only how many programs ask.
// The sessions, actors and programs are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { needsYouQueue, queueOfSummary, countOf, waitsWords, order } from "../core/needs-you.js";

const T = 1791230000000;
const session = (n, kind, since, more = {}) => ({ id: `claude:0190d3e0-0000-7000-8000-00000000000${n}`, app: "claude", title: `Session ${n}`, kind, text: `What about ${n}?`, since, ...more });
const actor = (slug, name, needsYou) => ({ id: `actor:${slug}`, name, kind: "agent", ...(needsYou ? { needsYou } : {}) });

test("the queue: everything that waits on you, the longest wait first, with what answers each", () => {
  const q = needsYouQueue({
    sessions: [session(1, "question", T + 3000), session(2, "permission", T + 1000), session(3, "review", T)],
    actors: [actor("wren", "Wren", { kind: "question", text: "Ship tonight?", since: T + 2000 }), actor("moth", "Moth"), { id: "actor:you", name: "You", kind: "human", needsYou: { kind: "question", text: "x", since: T } }],
    requests: [{ id: "req_1", name: "Heron", actor: "actor:heron", at: T + 4000, expiresAt: T + 604000 }],
    gatewayAsks: [{ id: "gw_1", name: "Muse", actor: "actor:muse", at: T + 500, expiresAt: T + 600500 }],
    canDecide: true,
  });
  assert.deepEqual(
    q.map((it) => [it.kind, it.title]),
    [["gateway", "Muse"], ["permission", "Session 2"], ["actor", "Wren"], ["question", "Session 1"], ["request", "Heron"]],
    "oldest first; a session ready for review, an actor that doesn't wait and you stay out",
  );
  assert.deepEqual(q.find((it) => it.kind === "question").actions, ["show", "open", "dismiss"]);
  assert.deepEqual(q.find((it) => it.kind === "actor").actions, ["show", "message"]);
  assert.deepEqual(q.find((it) => it.kind === "request").actions, ["allow", "deny"]);
  assert.equal(q.find((it) => it.kind === "request").expiresAt, T + 604000, "a request says when it lapses");
  assert.deepEqual(q.map((it) => it.key), ["gateway:gw_1", `session:${session(2).id}`, "actor:actor:wren", `session:${session(1).id}`, "request:req_1"]);
  assert.equal(new Set(q.map((it) => it.key)).size, q.length, "a key each");
  assert.equal(countOf(q), 5);
  assert.deepEqual(q.map(waitsWords), ["is asked for a gateway token", "needs permission", "asks", "asks", "asks to join XenoNet"]);
});

test("a window without the key: how many programs ask, never who, and nothing to press", () => {
  const q = needsYouQueue({ sessions: [session(1, "question", T)], actors: [actor("wren", "Wren", { kind: "permission", text: "May I?", since: T + 1 })], requests: [{ id: "req_1", name: "Heron", at: T }], canDecide: false, waiting: 3 });
  assert.deepEqual(q.map((it) => it.kind), ["question", "actor", "requests"], "undated after dated");
  const asks = q.at(-1);
  assert.deepEqual([asks.title, asks.count, asks.actions, waitsWords(asks)], ["3 programs", 3, [], "ask to join XenoNet"]);
  assert.ok(!JSON.stringify(q).includes("Heron"), "a request's name needs the key");
  assert.deepEqual(q[1].actions, ["show"], "writing to an actor is the window's");
  assert.equal(countOf(q), 5, "each program counts");
  assert.equal(waitsWords(needsYouQueue({ waiting: 1 })[0]), "asks to join XenoNet");
  assert.deepEqual(needsYouQueue({ waiting: 0 }), []);
});

test("another machine's waits: shown, never opened or dismissed from here, and a stale one last and uncounted", () => {
  const q = needsYouQueue({ sessions: [session(1, "question", T + 5000), session(2, "question", T, { host: "studio", stale: true }), session(3, "permission", T + 1000, { host: "studio" })] });
  assert.deepEqual(q.map((it) => [it.title, it.host ?? null]), [["Session 3", "studio"], ["Session 1", null], ["Session 2", "studio"]]);
  assert.deepEqual(q[0].actions, ["show"]);
  assert.equal(countOf(q), 2);
});

test("ties keep their places, and odd input is left out rather than thrown on", () => {
  const a = { key: "session:a", since: T };
  const b = { key: "session:b", since: T };
  assert.deepEqual([b, a].sort(order), [a, b]);
  const q = needsYouQueue({ sessions: [null, { kind: "question" }, session(1, "question", "soon")], actors: [null, { id: "actor:x", needsYou: { kind: "odd" } }] });
  assert.deepEqual(q.map((it) => [it.kind, it.since]), [["actor", null], ["question", null]], "undated, by key");
  assert.equal(waitsWords(q[0]), "waits for you");
  assert.equal(q[0].text, "Waiting for you");
  assert.deepEqual(needsYouQueue(), []);
});

test("from /api/summary: its sessions, the actors that wait and how many programs ask, in the panel's order", () => {
  const summary = {
    needsYou: [session(1, "question", T + 2000), session(2, "permission", T + 4000, { host: "studio" })],
    actorsNeedYou: [{ id: "actor:wren", name: "Wren", kind: "question", text: "Ship tonight?", since: T + 3000 }],
    requests: 2,
  };
  const q = queueOfSummary(summary);
  assert.deepEqual(q.map((it) => [it.kind, it.title]), [["question", "Session 1"], ["actor", "Wren"], ["permission", "Session 2"], ["requests", "2 programs"]]);
  assert.deepEqual(q[1].actions, ["show"], "the summary has no key");
  assert.equal(countOf(q), 5);
  assert.deepEqual(queueOfSummary({ needsYou: [] }), [], "an older XenoNet's summary, without the rest");
  assert.deepEqual(queueOfSummary(null), []);
});

test("a run's permission prompt, from this machine or another, with Allow and Deny in the window alone; and a machine whose Claude Code needs a sign-in for XenoNet's runs", () => {
  const ask = { id: "ask_1", host: "neo-rex", session: "claude:0190d3e0-0000-7000-8000-00000000000a", run: "run_1", title: "neo-rex · Fix the login form wants to run Bash", tool: "Bash", input: "npm test", at: T - 30e3, until: T + 570e3 };
  const hint = { host: "neo-rex", fix: "Claude Code on neo-rex needs a sign-in for XenoNet's runs: run xenonet runs sign-in there", since: T - 60e3 };
  const q = needsYouQueue({ asks: [ask, { host: "x" }], hints: [hint, { fix: "no host" }], canDecide: true });
  assert.deepEqual(
    q.map((it) => [it.key, it.kind, it.actions]),
    [
      ["sign-in:neo-rex", "sign-in", ["dismiss"]],
      ["ask:ask_1", "ask", ["allow", "deny"]],
    ],
  );
  assert.deepEqual([q[1].host, q[1].tool, q[1].text, q[1].run, q[1].expiresAt], ["neo-rex", "Bash", "npm test", "run_1", T + 570e3]);
  assert.deepEqual([q[0].title, q[0].text], ["Claude Code on neo-rex", hint.fix]);
  assert.equal(waitsWords(q[1]), "wants to run a tool");
  assert.equal(waitsWords(q[0]), "needs a sign-in for XenoNet's runs");
  // Without the window's key the hint shows, and no ask: only the window answers one.
  assert.deepEqual(
    needsYouQueue({ asks: [ask], hints: [hint] }).map((it) => it.kind),
    ["sign-in"],
  );
  assert.equal(countOf(q), 2);
});
