// What the computer running XenoNet is doing, for the console's system panel
// (src/system-panel.js): the CPU (its model, how busy, each core), memory,
// and the GPU (its name, how busy, its memory). Read only while a window
// asks, every couple of seconds; a minute after the last ask, the GPU
// reader stops too. None of it leaves the computer.
//
// CPU and memory come from Node. The GPU from what the system has:
//   - NVIDIA's nvidia-smi (Windows, Linux), left running in its loop mode,
//     since starting it takes a third of a second;
//   - a Mac's IOAccelerator statistics (ioreg); Apple's GPUs share memory
//     with the CPU;
//   - on Linux, the amdgpu driver's files in /sys;
//   - on Windows without nvidia-smi, Windows' own GPU counters, in a
//     PowerShell loop every five seconds, since they take a second or more
//     to read.
import os from "node:os";
import fs from "node:fs";
import path from "node:path";
import { spawn as spawnChild, execFile } from "node:child_process";
import { onPath } from "./paths.js";

export const HISTORY = 90; // samples kept for the traces: three minutes at 2 s
const FRESH_MS = 900; // a read this soon after the last one gets the same
const IDLE_MS = 60e3;
const STALE_MS = 15e3; // a GPU reading older than this isn't shown
const MIB = 1024 * 1024;

const frac = (part, whole) => (whole > 0 ? Math.min(1, Math.max(0, part / whole)) : 0);
const num = (s) => {
  const n = Number(String(s ?? "").trim());
  return String(s ?? "").trim() !== "" && Number.isFinite(n) ? n : null;
};

// "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz" → "Intel Core i7-9700K".
export function cpuModel(model) {
  return String(model ?? "")
    .replace(/\((?:R|TM|C)\)/gi, "")
    .replace(/\bCPU\b/g, "")
    .replace(/@\s*[\d.]+\s*GHz/i, "")
    .replace(/\b\d+-Core Processor\b/i, "")
    .replace(/\bProcessor\b/i, "")
    .replace(/\s+/g, " ")
    .trim() || null;
}

// One line of nvidia-smi --query-gpu=index,name,utilization.gpu,
// memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits:
// "0, NVIDIA RTX A4500, 41, 6000, 20470, 58". A figure the
// card doesn't report reads [N/A] or [Not Supported].
export const NVIDIA_QUERY = "index,name,utilization.gpu,memory.used,memory.total,temperature.gpu";
export function parseNvidia(line) {
  const parts = String(line).split(",").map((s) => s.trim());
  if (parts.length < 6) return null;
  const index = num(parts[0]);
  const [usage, used, total, temp] = parts.slice(-4).map(num);
  const name = parts.slice(1, -4).join(", ");
  if (index == null || !name) return null;
  return { index, name, usage: usage == null ? null : usage / 100, memUsed: used == null ? null : used * MIB, memTotal: total == null ? null : total * MIB, temp, shared: false };
}

// ioreg -r -d 1 -w 0 -c IOAccelerator: on Apple silicon, "model" names the
// chip and PerformanceStatistics has "Device Utilization %" and "In use
// system memory"; an AMD card in an Intel Mac has "vramUsedBytes" and
// "VRAM,totalMB" instead.
export function parseIoreg(text, totalMem) {
  const s = String(text ?? "");
  if (!/PerformanceStatistics/.test(s)) return null;
  const stat = (key) => {
    const m = s.match(new RegExp(`"${key.replace(/[%,]/g, (c) => `\\${c}`)}"=(\\d+)`));
    return m ? Number(m[1]) : null;
  };
  const usage = stat("Device Utilization %") ?? stat("GPU Activity\\(%\\)");
  const vramUsed = stat("vramUsedBytes");
  const vramTotal = stat("VRAM,totalMB");
  const name = s.match(/"model" = "([^"]+)"/)?.[1] ?? s.match(/"model" = <"([^"]+)">/)?.[1] ?? null;
  const cores = s.match(/"gpu-core-count" = (\d+)/)?.[1];
  if (vramTotal) return { index: 0, name: name ?? "GPU", usage: usage == null ? null : usage / 100, memUsed: vramUsed, memTotal: vramTotal * MIB, temp: null, shared: false };
  return { index: 0, name: name ? `${name}${cores ? ` · ${cores}-core GPU` : ""}` : "GPU", usage: usage == null ? null : usage / 100, memUsed: stat("In use system memory"), memTotal: totalMem ?? null, temp: null, shared: true };
}

// The PowerShell loop's lines (windowsGpuScript): "A" and the display
// adapters the registry lists, with their memory; then every few seconds
// "S" and, by adapter, the 3D engines' use added up (percent) and the
// dedicated memory in use (bytes). Windows doesn't say which adapter is
// which, so the one using the most dedicated memory is taken for the one
// with the most, which is right for the usual one card beside a chip's own.
export function parseWindowsGpu(line, st = { adapters: [], latest: null }) {
  const kind = String(line).slice(0, 2);
  let body;
  try {
    body = JSON.parse(String(line).slice(2));
  } catch {
    return st;
  }
  if (kind === "A ") {
    const list = (Array.isArray(body) ? body : [body]).filter((a) => a?.name);
    st.adapters = list.map((a) => ({ name: String(a.name), mem: Number(a.mem) || 0 })).sort((a, b) => b.mem - a.mem);
  } else if (kind === "S ") {
    const luids = Object.values(body ?? {}).filter((v) => v && typeof v === "object");
    if (!luids.length) return st;
    const top = luids.sort((a, b) => (Number(b.m) || 0) - (Number(a.m) || 0))[0];
    const card = st.adapters[0] ?? null;
    st.latest = { index: 0, name: card?.name ?? "GPU", usage: Math.min(1, (Number(top.u) || 0) / 100), memUsed: Number(top.m) || 0, memTotal: card?.mem || null, temp: null, shared: false };
  }
  return st;
}

// The loop ends itself once the service that started it is gone, so a
// service that crashed or was killed leaves none behind.
export const windowsGpuScript = (parent) => [
  "$ErrorActionPreference='SilentlyContinue'",
  `$parent=${Number(parent) | 0}`,
  "$cls='HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}'",
  "$a=@(Get-ChildItem $cls | ForEach-Object { $p=Get-ItemProperty $_.PSPath; if ($p.DriverDesc) { $m=$p.'HardwareInformation.qwMemorySize'; if (-not $m) { $m=$p.'HardwareInformation.MemorySize'; if ($m -is [byte[]]) { $m=[BitConverter]::ToUInt32($m,0) } }; [pscustomobject]@{name=$p.DriverDesc;mem=[uint64]$m} } })",
  "'A '+(ConvertTo-Json -Compress -InputObject $a)",
  "while (Get-Process -Id $parent) { $o=@{}; foreach ($x in (Get-Counter '\\GPU Engine(*engtype_3D)\\Utilization Percentage','\\GPU Adapter Memory(*)\\Dedicated Usage').CounterSamples) { if ($x.InstanceName -match 'luid_(0x[0-9a-f]+_0x[0-9a-f]+)') { $l=$Matches[1]; if (-not $o[$l]) { $o[$l]=@{u=0;m=0} }; if ($x.Path -like '*utilization percentage') { $o[$l].u+=$x.CookedValue } else { $o[$l].m+=$x.CookedValue } } }; 'S '+(ConvertTo-Json -Compress -InputObject $o); [Console]::Out.Flush(); Start-Sleep -Seconds 5 }",
].join("; ");

// Where nvidia-smi is: on PATH, or where NVIDIA's driver puts it on
// Windows.
export function findNvidiaSmi({ platform = process.platform, env = process.env, exists = fs.existsSync } = {}) {
  const found = onPath("nvidia-smi", { env, platform, canRun: (f) => { if (!exists(f)) throw new Error("missing"); } });
  if (found) return found;
  if (platform !== "win32") return null;
  const root = env.SystemRoot ?? env.SYSTEMROOT ?? "C:\\Windows";
  for (const f of [path.win32.join(root, "System32", "nvidia-smi.exe"), path.win32.join(env.ProgramFiles ?? "C:\\Program Files", "NVIDIA Corporation", "NVSMI", "nvidia-smi.exe")]) if (exists(f)) return f;
  return null;
}

// A program kept running that prints a line per reading: started when
// asked, stopped when idle, started again after it exits, a little later
// each time, and given up on after three exits with nothing read.
function streamed({ spawn, file, args, onLine, now }) {
  let child = null;
  let buf = "";
  let quiet = 0; // exits in a row with nothing read
  let readSince = false;
  let retryAt = 0;
  let stopping = false;
  return {
    get dead() {
      return quiet >= 3;
    },
    start() {
      if (child || quiet >= 3 || now() < retryAt) return;
      stopping = false;
      readSince = false;
      try {
        child = spawn(file, args, { windowsHide: true, stdio: ["ignore", "pipe", "ignore"] });
      } catch {
        child = null;
        quiet++;
        retryAt = now() + [5e3, 20e3, 60e3][Math.min(quiet - 1, 2)];
        return;
      }
      const me = child;
      me.stdout?.setEncoding?.("utf8");
      me.stdout?.on("data", (d) => {
        buf += d;
        let i;
        while ((i = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, i).trim();
          buf = buf.slice(i + 1);
          if (!line) continue;
          readSince = true;
          quiet = 0;
          onLine(line);
        }
      });
      const gone = () => {
        if (child !== me) return;
        child = null;
        buf = "";
        if (stopping) return;
        if (!readSince) quiet++;
        retryAt = now() + [5e3, 20e3, 60e3][Math.min(Math.max(quiet - 1, 0), 2)];
      };
      me.on("error", gone);
      me.on("exit", gone);
    },
    stop() {
      stopping = true;
      const c = child;
      child = null;
      buf = "";
      try {
        c?.kill();
      } catch {}
    },
  };
}

function nvidiaReader({ file, spawn, now }) {
  const cards = new Map();
  const proc = streamed({
    spawn,
    file,
    args: [`--query-gpu=${NVIDIA_QUERY}`, "--format=csv,noheader,nounits", "-lms", "2000"],
    now,
    onLine: (line) => {
      const g = parseNvidia(line);
      if (g) cards.set(g.index, { ...g, at: now() });
    },
  });
  return {
    source: "nvidia-smi",
    async read() {
      proc.start();
      if (proc.dead) return null;
      return [...cards.values()].filter((g) => now() - g.at < STALE_MS).sort((a, b) => a.index - b.index).map(({ at, ...g }) => g);
    },
    stop: () => proc.stop(),
  };
}

function windowsReader({ spawn, env, now, parent }) {
  const st = { adapters: [], latest: null, at: 0 };
  const proc = streamed({
    spawn,
    file: path.win32.join(env.SystemRoot ?? env.SYSTEMROOT ?? "C:\\Windows", "System32", "WindowsPowerShell", "v1.0", "powershell.exe"),
    args: ["-NoProfile", "-NonInteractive", "-Command", windowsGpuScript(parent)],
    now,
    onLine: (line) => {
      const before = st.latest;
      parseWindowsGpu(line, st);
      if (st.latest !== before) st.at = now();
    },
  });
  return {
    source: "windows",
    async read() {
      proc.start();
      if (proc.dead) return null;
      return st.latest && now() - st.at < STALE_MS * 2 ? [st.latest] : [];
    },
    stop: () => proc.stop(),
  };
}

// A reading run each time it's asked for, at most every couple of seconds.
function polled(source, fetch, now) {
  let last = null;
  let at = 0;
  let pending = null;
  return {
    source,
    async read() {
      if (now() - at < 1800) return last;
      pending ??= fetch()
        .catch(() => null)
        .then((v) => {
          last = v;
          at = now();
          pending = null;
          return v;
        });
      return pending;
    },
    stop() {},
  };
}

function appleReader({ run, totalMem, now }) {
  return polled(
    "ioreg",
    async () => {
      const r = await run("/usr/sbin/ioreg", ["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]);
      const g = r.code === 0 ? parseIoreg(r.stdout, totalMem()) : null;
      return g ? [g] : [];
    },
    now,
  );
}

async function amdReader({ readFile, readdir, now }) {
  const base = "/sys/class/drm";
  const cards = [];
  for (const name of await readdir(base).catch(() => [])) {
    if (!/^card\d+$/.test(name)) continue;
    const dev = `${base}/${name}/device`;
    if ((await readFile(`${dev}/gpu_busy_percent`).catch(() => null)) == null) continue;
    cards.push(dev);
  }
  if (!cards.length) return null;
  return polled(
    "amdgpu",
    async () =>
      Promise.all(
        cards.map(async (dev, index) => {
          const read = (f) => readFile(`${dev}/${f}`).then((s) => s.trim()).catch(() => null);
          const [busy, used, total, label] = await Promise.all([read("gpu_busy_percent"), read("mem_info_vram_used"), read("mem_info_vram_total"), read("product_name")]);
          return { index, name: label || "AMD GPU", usage: num(busy) == null ? null : num(busy) / 100, memUsed: num(used), memTotal: num(total), temp: null, shared: false };
        }),
      ),
    now,
  );
}

function runFile(file, args, ms = 3000) {
  return new Promise((resolve) =>
    execFile(file, args, { timeout: ms, windowsHide: true, maxBuffer: 4 << 20 }, (err, stdout) => resolve({ code: err ? (err.code ?? 1) : 0, stdout: String(stdout ?? "") })),
  );
}

export function createSystemStats({ platform = process.platform, env = process.env, osm = os, spawn = spawnChild, run = runFile, readFile = (f) => fs.promises.readFile(f, "utf8"), readdir = (d) => fs.promises.readdir(d), exists = fs.existsSync, now = Date.now, wait = (ms) => new Promise((r) => setTimeout(r, ms)), idleMs = IDLE_MS, parent = process.pid, onExit = (fn) => process.once("exit", fn) } = {}) {
  let prev = null; // the CPUs' times at the last reading
  let last = null;
  let lastAt = 0;
  const history = [];
  let gpu; // undefined until picked; null when there's nothing to read
  let picking = null;
  let idle = null;

  const times = () =>
    osm.cpus().map((c) => {
      const t = c.times;
      const busy = t.user + t.nice + t.sys + t.irq;
      return { busy, total: busy + t.idle };
    });

  async function cpu() {
    if (!prev) {
      prev = times();
      await wait(200);
    }
    const cpus = osm.cpus();
    const now_ = times();
    const cores = now_.length === prev.length ? now_.map((t, i) => frac(t.busy - prev[i].busy, t.total - prev[i].total)) : now_.map(() => 0);
    const busy = now_.reduce((s, t, i) => s + t.busy - (prev[i]?.busy ?? t.busy), 0);
    const total = now_.reduce((s, t, i) => s + t.total - (prev[i]?.total ?? t.total), 0);
    prev = now_;
    const mhz = cpus[0]?.speed || null;
    return { model: cpuModel(cpus[0]?.model), threads: cpus.length, ghz: mhz ? Math.round(mhz / 100) / 10 : null, usage: frac(busy, total), cores };
  }

  async function pick() {
    if (platform === "darwin") return appleReader({ run, totalMem: () => osm.totalmem(), now });
    const smi = findNvidiaSmi({ platform, env, exists });
    if (smi) return nvidiaReader({ file: smi, spawn, now });
    if (platform === "linux") return amdReader({ readFile, readdir, now });
    if (platform === "win32") return windowsReader({ spawn, env, now, parent });
    return null;
  }

  async function gpus() {
    if (gpu === undefined) {
      picking ??= pick().catch(() => null);
      gpu = await picking;
      // A reader left running stops with the service, however it ends.
      if (gpu) onExit(() => gpu.stop());
    }
    return gpu ? ((await gpu.read().catch(() => null)) ?? null) : null;
  }

  function sleepSoon() {
    clearTimeout(idle);
    idle = setTimeout(() => gpu?.stop(), idleMs);
    idle.unref?.();
  }

  return {
    async read() {
      sleepSoon();
      if (last && now() - lastAt < FRESH_MS) return last;
      const [c, g] = await Promise.all([cpu(), gpus()]);
      const total = osm.totalmem();
      const memory = { total, used: Math.max(0, total - osm.freemem()) };
      const first = g?.[0] ?? null;
      const at = now();
      if (!history.length || at - history.at(-1).at >= 1500) {
        history.push({ at, cpu: c.usage, gpu: first?.usage ?? null, mem: frac(memory.used, memory.total) });
        if (history.length > HISTORY) history.shift();
      }
      last = { at, platform, arch: osm.arch?.() ?? null, uptime: Math.round(osm.uptime?.() ?? 0), cpu: c, memory, gpus: g ?? [], gpu: gpu ? (g == null ? "unavailable" : gpu.source) : null, history: history.slice() };
      lastAt = at;
      return last;
    },
    stop() {
      clearTimeout(idle);
      gpu?.stop();
    },
  };
}

// The demo's computer, made up: it breathes slowly, the same way each time
// the clock comes round.
export function createDemoSystemStats({ now = Date.now } = {}) {
  const wave = (t, period, phase = 0) => Math.sin((t / period) * Math.PI * 2 + phase);
  const sample = (t) => {
    const cpu = 0.34 + 0.16 * wave(t, 41e3) + 0.07 * wave(t, 9.7e3, 1.3);
    const gpu = 0.47 + 0.24 * wave(t, 57e3, 0.6) + 0.08 * wave(t, 6.1e3, 2.1);
    const mem = 0.61 + 0.04 * wave(t, 83e3, 0.2);
    return { cpu, gpu, mem };
  };
  const GB = 1024 ** 3;
  return {
    async read() {
      const at = now();
      const s = sample(at);
      const step = 2000;
      const history = [];
      for (let i = HISTORY - 1; i >= 0; i--) {
        const t = Math.floor(at / step) * step - i * step;
        const h = sample(t);
        history.push({ at: t, cpu: h.cpu, gpu: h.gpu, mem: h.mem });
      }
      const cores = Array.from({ length: 16 }, (_, i) => Math.min(1, Math.max(0.02, s.cpu + 0.3 * wave(at, 3e3 + i * 410, i))));
      return {
        at,
        platform: "linux",
        arch: "x64",
        uptime: 3 * 86400 + 4 * 3600 + 12 * 60,
        cpu: { model: "AMD Ryzen 7 7700X", threads: 16, ghz: 4.5, usage: s.cpu, cores },
        memory: { total: 64 * GB, used: Math.round(s.mem * 64 * GB) },
        gpus: [{ index: 0, name: "NVIDIA GeForce RTX 4080", usage: s.gpu, memUsed: Math.round((7.4 + 1.2 * wave(at, 30e3)) * GB), memTotal: 16 * GB, temp: Math.round(56 + 9 * s.gpu), shared: false }],
        gpu: "demo",
        history,
      };
    },
    stop() {},
  };
}
