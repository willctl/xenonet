// The network across machines as the console words it (src/network-model.js):
// the state read leniently, the link in a word, each machine's line, other
// machines' agents from the engine's list, what of theirs waits on you and
// where that sounds, and the join line's secret part.
import { test } from "node:test";
import assert from "node:assert/strict";
import { stateOf, inNetwork, linkWords, skewWords, hostLine, hostTags, outboxWords, remoteAgents, remoteWaits, chimeHere, hostIdProblem, joinOf, joinFields, ALONE, TICKET_MS } from "../src/network-model.js";
import { NetworkState } from "../core/schema/network.js";

const NOW = Date.UTC(2026, 9, 20, 14, 30);
const host = (id, more = {}) => ({ id, keyId: `xnk1_${"a".repeat(32)}`, window: true, founder: false, gateway: false, online: true, seen: NOW, agents: 0, waitsOnYou: 0, you: false, platform: "darwin", ...more });
const founder = () => ({
  ...ALONE,
  network: { id: `net_${"0".repeat(32)}`, name: "home" },
  self: { id: "studio", keyId: `xnk1_${"b".repeat(32)}`, window: true, founder: true, gateway: true },
  gateway: { address: "gw.example.net", keyId: `xnk1_${"b".repeat(32)}`, kind: "tunnel", link: "live", since: NOW - 3600e3, reason: null, skewMs: 0 },
  hosts: [host("studio", { founder: true, gateway: true, you: true }), host("laptop", { agents: 2 }), host("rack", { window: false, online: false, seen: NOW - 7200e3, agents: 1 })],
  rosterHead: "abc",
});

test("anything but a network/state/v1 answer is a machine on its own, and the answers it reads are the schema's", () => {
  for (const raw of [null, undefined, "x", {}, { schema: "network/state/v2" }, { error: "not found" }]) {
    const s = stateOf(raw);
    assert.equal(inNetwork(s), false);
    assert.deepEqual([s.hosts, s.tickets, s.rosterMismatch], [[], [], []]);
  }
  NetworkState.parse(stateOf(null));
  const s = stateOf(founder());
  NetworkState.parse(s);
  assert.equal(inNetwork(s), true);
  // A list that isn't one, from an older or a newer service, is empty.
  assert.deepEqual(stateOf({ ...founder(), hosts: "nope", rosterMismatch: [5, "rack"] }).rosterMismatch, ["rack"]);
});

test("the link to the gateway in a word, with since when and why", () => {
  assert.equal(linkWords(null).state, "none");
  assert.match(linkWords(null).sub, /Set up… in the Actors panel/);
  assert.deepEqual([linkWords({ link: "live", since: NOW }).state, linkWords({ link: "live" }).word], ["ok", "Linked"]);
  const down = linkWords({ link: "offline", since: NOW - 25 * 60e3, reason: "timed out" }, NOW);
  assert.equal(down.word, "Can't reach the gateway");
  assert.match(down.sub, /since .*: timed out\. This machine keeps working/);
  assert.match(linkWords({ link: "removed" }).sub, /xenonet net leave/);
  assert.equal(linkWords({ link: "connecting" }).state, "wait");
  assert.equal(skewWords(500), "");
  assert.equal(skewWords(-3000), "clock 3 s off the gateway's");
  assert.equal(skewWords(5 * 60e3), "clock 5 min off the gateway's");
});

test("each machine's line and marks", () => {
  const s = founder();
  assert.equal(hostLine(s.hosts[1], NOW), "online · 2 agents");
  assert.equal(hostLine({ ...s.hosts[1], waitsOnYou: 1 }, NOW), "online · 2 agents · 1 waiting on you");
  assert.match(hostLine(s.hosts[2], NOW), /^offline since .+ · 1 agent when last seen$/);
  assert.deepEqual(hostTags(s.hosts[0], s.self), ["this machine", "you're here", "founder", "gateway", "window"]);
  assert.deepEqual(hostTags(s.hosts[2], s.self), []);
  assert.equal(outboxWords({ frames: 0, bytes: 0, oldest: null }), "Nothing waits to go to other machines.");
  assert.match(outboxWords({ frames: 3, bytes: 2816, oldest: NOW - 60e3 }, NOW), /^3 items wait in the outbox \(3 KB\), the oldest from .+\. They go when the gateway answers\.$/);
});

test("other machines' agents come from the engine's list by their @<host>: ids, however it words a wait", () => {
  const list = [
    { id: "claude:11111111-2222-3333-4444-555555555555", app: "claude", title: "Here", status: "working" },
    { id: "@laptop:claude:6f1c2b9e-0d4a-4c1e-9b7a-3e5d2a1f0c11", host: "laptop", app: "claude", title: "Fix the login redirect", status: "waiting", waitsOnYou: true, needsYou: "question", question: "Keep the query string?", since: NOW - 90e3 },
    { id: "@laptop:codex:019a7c55-1e2f-7a3b-8c4d-5e6f7a8b9c0d", app: "codex", title: "Tidy the tests", status: "idle", lastActive: NOW - 60e3 },
    { id: "@rack:claude:8d0f6a21-7b3c-4e9d-a1f2-5c6e7d8b9a03", title: "Import", needsYou: { kind: "permission", text: "Run the migration?", since: NOW - 10e3 } },
    { id: "@Bad Host:x" },
    null,
  ];
  const agents = remoteAgents(list);
  assert.deepEqual(agents.map((a) => [a.host, a.waits, a.kind, a.text]), [
    ["laptop", true, "question", "Keep the query string?"],
    ["laptop", false, null, null],
    ["rack", true, "permission", "Run the migration?"],
  ]);
  assert.deepEqual(remoteAgents("nope"), []);

  const s = founder();
  // You're here, at studio: a wait elsewhere sounds here.
  const waits = remoteWaits(s, agents);
  assert.deepEqual(waits.map((w) => [w.kind, w.host, w.sub, w.chime, w.stale]), [
    ["remote-need", "laptop", "is waiting for you", true, false],
    ["remote-need", "rack", "needs permission", true, true],
  ]);
  // You're at laptop: it sounds there, not here.
  const away = { ...s, hosts: s.hosts.map((x) => ({ ...x, you: x.id === "laptop" })) };
  assert.equal(chimeHere(away), false);
  assert.ok(remoteWaits(away, agents).every((w) => !w.chime));
  // At none of them: everywhere.
  assert.equal(chimeHere({ ...s, hosts: s.hosts.map((x) => ({ ...x, you: false })) }), true);
  // A machine that shares titles only sends no text: the card says where it shows.
  assert.equal(remoteWaits(s, [{ ...agents[0], text: null }])[0].text, "The question shows in laptop's console.");
  // Out of a network, nothing.
  assert.deepEqual(remoteWaits(stateOf(null), agents), []);
});

test("a new machine's id, and the join line's secret part", () => {
  const s = founder();
  assert.equal(hostIdProblem("desk", s), null);
  assert.match(hostIdProblem("Desk", s), /lowercase/);
  assert.match(hostIdProblem("d", s), /2 to 32/);
  assert.match(hostIdProblem("laptop", s), /Replace/);
  assert.equal(joinOf(null), null);
  assert.equal(joinOf({ ok: true }), null);
  assert.deepEqual(joinOf({ ticket: "KX7Q-49MZ-T2", address: "gw.example.net", expires: NOW }, NOW), { line: "xenonet net join gw.example.net KX7Q-49MZ-T2", secret: "KX7Q-49MZ-T2", expires: NOW });
  assert.deepEqual(joinOf({ line: "xenonet net join 10.0.0.5:47850 AAAA-BBBB-CC" }, NOW), { line: "xenonet net join 10.0.0.5:47850 AAAA-BBBB-CC", secret: "AAAA-BBBB-CC", expires: NOW + TICKET_MS });
  assert.deepEqual(joinFields(" xenonet net join gw.example.net KX7Q-49MZ-T2 ", ""), { address: "gw.example.net", ticket: "KX7Q-49MZ-T2" });
  assert.deepEqual(joinFields(" gw.example.net ", " KX7Q-49MZ-T2"), { address: "gw.example.net", ticket: "KX7Q-49MZ-T2" });
});
