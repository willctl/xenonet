// The delivery router and its ways in (core/delivery): a Claude Code
// channel's long poll, an OpenCode server found by its process and port,
// typing into a tmux pane only when every check holds, and the states a
// message goes through: live, next turn, notified, acked.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { DeliveryRouter } from "../core/delivery/router.js";
import { DeliveryPolicy } from "../core/delivery/policy.js";
import { Channels } from "../core/delivery/channel.js";
import { createOpenCode, openCodeId, parsePs, listeningPorts } from "../core/delivery/opencode.js";
import { createTmux } from "../core/delivery/tmux.js";
import { channelEnabled, listenChannel } from "../mcp/xenonet.js";

const A = "claude:aaaaaaaa-0001", O = "opencode:ses-cccccccc0003", B = "codex:bbbbbbbb-0002";

function rig({ views = {}, opencode = null, tmux = null, ackMs = 50, pid = 4242 } = {}) {
  const engine = new EventEmitter();
  const seen = [];
  engine.view = async (id) => ({ id, title: id === "actor:muse" ? "Muse" : `t-${id}`, status: "idle", ...views[id] });
  engine.store = { markSeen: (agent, ids) => seen.push([agent, ...ids]), markUnseen: (agent, ids) => seen.push(["unseen", agent, ...ids]), contact: () => ({ pid }) };
  const policy = new DeliveryPolicy();
  const channels = new Channels();
  const notes = [];
  const router = new DeliveryRouter({ engine, policy, channels, opencode, tmux, ackMs, notify: (n) => notes.push(n) });
  const states = [];
  router.on("delivery", (d) => states.push(`${d.agent}:${d.state}${d.ackedAt ? ":acked" : ""}`));
  return { engine, policy, channels, router, seen, notes, states };
}
const msg = (over = {}) => ({ id: 1, from: "actor:muse", to: [A], text: "hello", ...over });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

test("a Claude Code session listening on its channel gets the message live, framed; its next hook acks it", async () => {
  const r = rig();
  const poll = r.channels.poll(A, 1000);
  const [rec] = await r.router.route(msg());
  assert.equal(rec.state, "live");
  assert.equal(rec.how, "channel");
  const [item] = await poll;
  assert.match(item.text, /^\[XenoNet from Muse\] \(message #1;[\s\S]*hello$/);
  assert.deepEqual(r.seen, [[A, 1]], "taken, so the hooks don't hand it over again");
  r.engine.emit("contact", { agent: A, via: "mcp" });
  assert.equal(rec.ackedAt, null, "the channel's own poll isn't an ack");
  r.engine.emit("contact", { agent: A, via: "hook" });
  assert.ok(rec.ackedAt);
});

test("a channel only wakes an idle session: a working one gets the message from its hooks, during the turn under way", async () => {
  const r = rig({ views: { [A]: { status: "working" } } });
  r.channels.poll(A, 1000);
  const [rec] = await r.router.route(msg());
  assert.equal(rec.state, "next-turn");
  assert.deepEqual(r.seen, []);
});

test("a push nothing acked goes back to the hooks, so a channel Claude Code dropped still reaches the next turn", async () => {
  const r = rig();
  r.channels.poll(A, 1000);
  const [rec] = await r.router.route(msg());
  assert.equal(rec.how, "channel");
  await sleep(80);
  assert.equal(rec.state, "notified");
  assert.deepEqual(r.seen, [[A, 1], ["unseen", A, 1]]);
  assert.equal(r.notes.length, 1);
});

test("the sender hears how each message will get there: right away through a channel or OpenCode, else from the hooks", async () => {
  const r = rig({ opencode: { deliver: async () => ({ ok: true }) } });
  const idle = { id: A, app: "claude", status: "idle" };
  assert.equal(r.router.way("you", idle), null, "no channel open");
  r.channels.poll(A, 1000);
  assert.equal(r.router.way("you", idle), "right away, through its channel");
  assert.equal(r.router.way("you", { ...idle, status: "working" }), null);
  r.policy.setPaused(true);
  assert.equal(r.router.way("you", idle), null, "paused: no wakes");
  r.policy.setPaused(false);
  const oc = { id: O, app: "opencode", status: "working" };
  assert.equal(r.router.way("you", oc), null, "no consent");
  r.policy.allow(O);
  assert.equal(r.router.way("you", oc), "right away, through its OpenCode server");
  assert.equal(r.router.way("actor:muse", oc), null, "an actor needs remote consent");
});

test("no way in: the message waits for the next turn, is acked when the hooks hand it over, and else you're told after the ack time", async () => {
  let r = rig();
  let [rec] = await r.router.route(msg({ to: [B] }));
  assert.equal(rec.state, "next-turn");
  r.engine.emit("seen", { agent: B, ids: [1] });
  assert.ok(rec.ackedAt);
  await sleep(80);
  assert.equal(r.notes.length, 0);

  r = rig();
  [rec] = await r.router.route(msg({ to: [B], id: 2 }));
  await sleep(80);
  assert.equal(rec.state, "notified");
  assert.equal(r.notes[0].agent, B);
  assert.deepEqual(r.states, [`${B}:next-turn`, `${B}:notified`]);
});

test("OpenCode: only with your consent for the session; then prompt_async on the server that has it, acked by its answer", async () => {
  const sent = [];
  const opencode = { deliver: async (id, text) => (sent.push([id, text]), { ok: true, where: "http://127.0.0.1:5555" }) };
  const r = rig({ opencode });
  let [rec] = await r.router.route(msg({ to: [O] }));
  assert.equal(rec.state, "next-turn");
  assert.match(rec.why, /no consent/);
  r.policy.allow(O, { remote: true });
  [rec] = await r.router.route(msg({ to: [O], id: 3 }));
  assert.equal(rec.state, "live");
  assert.ok(rec.ackedAt);
  assert.equal(sent.length, 1);
});

test("wakes are held by the pause switch and the wake budget; a working session isn't woken by typing", async () => {
  const typed = [];
  const tmux = { deliver: async (x) => (typed.push(x), { ok: true, where: "%1" }) };
  let r = rig({ tmux });
  r.policy.allow(B, { remote: true });
  r.policy.setPaused(true);
  let [rec] = await r.router.route(msg({ to: [B] }));
  assert.match(rec.why, /no wake: paused/);
  r.policy.setPaused(false);
  [rec] = await r.router.route(msg({ to: [B], id: 5 }));
  assert.equal(rec.how, "tmux");
  r = rig({ tmux, views: { [B]: { status: "working" } } });
  r.policy.allow(B, { remote: true });
  [rec] = await r.router.route(msg({ to: [B], id: 6 }));
  assert.equal(rec.state, "next-turn");
});

test("typing needs a sender XenoNet verified by token, or you; a session's message never types", async () => {
  const tmux = { deliver: async () => ({ ok: true, where: "%1" }) };
  const r = rig({ tmux });
  r.policy.allow(B);
  const [rec] = await r.router.route(msg({ to: [B], from: A }));
  assert.match(rec.why, /only you or an actor/);
  const [mine] = await r.router.route(msg({ to: [B], from: "you", id: 9 }));
  assert.equal(mine.how, "tmux");
});

test("tmux: types only into the pane whose terminal runs the session in its foreground, checked twice, literally, one line", async () => {
  const calls = [];
  let fg = "S+";
  const exec = async (cmd, args) => {
    calls.push([cmd, ...args]);
    if (cmd === "ps") return { ok: true, out: `pts/3 ${fg}\n` };
    if (args[0] === "list-panes") return { ok: true, out: "%1\t100\t/dev/pts/2\tbash\n%7\t200\t/dev/pts/3\tclaude\n" };
    return { ok: true, out: "" };
  };
  const t = createTmux({ exec, platform: "linux" });
  const r = await t.deliver({ pid: 4242, text: "hi\nthere\x1b[2J\x03" });
  assert.deepEqual(r, { ok: true, where: "%7" });
  const sends = calls.filter((c) => c[1] === "send-keys");
  assert.deepEqual(sends[0], ["tmux", "send-keys", "-t", "%7", "-l", "--", "hi there"]);
  assert.deepEqual(sends[1], ["tmux", "send-keys", "-t", "%7", "Enter"]);
  fg = "S";
  calls.length = 0;
  assert.equal((await t.deliver({ pid: 4242, text: "x" })).ok, false, "in the background: nothing typed");
  assert.equal(calls.filter((c) => c[1] === "send-keys").length, 0);
  assert.equal((await createTmux({ exec, platform: "win32" }).deliver({ pid: 4242, text: "x" })).ok, false);
});

test("OpenCode discovery: its processes, their listening ports, the server that has the session in hand", async () => {
  assert.equal(openCodeId("opencode:ses-abc123"), "ses_abc123");
  assert.deepEqual(parsePs(" 12 opencode\n 13 /usr/bin/vim\n"), [{ pid: 12, args: "opencode" }, { pid: 13, args: "/usr/bin/vim" }]);
  const lsof = await listeningPorts(12, { platform: "darwin", exec: async () => "p12\nn127.0.0.1:41234\nn*:5000\n" });
  assert.deepEqual(lsof, [41234, 5000]);
  const proc = await listeningPorts(12, {
    platform: "linux",
    readdir: () => ["3", "4"],
    readlink: (p) => (p.endsWith("/3") ? "socket:[999]" : "/dev/null"),
    readFile: (f) => (f.endsWith("tcp") ? "  sl local rem st tx rx tr tm retr uid timeout inode\n   0: 0100007F:A0E2 00000000:0000 0A 00000000:00000000 00:00000000 00000000 1000 0 999\n" : ""),
  });
  assert.deepEqual(proc, [0xa0e2]);
  const posted = [];
  const fetch = async (url, opts = {}) => {
    const u = new URL(url);
    if (opts.method === "POST") return (posted.push(u.port + u.pathname), { status: 204, ok: true });
    const body = u.pathname === "/global/health" ? { healthy: true } : u.pathname === "/session/status" ? (u.port === "2" ? { ses_abc123: { type: "idle" } } : {}) : {};
    return { ok: true, status: 200, json: async () => body };
  };
  const oc = createOpenCode({ fetch, processes: async () => [{ pid: 1, args: "opencode" }, { pid: 2, args: "opencode --port 2" }], ports: async () => [1] });
  const r = await oc.deliver("opencode:ses-abc123", "hi");
  assert.deepEqual(r, { ok: true, where: "http://127.0.0.1:2" });
  assert.deepEqual(posted, ["2/session/ses_abc123/prompt_async"]);
});

test("channel: a poll waits and answers with what comes; items queue between polls; no live poll, no push", async () => {
  let now = 0;
  const ch = new Channels({ now: () => now });
  assert.equal(ch.push(A, { id: 1 }), false);
  const p = ch.poll(A, 1000);
  assert.ok(ch.push(A, { id: 1 }));
  assert.deepEqual(await p, [{ id: 1 }]);
  assert.ok(ch.push(A, { id: 2 }), "a poll just ended: the next one is on its way");
  assert.deepEqual(await ch.poll(A, 1000), [{ id: 2 }]);
  now += 10e3;
  assert.equal(ch.push(A, { id: 3 }), false);
});

test("the MCP server listens as a channel only when the session was started with XenoNet as one, and pushes notifications/claude/channel", async () => {
  const env = { CLAUDE_CODE_SESSION_ID: "abc", CLAUDE_PID: "77" };
  const o = { env, name: "xenonet" };
  assert.equal(channelEnabled({ ...o, cmdline: () => "claude --resume x" }), false);
  assert.equal(channelEnabled({ ...o, cmdline: () => "claude --dangerously-load-development-channels server:xenonet" }), true);
  assert.equal(channelEnabled({ ...o, name: "xenonet-dev", cmdline: () => "claude --channels plugin:x server:xenonet-dev" }), true);
  assert.equal(channelEnabled({ ...o, cmdline: () => "claude --channels plugin:x server:xenonet-dev" }), false, "XenoNet Dev's channel, not this server's");
  assert.equal(channelEnabled({ ...o, env: { ...env, XENONET_CHANNEL: "1" }, cmdline: () => "" }), true);
  assert.equal(channelEnabled({ ...o, env: { XENONET_CHANNEL: "1" }, cmdline: () => "" }), false, "not Claude Code");
  // Started from inside another session, which left its own CLAUDE_PID: the
  // parent is Claude Code, so it's the one asked.
  const lines = { 10: "/usr/local/bin/claude --dangerously-load-development-channels server:xenonet", 77: "claude --resume other" };
  assert.equal(channelEnabled({ ...o, ppid: 10, cmdline: (pid) => lines[pid] }), true);
  assert.equal(channelEnabled({ ...o, ppid: 10, cmdline: (pid) => ({ ...lines, 10: "claude", 77: lines[10] })[pid] }), false);
  // A launcher between them: CLAUDE_PID names the session.
  assert.equal(channelEnabled({ ...o, ppid: 10, cmdline: (pid) => ({ 10: "/bin/sh -c xenonet mcp", 77: lines[10] })[pid] }), true);
  const sent = [];
  const server = { server: { notification: async (n) => sent.push(n) } };
  await listenChannel(() => server, { poll: async () => ({ items: [{ id: 4, from: "Muse\n<x>", text: "[XenoNet from Muse] hi" }] }), once: true });
  assert.deepEqual(sent, [{ method: "notifications/claude/channel", params: { content: "[XenoNet from Muse] hi", meta: { from: "Musex", message_id: "4" } } }]);
});
