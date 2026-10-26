// The machines XenoNet runs on and their load, as the load panel
// (system-panel.js) and the Machines view (machines.js) show them. On its
// own, that's this computer. In a network (network mode) it's every
// machine of the roster, in the roster's order: this one's reading from
// GET /api/system, the others' from GET /api/system?hosts=1, which each
// machine's relay sends while a window host's console watches it (their
// load reaches window hosts' consoles only). No DOM here: the panel, the
// view, the driver and the tests share it.
import { inNetwork } from "./network-model.js";
import { HERE } from "./hosts.js";

// The host palette's size: --host-0 … --host-5 in style.css, clear of the
// accent, the apps' colors and violet, which the rings and the map use.
export const HOST_COLORS = 6;
// Another machine's reading older than this isn't counted: its relay sends
// one every 5 s while it's watched.
export const STALE_MS = 30e3;
export const OS_NAME = { darwin: "macOS", win32: "Windows", linux: "Linux" };
export const PLATFORMS = ["darwin", "win32", "linux"];
export const SORTS = ["name", "cpu", "gpu", "ram", "seen"];

const list = (v) => (Array.isArray(v) ? v : []);
const num = (v) => (typeof v === "number" && Number.isFinite(v) ? v : null);
const share = (v) => (num(v) == null ? null : Math.min(1, Math.max(0, v)));

// A reading kept to what the panel and the view use, with numbers only
// where they're numbers.
function readingOf(r) {
  if (!r || typeof r !== "object") return null;
  const cpu = r.cpu ?? {};
  const hw = r.hardware ?? {};
  return {
    at: num(r.at),
    platform: typeof r.platform === "string" ? r.platform : null,
    arch: typeof r.arch === "string" ? r.arch : null,
    uptime: num(r.uptime),
    cpu: { usage: share(cpu.usage), threads: num(cpu.threads) ?? 0, model: typeof cpu.model === "string" ? cpu.model : null, ghz: num(cpu.ghz) },
    memory: { used: num(r.memory?.used) ?? 0, total: num(r.memory?.total) ?? 0 },
    gpus: list(r.gpus).map((g) => ({ name: String(g?.name ?? "GPU"), usage: share(g?.usage), memUsed: num(g?.memUsed), memTotal: num(g?.memTotal), temp: num(g?.temp), shared: !!g?.shared })),
    source: typeof r.gpu === "string" ? r.gpu : null,
    hardware: {
      platform: typeof hw.platform === "string" ? hw.platform : null,
      arch: typeof hw.arch === "string" ? hw.arch : null,
      cpu: { model: typeof hw.cpu?.model === "string" ? hw.cpu.model : null, threads: num(hw.cpu?.threads), ghz: num(hw.cpu?.ghz) },
      memory: { total: num(hw.memory?.total) },
      gpus: list(hw.gpus).map((g) => ({ name: String(g?.name ?? "GPU"), memTotal: num(g?.memTotal), shared: !!g?.shared })),
    },
  };
}

// A machine's GPUs' mean use, or null with none read.
const gpuMean = (gpus) => {
  const read = gpus.filter((g) => g.usage != null);
  return read.length ? read.reduce((s, g) => s + g.usage, 0) / read.length : null;
};

// One machine: its roster row and its reading. `why` says why one with no
// reading has none: off (offline), window (this machine isn't a window
// host, and only those get others' load), or none (not shared, or not yet).
function machine(row, r, { here, color, now, windowed }) {
  const reading = readingOf(r);
  const online = here || row?.online !== false;
  const fresh = !!reading && (here || (reading.at != null && now - reading.at < STALE_MS));
  const cpu = reading?.cpu.usage ?? null;
  const ram = reading?.memory.total ? reading.memory.used / reading.memory.total : null;
  return {
    id: row?.id ?? null,
    key: here ? HERE : row.id,
    here,
    online,
    seen: num(row?.seen),
    window: !!row?.window,
    founder: !!row?.founder,
    gateway: !!row?.gateway,
    platform: reading?.platform ?? (typeof row?.platform === "string" ? row.platform : null),
    color,
    reading,
    counted: online && fresh,
    cpu,
    gpu: reading ? gpuMean(reading.gpus) : null,
    ram: ram == null ? null : Math.min(1, Math.max(0, ram)),
    why: reading ? null : !online ? "off" : !windowed ? "window" : "none",
  };
}

// The network's load: the busy share of every thread, of every GPU (a
// mean, with the dedicated memory beside it; a GPU that shares the
// computer's memory counts its use, its memory counts as memory), and of
// memory, over the machines counted (online, with a fresh reading). Each
// machine's part of each, for the stacked bars.
export function totalsOf(machines) {
  const counted = machines.filter((m) => m.counted);
  const threads = counted.reduce((s, m) => s + m.reading.cpu.threads, 0);
  const busy = counted.reduce((s, m) => s + (m.cpu ?? 0) * m.reading.cpu.threads, 0);
  const ramUsed = counted.reduce((s, m) => s + m.reading.memory.used, 0);
  const ramTotal = counted.reduce((s, m) => s + m.reading.memory.total, 0);
  const gpus = counted.flatMap((m) => m.reading.gpus.map((g) => ({ ...g, host: m.id, here: m.here, color: m.color })));
  const read = gpus.filter((g) => g.usage != null);
  const own = gpus.filter((g) => !g.shared && g.memTotal);
  const vramTotal = own.reduce((s, g) => s + g.memTotal, 0);
  const busiest = read.reduce((b, g) => (!b || g.usage > b.usage ? g : b), null);
  const part = (m, v) => ({ id: m.id, here: m.here, color: m.color, v });
  return {
    machines: machines.length,
    online: machines.filter((m) => m.online).length,
    counted: counted.length,
    threads,
    cpu: threads ? busy / threads : null,
    ram: { used: ramUsed, total: ramTotal, share: ramTotal ? ramUsed / ramTotal : null },
    gpu: {
      count: gpus.length,
      mean: read.length ? read.reduce((s, g) => s + g.usage, 0) / read.length : null,
      vram: vramTotal ? { used: own.reduce((s, g) => s + (g.memUsed ?? 0), 0), total: vramTotal, share: own.reduce((s, g) => s + (g.memUsed ?? 0), 0) / vramTotal } : null,
      busiest: busiest ? { host: busiest.host, here: busiest.here, name: busiest.name, usage: busiest.usage } : null,
    },
    parts: {
      cpu: counted.map((m) => part(m, threads ? ((m.cpu ?? 0) * m.reading.cpu.threads) / threads : 0)),
      gpu: counted.map((m) => part(m, read.length ? m.reading.gpus.reduce((s, g) => s + (g.usage ?? 0), 0) / read.length : 0)),
      ram: counted.map((m) => part(m, ramTotal ? m.reading.memory.used / ramTotal : 0)),
    },
  };
}

// The machines and their totals, from a reading (with ?hosts=1, the
// others') and the network's state (network/state/v1, read leniently).
// Out of a network: this computer alone, with no id.
export function machinesOf(reading, net, now = Date.now()) {
  const network = inNetwork(net);
  if (!network) {
    const one = machine({ online: true }, reading, { here: true, color: 0, now, windowed: true });
    return { network: false, self: null, windowed: true, machines: [one], totals: totalsOf([one]) };
  }
  const self = net.self?.id ?? null;
  const windowed = !!net.self?.window;
  const readings = new Map(list(reading?.hosts).filter((r) => r && typeof r.host === "string").map((r) => [r.host, r]));
  const roster = list(net.hosts).filter((x) => x && typeof x.id === "string");
  if (self && !roster.some((x) => x.id === self)) roster.unshift({ id: self, online: true, window: windowed, founder: !!net.self?.founder, gateway: !!net.self?.gateway });
  const machines = roster.map((row, i) => {
    const here = row.id === self;
    return machine(row, here ? reading : (readings.get(row.id) ?? null), { here, color: i % HOST_COLORS, now, windowed });
  });
  return { network: true, self, windowed, machines, totals: totalsOf(machines) };
}

export const hasGpu = (m) => (m.reading?.gpus.length ?? 0) > 0 || (m.reading?.hardware.gpus.length ?? 0) > 0;

// The view's filters: the machines picked (the Filters chips' keys, @here
// and ids; none picks all), a platform, only those with a GPU, only those
// online.
export function filterMachines(machines, { keys = [], platform = null, gpu = false, online = false } = {}) {
  const picked = new Set(keys);
  return machines.filter(
    (m) =>
      (!picked.size || picked.has(m.key)) &&
      (!platform || m.platform === platform) &&
      (!gpu || hasGpu(m)) &&
      (!online || m.online),
  );
}

// By name, or the busiest first (CPU, GPU, memory), or the last seen first
// (this machine and those online, then the rest by when they were heard).
export function sortMachines(machines, by = "name") {
  const name = (a, b) => String(a.id ?? "").localeCompare(String(b.id ?? ""));
  const desc = (k) => (a, b) => (b[k] ?? -1) - (a[k] ?? -1) || name(a, b);
  const seen = (m) => (m.here || m.online ? Infinity : (m.seen ?? -Infinity));
  const order = { cpu: desc("cpu"), gpu: desc("gpu"), ram: desc("ram"), seen: (a, b) => seen(b) - seen(a) || name(a, b) }[by] ?? name;
  return [...machines].sort(order);
}

// Each machine's sessions on the map, by app, from the graph: this
// machine's under @here, the others' under their ids.
export function sessionsByMachine(graph) {
  const out = new Map();
  for (const n of list(graph?.nodes)) {
    if (n?.kind !== "session") continue;
    const key = typeof n.host === "string" && n.host ? n.host : HERE;
    const c = out.get(key) ?? out.set(key, { total: 0, apps: {} }).get(key);
    c.total++;
    c.apps[n.app ?? "other"] = (c.apps[n.app ?? "other"] ?? 0) + 1;
  }
  return out;
}
