// The OpenCode demo (scripts/demo/opencode-named.mjs) and the tools it gives
// each OpenCode session (scripts/demo/opencode-tool.js): what each round
// asks, the sessions' config, the tools' calls to XenoNet, and the timings
// read back from XenoNet's records and OpenCode's. Made-up sessions,
// messages and times; the real run needs OpenCode and tmux.
import { test } from "node:test";
import assert from "node:assert/strict";
import { plan, kickoff, parseArgs, sh, seededConsent, openCodeConfig, agentsMd, timings, toolCalls, arrivals, stats, report, PROJECTS } from "../scripts/demo/opencode-named.mjs";
import * as tool from "../scripts/demo/opencode-tool.js";

test("each round asks by title words and by selector, and says which round it is", () => {
  const three = plan(3, 2);
  assert.deepEqual(three.map((s) => [s.from, s.to, s.expect]), [["atlas", "docs site", 2], ["cirrus", "app:opencode", 3], ["borealis", "project:cirrus app:opencode", 2]]);
  assert.ok(three.every((s) => s.text.endsWith("(round 2)")), "so the same question isn't refused as a repeat a minute later");
  assert.deepEqual(plan(2, 1).map((s) => [s.from, s.to]), [["atlas", "docs site"], ["borealis", "project:atlas app:opencode"]]);
  assert.match(kickoff(three[0]), /xenonet_send with to "docs site" and text "What's your release codename\? \(round 2\)"/);
  assert.ok(PROJECTS.every((p) => p.title.toLowerCase().includes(p.name)), "each title names its project");
});

test("the sessions read and message, and do nothing else", () => {
  const c = openCodeConfig("opencode/big-pickle");
  assert.equal(c.model, "opencode/big-pickle");
  for (const t of ["bash", "edit", "write", "webfetch", "task"]) assert.equal(c.tools[t], false, t);
  assert.equal(c.permission.bash, "deny");
  assert.equal(c.share, "disabled");
  assert.match(agentsMd(PROJECTS[0]), /answer it once with xenonet_reply/);
  assert.match(agentsMd(PROJECTS[0]), /Never answer a reply/);
});

test("options: defaults, limits, and nothing it doesn't know", () => {
  const o = parseArgs([]);
  assert.equal(o.sessions, 3);
  assert.equal(o.model, "opencode/big-pickle");
  assert.equal(o.seedConsent, false);
  assert.equal(parseArgs(["--sessions", "2", "--rounds", "4", "--seed-consent"]).rounds, 4);
  assert.throws(() => parseArgs(["--sessions", "5"]), /2 or 3/);
  assert.throws(() => parseArgs(["--rounds", "9"]), /wake budget/);
  assert.throws(() => parseArgs(["--model", "big pickle"]), /provider\/model/);
  assert.throws(() => parseArgs(["--port"]), /needs a value/);
  assert.throws(() => parseArgs(["--yes"]), /unknown option/);
});

test("shell words and the seeded consent file", () => {
  assert.equal(sh("it's"), `'it'\\''s'`);
  const data = seededConsent(["opencode:ses-a1b2c3d4"], { now: 1000, hours: 1, previous: { paused: false, grants: {}, consent: { "opencode:ses-old0000": { until: 5 } } } });
  assert.deepEqual(data.consent["opencode:ses-a1b2c3d4"], { until: 1000 + 3600e3, remote: false, at: 1000 });
  assert.ok(data.consent["opencode:ses-old0000"], "what was there stays");
});

test("the tools call XenoNet as the OpenCode session, from its folder and process", async () => {
  assert.equal(tool.agentId("ses_eb8073beeffe"), "opencode:ses-eb8073beeffe");
  for (const name of ["peers", "send", "reply", "inbox"]) {
    const t = tool[name];
    assert.equal(typeof t.description, "string");
    assert.equal(typeof t.execute, "function");
    for (const [k, v] of Object.entries(t.args)) assert.ok(v.type && v.description, `${name}.${k} is JSON Schema with a description`);
  }
  const calls = [];
  const fetch = async (url, opts) => (calls.push({ url, opts }), { ok: true, json: async () => ({ id: 7, to: [{ id: "opencode:ses-b", title: "Borealis: docs site", project: "borealis", delivery: "right away, through its OpenCode server" }] }) });
  const ctx = { sessionID: "ses_aaaa1111", directory: "/w/atlas" };
  const out = await tool.call("send", { to: "docs site", text: "hi" }, ctx, { fetch });
  assert.equal(out.id, 7);
  assert.match(calls[0].url, /^http:\/\/127\.0\.0\.1:\d+\/api\/engine\/send$/);
  assert.equal(calls[0].opts.headers["x-xenonet-agent"], "opencode:ses-aaaa1111");
  assert.deepEqual(JSON.parse(calls[0].opts.body), { to: "docs site", text: "hi", cwd: "/w/atlas", pid: process.pid });
  assert.equal(tool.sentText(out), "Sent as message #7 to:\n- Borealis: docs site (opencode:ses-b, borealis): right away, through its OpenCode server");
  const refused = await tool.call("send", {}, ctx, { fetch: async () => ({ ok: false, status: 400, json: async () => ({ error: "no session or actor matches \"x\"" }) }) });
  assert.equal(tool.sentText(refused), 'Not sent: no session or actor matches "x"');
  const down = await tool.call("send", {}, ctx, { fetch: async () => { throw new Error("ECONNREFUSED"); } });
  assert.match(down.error, /isn't answering/);
  assert.equal(tool.inboxText([]), "No new messages.");
  assert.equal(tool.inboxText([{ id: 3, from: { id: "opencode:ses-c", title: "Cirrus" }, text: "hello", replyTo: 2 }]), "#3 from Cirrus (opencode:ses-c), in reply to #2:\nhello");
  assert.equal(tool.peersText([]), "No other session XenoNet can reach is at work.");
});

// Two sessions as OpenCode keeps them: Atlas asks Borealis (message #1) by
// title words; XenoNet puts it into Borealis, whose turn answers (#2).
function made() {
  const A = "opencode:ses-atlas0001", B = "opencode:ses-borea0002";
  const t0 = 1_000_000;
  const toolPart = (tool, input, id, start) => ({ type: "tool", tool, state: { status: "completed", input, output: `Sent as message #${id} to:\n- …`, time: { start, end: start + 40 } } });
  const sessions = {
    [A]: { title: "Atlas: billing API", messages: [
      { info: { id: "msg_a1", role: "user", time: { created: t0 - 3000 } }, parts: [{ type: "text", text: "Send one message…" }] },
      { info: { id: "msg_a2", role: "assistant", parentID: "msg_a1", time: { created: t0 - 2990, completed: t0 + 500 } }, parts: [toolPart("xenonet_send", { to: "docs site", text: "Codename? (round 1)" }, 1, t0)] },
      { info: { id: "msg_a3", role: "user", time: { created: t0 + 4100 } }, parts: [{ type: "text", text: "[XenoNet from Borealis: docs site] (message #2; sender verified by XenoNet. …)\nLarch (round 1)" }] },
      { info: { id: "msg_a4", role: "assistant", parentID: "msg_a3", time: { created: t0 + 4120, completed: t0 + 6000 } }, parts: [{ type: "text", text: "Borealis says Larch." }] },
    ] },
    [B]: { title: "Borealis: docs site", messages: [
      { info: { id: "msg_b1", role: "user", time: { created: t0 + 60 } }, parts: [{ type: "text", text: "[XenoNet from Atlas: billing API] (message #1; sender verified by XenoNet. …)\nCodename? (round 1)" }] },
      { info: { id: "msg_b2", role: "assistant", parentID: "msg_b1", time: { created: t0 + 75, completed: t0 + 4200 } }, parts: [{ type: "tool", tool: "read", state: { status: "completed", input: {}, output: "…" } }, toolPart("xenonet_reply", { message: 1, text: "Larch (round 1)" }, 2, t0 + 4000)] },
    ] },
  };
  const messages = [
    { id: 1, from: A, target: B, text: "Codename? (round 1)", at: t0 + 25, to: [{ id: B, seen: true }] },
    { id: 2, from: B, target: A, text: "Larch (round 1)", at: t0 + 4030, to: [{ id: A, seen: true }] },
  ];
  const deliveries = [
    { message: 1, agent: B, state: "live", how: "opencode", at: t0 + 26, ackedAt: t0 + 58 },
    { message: 2, agent: A, state: "live", how: "opencode", at: t0 + 4031, ackedAt: t0 + 4090 },
  ];
  return { A, B, t0, sessions, messages, deliveries };
}

test("timings: from the sender's tool call to XenoNet, to OpenCode's server, to the turn, to the answer", () => {
  const m = made();
  assert.deepEqual(toolCalls(m.sessions[m.B].messages).map((c) => [c.tool, c.message, c.input.message]), [["xenonet_reply", 2, 1]]);
  assert.deepEqual(arrivals(m.sessions[m.B].messages).get(1), { in: m.t0 + 60, turn: m.t0 + 75, done: m.t0 + 4200 });
  const rows = timings(m);
  assert.deepEqual(rows.map((r) => [r.message, r.by, r.asked, r.accepted, r.pushed, r.stored, r.turn, r.answered]), [
    [1, "title words", "docs site", 25, 58, 60, 75, 4030],
    [2, "reply", null, 30, 90, 100, 120, null],
  ]);
  assert.equal(rows[0].from, "Atlas: billing API");
  assert.equal(rows[0].how, "opencode");
  const text = report(rows);
  assert.match(text, /#1 +Atlas: billing API +Borealis: docs site +title words: docs site +25 ms +58 ms +75 ms +4\.0 s/);
  assert.match(text, /the recipient's turn starts +98 ms +75 ms to 120 ms +\(2\)/);
  assert.doesNotMatch(text, /didn't go in through/);
  assert.match(text, /#1 Atlas: billing API, to "docs site": Codename\? \(round 1\)\n  #2 Borealis: docs site, replying: Larch \(round 1\)/);
});

test("timings: a message to a session outside the demo, or one that waited for a turn, is told apart", () => {
  const m = made();
  m.messages.push({ id: 3, from: m.A, target: "app:opencode", text: "x", at: m.t0 + 7000, to: [{ id: "opencode:ses-elsewhere", seen: false }, { id: m.B, seen: false }] });
  m.sessions[m.A].messages.push({ info: { id: "msg_a5", role: "assistant", time: { created: m.t0 + 6900 } }, parts: [{ type: "tool", tool: "xenonet_send", state: { status: "completed", input: { to: "app:opencode", text: "x" }, output: "Sent as message #3 to:", time: { start: m.t0 + 6950 } } }] });
  m.deliveries.push({ message: 3, agent: m.B, state: "next-turn", how: "hooks", at: m.t0 + 7001, ackedAt: null });
  const rows = timings(m);
  const third = rows.filter((r) => r.message === 3);
  assert.equal(third.length, 1, "only the demo's own sessions");
  assert.equal(third[0].by, "selector");
  assert.equal(third[0].pushed, null);
  assert.match(report(rows), /1 of 3 didn't go in through OpenCode's server: #3 next-turn/);
});

test("stats: median and range, leaving out what wasn't measured", () => {
  assert.deepEqual(stats([5, null, 1, 3]), { n: 3, median: 3, min: 1, max: 5 });
  assert.deepEqual(stats([4, 1, 3, 2]), { n: 4, median: 2.5, min: 1, max: 4 });
  assert.equal(stats([null, undefined]), null);
});

test("timings: a question by selector counts each recipient's own answer", () => {
  const m = made();
  const C = "opencode:ses-cirru0003";
  const t = m.t0 + 10_000;
  const call = (tool, input, id, start) => ({ type: "tool", tool, state: { status: "completed", input, output: `Sent as message #${id} to:`, time: { start } } });
  const asked = (id, at) => ({ info: { id: `msg_in${id}`, role: "user", time: { created: at } }, parts: [{ type: "text", text: `[XenoNet from Cirrus] (message #${id}; …)\nWhich version? (round 1)` }] });
  m.sessions[C] = { title: "Cirrus: mobile app", messages: [{ info: { id: "msg_c1", role: "assistant", time: { created: t - 10 } }, parts: [call("xenonet_send", { to: "app:opencode", text: "Which version? (round 1)" }, 3, t)] }] };
  m.sessions[m.A].messages.push(asked(3, t + 30), { info: { id: "msg_a9", role: "assistant", time: { created: t + 40 } }, parts: [call("xenonet_reply", { message: 3, text: "2.4.1" }, 5, t + 900)] });
  m.sessions[m.B].messages.push(asked(3, t + 35), { info: { id: "msg_b9", role: "assistant", time: { created: t + 45 } }, parts: [call("xenonet_reply", { message: 3, text: "0.9.0" }, 4, t + 600)] });
  m.messages.push(
    { id: 3, from: C, target: "app:opencode", text: "Which version? (round 1)", at: t + 20, to: [{ id: m.A }, { id: m.B }] },
    { id: 4, from: m.B, target: C, text: "0.9.0", at: t + 610, to: [{ id: C }] },
    { id: 5, from: m.A, target: C, text: "2.4.1", at: t + 910, to: [{ id: C }] },
  );
  const rows = timings(m).filter((r) => r.message === 3);
  assert.deepEqual(rows.map((r) => [r.to, r.answered]), [["Atlas: billing API", 910], ["Borealis: docs site", 610]]);
});
