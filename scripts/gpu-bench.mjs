// What XenoNet costs the machine while it sits open: GPU time, CPU and
// memory for the window's processes, in the states a console spends its
// day in. macOS only, since it reads each process's GPU time from ioreg
// (AGXDeviceUserClient's accumulatedGPUTime, the figure Activity Monitor's
// GPU column shows).
//
//   npm run bench:gpu                         every state, in the demo instance
//   npm run bench:gpu -- rest covered         only these states
//   npm run bench:gpu -- --big 400            a larger made-up world (see below)
//   npm run bench:gpu -- --seconds 15 --out before.json
//   npm run bench:gpu -- --plain --out plain.json   with plain panels (no glass blur)
//   npm run bench:gpu -- --compare before.json after.json
//
// It starts the demo instance (made-up data) at its usual 1600 by 960, or
// the test instance on a spare port with --instance test, and refuses to
// touch a copy that's already running. The states, in this order:
//
//   unfocused  shown, not focused, nothing touched (the window opens this way)
//   rest       focused, the layout cooled, nobody touching it
//   orbit      focused, the camera dragged round and round
//   busy       focused, many sessions working: sparks, packets, messages,
//              tokens and feed lines at a steady clip, a collision and a
//              question waiting
//   covered    unfocused, under another window that covers all of it
//   minimized  in the Dock
//   hidden     closed to the tray or menu bar
//
// The focused states bring the window to the front, then give focus back
// to the app that had it. "covered" opens a black window of its own over
// XenoNet's for its sample, nothing else.
//
// --big N draws N made-up sessions in about N/8 projects instead of the
// demo's map (in the page only: the server keeps the demo's), a third of
// them working, so the costs of a large map show.
//
// --plain draws the panels solid, without the glass blur behind them (the
// palette's plain panels), for the run, and puts them back after, so a run
// with it and one without show what the blur costs.
//
// --seen counts the window as seen in the states where it's shown
// (unfocused, rest, orbit, busy), whatever the desktop app says, for a run
// while the screen is locked: the page draws nothing then, as it should,
// and only its own pace decides with --seen. The window is still drawn and
// composited; the device's utilization lacks the screen's own share.
//
// For each state: the share of one GPU the window's GPU process kept busy
// (gpu %), the device's own utilization (every app, as ioreg reports it),
// the CPU of XenoNet's main, renderer and GPU processes and of the service
// (the xenonet binary on the instance's port), their resident memory, the
// page's JS heap, the frames the page drew per second, and what a frame
// cost: the composer's GPU time (a WebGL timer query), the GPU process's
// GPU time per frame (compositing included) and the main thread's time for
// the map's frame. Apple's GPUs lower their clocks under light load, so a
// lighter frame shows as less GPU time but not in proportion; frames drawn
// says what the page asked for. Through ANGLE's Metal backend a timer
// query spans whole command buffers and reads high, so on a Mac the GPU
// process's figure is the one to compare.
//
// Before it starts XenoNet, it samples the device's utilization for as
// long as a state (--baseline seconds; 0 skips it), since that figure
// counts every other app too.
import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(import.meta.url);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------- measuring ----------

// Each process's GPU time so far, in ns, by pid.
function gpuTimes() {
  const out = execFileSync("ioreg", ["-r", "-c", "AGXDeviceUserClient", "-d", "1", "-l", "-w", "0"], { encoding: "utf8", maxBuffer: 64 << 20 });
  const by = new Map();
  for (const block of out.split("+-o ")) {
    const pid = Number(/"IOUserClientCreator" = "pid (\d+),/.exec(block)?.[1]);
    if (!pid) continue;
    let ns = 0;
    for (const m of block.matchAll(/"accumulatedGPUTime"=(\d+)/g)) ns += Number(m[1]);
    by.set(pid, (by.get(pid) ?? 0) + ns);
  }
  return by;
}

const deviceUtilization = () => Number(/"Device Utilization %"=(\d+)/.exec(execFileSync("ioreg", ["-r", "-d", "1", "-w", "0", "-c", "IOAccelerator"], { encoding: "utf8" }))?.[1] ?? NaN);

// [[dd-]hh:]mm:ss.ss, as ps prints CPU time, in seconds.
function seconds(t) {
  const [days, rest] = t.includes("-") ? t.split("-") : [0, t];
  return Number(days) * 86400 + rest.split(":").reduce((a, v) => a * 60 + Number(v), 0);
}

// CPU time (s) and resident memory (MB) of these pids.
function cpuAndMemory(pids) {
  const out = execFileSync("ps", ["-o", "pid=,time=,rss=", "-p", pids.join(",")], { encoding: "utf8" });
  const by = new Map();
  for (const line of out.trim().split("\n")) {
    const [pid, time, rss] = line.trim().split(/\s+/);
    by.set(Number(pid), { cpu: seconds(time), mb: Number(rss) / 1024 });
  }
  return by;
}

// The process listening on this port: the service, the xenonet binary,
// which runs apart from Desktop.
function listenerOn(port) {
  try {
    return execFileSync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN", "-t"], { encoding: "utf8" }).trim().split("\n").map(Number).filter(Boolean);
  } catch {
    return [];
  }
}

// XenoNet's processes: the main one and its helpers, by kind, and the
// service on the instance's port.
function processesOf(mainPid, port) {
  const out = execFileSync("ps", ["-A", "-o", "pid=,ppid=,command="], { encoding: "utf8", maxBuffer: 16 << 20 });
  const kinds = { main: [mainPid], renderer: [], gpu: [], server: [], service: listenerOn(port).filter((p) => p !== mainPid) };
  for (const line of out.split("\n")) {
    const m = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line);
    if (!m || Number(m[2]) !== mainPid) continue;
    const cmd = m[3];
    if (cmd.includes("--type=gpu-process")) kinds.gpu.push(Number(m[1]));
    else if (cmd.includes("--type=renderer")) kinds.renderer.push(Number(m[1]));
    else if (cmd.includes("node.mojom.NodeService")) kinds.server.push(Number(m[1]));
  }
  return kinds;
}

// The page's own count: each frame it drew (a render of the composer), by
// wall-clock time, so a window of time can be counted afterwards without a
// request to the page in the middle of it. With each frame, what it cost:
// the GPU time of the composer's render, from a WebGL timer query where the
// GPU offers one, and the main thread's time for the whole frame, the map's
// own frame loop (scene.js) timed as requestAnimationFrame runs it.
const COUNTER = `return (() => {
  if (window.__benchFrames) return true;
  const scene = window.__xenonet.scene;
  const c = scene.graph.postProcessingComposer();
  const gl = scene.graph.renderer().getContext();
  const timer = gl.getExtension("EXT_disjoint_timer_query_webgl2");
  const log = (window.__benchFrames = []);
  const pending = [];
  const render = c.render;
  // Each frame's time, the frame rate the page kept for it (render-
  // budget.js; absent before it), whether it was driven, and later its
  // costs in ms: [time, fps, driven, gpu, cpu].
  c.render = function (...a) {
    const st = scene.drawingState?.();
    const row = [performance.timeOrigin + performance.now(), st?.fps ?? null, !!st?.driven, null, null];
    log.push(row);
    if (log.length > 20000) log.splice(0, 10000);
    if (!timer || pending.length > 64) return render.apply(this, a);
    const q = gl.createQuery();
    gl.beginQuery(timer.TIME_ELAPSED_EXT, q);
    try {
      return render.apply(this, a);
    } finally {
      gl.endQuery(timer.TIME_ELAPSED_EXT);
      pending.push([q, row]);
    }
  };
  // A query's answer comes a frame or more later; one the GPU spoiled (a
  // disjoint, such as the clock changing) is dropped.
  const collect = () => {
    if (timer) {
      const spoiled = gl.getParameter(timer.GPU_DISJOINT_EXT);
      while (pending.length && gl.getQueryParameter(pending[0][0], gl.QUERY_RESULT_AVAILABLE)) {
        const [q, row] = pending.shift();
        if (!spoiled) row[3] = gl.getQueryParameter(q, gl.QUERY_RESULT) / 1e6;
        gl.deleteQuery(q);
      }
    }
    setTimeout(collect, 100);
  };
  collect();
  // The frame loop is the callback named loop; a call that drew is the one
  // whose frame the composer logged.
  const raf = window.requestAnimationFrame;
  window.requestAnimationFrame = (fn) =>
    raf((now) => {
      if (fn.name !== "loop") return fn(now);
      const n = log.length, t0 = performance.now();
      try {
        return fn(now);
      } finally {
        if (log.length > n) log[log.length - 1][4] = performance.now() - t0;
      }
    });
  return true;
})()`;
const framesBetween = (t0, t1) => `return (() => {
  const log = (window.__benchFrames ?? []).filter(([t]) => t >= ${t0} && t < ${t1});
  const n = log.length;
  // The share of frames drawn at each rate the page kept.
  const rates = {};
  for (const [, r] of log) rates[r === Infinity ? "every" : r] = (rates[r === Infinity ? "every" : r] ?? 0) + 1 / Math.max(1, n);
  for (const k in rates) rates[k] = Math.round(rates[k] * 100) / 100;
  // Mean, median and 95th percentile of each frame's cost, in ms.
  const spread = (i) => {
    const v = log.map((row) => row[i]).filter((x) => x != null).sort((a, b) => a - b);
    if (!v.length) return null;
    const r = (x) => Math.round(x * 100) / 100;
    return { mean: r(v.reduce((a, b) => a + b, 0) / v.length), p50: r(v[Math.floor(v.length / 2)]), p95: r(v[Math.min(v.length - 1, Math.floor(v.length * 0.95))]), n: v.length };
  };
  return { frames: n, driven: log.filter((row) => row[2]).length, rates, gpuMs: spread(3), cpuMs: spread(4), heapMB: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1e5) / 10 : null, render: window.__xenonet.driver.state().render ?? null };
})()`;

// The device's utilization once a second over `ms`: its mean, median and
// 90th percentile. It counts every app on the machine, and moves in jumps.
async function utilization(ms) {
  const util = [];
  for (const until = Date.now() + ms; Date.now() < until; ) {
    await sleep(1000);
    util.push(deviceUtilization());
  }
  const v = util.filter(Number.isFinite).sort((a, b) => a - b);
  const pct = (x) => Math.round(x * 10) / 10;
  return { mean: pct(v.reduce((a, b) => a + b, 0) / Math.max(1, v.length)), p50: v[Math.floor(v.length / 2)] ?? null, p90: v[Math.min(v.length - 1, Math.floor(v.length * 0.9))] ?? null };
}

// A sample of `ms`: everything above, as rates over that time.
async function sample(s, procs, ms) {
  const pids = Object.values(procs).flat();
  const g0 = gpuTimes(), c0 = cpuAndMemory(pids), t0 = Date.now(), h0 = process.hrtime.bigint();
  const util = await utilization(ms);
  const g1 = gpuTimes(), c1 = cpuAndMemory(pids), t1 = Date.now(), wall = Number(process.hrtime.bigint() - h0);
  const page = await s.eval(framesBetween(t0, t1)).catch(() => ({ frames: null, heapMB: null }));
  const pct = (v) => Math.round(v * 10) / 10;
  const sum = (pidsOf, f) => pidsOf.reduce((a, p) => a + f(p), 0);
  const cpu = (kind) => pct((sum(procs[kind], (p) => (c1.get(p)?.cpu ?? 0) - (c0.get(p)?.cpu ?? 0)) / (wall / 1e9)) * 100);
  const mem = (kind) => Math.round(sum(procs[kind], (p) => c1.get(p)?.mb ?? 0));
  const gpuNs = sum(procs.gpu, (p) => (g1.get(p) ?? 0) - (g0.get(p) ?? 0));
  return {
    gpu: pct((gpuNs / wall) * 100),
    device: util.mean,
    deviceSpread: util,
    fps: page.frames == null ? null : pct(page.frames / ((t1 - t0) / 1000)),
    // What a frame cost: the GPU process's GPU time over the frames drawn
    // (the window's compositing included), the composer's own GPU time and
    // the main thread's time for the map's frame.
    frame: page.frames ? { gpuProcessMs: Math.round((gpuNs / 1e6 / page.frames) * 100) / 100, gpuMs: page.gpuMs, cpuMs: page.cpuMs } : null,
    // Frames drawn while a request to the window was in hand (someone else
    // driving it spoils a sample).
    drivenFrames: page.driven,
    // The page's own word (none before the frame budget): the rates it
    // kept over the sample, and whether it could be seen and was focused.
    rates: page.rates,
    page: page.render ? { seen: page.render.seen, focused: page.render.focused, pace: page.render.pace } : undefined,
    cpu: { main: cpu("main"), renderer: cpu("renderer"), gpu: cpu("gpu"), server: cpu("server"), service: cpu("service") },
    memMB: { main: mem("main"), renderer: mem("renderer"), gpu: mem("gpu"), server: mem("server"), service: mem("service"), heap: page.heapMB },
  };
}

// ---------- the states ----------

// The app in front, as lsappinfo knows it, by bundle id.
function frontApp() {
  try {
    const asn = execFileSync("lsappinfo", ["front"], { encoding: "utf8" }).trim();
    return /"CFBundleIdentifier"="([^"]+)"/.exec(execFileSync("lsappinfo", ["info", "-only", "bundleID", asn], { encoding: "utf8" }))?.[1] ?? null;
  } catch {
    return null;
  }
}
function giveFocusBack(bundle) {
  if (bundle) {
    try {
      execFileSync("open", ["-b", bundle]);
    } catch {}
  }
}

// Made-up activity at a steady clip, in the page, for "busy": tool calls,
// prompts, turn ends, tokens and messages among the sessions drawn, and
// each in the activity feed too, as the live stream would.
const BUSY_START = `return (() => {
  const x = window.__xenonet;
  const ids = x.scene.nodes().filter((n) => n.kind === "session").map((n) => n.id);
  if (!ids.length) return 0;
  const pick = () => ids[Math.floor(Math.random() * ids.length)];
  const cats = ["read", "edit", "shell", "search", "mcp", "web"];
  let i = 0;
  window.__benchBusy = setInterval(() => {
    const id = pick();
    const kind = i % 9 === 0 ? "prompt" : i % 13 === 0 ? "turn-end" : "tool";
    const ev = { sessionId: id, kind, category: cats[i % cats.length], tool: "Bash", detail: "npm test", ts: Date.now() };
    x.scene.activity(ev);
    x.hud.activity(ev);
    x.scene.tokens({ sessionId: pick(), n: 200 + Math.floor(Math.random() * 4000) });
    if (i % 10 === 0) x.scene.message(id, [pick(), pick()]);
    i++;
  }, 60);
  return ids.length;
})()`;
const BUSY_STOP = `return (clearInterval(window.__benchBusy), true)`;

// --seen: the page's drawing state with seen held true, then handed back
// to the page, which reads the window's real state again on a visibility
// change (main.js).
const SEEN_ON = `return (() => {
  const scene = window.__xenonet.scene;
  if (!scene.__benchSetDrawing) {
    const set = (scene.__benchSetDrawing = scene.setDrawing);
    scene.setDrawing = (patch) => set({ ...patch, seen: true });
  }
  scene.setDrawing({});
  return scene.drawingState().seen;
})()`;
const SEEN_OFF = `return (() => {
  const scene = window.__xenonet.scene;
  if (scene.__benchSetDrawing) scene.setDrawing = scene.__benchSetDrawing;
  delete scene.__benchSetDrawing;
  document.dispatchEvent(new Event("visibilitychange"));
  return scene.drawingState().seen;
})()`;

// --big: a made-up map of n sessions, drawn in place of the demo's (which
// the server keeps sending; the page ignores it until the end).
const bigWorld = (n) => `return (() => {
  const x = window.__xenonet, scene = x.scene;
  const nodes = scene.nodes();
  const skip = ["x", "y", "z", "vx", "vy", "vz", "fx", "fy", "fz", "index"];
  const clean = (o) => Object.fromEntries(Object.entries(o).filter(([k, v]) => !k.startsWith("__") && !skip.includes(k) && !(v && typeof v === "object" && (v.isObject3D || typeof v.toJSON === "function"))));
  const core = clean(nodes.find((n) => n.kind === "core"));
  const project = clean(nodes.find((n) => n.kind === "project"));
  const session = clean(nodes.find((n) => n.kind === "session"));
  const liveOf = clean(x.live.sessions[0]);
  const out = { nodes: [core], links: [], relations: [] };
  const live = [];
  const projects = Math.max(1, Math.round(${n} / 8));
  for (let p = 0; p < projects; p++) {
    const id = "project:bench-" + p;
    out.nodes.push({ ...project, id, name: "bench-" + p, title: "bench-" + p, active: p % 3 === 0, pinned: false });
    out.links.push({ source: "core", target: id, kind: "contains" });
  }
  for (let i = 0; i < ${n}; i++) {
    const id = "claude:bench-" + i, home = "project:bench-" + (i % projects);
    out.nodes.push({ ...session, id, title: "Session " + i, activity: i % 3 === 0 ? "active" : "recent", pinned: false });
    out.links.push({ source: home, target: id, kind: "session" });
    live.push({ ...liveOf, id, status: i % 3 === 0 ? (i % 2 ? "tool" : "thinking") : "idle", needsYou: null, stuck: null, mcp: [] });
  }
  const setData = scene.setData.bind(scene), setLive = scene.setLive.bind(scene), setVisible = scene.setVisible.bind(scene), dim = scene.dim.bind(scene);
  const snap = { ...x.live, sessions: live, collisions: [], collab: [], resources: [] };
  setVisible(null);
  dim(null);
  setData(out);
  setLive(snap);
  setTimeout(() => scene.reset(), 3600);
  scene.setData = (d) => d;
  scene.setLive = scene.setVisible = scene.dim = () => {};
  window.__benchRestore = () => Object.assign(scene, { setData, setLive, setVisible, dim });
  return { nodes: out.nodes.length, links: out.links.length, working: live.filter((s) => s.status !== "idle").length };
})()`;

async function orbit(s, ms, { width, height }) {
  const cx = Math.round(width / 2), cy = Math.round(height / 2);
  let dir = 1;
  for (const until = Date.now() + ms; Date.now() < until; dir = -dir) {
    await s.input({ drag: { from: { x: cx - 220 * dir, y: cy - 40 }, to: { x: cx + 220 * dir, y: cy + 40 }, steps: 40 }, settle: false }).catch(() => {});
  }
}

// Opens the cover window over XenoNet's (in screen points, with a margin)
// and returns a function that closes it.
async function cover(rect) {
  const { createRequire } = await import("node:module");
  const electron = createRequire(import.meta.url)("electron");
  const m = 24;
  const child = spawn(electron, [here, "--cover", [rect.x - m, rect.y - m, rect.width + 2 * m, rect.height + 2 * m].join(",")], { stdio: ["pipe", "pipe", "ignore"] });
  await new Promise((resolve, reject) => {
    child.stdout.on("data", (d) => String(d).includes("covering") && resolve());
    child.once("exit", (code) => reject(new Error(`the cover window exited (${code})`)));
    setTimeout(() => reject(new Error("the cover window didn't open")), 20000);
  });
  return async () => {
    child.stdin.end("bye\n");
    await new Promise((r) => (child.exitCode !== null ? r() : child.once("exit", r)));
  };
}

// ---------- comparing two runs ----------

function compare(beforeFile, afterFile) {
  const before = JSON.parse(fs.readFileSync(beforeFile, "utf8"));
  const after = JSON.parse(fs.readFileSync(afterFile, "utf8"));
  const row = (name, b, a) => `${name.padEnd(10)} gpu ${fmt(b?.gpu)} → ${fmt(a?.gpu)} %   fps ${fmt(b?.fps)} → ${fmt(a?.fps)}   frame gpu ${fmt(b?.frame?.gpuMs?.mean)} → ${fmt(a?.frame?.gpuMs?.mean)} ms   frame cpu ${fmt(b?.frame?.cpuMs?.mean)} → ${fmt(a?.frame?.cpuMs?.mean)} ms   renderer cpu ${fmt(b?.cpu.renderer)} → ${fmt(a?.cpu.renderer)} %   main cpu ${fmt(b?.cpu.main)} → ${fmt(a?.cpu.main)} %   gpu-process cpu ${fmt(b?.cpu.gpu)} → ${fmt(a?.cpu.gpu)} %`;
  const fmt = (v) => (v == null ? "  —" : String(v).padStart(5));
  for (const name of Object.keys(after.states)) console.log(row(name, before.states[name], after.states[name]));
}

// ---------- the run ----------

async function main() {
  const argv = process.argv.slice(2);
  if (argv[0] === "--compare") return compare(argv[1], argv[2]);
  if (process.platform !== "darwin") throw new Error("the GPU bench reads ioreg, so it runs on macOS only");
  const opt = (name, fallback) => (argv.includes(`--${name}`) ? argv[argv.indexOf(`--${name}`) + 1] : fallback);
  const id = opt("instance", "demo");
  if (!["demo", "test"].includes(id)) throw new Error("the bench runs on made-up data only: --instance demo or test");
  const ms = Number(opt("seconds", 10)) * 1000;
  const big = Number(opt("big", 0));
  const outFile = opt("out", null);
  const forceSeen = argv.includes("--seen");
  const plain = argv.includes("--plain");
  const baseline = Number(opt("baseline", ms / 1000)) * 1000;
  const ALL = ["unfocused", "rest", "orbit", "busy", "covered", "minimized", "hidden"];
  const flags = new Set(["--instance", "--seconds", "--big", "--out", "--baseline"]);
  const wanted = argv.filter((a, i) => !a.startsWith("--") && !flags.has(argv[i - 1]));
  for (const w of wanted) if (!ALL.includes(w)) throw new Error(`no state "${w}"; there's ${ALL.join(", ")}`);
  const states = wanted.length ? ALL.filter((x) => wanted.includes(x)) : ALL;

  const { xenonet, userDataDir, lockHolder, INSTANCES } = await import("../core/control.js");
  // A copy already running is someone's (another session's checks, or the
  // end-to-end tests): it's left alone, and the bench doesn't run.
  const holder = await lockHolder(userDataDir(INSTANCES[id].name));
  if (holder || (await xenonet(id).health())) throw new Error(`${INSTANCES[id].name} is already running${holder ? ` (process ${holder})` : ""}; quit it first, or use --instance ${id === "demo" ? "test" : "demo"}`);
  const port = id === "test" ? await new Promise((resolve) => { const p = net.createServer().listen(0, "127.0.0.1", () => { const { port } = p.address(); p.close(() => resolve(port)); }); }) : INSTANCES[id].port;
  const s = xenonet(id, { port });
  const front = frontApp();
  const result = { at: new Date().toISOString(), instance: id, seconds: ms / 1000, big, seen: forceSeen, plain, machine: `${os.cpus()[0]?.model ?? "?"}, ${Math.round(os.totalmem() / 2 ** 30)} GB`, states: {} };
  const log = (...a) => console.error("[bench:gpu]", ...a);
  try {
    // The machine without XenoNet, for the device's utilization, which
    // counts every app.
    if (baseline) {
      log("the machine without XenoNet…");
      result.machine = await utilization(baseline);
      log(`the machine without XenoNet: device ${result.machine.mean}% (median ${result.machine.p50}%, 90th percentile ${result.machine.p90}%)`);
    }
    process.env.XENONET_MUTED = "1";
    const p = await s.launch({ show: "inactive", timeout: 180000 });
    const procs = processesOf(p.pid, port);
    const page = await s.eval("return { dpr: devicePixelRatio, width: innerWidth, height: innerHeight, x: screenX, y: screenY, outerWidth, outerHeight, nodes: window.__xenonet.scene.nodes().length }");
    Object.assign(result, { window: p.window, page, processes: procs });
    log(`${INSTANCES[id].name}: window ${p.window.width}x${p.window.height} at ${page.dpr}x, ${page.nodes} nodes; processes ${JSON.stringify(procs)}`);
    if (p.window.width !== 1600 || p.window.height !== 960) log(`the window is ${p.window.width}x${p.window.height}, not 1600x960 (a small screen?)`);
    if (big) {
      result.bigWorld = await s.eval(bigWorld(big));
      log(`big world: ${JSON.stringify(result.bigWorld)}`);
    }
    if (plain) {
      await s.driver("plainPanels", true);
      log("plain panels: drawn solid, without the glass blur");
    }
    await s.driver("settled", { timeout: 60000 }).catch((e) => log(`not settled: ${e.message}`));
    await s.eval(COUNTER);
    // Requests to the window count as someone working it (the page keeps
    // drawing while it's driven), so each state starts a few seconds after
    // the last one.
    const quiet = () => sleep(5000);
    const shown = new Set(["unfocused", "rest", "orbit", "busy"]);
    const run = async (name, setup, during = null, teardown = null) => {
      if (!states.includes(name)) return;
      await setup?.();
      if (forceSeen && shown.has(name)) await s.eval(SEEN_ON);
      await quiet();
      log(`${name}…`);
      const [r] = await Promise.all([sample(s, procs, ms), during?.()]);
      result.states[name] = r;
      log(`${name}: gpu ${r.gpu}% (device ${r.device}%), ${r.fps} fps ${JSON.stringify(r.rates)}, frame gpu ${r.frame?.gpuMs?.mean ?? "—"} ms (gpu process ${r.frame?.gpuProcessMs ?? "—"} ms) cpu ${r.frame?.cpuMs?.mean ?? "—"} ms, cpu main ${r.cpu.main}% renderer ${r.cpu.renderer}% gpu ${r.cpu.gpu}% service ${r.cpu.service}%, renderer ${r.memMB.renderer} MB, heap ${r.memMB.heap} MB${r.drivenFrames ? `, ${r.drivenFrames} frames driven` : ""}`);
      if (forceSeen && shown.has(name)) await s.eval(SEEN_OFF);
      await teardown?.();
    };
    await run("unfocused", null);
    // The focused states. macOS may refuse to bring the window in front of
    // the app you're in; the log says so, and the page's own word is in
    // the result (page.focused).
    const focus = async () => {
      const w = await s.window({ show: "front" });
      if (!w.window?.focused) log("the window didn't take focus");
      await s.driver("settled", { timeout: 30000 }).catch(() => {});
    };
    await run("rest", focus);
    await run("orbit", focus, () => orbit(s, ms, page), () => s.driver("settled", { timeout: 30000 }).catch(() => {}));
    await run(
      "busy",
      async () => {
        await focus();
        if (!big) {
          await s.simulate("collision").catch(() => {});
          await s.simulate("needs-you").catch(() => {});
        }
        await s.eval(BUSY_START);
      },
      null,
      async () => {
        await s.eval(BUSY_STOP);
        if (!big) await s.simulate("clear").catch(() => {});
      },
    );
    giveFocusBack(front);
    let uncover = null;
    await run(
      "covered",
      async () => {
        const r = await s.eval("return { x: screenX, y: screenY, width: outerWidth, height: outerHeight }");
        uncover = await cover(r);
      },
      null,
      async () => uncover?.(),
    );
    await run("minimized", () => s.window({ minimize: true }), null, () => s.window({ show: "inactive" }));
    await run("hidden", () => s.window({ hide: true }), null, () => s.window({ show: "inactive" }));
    if (big) await s.eval("return (window.__benchRestore?.(), true)");
  } finally {
    giveFocusBack(front);
    if (plain) await s.driver("plainPanels", false).catch(() => {});
    await s.quit().catch(() => {});
    if (id === "test") fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  }
  const text = JSON.stringify(result, null, 2);
  if (outFile) fs.writeFileSync(path.resolve(outFile), text + "\n");
  console.log(text);
}

// ---------- the cover window (run by Electron, not Node) ----------
//
// `electron scripts/gpu-bench.mjs --cover x,y,w,h` opens an opaque window
// over that rectangle that never takes focus, and closes when its parent
// says so on stdin or after a minute.
async function coverWindow() {
  const { app, BrowserWindow } = await import("electron");
  const [x, y, width, height] = process.argv[process.argv.indexOf("--cover") + 1].split(",").map(Number);
  app.dock?.hide();
  setTimeout(() => app.quit(), 60e3);
  await app.whenReady();
  const win = new BrowserWindow({ x, y, width, height, frame: false, show: false, focusable: false, hasShadow: false, resizable: false, alwaysOnTop: true, backgroundColor: "#000000", enableLargerThanScreen: true });
  win.setBounds({ x, y, width, height });
  win.showInactive();
  console.log("covering");
  process.stdin.on("data", () => app.quit());
  process.stdin.on("end", () => app.quit());
}

// Electron's ready event waits for its entry module to finish loading, so
// the cover window isn't awaited here.
if (process.versions.electron) coverWindow();
else await main();
