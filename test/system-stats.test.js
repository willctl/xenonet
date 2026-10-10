// This computer's load for the system panel (core/system-stats.js): CPU
// use from the cores' times between readings, memory, and the GPU from
// nvidia-smi kept running (started when asked, stopped when idle, given up
// on when it can't start), a Mac's ioreg, Windows' counters, or AMD's files
// in /sys; and the demo's made-up computer. The machines are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { createSystemStats, createDemoSystemStats, cpuModel, parseNvidia, parseIoreg, parseWindowsGpu, findNvidiaSmi, windowsGpuScript, HISTORY } from "../core/system-stats.js";

const GB = 1024 ** 3;

function fakeOs({ threads = 4 } = {}) {
  const t = Array.from({ length: threads }, () => ({ user: 0, nice: 0, sys: 0, irq: 0, idle: 0 }));
  return {
    t,
    // Each core spends `busy` of the next 1000 ms working.
    tick(busy) {
      t.forEach((c, i) => {
        const b = Array.isArray(busy) ? busy[i] : busy;
        c.user += 1000 * b;
        c.idle += 1000 * (1 - b);
      });
    },
    cpus: () => t.map((times) => ({ model: "Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz", speed: 3600, times: { ...times } })),
    totalmem: () => 32 * GB,
    freemem: () => 12 * GB,
    arch: () => "x64",
    uptime: () => 3600,
  };
}

function fakeChild() {
  const child = new EventEmitter();
  child.stdout = new EventEmitter();
  child.stdout.setEncoding = () => {};
  child.killed = false;
  child.kill = () => {
    child.killed = true;
    child.emit("exit", null);
  };
  return child;
}

test("CPU use: from the cores' times between readings, each core and all of them, with the model in plain words", async () => {
  const osm = fakeOs({ threads: 4 });
  let t = 1_800_000_000_000;
  const stats = createSystemStats({ platform: "freebsd", osm, now: () => t, wait: async () => osm.tick([1, 0.5, 0, 0.5]) });
  const r = await stats.read();
  assert.deepEqual(r.cpu, { model: "Intel Core i7-9700K", threads: 4, ghz: 3.6, usage: 0.5, cores: [1, 0.5, 0, 0.5] });
  assert.deepEqual(r.memory, { total: 32 * GB, used: 20 * GB });
  assert.deepEqual([r.gpus, r.gpu], [[], null], "no GPU reader on this system");
  t += 500;
  assert.equal(await stats.read(), r, "a read this soon is the same one");
  osm.tick(0.25);
  t += 2000;
  const next = await stats.read();
  assert.equal(next.cpu.usage, 0.25);
  assert.deepEqual(next.history.map((h) => h.cpu), [0.5, 0.25]);
  for (let i = 0; i < HISTORY + 5; i++) {
    osm.tick(0.1);
    t += 2000;
    await stats.read();
  }
  assert.equal((await stats.read()).history.length, HISTORY, "only the last few minutes are kept");
  stats.stop();
});

test("plain CPU names", () => {
  assert.equal(cpuModel("Intel(R) Core(TM) i9-14900K"), "Intel Core i9-14900K");
  assert.equal(cpuModel("AMD Ryzen 9 7950X 16-Core Processor            "), "AMD Ryzen 9 7950X");
  assert.equal(cpuModel("Apple M2 Pro"), "Apple M2 Pro");
  assert.equal(cpuModel(""), null);
});

test("nvidia-smi's lines, a figure the card doesn't report, and a name with a comma", () => {
  assert.deepEqual(parseNvidia("0, NVIDIA RTX A4500, 41, 6000, 20470, 58"), { index: 0, name: "NVIDIA RTX A4500", usage: 0.41, memUsed: 6000 * 1024 * 1024, memTotal: 20470 * 1024 * 1024, temp: 58, shared: false });
  assert.deepEqual(parseNvidia("1, Tesla T4, [N/A], 10, 15360, [Not Supported]"), { index: 1, name: "Tesla T4", usage: null, memUsed: 10 * 1024 * 1024, memTotal: 15360 * 1024 * 1024, temp: null, shared: false });
  assert.equal(parseNvidia("2, Odd, Name, 5, 1, 2, 3").name, "Odd, Name");
  assert.equal(parseNvidia("Failed to initialize NVML: Driver/library version mismatch"), null);
});

test("nvidia-smi is left running while asked, stopped when idle, and given up on when it can't start", async () => {
  const osm = fakeOs();
  let t = 1_800_000_000_000;
  const children = [];
  let refuse = false;
  const spawn = (file, args) => {
    if (refuse) throw Object.assign(new Error("spawn EPERM"), { code: "EPERM" });
    const c = fakeChild();
    c.file = file;
    c.args = args;
    children.push(c);
    return c;
  };
  const atExit = [];
  const stats = createSystemStats({ platform: "linux", env: { PATH: "/usr/bin" }, exists: (f) => f.replaceAll("\\", "/") === "/usr/bin/nvidia-smi", osm, spawn, now: () => t, wait: async () => osm.tick(0.3), idleMs: 20, onExit: (fn) => atExit.push(fn) });
  const first = await stats.read();
  assert.deepEqual([first.gpu, first.gpus], ["nvidia-smi", []], "nothing read yet");
  assert.equal(children.length, 1);
  assert.equal(children[0].file.replaceAll("\\", "/"), "/usr/bin/nvidia-smi");
  assert.deepEqual(children[0].args.slice(-2), ["-lms", "2000"], "its own loop, not a start every time");
  children[0].stdout.emit("data", "0, NVIDIA GeForce RTX 4070, 41, 6000, 12282, 58\n0, NVIDIA GeForce RTX 4070, 4");
  children[0].stdout.emit("data", "3, 6100, 12282, 59\n");
  t += 2000;
  const r = await stats.read();
  assert.deepEqual([r.gpus.length, r.gpus[0].usage, r.gpus[0].temp, r.history.at(-1).gpu], [1, 0.43, 59, 0.43]);
  assert.equal(children.length, 1, "the same one, still running");
  await new Promise((res) => setTimeout(res, 40));
  assert.equal(children[0].killed, true, "stopped once nothing asked for a while");
  t += 2000;
  await stats.read();
  assert.equal(children.length, 2, "started again when asked");
  atExit.forEach((fn) => fn());
  assert.equal(children[1].killed, true, "and stopped with the service, however it ends");
  t += 2000;
  await stats.read();
  // One that won't start is tried again a little later each time, then
  // left alone.
  assert.equal(children.length, 3, "and started again after");
  children.at(-1).emit("exit", 1);
  refuse = true;
  for (let i = 0; i < 4; i++) {
    t += 61e3;
    await stats.read();
  }
  t += 61e3;
  const gaveUp = await stats.read();
  assert.equal(gaveUp.gpu, "unavailable");
  stats.stop();
});

test("Windows' own counters: the adapter with the most memory, the busiest use of it", () => {
  const st = parseWindowsGpu('A [{"name":"Intel(R) UHD Graphics 770","mem":134217728},{"name":"NVIDIA RTX A4500","mem":21474836480}]');
  assert.deepEqual(st.adapters.map((a) => a.name), ["NVIDIA RTX A4500", "Intel(R) UHD Graphics 770"]);
  parseWindowsGpu('S {"0x00000000_0x00001a2b":{"u":2.5,"m":0},"0x00000000_0x00003c4d":{"u":64.3,"m":2147483648}}', st);
  assert.deepEqual(st.latest, { index: 0, name: "NVIDIA RTX A4500", usage: 0.643, memUsed: 2147483648, memTotal: 21474836480, temp: null, shared: false });
  parseWindowsGpu('A {"name":"Only One","mem":0}', st);
  assert.deepEqual(st.adapters, [{ name: "Only One", mem: 0 }], "one adapter comes as an object, not a list");
  assert.equal(parseWindowsGpu("garbage", st), st);
  // The loop ends itself once the service that started it is gone.
  assert.ok(windowsGpuScript(4242).includes("$parent=4242;"));
  assert.ok(windowsGpuScript(4242).includes("while (Get-Process -Id $parent) {"));
  assert.ok(windowsGpuScript("4242; Remove-Item x").includes("$parent=0;"), "only a number goes in");
});

test("a Mac's GPU: Apple's shares memory; an AMD card in an Intel Mac has its own", () => {
  const apple = `+-o AGXAcceleratorG14X  <class AGXAcceleratorG14X>\n    {\n      "model" = "Apple M2 Pro"\n      "gpu-core-count" = 19\n      "PerformanceStatistics" = {"In use system memory (driver)"=0,"Alloc system memory"=2826960896,"Device Utilization %"=37,"In use system memory"=1610612736,"Renderer Utilization %"=35}\n    }`;
  assert.deepEqual(parseIoreg(apple, 32 * GB), { index: 0, name: "Apple M2 Pro · 19-core GPU", usage: 0.37, memUsed: 1610612736, memTotal: 32 * GB, temp: null, shared: true });
  const amd = `"model" = <"AMD Radeon Pro 5500M">\n"PerformanceStatistics" = {"GPU Activity(%)"=12,"vramUsedBytes"=1073741824,"VRAM,totalMB"=8192}`;
  assert.deepEqual(parseIoreg(amd, 32 * GB), { index: 0, name: "AMD Radeon Pro 5500M", usage: 0.12, memUsed: GB, memTotal: 8 * GB, temp: null, shared: false });
  assert.equal(parseIoreg("", 1), null);
});

test("AMD on Linux: the amdgpu driver's files", async () => {
  const files = { "/sys/class/drm/card1/device/gpu_busy_percent": "27\n", "/sys/class/drm/card1/device/mem_info_vram_used": String(2 * GB), "/sys/class/drm/card1/device/mem_info_vram_total": String(8 * GB) };
  const osm = fakeOs();
  const stats = createSystemStats({ platform: "linux", env: { PATH: "" }, exists: () => false, osm, readdir: async () => ["card0", "card1", "card1-DP-1", "renderD128"], readFile: async (f) => (f in files ? files[f] : Promise.reject(new Error("ENOENT"))), wait: async () => osm.tick(0.2) });
  const r = await stats.read();
  assert.deepEqual([r.gpu, r.gpus], ["amdgpu", [{ index: 0, name: "AMD GPU", usage: 0.27, memUsed: 2 * GB, memTotal: 8 * GB, temp: null, shared: false }]]);
  stats.stop();
});

test("where nvidia-smi is on Windows: on PATH, or where NVIDIA's driver puts it", () => {
  const env = { PATH: "C:\\Tools", PATHEXT: ".EXE", SystemRoot: "C:\\Windows", ProgramFiles: "C:\\Program Files" };
  assert.equal(findNvidiaSmi({ platform: "win32", env, exists: (f) => f === "C:\\Windows\\System32\\nvidia-smi.exe" }), "C:\\Windows\\System32\\nvidia-smi.exe");
  assert.equal(findNvidiaSmi({ platform: "win32", env, exists: (f) => f === "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvidia-smi.exe" }), "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvidia-smi.exe");
  assert.equal(findNvidiaSmi({ platform: "linux", env: { PATH: "/usr/bin" }, exists: () => false }), null);
});

test("the demo's computer is made up, with a full history to draw", async () => {
  const r = await createDemoSystemStats({ now: () => 1_800_000_000_000 }).read();
  assert.equal(r.gpu, "demo");
  assert.equal(r.history.length, HISTORY);
  assert.ok(r.cpu.cores.every((c) => c >= 0 && c <= 1));
  assert.ok(r.gpus[0].memUsed < r.gpus[0].memTotal);
});
