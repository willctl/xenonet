// The network across machines as the console words it (src/network-model.js):
// the state read leniently, the link in a word, each machine's line, other
// machines' agents from the engine's list with the detail their machine
// shares, what of theirs waits on you or seems stuck, where what waits
// sounds, and the join line's secret part.
import { test } from "node:test";
import assert from "node:assert/strict";
import { stateOf, inNetwork, linkWords, hostLine, hostTags, inviteWords, remoteAgents, remoteWaits, chimeHere, hostIdProblem, joinOf, joinFields, proposalWords, frozenWords, reachOf, hostCan, widenWords, delegationWords, STARTS, START_WORDS, ALONE, INVITE_MS, AWAY_MS } from "../src/network-model.js";
import { parse } from "./harness/schema.js";

const NOW = Date.UTC(2026, 9, 20, 14, 30);
const host = (id, more = {}) => ({ id, keyId: `xnk1_${"a".repeat(32)}`, window: true, founder: false, online: true, seen: NOW, agents: 0, waitsOnYou: 0, you: false, platform: "darwin", ...more });
const founder = () => ({
  ...ALONE,
  network: { id: `net_${"0".repeat(32)}`, name: "home" },
  self: { id: "studio", keyId: `xnk1_${"b".repeat(32)}`, window: true, founder: true },
  link: { state: "live", since: NOW - 3600e3, reason: null },
  hosts: [host("studio", { founder: true, you: true }), host("laptop", { agents: 2 }), host("rack", { window: false, online: false, seen: NOW - 7200e3, agents: 1 })],
  rosterHead: "abc",
});

test("anything but a network/state/v1 answer is a machine on its own, and the answers it reads are the schema's", () => {
  for (const raw of [null, undefined, "x", {}, { schema: "network/state/v2" }, { error: "not found" }]) {
    const s = stateOf(raw);
    assert.equal(inNetwork(s), false);
    assert.deepEqual([s.hosts, s.invites.open, s.rosterMismatch], [[], [], []]);
  }
  parse("network/state/v1", stateOf(null));
  const s = stateOf(founder());
  parse("network/state/v1", s);
  assert.equal(inNetwork(s), true);
  // A list that isn't one, from an older or a newer service, is empty.
  assert.deepEqual(stateOf({ ...founder(), hosts: "nope", rosterMismatch: [5, "rack"] }).rosterMismatch, ["rack"]);
});

test("a machine in several networks: each listed, the first at the top, and the one picked in its place", () => {
  const work = { id: `net_${"e".repeat(32)}`, name: "work", host: "studio-w", hosts: [host("studio-w"), host("lab")], link: { state: "live", since: NOW, reason: null }, self: { id: "studio-w", keyId: `xnk1_${"d".repeat(32)}`, window: false, founder: false }, sharing: ["prj_web", 5], relays: ["https://relay.example.net", null] };
  const raw = { ...founder(), sharing: "all", networks: [{ id: founder().network.id, name: "home", host: "studio", hosts: founder().hosts, sharing: "all", relays: [] }, work, { name: "no id" }] };
  const first = stateOf(raw);
  assert.deepEqual(first.networks.map((n) => [n.name, n.host]), [["home", "studio"], ["work", "studio-w"]], "one with no id isn't listed");
  assert.deepEqual([first.network.name, first.self.id, first.hosts.length], ["home", "studio", 3]);
  const picked = stateOf(raw, work.id);
  assert.deepEqual([picked.network, picked.self.id, picked.hosts.map((x) => x.id), picked.link.since], [{ id: work.id, name: "work" }, "studio-w", ["studio-w", "lab"], NOW]);
  assert.deepEqual([picked.sharing, picked.relays], [["prj_web"], ["https://relay.example.net"]], "read leniently");
  assert.equal(stateOf(raw, "net_gone").network.name, "home", "a network that isn't there any more: the first");
  parse("network/state/v1", picked);
});

test("how this machine stands in the network, in a word, with since when and why", () => {
  assert.equal(linkWords(null).state, "off");
  assert.deepEqual([linkWords({ state: "live", since: NOW }).state, linkWords({ state: "live" }).word], ["ok", "In the network"]);
  assert.match(linkWords({ state: "live", since: NOW - 60e3 }, NOW).sub, /^Reaching its machines since /);
  const out = linkWords({ state: "removed", since: NOW, reason: "removed by studio" }, NOW);
  assert.equal(out.state, "bad");
  assert.match(out.sub, /: removed by studio\. Run xenonet net leave/);
  assert.equal(linkWords({ state: "off" }).word, "Off");
});

test("each machine's line and marks", () => {
  const s = founder();
  assert.equal(hostLine(s.hosts[1], NOW), "online · 2 agents");
  assert.equal(hostLine({ ...s.hosts[1], waitsOnYou: 1 }, NOW), "online · 2 agents · 1 waiting on you");
  assert.match(hostLine(s.hosts[2], NOW), /^offline since .+ · 1 agent when last seen$/);
  assert.deepEqual(hostTags(s.hosts[0], s.self), ["this machine", "you're here", "founder", "window"]);
  assert.deepEqual(hostTags(s.hosts[2], s.self), []);
  // What its mailboxes hold for a machine that's away.
  assert.match(hostLine({ ...s.hosts[2], held: 3 }, NOW), /^offline since .+ · 1 agent when last seen · 3 items held for it$/);
  assert.match(inviteWords({ host: "desk", window: true, expires: NOW + 60e3, wrong: 2 }, NOW), /^desk, with a window: works until .+ \(2 wrong codes\)$/);
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

test("detail from another machine: the question's text, the last message and why it seems stuck, cut to the schema's lengths", () => {
  const [a, b, c] = remoteAgents([
    { id: "@laptop:claude:6f1c2b9e-0d4a-4c1e-9b7a-3e5d2a1f0c11", host: "laptop", title: "Fix the login redirect", waitsOnYou: true, needsYou: "question", question: "q".repeat(500), lastMessage: "<b>done</b>", stuck: null, projectKey: "prj_atlasweb", cwd: "~/src/atlas-web" },
    { id: "@laptop:codex:019a7c55-1e2f-7a3b-8c4d-5e6f7a8b9c0d", host: "laptop", title: "Tidy the tests", status: "working", stuck: ["ran cargo test 3 times in a row", 7, ""], lastMessage: "  " },
    // A machine that shares titles only: it waits, with no text.
    { id: "@rack:claude:8d0f6a21-7b3c-4e9d-a1f2-5c6e7d8b9a03", host: "rack", title: "Import", waitsOnYou: true, question: null, focus: "importing" },
  ]);
  assert.deepEqual([a.text.length, a.lastMessage, a.projectKey, a.cwd, a.stuck], [320, "<b>done</b>", "prj_atlasweb", "~/src/atlas-web", null]);
  assert.deepEqual([b.waits, b.stuck, b.lastMessage], [false, ["ran cargo test 3 times in a row"], null]);
  assert.deepEqual([c.waits, c.text], [true, null], "what it says it's doing is never taken for its question");
  // An id that isn't its host's is dropped.
  assert.deepEqual(remoteAgents([{ id: "@laptop:claude:x", host: "rack" }]), []);
  const cards = remoteWaits(founder(), [a, b, c], NOW);
  assert.deepEqual(cards.map((x) => [x.kind, x.id === a.id ? "a" : x.id === b.id ? "b" : "c", x.text.slice(0, 32)]), [
    ["remote-need", "a", "q".repeat(32)],
    ["remote-stuck", "b", "ran cargo test 3 times in a row"],
    ["remote-need", "c", "The question shows in rack's con"],
  ]);
  assert.equal(cards[1].name, "Tidy the tests");
});

test("what waits sounds where you last worked, or everywhere after five minutes away from every console", () => {
  const s = founder();
  const at = (studio, laptop, more = {}) => ({ ...s, hosts: s.hosts.map((x) => ({ ...x, you: false, youAt: x.id === "studio" ? studio : x.id === "laptop" ? laptop : null, ...(more[x.id] ?? {}) })) });
  // A machine on its own always chimes.
  assert.equal(chimeHere(stateOf(null), NOW), true);
  // You worked here last: here, not on laptop.
  assert.equal(chimeHere(at(NOW - 20e3, NOW - 40 * 60e3), NOW), true);
  const fromLaptop = { ...at(NOW - 20e3, NOW - 40 * 60e3), self: { ...s.self, id: "laptop" } };
  assert.equal(chimeHere(fromLaptop, NOW), false);
  // You moved to laptop: there, not here.
  assert.equal(chimeHere(at(NOW - 3 * 60e3, NOW - 30e3), NOW), false);
  // Both in the same half minute: both.
  assert.equal(chimeHere(at(NOW - 30e3, NOW - 30e3), NOW), true);
  // Five minutes away from every console: everywhere.
  assert.equal(chimeHere(at(NOW - AWAY_MS - 60e3, NOW - AWAY_MS), NOW), true);
  // A machine that's down doesn't keep you there.
  assert.equal(chimeHere(at(NOW - 3 * 60e3, NOW - 30e3, { laptop: { online: false } }), NOW), true);
  // The cards say so, local or not: it's one rule.
  const agents = remoteAgents([{ id: "@laptop:claude:a", host: "laptop", title: "t", waitsOnYou: true, question: "?" }]);
  assert.equal(remoteWaits(at(NOW - 3 * 60e3, NOW - 30e3), agents, NOW)[0].chime, false);
});

test("a new machine's id, and the join line's secret part", () => {
  const s = founder();
  assert.equal(hostIdProblem("desk", s), null);
  assert.match(hostIdProblem("Desk", s), /lowercase/);
  assert.match(hostIdProblem("d", s), /2 to 32/);
  assert.match(hostIdProblem("laptop", s), /Replace/);
  assert.equal(joinOf(null), null);
  assert.equal(joinOf({ ok: true }), null);
  assert.equal(joinOf({ ticket: "KX7Q-49MZ-T2", address: "gw.example.net" }), null, "a ticket is no invite");
  const invite = "xni2_aGVsbG8";
  // Through the network's relay, the line needs no address; with none, it
  // names where the inviter is, or the LAN.
  assert.deepEqual(joinOf({ invite, code: "KX7Q-49MZ-T2", relay: "https://relay.example.net", expires: NOW }, NOW), { line: `xenonet net join ${invite} KX7Q-49MZ-T2`, secret: "KX7Q-49MZ-T2", expires: NOW });
  assert.deepEqual(joinOf({ invite, code: "AAAA-BBBB-CC", relay: null, at: ["192.168.1.20:47861"] }, NOW), { line: `xenonet net join ${invite} AAAA-BBBB-CC --at 192.168.1.20:47861`, secret: "AAAA-BBBB-CC", expires: NOW + INVITE_MS });
  assert.equal(joinOf({ invite, code: "AAAA-BBBB-CC", lan: true }, NOW).line, `xenonet net join ${invite} AAAA-BBBB-CC --mdns`);
  assert.deepEqual(joinFields(` xenonet net join ${invite} KX7Q-49MZ-T2 --at 10.0.0.5:47861 `, ""), { invite, code: "KX7Q-49MZ-T2", at: "10.0.0.5:47861", lan: false });
  assert.deepEqual(joinFields(`xenonet net join ${invite} KX7Q-49MZ-T2 --mdns`, ""), { invite, code: "KX7Q-49MZ-T2", at: null, lan: true });
  assert.deepEqual(joinFields(` ${invite} `, " KX7Q-49MZ-T2"), { invite, code: "KX7Q-49MZ-T2", at: null, lan: false });
});

test("what waits for approval says what it does and who asked", () => {
  const add = { id: "e1", op: "add", host: "den", window: true, signers: ["laptop"], at: NOW };
  const settings = { id: "e2", op: "settings", host: "desk", window: false, signers: ["desk"], at: NOW };
  assert.equal(proposalWords(add), "add den as a window host, asked by laptop");
  assert.equal(proposalWords(settings), "settings desk, asked by desk");
  // Read leniently: one without an id can't be approved, so it isn't listed.
  const s = stateOf({ ...founder(), proposals: [add, { op: "add", host: "x1" }, null] });
  assert.deepEqual(s.proposals.map((p) => p.id), ["e1"]);
  assert.deepEqual(stateOf(null).proposals, []);
});

test("a frozen feed says which machine signed two entries at one place", () => {
  const f = { author: "xns1_simulatedlaptopsigningkeysofhome", host: "laptop", at: { inc: 1794400000000, seq: 7 } };
  assert.equal(frozenWords(f), "laptop signed two different entries at one place in its feed (entry 7), so nothing more from it is taken here");
  assert.match(frozenWords({ author: f.author, host: null }), /^xns1_simulatedlaptopsigningkeysofhome signed/);
  const s = stateOf({ ...founder(), frozen: [f, { host: "x1" }, null] });
  assert.deepEqual(s.frozen.map((x) => x.host), ["laptop"]);
  assert.deepEqual(stateOf(null).frozen, []);
});

test("who may start and wake sessions on each machine, and what its XenoNet takes", () => {
  const s = stateOf({ ...founder(), start: "network", wake: "consent" });
  parse("network/state/v1", s);
  assert.deepEqual([s.start, s.wake], ["network", "consent"]);
  assert.deepEqual([stateOf(null).start, stateOf(null).wake], ["you", "consent"], "you, and your consent, unless widened");
  const widen = { id: "wid_1", setting: "start", value: "network", by: "laptop", at: NOW };
  const desk = host("desk", { window: false, reach: { start: "you", wake: "network", pending: [widen, { setting: "start" }, null] }, can: ["start", "answer"] });
  assert.deepEqual(reachOf(desk), { start: "you", wake: "network", pending: [widen], delegated: null });
  assert.equal(widenWords(widen, "desk"), "laptop asked to let the network start sessions on desk");
  // A machine on delegate says who put it there and when; the widening to it says so too.
  const delegated = host("desk", { window: false, reach: { start: "delegate", wake: "consent", pending: [{ ...widen, value: "delegate" }], delegated: { by: "studio", at: NOW } } });
  parse("network/state/v1", stateOf({ ...founder(), hosts: [delegated], start: "delegate", delegated: { by: "studio", at: NOW } }));
  assert.deepEqual(reachOf(delegated), { start: "delegate", wake: "consent", pending: [{ ...widen, value: "delegate" }], delegated: { by: "studio", at: NOW } });
  assert.equal(widenWords({ ...widen, value: "delegate" }, "desk"), "laptop asked to put desk on your standing delegation");
  assert.match(delegationWords({ by: "studio", at: NOW }, NOW), /^Delegated by studio, [^.]+\. Sessions and actors that ask this machine for work run without asking about reading, editing files under the folder they start in, routine commands/);
  assert.match(delegationWords(null), /^Delegated\. Sessions and actors/);
  assert.ok(START_WORDS.delegate && STARTS.includes("delegate"));
  assert.ok(hostCan(desk, "start") && hostCan(desk, "answer"));
  // An older XenoNet says neither: it takes no start.
  const old = host("rack", { reach: null });
  assert.equal(reachOf(old), null);
  assert.equal(hostCan(old, "start"), false);
  assert.deepEqual(reachOf(host("x", { reach: { start: "everyone", wake: 3 } })), { start: "you", wake: "consent", pending: [], delegated: null }, "read leniently");
});
