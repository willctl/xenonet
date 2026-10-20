// A host's link (core/hosts/link.js): against relays a script plays on a
// made-up machine (test/harness/procs.js) with a clock the test moves.
// test/hosts-server.test.js runs one against a real relay and a real demo
// XenoNet, through a stand-in for ssh.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { HostLink, transport, whyEnded, shellPath, trustCommand, SSH_OPTIONS } from "../core/hosts/link.js";
import { hostEntry } from "../core/hosts/config.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeClock } from "./harness/clock.js";

const KEY = "9a2b0000000000000000000000000002";
const CONFIG = "/made/up/XenoNet/ssh_config";
const vm1 = () => hostEntry({ id: "vm1", target: "lab@127.0.0.1", sshPort: 60851 });

function world({ entry = vm1(), ...opts } = {}) {
  const clock = fakeClock();
  const machine = fakeProcesses({ clock });
  const link = new HostLink({ entry, sshConfig: CONFIG, spawn: machine.spawn, clock, random: () => 0.5, root: "/src/xenonet", ...opts });
  const states = [];
  const snapshots = [];
  const learned = [];
  const events = [];
  link.on("state", (i) => states.push(i.state));
  link.on("snapshot", (s) => snapshots.push(s));
  link.on("learn", (k) => learned.push(k));
  link.on("event", (e) => events.push(e));
  // Lets frames cross the made-up pipes both ways.
  const settle = () => clock.flush(30);
  return { clock, machine, link, states, snapshots, learned, events, settle };
}

const helloFrom = (clock, extra = {}) => ({ t: "hello", schema: "host/hello/v1", protocol: 1, version: "0.4.0", host: { id: "vm1", key: KEY }, platform: "linux", demo: false, now: clock.now(), sinceMs: 7 * 86400e3, detail: false, ...extra });
const snapshot = { t: "snapshot", at: 1, graph: { generatedAt: 1, nodes: [{ id: "core", kind: "core" }], links: [], relations: [], identities: [] }, live: { at: 1, sessions: [] }, engine: null };

// A relay a script plays: hello, a map, and answers to times (its clock
// three minutes ahead) and gets.
const relay =
  (clock, { hello = {}, snap = true, ahead = 180e3, onGet = null, keep = null } = {}) =>
  (io) => {
    keep?.(io);
    io.json(helloFrom(clock, hello));
    if (snap) io.json(snapshot);
    io.on("line", (l) => {
      const f = JSON.parse(l);
      if (f.t === "time") io.json({ t: "time", id: f.id, now: clock.now() + ahead });
      if (f.t === "get") (onGet ?? ((x, g) => x.json({ t: "res", id: g.id, status: 200, body: { path: g.path } })))(io, f);
    });
  };

test("a link says hello, takes the map, learns the key and the clock offset, and reads through the relay", async () => {
  const w = world();
  w.machine.program("ssh", relay(w.clock));
  w.link.start();
  await w.settle();
  assert.deepEqual(w.states, ["connecting", "live"]);
  assert.equal(w.snapshots.length, 1);
  assert.deepEqual(w.learned, [{ key: KEY, name: "vm1" }]);
  const info = w.link.info();
  assert.deepEqual([info.state, info.offsetMs, info.rttMs, info.target, info.hello.version, info.hello.platform], ["live", 180e3, 0, "lab@127.0.0.1:60851", "0.4.0", "linux"]);
  assert.deepEqual(await w.link.get("/api/session/claude:x/stats"), { status: 200, body: { path: "/api/session/claude:x/stats" } });

  const [{ file, args }] = w.machine.spawned;
  assert.equal(file, "ssh");
  assert.deepEqual(args, ["-F", CONFIG, ...SSH_OPTIONS, "--", "vm1", "xenonet", "relay", "--port", "47840", "--since", "7d"]);
  w.link.stop();
});

test("no hello in time: offline, then back after the backoff", async () => {
  const w = world();
  let runs = 0;
  w.machine.program("ssh", (io) => (++runs === 1 ? undefined : relay(w.clock)(io)));
  w.link.start();
  await w.settle();
  assert.equal(w.link.state, "connecting");
  await w.clock.advance(20e3);
  assert.equal(w.link.state, "offline");
  assert.deepEqual(w.link.info().error, { code: "no-hello", message: "vm1 didn't answer in 20 s" });
  assert.equal(w.link.info().nextTryAt - w.clock.now(), 2000);
  assert.equal(w.machine.running().length, 0, "the silent one was stopped");
  await w.clock.advance(2000);
  await w.settle();
  assert.equal(w.link.state, "live");
  assert.equal(w.link.info().error, null);
  w.link.stop();
});

test("a connection that keeps ending backs off from 2 to 60 seconds, and a live one starts the count again", async () => {
  const w = world();
  let up = false;
  let io = null;
  w.machine.program("ssh", (x) => (up ? relay(w.clock, { keep: (k) => (io = k) })(x) : (x.json({ t: "bye", code: "no-xenonet", message: "no XenoNet answers on 127.0.0.1:47840 on this machine" }), { code: 3 })));
  w.link.start();
  const waits = [];
  for (let i = 0; i < 7; i++) {
    await w.settle();
    assert.equal(w.link.state, "offline");
    waits.push(w.link.info().nextTryAt - w.clock.now());
    await w.clock.advance(waits.at(-1));
  }
  assert.deepEqual(waits, [2000, 4000, 8000, 16000, 32000, 60000, 60000]);
  assert.equal(w.link.info().error.code, "no-xenonet");
  assert.match(w.link.info().error.fix, /Start XenoNet on vm1/);
  up = true;
  await w.clock.advance(60000);
  await w.settle();
  assert.equal(w.link.state, "live");
  io.exit(1);
  await w.settle();
  assert.equal(w.link.state, "offline");
  assert.equal(w.link.info().nextTryAt - w.clock.now(), 2000, "after a good connection, the count starts again");
  w.link.stop();

  // The jitter spreads retries by a quarter either way.
  const low = world({ random: () => 0 });
  low.machine.program("ssh", () => ({ code: 1 }));
  low.link.start();
  await low.settle();
  assert.equal(low.link.info().nextTryAt - low.clock.now(), 1500);
  low.link.stop();
});

test("silence makes a host stale, and any frame brings it back", async () => {
  const w = world();
  let io = null;
  w.machine.program("ssh", relay(w.clock, { keep: (k) => (io = k) }));
  w.link.start();
  await w.settle();
  await w.clock.advance(40e3);
  assert.equal(w.link.state, "live");
  await w.clock.advance(10e3);
  assert.equal(w.link.state, "stale", "no frame for 45 seconds");
  io.json({ t: "ping", now: w.clock.now() });
  await w.settle();
  assert.equal(w.link.state, "live");
  io.json({ t: "event", name: "pulse", data: { nodeId: "claude:x", reason: "updated" } });
  await w.settle();
  assert.deepEqual(w.events, [{ name: "pulse", data: { nodeId: "claude:x", reason: "updated" } }]);
  w.link.stop();
});

test("a hello that's wrong ends the connection with what needs doing", async () => {
  const cases = [
    [{ hello: { protocol: 2, version: "0.9.0" } }, {}, "protocol", /runs XenoNet 0\.9\.0, whose relay speaks protocol 2; this XenoNet speaks 1/],
    [{ hello: { host: { id: "vm1" } } }, {}, "bad-hello", /isn't a relay's hello/],
    [{ hello: { schema: "host/hello/v9" } }, {}, "bad-hello", /isn't a relay's hello/],
    // A relay that says it's another XenoNet than the one this host was.
    [{}, { entry: { ...vm1(), key: "1111aaaa1111aaaa1111aaaa1111aaaa" } }, "key-changed", /another XenoNet answered as vm1/],
    [{}, { demoOnly: true }, "not-demo", /shows real sessions, and this XenoNet shows made-up data only/],
  ];
  for (const [script, opts, code, message] of cases) {
    const w = world(opts);
    w.machine.program("ssh", relay(w.clock, script));
    w.link.start();
    await w.settle();
    assert.equal(w.link.state, "error", code);
    assert.equal(w.link.info().error.code, code);
    assert.match(w.link.info().error.message, message);
    assert.equal(w.snapshots.length, 0, "nothing it sent is taken");
    assert.equal(w.link.info().nextTryAt - w.clock.now(), 60000, "tried again only every minute");
    assert.equal(w.machine.running().length, 0);
    w.link.stop();
  }
  // A demo world is made afresh at every start, key and all: its new key
  // is taken, and learned.
  const demo = world({ entry: { ...vm1(), key: "1111aaaa1111aaaa1111aaaa1111aaaa" } });
  demo.machine.program("ssh", relay(demo.clock, { hello: { demo: true } }));
  demo.link.start();
  await demo.settle();
  assert.equal(demo.link.state, "live");
  assert.deepEqual(demo.learned, [{ key: KEY, name: "vm1" }]);
  demo.link.stop();
});

test("what ssh says becomes a reason, and a fix", async () => {
  const entry = vm1();
  const say = (exitCode, stderr, extra = {}) => whyEnded({ entry, exitCode, stderr, sshConfig: "/Users/nova/Library/Application Support/XenoNet/ssh_config", tilde: (p) => p.replace("/Users/nova", "~"), ...extra });
  const hostKey = say(255, "No ED25519 host key is known for [127.0.0.1]:60851 and you have requested strict checking.\r\nHost key verification failed.\r\n");
  assert.deepEqual([hostKey.state, hostKey.code], ["error", "host-key"]);
  assert.equal(hostKey.fix, "Check it and accept it once in a terminal: ssh -F ~/'Library/Application Support/XenoNet/ssh_config' vm1", "a ~ the shell expands: quoted, neither it nor ssh would");
  assert.equal(say(255, "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@").code, "host-key");
  assert.deepEqual([say(255, "lab@127.0.0.1: Permission denied (publickey).").code, say(255, "lab@127.0.0.1: Permission denied (publickey).").state], ["refused", "error"]);
  const away = say(255, "ssh: connect to host 127.0.0.1 port 60851: Connection refused");
  assert.deepEqual([away.state, away.code, away.message], ["offline", "unreachable", "ssh couldn't reach vm1: connect to host vm1 port 60851: Connection refused"], "the machine's own name stays out of it");
  assert.equal(say(255, "ssh: Could not resolve hostname 127.0.0.1: nodename nor servname provided").message, "ssh couldn't reach vm1: Could not resolve hostname vm1: nodename nor servname provided");
  assert.equal(say(1, "", { bye: { code: "weird", message: "lab@127.0.0.1 said no" } }).message, "the relay on vm1 stopped: vm1 said no");
  assert.equal(say(255, "ssh: Could not resolve hostname vm9: nodename nor servname provided").code, "unreachable");
  for (const s of ["bash: xenonet: command not found", "zsh:1: command not found: xenonet", "sh: 1: xenonet: not found"]) assert.equal(say(127, s).code, "no-relay", s);
  assert.equal(say(null, "", { spawnError: Object.assign(new Error("spawn ssh ENOENT"), { code: "ENOENT" }) }).code, "no-ssh");
  assert.deepEqual([say(5, "", { bye: { code: "old-xenonet", message: "x" } }).state, say(4, "", { bye: { code: "xenonet-gone" } }).state], ["error", "offline"]);
  assert.equal(say(1, "boom").message, "the relay on vm1 stopped (exit 1): boom");

  // And through a link: ssh failing before any frame.
  const w = world();
  w.machine.program("ssh", { stderr: "Host key verification failed.\n", code: 255 });
  w.link.start();
  await w.settle();
  assert.deepEqual([w.link.state, w.link.info().error.code], ["error", "host-key"]);
  w.link.stop();
  const none = world();
  none.link.start(); // no ssh on this made-up machine at all
  await none.settle();
  assert.deepEqual([none.link.state, none.link.info().error.code], ["error", "no-ssh"]);
  none.link.stop();
});

test("the command to accept a host's key runs as it's printed", { skip: process.platform === "win32" && "no sh to try it in" }, () => {
  const config = "~/Library/Application Support/XenoNet Dev/ssh_config";
  assert.equal(shellPath(config), "~/'Library/Application Support/XenoNet Dev/ssh_config'");
  assert.equal(shellPath("~/.config/xenonet/ssh_config"), "~/.config/xenonet/ssh_config", "quotes only where they're needed");
  assert.equal(shellPath("/opt/xenonet state/ssh_config"), "'/opt/xenonet state/ssh_config'", "outside the home folder, quoted whole");
  assert.equal(trustCommand("vm1", "/Users/nova/x y/ssh_config", (p) => p.replace("/Users/nova", "~")), "ssh -F ~/'x y/ssh_config' vm1");
  assert.equal(trustCommand("vm1", null), "ssh vm1");
  // What a shell makes of each printed word, with a made-up home folder:
  // the path ssh is given, ~ and all expanded, quotes and all taken off.
  for (const [printed, path] of [
    [shellPath(config), "/Users/nova/Library/Application Support/XenoNet Dev/ssh_config"],
    [shellPath("~/it's here/ssh_config"), "/Users/nova/it's here/ssh_config"],
    [shellPath("/opt/xenonet state/ssh_config"), "/opt/xenonet state/ssh_config"],
  ]) {
    const sh = spawnSync("/bin/sh", ["-c", `printf '%s' ${printed}`], { env: { HOME: "/Users/nova", PATH: "/usr/bin:/bin" }, encoding: "utf8" });
    assert.equal(sh.stdout, path, printed);
  }
});

test("frames a host mustn't send are dropped, and the link carries on", async () => {
  const w = world();
  w.machine.program("ssh", (io) => {
    io.json({ t: "snapshot", ...snapshot, graph: { nodes: [{ id: "early" }] } }); // before the hello
    io.json(helloFrom(w.clock));
    io.write(`{"t":"event","name":"live","data":"${"x".repeat(17 << 20)}"}\n`); // over 16 MB
    io.json({ t: "exec", cmd: "touch /tmp/pwned" });
    io.write("not json\n");
    io.json({ t: "res", id: 999, status: 200, body: {} }); // nothing was asked
    io.json({ t: "snapshot", graph: "not a map", live: {} });
    io.json({ t: "event", name: "x".repeat(100), data: {} });
    io.write(`{"t":"event","name":"live","data":{"deep":${"[".repeat(100000)}${"]".repeat(100000)}}}\n`); // would overflow the stack
    io.json(snapshot);
  });
  w.link.start();
  await w.settle();
  assert.equal(w.link.state, "live");
  assert.equal(w.snapshots.length, 1);
  assert.equal(w.snapshots[0].graph.nodes[0].id, "core", "only the real one");
  assert.equal(w.link.info().counts.dropped, 8);
  assert.equal(w.events.length, 0);
  w.link.stop();
});

test("a get waits only so long, and ends with the connection", async () => {
  const w = world();
  let io = null;
  w.machine.program("ssh", relay(w.clock, { keep: (k) => (io = k), onGet: () => {} }));
  w.link.start();
  await w.settle();
  const slow = assert.rejects(w.link.get("/api/graph", { timeoutMs: 1000 }), (err) => err.status === 504 && /didn't answer in 1 s/.test(err.message));
  await w.clock.advance(1000);
  await slow;
  const cut = assert.rejects(w.link.get("/api/graph"), (err) => err.status === 503);
  io.exit(1);
  await w.settle();
  await cut;
  await assert.rejects(w.link.get("/api/graph"), (err) => err.status === 503 && /isn't connected \(offline\)/.test(err.message));
  w.link.stop();
});

test("ssh's arguments are fixed words and checked values, and the local relay runs without this XenoNet's settings", () => {
  const entry = hostEntry({ id: "vm1", target: "lab@vm1.local", command: "/home/lab/.local/bin/xenonet", since: "12h", detail: true, start: true });
  const t = transport(entry, { sshConfig: CONFIG });
  assert.deepEqual([t.file, t.args], ["ssh", ["-F", CONFIG, ...SSH_OPTIONS, "--", "vm1", "/home/lab/.local/bin/xenonet", "relay", "--port", "47840", "--since", "12h", "--detail", "--start"]]);
  for (const w of t.args.slice(t.args.indexOf("--") + 1)) assert.match(w, /^[A-Za-z0-9_./:=+-]+$/, "nothing a shell reads as more than a word");
  const bad = [
    { ...entry, id: "-oProxyCommand=evil" },
    { ...entry, command: "xenonet; rm -rf ~" },
    { ...entry, command: "$(touch /tmp/x)" },
    { ...entry, port: "47840 && id" },
    { ...entry, since: "7d; id" },
    { ...entry, transport: "telnet" },
  ];
  for (const e of bad) assert.throws(() => transport(e, { sshConfig: CONFIG }), undefined, JSON.stringify(e));
  assert.throws(() => transport(entry, {}), /ssh config/);
  assert.deepEqual(transport(entry, { sshConfig: CONFIG, sshBin: ["/usr/bin/node", "fake-ssh.mjs"] }).args.slice(0, 3), ["fake-ssh.mjs", "-F", CONFIG]);

  // The local transport runs the xenonet binary's relay, with no Node and
  // none of this XenoNet's XENONET_ settings.
  const local = transport(hostEntry({ id: "xenonet", local: true, port: 47840 }), { bin: "/x/launcher/xenonet", env: { PATH: "/usr/bin", XENONET_DEMO: "1", XENONET_PORT: "47843", XENONET_STATE_DIR: "/x", HOME: "/Users/nova" } });
  assert.deepEqual([local.file, local.args], ["/x/launcher/xenonet", ["relay", "--port", "47840", "--since", "7d"]]);
  assert.deepEqual(local.env, { PATH: "/usr/bin", HOME: "/Users/nova" });
  // By default, the checkout's own build; with none built yet, the JS
  // service runs the checkout's own relay on this Node.
  assert.match(transport(hostEntry({ id: "xenonet", local: true, port: 47840 }), { root: "/src/xenonet", env: {} }).file, /^\/src\/xenonet\/target\/(release|debug)\/xenonet(\.exe)?$/, "xenonet.exe on Windows");
  const checkout = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-link-checkout-"));
  try {
    fs.mkdirSync(path.join(checkout, "cli"));
    fs.writeFileSync(path.join(checkout, "cli", "xenonet.js"), "");
    const js = transport(hostEntry({ id: "xenonet", local: true, port: 47840 }), { root: checkout, env: {} });
    assert.deepEqual([js.file, js.args], [process.execPath, [path.join(checkout, "cli", "xenonet.js"), "relay", "--port", "47840", "--since", "7d"]]);
  } finally {
    fs.rmSync(checkout, { recursive: true, force: true });
  }
});
