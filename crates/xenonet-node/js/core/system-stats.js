// core/system-stats.js in Rust (crates/xenonet-app/src/system_stats.rs):
// what the computer running XenoNet is doing, for the console's system
// panel. The readings' parsers, the CPU arithmetic, the history and each
// GPU reader's state are the Rust's; this starts and stops the programs
// the readers keep running, reads the files, and keeps the timers, with
// the functions given.
import os from "node:os";
import fs from "node:fs";
import { spawn as spawnChild, execFile } from "node:child_process";
import { native } from "../index.js";

const C = native.appSystemConstants();
export const HISTORY = C.history;
export const NVIDIA_QUERY = C.nvidiaQuery;

export const cpuModel = (model) => native.appCpuModel(String(model ?? "")) ?? null;
export const parseNvidia = (line) => native.appParseNvidia(String(line)) ?? null;
export const parseIoreg = (text, totalMem) => native.appParseIoreg(String(text ?? ""), totalMem ?? null) ?? null;

export function parseWindowsGpu(line, st = { adapters: [], latest: null }) {
  const r = native.appParseWindowsGpu(String(line), st.adapters ?? []);
  if (r.adapters) st.adapters = r.adapters;
  if ("latest" in r) st.latest = r.latest;
  return st;
}

export const windowsGpuScript = (parent) => native.appWindowsGpuScript(Number(parent));

const text = (v) => (v == null ? null : String(v));

export const findNvidiaSmi = ({ platform = process.platform, env = process.env, exists = fs.existsSync } = {}) =>
  native.appFindNvidiaSmi(String(platform), String(env.PATH ?? ""), text(env.PATHEXT), text(env.SystemRoot ?? env.SYSTEMROOT), text(env.ProgramFiles), (f) => !!exists(f)) ?? null;

// A program kept running that prints a line a reading; when to start it
// again, and when to give up, is the Rust's.
function streamed(kind, { spawn, file, args, now }) {
  const core = new native.AppGpuStream(kind);
  let child = null;
  return {
    core,
    get dead() {
      return core.dead;
    },
    start() {
      const gen = core.start(now());
      if (gen == null) return;
      try {
        child = spawn(file, args, { windowsHide: true, stdio: ["ignore", "pipe", "ignore"] });
      } catch {
        child = null;
        core.spawnFailed(now());
        return;
      }
      const me = child;
      me.stdout?.setEncoding?.("utf8");
      me.stdout?.on("data", (d) => core.data(String(d), now()));
      const gone = () => {
        if (child === me) child = null;
        core.gone(gen, now());
      };
      me.on("error", gone);
      me.on("exit", gone);
    },
    stop() {
      const c = child;
      child = null;
      if (core.stop()) {
        try {
          c?.kill();
        } catch {}
      }
    },
  };
}

function streamReader(source, kind, opts) {
  const proc = streamed(kind, opts);
  return {
    source,
    async read() {
      proc.start();
      if (proc.dead) return null;
      return proc.core.read(opts.now());
    },
    stop: () => proc.stop(),
  };
}

// A reading run each time it's asked for, at most every couple of seconds.
function polled(source, fetch, now) {
  const core = new native.AppPolled();
  let pending = null;
  return {
    source,
    async read() {
      const hit = core.fresh(now());
      if (hit !== undefined) return hit;
      pending ??= fetch()
        .catch(() => null)
        .then((v) => {
          core.set(v, now());
          pending = null;
          return v;
        });
      return pending;
    },
    stop() {},
  };
}

const appleReader = ({ run, totalMem, now }) =>
  polled(
    "ioreg",
    async () => {
      const r = await run("/usr/sbin/ioreg", ["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"]);
      const g = r.code === 0 ? parseIoreg(r.stdout, totalMem()) : null;
      return g ? [g] : [];
    },
    now,
  );

async function amdReader({ readFile, readdir, now }) {
  const base = "/sys/class/drm";
  const cards = [];
  for (const name of native.appDrmCards((await readdir(base).catch(() => [])).map(String))) {
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
          const read = (f) => readFile(`${dev}/${f}`).then((s) => String(s).trim()).catch(() => null);
          const [busy, used, total, label] = await Promise.all([read("gpu_busy_percent"), read("mem_info_vram_used"), read("mem_info_vram_total"), read("product_name")]);
          return native.appAmdCard(index, busy, used, total, label);
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

export function createSystemStats({ platform = process.platform, env = process.env, osm = os, spawn = spawnChild, run = runFile, readFile = (f) => fs.promises.readFile(f, "utf8"), readdir = (d) => fs.promises.readdir(d), exists = fs.existsSync, now = Date.now, wait = (ms) => new Promise((r) => setTimeout(r, ms)), idleMs = C.idleMs, parent = process.pid, onExit = (fn) => process.once("exit", fn) } = {}) {
  const core = new native.AppSystemStats();
  let last = null;
  let gpu; // undefined until picked; null when there's nothing to read
  let picking = null;
  let idle = null;

  const times = () => osm.cpus().map((c) => c.times);

  async function cpu() {
    if (!core.hasPrev()) {
      core.setPrev(times());
      await wait(200);
    }
    const cpus = osm.cpus();
    return core.cpu(text(cpus[0]?.model), cpus[0]?.speed == null ? null : Number(cpus[0].speed), cpus.length, times());
  }

  async function pick() {
    if (platform === "darwin") return appleReader({ run, totalMem: () => osm.totalmem(), now });
    const smi = findNvidiaSmi({ platform, env, exists });
    if (smi) return streamReader("nvidia-smi", "nvidia", { file: smi, args: C.nvidiaArgs, spawn, now });
    if (platform === "linux") return amdReader({ readFile, readdir, now });
    if (platform === "win32") {
      const { file, args } = native.appWindowsGpuCommand(text(env.SystemRoot ?? env.SYSTEMROOT), Number(parent));
      return streamReader("windows", "windows", { file, args, spawn, now });
    }
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
      if (last && core.fresh(now())) return last;
      const [c, g] = await Promise.all([cpu(), gpus()]);
      last = core.reading(Number(now()), String(platform), osm.arch?.() ?? null, Number(osm.uptime?.() ?? 0), c, osm.totalmem(), osm.freemem(), g, gpu ? gpu.source : null);
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
export const createDemoSystemStats = ({ now = Date.now } = {}) => ({ read: async () => native.appDemoSystemReading(Number(now())), stop() {} });
