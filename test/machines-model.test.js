// The machines and their load (src/machines-model.js), as the load panel and
// the Machines view show them: this computer on its own, and in a network
// every machine of the roster with the network's totals, each machine's
// part of each, the view's filters and sorts, and the sessions each has on
// the map. Made-up readings; no browser.
import { test } from "node:test";
import assert from "node:assert/strict";
import { machinesOf, totalsOf, filterMachines, sortMachines, sessionsByMachine, HOST_COLORS, STALE_MS } from "../src/machines-model.js";
import { HERE } from "../src/hosts.js";

const GB = 1024 ** 3;
const NOW = 1_800_000_000_000;

const reading = ({ host, at = NOW, cpu, threads, used, total, gpus = [], platform = "linux" }) => ({
  ...(host ? { host } : {}),
  at,
  platform,
  arch: "x64",
  uptime: 3600,
  cpu: { model: "CPU", threads, ghz: 3, usage: cpu },
  memory: { used: used * GB, total: total * GB },
  gpus: gpus.map((g, index) => ({ index, name: g.name ?? "GPU", usage: g.usage, memUsed: g.used == null ? null : g.used * GB, memTotal: g.total == null ? null : g.total * GB, temp: null, shared: !!g.shared })),
  gpu: gpus.length ? "nvidia-smi" : null,
  hardware: { platform, arch: "x64", cpu: { model: "CPU", threads, ghz: 3 }, memory: { total: total * GB }, gpus: gpus.map((g) => ({ name: g.name ?? "GPU", memTotal: g.total == null ? null : g.total * GB, shared: !!g.shared })) },
});

const row = (id, extra = {}) => ({ id, keyId: "x", window: true, founder: false, gateway: false, online: true, seen: NOW, agents: 0, waitsOnYou: 0, you: false, platform: "linux", ...extra });
const net = (hosts, self = "studio", window = true) => ({ schema: "network/state/v1", network: { id: "net_x", name: "home" }, self: { id: self, window, founder: true, gateway: true }, hosts });

// studio (here): 16 threads at 50%, 64 GB with 32 used, an NVIDIA card at
// 80% with 8 of its 16 GB. laptop: an Apple chip, 8 threads at 25%, 32 GB
// with 8 used, its GPU at 20% sharing the memory. rack: offline.
const here = reading({ cpu: 0.5, threads: 16, used: 32, total: 64, gpus: [{ usage: 0.8, used: 8, total: 16, name: "RTX" }] });
const laptop = reading({ host: "laptop", cpu: 0.25, threads: 8, used: 8, total: 32, platform: "darwin", gpus: [{ usage: 0.2, used: 4, total: 32, shared: true, name: "M3" }] });
const rack = reading({ host: "rack", at: NOW - 3600e3, cpu: 0.9, threads: 64, used: 100, total: 128, gpus: [{ usage: 1, used: 10, total: 24 }, { usage: 1, used: 10, total: 24 }] });

test("on its own, it's this computer: its own load, with no id", () => {
  const m = machinesOf(here, null, NOW);
  assert.equal(m.network, false);
  assert.equal(m.machines.length, 1);
  assert.deepEqual([m.machines[0].id, m.machines[0].here, m.machines[0].counted, m.machines[0].key], [null, true, true, HERE]);
  assert.equal(m.totals.cpu, 0.5);
  assert.equal(m.totals.ram.share, 0.5);
  assert.deepEqual([m.totals.gpu.mean, m.totals.gpu.count, m.totals.gpu.vram.share], [0.8, 1, 0.5]);
});

test("in a network: the roster's machines, the network's sums over those counted, and each one's part", () => {
  const m = machinesOf({ ...here, hosts: [laptop, rack] }, net([row("studio"), row("laptop", { platform: "darwin" }), row("rack", { online: false, seen: NOW - 3600e3, window: false })]), NOW);
  assert.equal(m.network, true);
  assert.deepEqual(m.machines.map((x) => [x.id, x.here, x.online, x.counted, x.color]), [["studio", true, true, true, 0], ["laptop", false, true, true, 1], ["rack", false, false, false, 2]]);
  const t = m.totals;
  assert.deepEqual([t.machines, t.online, t.counted, t.threads], [3, 2, 2, 24], "rack is offline: not in the sums");
  // CPU: the busy share of every thread, (0.5 × 16 + 0.25 × 8) / 24.
  assert.equal(t.cpu, 10 / 24);
  // Memory: every byte, 40 of 96 GB.
  assert.equal(t.ram.share, 40 / 96);
  // GPU: a mean of every GPU's use; Apple's shared one counts its use, but
  // its memory is memory, not VRAM.
  assert.equal(t.gpu.mean, 0.5);
  assert.deepEqual(t.gpu.vram, { used: 8 * GB, total: 16 * GB, share: 0.5 });
  assert.deepEqual(t.gpu.busiest, { host: "studio", here: true, name: "RTX", usage: 0.8 });
  // Each machine's part, which adds up to the ring.
  const sum = (k) => t.parts[k].reduce((s, p) => s + p.v, 0);
  assert.deepEqual(t.parts.cpu.map((p) => [p.id, p.v]), [["studio", 8 / 24], ["laptop", 2 / 24]]);
  assert.ok(Math.abs(sum("cpu") - t.cpu) < 1e-12 && Math.abs(sum("gpu") - t.gpu.mean) < 1e-12 && Math.abs(sum("ram") - t.ram.share) < 1e-12);
  // rack's last reading stays, for its parts, with since when it's away.
  assert.equal(m.machines[2].reading.hardware.gpus.length, 2);
  assert.equal(m.machines[2].why, null);
});

test("a machine with no reading says why; one too old isn't counted; the palette goes round", () => {
  const roster = [row("studio"), row("laptop"), row("den")];
  const m = machinesOf({ ...here, hosts: [{ ...laptop, at: NOW - STALE_MS - 1 }] }, net(roster), NOW);
  assert.deepEqual(m.machines.map((x) => [x.id, x.counted, x.why]), [["studio", true, null], ["laptop", false, null], ["den", false, "none"]]);
  // This machine isn't a window host: the others' load doesn't come here.
  const alone = machinesOf(here, net(roster, "studio", false), NOW);
  assert.deepEqual(alone.machines.map((x) => x.why), [null, "window", "window"]);
  assert.equal(alone.windowed, false);
  // This machine missing from the roster still comes first.
  assert.deepEqual(machinesOf(here, net([row("laptop")]), NOW).machines.map((x) => x.id), ["studio", "laptop"]);
  const many = machinesOf(here, net(Array.from({ length: HOST_COLORS + 2 }, (_, i) => row(i ? `m${i}` : "studio"))), NOW);
  assert.deepEqual(many.machines.map((x) => x.color), [0, 1, 2, 3, 4, 5, 0, 1]);
  // Nothing counted: no sums, rather than zeros.
  assert.deepEqual([totalsOf([]).cpu, totalsOf([]).ram.share, totalsOf([]).gpu.mean], [null, null, null]);
});

test("the view's filters (the Filters chips' keys, a platform, a GPU, online) and sorts", () => {
  const m = machinesOf({ ...here, hosts: [laptop, rack] }, net([row("studio"), row("laptop", { platform: "darwin" }), row("rack", { online: false, seen: NOW - 3600e3 })]), NOW).machines;
  const ids = (list) => list.map((x) => x.id);
  assert.deepEqual(ids(filterMachines(m, { keys: [HERE, "rack"] })), ["studio", "rack"]);
  assert.deepEqual(ids(filterMachines(m, { platform: "darwin" })), ["laptop"]);
  assert.deepEqual(ids(filterMachines(m, { online: true })), ["studio", "laptop"]);
  assert.deepEqual(ids(filterMachines(m, { gpu: true })), ["studio", "laptop", "rack"]);
  assert.deepEqual(ids(filterMachines([...m, { ...m[0], id: "bare", key: "bare", reading: null }], { gpu: true })), ["studio", "laptop", "rack"]);
  assert.deepEqual(ids(sortMachines(m, "name")), ["laptop", "rack", "studio"]);
  assert.deepEqual(ids(sortMachines(m, "cpu")), ["rack", "studio", "laptop"]);
  assert.deepEqual(ids(sortMachines(m, "seen")), ["laptop", "studio", "rack"], "online first, then by when they were heard");
});

test("each machine's sessions on the map, by app: this machine's under @here", () => {
  const graph = { nodes: [{ kind: "session", app: "claude" }, { kind: "session", app: "codex" }, { kind: "session", app: "claude", host: "laptop" }, { kind: "project" }] };
  const c = sessionsByMachine(graph);
  assert.deepEqual(c.get(HERE), { total: 2, apps: { claude: 1, codex: 1 } });
  assert.deepEqual(c.get("laptop"), { total: 1, apps: { claude: 1 } });
  assert.equal(sessionsByMachine(null).size, 0);
});
