// The doctor's view of hosts (core/doctor/index.js): with hosts on, a check
// for each, which says whether it's reachable, whether its relay answers in
// a protocol this XenoNet speaks, and whether its clock is within two
// minutes; without them, nothing new. And the hosts' own check of them all
// (core/hosts/doctor.js): that no two answer with the same key.
import { test } from "node:test";
import assert from "node:assert/strict";
import { machine, runDoctor, hostCheck, CHECKS } from "../core/doctor/index.js";
import { keyCheck, keyChecks } from "../core/hosts/doctor.js";

const NOW = Date.now();
const vm1 = (extra = {}) => ({ id: "vm1", state: "live", since: NOW - 60e3, lastHeard: NOW - 2e3, offsetMs: 300, version: "0.4.0", platform: "linux", demo: false, error: null, cached: false, target: "lab@127.0.0.1:60851", ...extra });

test("each state, said in words, with what to do", () => {
  assert.deepEqual(hostCheck(vm1(), NOW), { status: "ok", detail: "live: XenoNet 0.4.0 on linux" });
  assert.deepEqual(hostCheck(vm1({ offsetMs: -40e3, demo: true }), NOW), { status: "ok", detail: "live: XenoNet 0.4.0 on linux, with made-up data, its clock 40 s behind this one's" });
  const skewed = hostCheck(vm1({ offsetMs: 3 * 60e3 }), NOW);
  assert.equal(skewed.status, "warn");
  assert.equal(skewed.detail, "live, but its clock 3 min ahead of this one's");
  assert.match(skewed.fix, /timedatectl set-ntp true/);
  const keyed = hostCheck(vm1({ state: "error", error: { code: "host-key", message: "ssh doesn't know vm1's host key, or it changed", fix: "Check it and accept it once in a terminal: ssh -F '~/x/ssh_config' vm1" } }), NOW);
  assert.deepEqual(keyed, { status: "fail", detail: "ssh doesn't know vm1's host key, or it changed", fix: "Check it and accept it once in a terminal: ssh -F '~/x/ssh_config' vm1" });
  const away = hostCheck(vm1({ state: "offline", lastHeard: NOW - 14 * 60e3, cached: true, error: { code: "unreachable", message: "ssh couldn't reach vm1: Connection refused", fix: "Check that vm1 is on and reachable." } }), NOW);
  assert.deepEqual(away, { status: "warn", detail: "offline, last heard 14 min ago: ssh couldn't reach vm1: Connection refused; the map shows its last snapshot", fix: "Check that vm1 is on and reachable." });
  assert.equal(hostCheck(vm1({ state: "stale", lastHeard: NOW - 50e3 }), NOW).detail, "no word from it for 50 s");
  assert.deepEqual(hostCheck(vm1({ state: "connecting" }), NOW), { status: "info", detail: "connecting to lab@127.0.0.1:60851" });
});

test("a check per host when hosts are on, chosen with --only hosts, and none without", async () => {
  const ctx = machine({ hosts: () => [vm1(), vm1({ id: "vm2", state: "offline", error: { message: "no XenoNet is running on vm2" } })] });
  const run = await runDoctor(ctx, { only: ["hosts"] });
  assert.deepEqual(run.results.map((r) => [r.id, r.group, r.title, r.status]), [["hosts.vm1", "Hosts", "vm1 is connected", "ok"], ["hosts.vm2", "Hosts", "vm2 is connected", "warn"]]);
  assert.deepEqual(run.summary, { ok: 1, info: 0, warn: 1, fail: 0, skip: 0 });
  assert.equal((await runDoctor(machine({ hosts: () => [vm1()] }), { only: ["hosts.vm1"] })).results.length, 1);
  assert.ok(!(await runDoctor(machine({ hosts: null }), { only: ["xenonet.server"] })).results.some((r) => r.group === "Hosts"));
  assert.ok(!CHECKS.some((c) => c.id.startsWith("hosts.")), "nothing new in the list everyone runs");
});

test("the doctor runs the hosts' check of their keys beside each host's, with --only hosts too", async () => {
  const KEY = "7c1e0000000000000000000000000001";
  const run = await runDoctor(machine({ hosts: () => [vm1({ key: KEY }), vm1({ id: "vm2", key: KEY })] }), { only: ["hosts"] });
  assert.deepEqual(run.results.map((r) => [r.id, r.status]), [["hosts.vm1", "ok"], ["hosts.vm2", "ok"], ["hosts.keys.own", "warn"]]);
  assert.match(run.results.at(-1).detail, /^vm1 and vm2 answer with the same key/);
  const one = await runDoctor(machine({ hosts: () => [vm1({ key: KEY })] }), { only: ["hosts"] });
  assert.deepEqual(one.results.map((r) => r.id), ["hosts.vm1"], "nothing to compare with one host");
});

test("two hosts that answer with one key, and so share a snapshot in the cache, get a warning that says what to do", async () => {
  const KEY = { a: "7c1e0000000000000000000000000001", b: "7c1e0000000000000000000000000002" };
  // Nothing to compare: no hosts, one, or hosts that haven't answered yet.
  assert.deepEqual(keyChecks(null), []);
  assert.deepEqual(keyChecks([vm1({ key: KEY.a })]), []);
  assert.deepEqual(keyChecks([vm1(), vm1({ id: "vm2" })]), [], "no key learned yet");
  // XenoNet Dev's simulated hosts keep nothing in the cache.
  assert.deepEqual(keyChecks([vm1({ key: KEY.a, simulated: true }), vm1({ id: "vm2", key: KEY.a, simulated: true })]), []);

  const [own] = keyChecks([vm1({ key: KEY.a }), vm1({ id: "vm2", key: KEY.b })]);
  assert.deepEqual([own.id, own.group, own.title], ["hosts.keys.own", "Hosts", "Each host has a key of its own"]);
  assert.deepEqual(await own.run(), { status: "ok", detail: "vm1 and vm2 each answer with a key of their own" });

  // A state folder copied from one machine to another: vm2 and studio say
  // they're the same XenoNet.
  const [check] = keyChecks([vm1({ key: KEY.a }), vm1({ id: "vm2", key: KEY.b }), vm1({ id: "studio", key: KEY.b, state: "offline", cached: true })]);
  const r = await check.run();
  assert.equal(r.status, "warn");
  assert.match(r.detail, /^studio and vm2 answer with the same key, so they keep one snapshot in the cache between them/);
  assert.match(r.fix, /If studio and vm2 are one XenoNet reached two ways, remove one: xenonet hosts remove vm2\./);
  assert.match(r.fix, /take "self" out of hosts\.json in its state folder so it makes a key of its own.*xenonet hosts add vm2 … --replace\.$/);
  assert.doesNotMatch(r.detail, /vm1/, "vm1's key is its own");
  // Two pairs at once are both named.
  assert.match(keyCheck([vm1({ key: KEY.a }), vm1({ id: "vm2", key: KEY.a }), vm1({ id: "lab", key: KEY.b }), vm1({ id: "pi", key: KEY.b }), vm1({ id: "nas", key: KEY.b })]).detail, /^vm1 and vm2 answer with the same key; lab, nas and pi answer with the same key, so/);
});
