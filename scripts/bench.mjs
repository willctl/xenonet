// Timings for XenoNet's hot paths, on made-up data only: the graph build, a
// full refresh of the demo world, tailing transcripts, the server's start,
// the actors' observers' looks, the page's load and frame time in a hidden
// XenoNet Dev, switching themes there, the frame time in each theme, what
// the actors layer adds to a frame, and what other machines on the map do.
//
//   node scripts/bench.mjs [graph] [refresh] [tail] [server] [observers] [page] [themes] [looks] [actors] [hosts]   (default: all)
//   node scripts/bench.mjs --repo <checkout> ...       time another checkout's code
//   node scripts/bench.mjs --compare before.jsonl after.jsonl
//
// Prints one JSON object per part; save them (> file.jsonl) to compare runs.
// Medians are what to compare, from runs made back to back on one machine.
// The page part starts XenoNet Dev's test instance on a spare port, so it
// can't run while another test instance is up (the end-to-end tests, or
// someone looking at their work): it says so and skips.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const argv = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const summary = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  if (!s.length) return { n: 0 };
  const q = (p) => s[Math.min(s.length - 1, Math.round(p * (s.length - 1)))];
  const r = (v) => Math.round(v * 100) / 100;
  return { n: s.length, median: r(q(0.5)), p90: r(q(0.9)), p95: r(q(0.95)), p99: r(q(0.99)), min: r(s[0]), max: r(s.at(-1)) };
};
async function timeIt(fn) {
  const t = performance.now();
  await fn();
  return performance.now() - t;
}
const freePort = () =>
  new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });
const out = (name, data) => console.log(JSON.stringify({ part: name, ...data }));
const demoWorld = (port) => path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`);
// A stopped demo server's git can still be writing in a repo for a moment
// (ENOTEMPTY): retry, and if the folder still won't go, say so and carry
// on, since it's only a made-up world in the temp folder.
function removeWorld(port) {
  try {
    fs.rmSync(demoWorld(port), { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  } catch (err) {
    console.error(`[bench] couldn't remove ${demoWorld(port)}: ${err.code ?? err.message}`);
  }
}

// ---------- comparing two runs ----------

// Every median in `after` against the same one in `before`; slower by more
// than 10% is flagged.
function compare(beforeFile, afterFile) {
  const read = (f) => Object.fromEntries(fs.readFileSync(f, "utf8").split("\n").filter((l) => l.startsWith("{")).map((l) => JSON.parse(l)).map((o) => [o.part, o]));
  const before = read(beforeFile);
  const after = read(afterFile);
  const medians = (o, at = []) =>
    Object.entries(o ?? {}).flatMap(([k, v]) => (v && typeof v === "object" ? medians(v, [...at, k]) : k === "median" && typeof v === "number" ? [[at.join(" › "), v]] : []));
  let flagged = 0;
  for (const [part, a] of Object.entries(after)) {
    const b = before[part];
    if (!b) continue;
    const old = new Map(medians(b));
    for (const [key, v] of medians(a)) {
      if (!old.has(key)) continue;
      const was = old.get(key);
      const change = was ? (v - was) / was : 0;
      const flag = change > 0.1 ? "  ◀ slower" : "";
      if (flag) flagged++;
      console.log(`${part} › ${key}: ${was} → ${v} ms (${change >= 0 ? "+" : ""}${(change * 100).toFixed(1)}%)${flag}`);
    }
  }
  console.log(flagged ? `${flagged} median(s) more than 10% slower` : "nothing more than 10% slower");
  process.exit(flagged ? 1 : 0);
}

if (argv[0] === "--compare") compare(argv[1], argv[2]);

const repoAt = argv.indexOf("--repo");
const repo = path.resolve(repoAt >= 0 ? argv[repoAt + 1] : path.join(path.dirname(fileURLToPath(import.meta.url)), ".."));
const parts = argv.filter((a, i) => !a.startsWith("--") && !(repoAt >= 0 && i === repoAt + 1));
const want = (p) => !parts.length || parts.includes(p);
const imp = (rel) => import(pathToFileURL(path.join(repo, rel)).href);

// Every part runs on made-up data. XenoNet's config builds a demo world for
// its port when first imported, so this process gets a spare port of its
// own and never touches the demo instance's world.
const ownPort = await freePort();
process.env.XENONET_DEMO = "1";
process.env.XENONET_PORT = String(ownPort);

// ---------- graph build ----------

async function benchGraph() {
  const { buildGraph } = await imp("core/graph.js");
  const { pathKey } = await imp("core/paths.js");
  let x = 7;
  const rnd = () => (x = (x * 1103515245 + 12345) % 2147483648) / 2147483648;
  const pick = (a) => a[Math.floor(rnd() * a.length)];
  const now = Date.parse("2026-10-03T12:00:00Z");
  const root = "/bench/Projects";
  function input(P, S) {
    const projects = [];
    for (let g = 0; g < P / 10; g++) {
      const gp = `${root}/group-${g}`;
      projects.push({ name: `group-${g}`, path: gp, key: pathKey(gp), rel: `group-${g}`, role: "group", detected: "contains", marker: null, parent: null, git: null, stack: [], description: null, modifiedAt: now });
      for (let i = 0; i < 9; i++) {
        const name = i === 8 ? `proj-${g}-0-local-db` : `proj-${g}-${i}`;
        const p = `${gp}/${name}`;
        projects.push({ name, path: p, key: pathKey(p), rel: `group-${g}/${name}`, role: "project", detected: "repo", marker: ".git", parent: pathKey(gp), git: { branch: "main", origin: `git@github.com:org${g % 7}/${name}.git` }, stack: ["node"], description: "A project", modifiedAt: now });
      }
    }
    const real = projects.filter((p) => p.role === "project");
    const nativeProjects = Array.from({ length: 10 }, (_, i) => ({ id: `np-${i}`, name: `Native ${i}`, roots: [real[i * 3].path] }));
    const sessions = [];
    for (let i = 0; i < S; i++) {
      const app = i % 3 ? "claude" : "codex";
      const r = rnd();
      const cwd = r < 0.75 ? `${pick(real).path}${rnd() < 0.3 ? "/src/sub" : ""}` : r < 0.85 ? `/bench/elsewhere/folder-${i % 40}` : r < 0.9 ? "/bench" : `/tmp/scratch-${i % 13}`;
      sessions.push({
        id: `${app}:00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
        app,
        rawId: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
        title: `Session ${i}`,
        cwd,
        createdAt: now - rnd() * 30 * 86400e3,
        updatedAt: now - rnd() * 30 * 86400e3,
        archived: rnd() < 0.2,
        parentId: i > 10 && rnd() < 0.1 ? sessions[Math.floor(rnd() * i)].id : null,
        parentKind: null,
        nativeProjectId: app === "codex" && rnd() < 0.1 ? `np-${i % 10}` : null,
        origin: rnd() < 0.05 ? `git@github.com:org1/proj-1-1.git` : null,
      });
    }
    const pages = Array.from({ length: 100 }, (_, i) => ({ slug: `page-${i}`, title: `Page ${i}`, type: "topic", summary: "", tags: [], updated: now, updatedBy: null, links: [`page-${(i + 1) % 100}`, `page-${(i + 7) % 100}`], projects: [real[i % real.length].key], path: `/bench/wiki/pages/page-${i}.md`, writes: [] }));
    const wiki = { dir: "/bench/wiki", pages, inbox: [], log: [] };
    const repos = new Map(real.filter((_, i) => i % 2).map((p) => [p.key, { branch: "main", dirty: 2, ahead: 0 }]));
    const layout = Object.fromEntries(real.filter((_, i) => i % 10 === 0).map((p) => [`project:${p.key}`, { x: 1, y: 2, z: 3 }]));
    const resources = ["agent-wiki", "Claude_Browser", "github", "linear", "sentry"].map((server, i) => ({ server, ts: now - i * 1000 }));
    return { projectsDir: root, homeDir: "/bench", projects, sessions, nativeProjects, resources, wiki, repos, layout, now, windows: { activeWindowMs: 600e3, recentWindowMs: 86400e3, projectActiveMs: 3600e3 } };
  }
  const result = {};
  for (const [label, P, S] of [["small (40 projects, 300 sessions)", 40, 300], ["large (400 projects, 4000 sessions)", 400, 4000]]) {
    const args = input(P, S);
    for (let i = 0; i < 5; i++) buildGraph(structuredClone(args));
    const times = [];
    for (let i = 0; i < 30; i++) {
      const a = structuredClone(args);
      times.push(await timeIt(() => buildGraph(a)));
    }
    result[label] = summary(times);
  }
  out("graph build (ms)", result);
}

// ---------- full state refresh (demo world) ----------

async function benchRefresh() {
  const { XenoNetState } = await imp("core/state.js");
  const st = new XenoNetState();
  const cold = await timeIt(() => st.refresh());
  for (let i = 0; i < 3; i++) await st.refresh();
  const times = [];
  for (let i = 0; i < 30; i++) times.push(await timeIt(() => st.refresh()));
  const sources = Object.fromEntries(st.sources.filter((s) => s.ms != null).map((s) => [s.name, s.ms]));
  st.stop();
  out("full state refresh, demo world (ms)", { cold: Math.round(cold), warm: summary(times), lastSources: sources });
}

// ---------- transcript tailing ----------

const iso = (t) => new Date(t).toISOString();
function claudeLines(n, t0, tag) {
  let s = "";
  for (let i = 0; i < n; i++) {
    const ts = iso(t0 + i * 1000);
    if (i % 4 === 0) s += JSON.stringify({ type: "user", timestamp: ts, cwd: "/bench/Projects/x", sessionId: tag, message: { role: "user", content: `Please look at the parser and fix case ${i} in the tokenizer, then run the tests again.` } }) + "\n";
    else if (i % 4 === 1)
      s += JSON.stringify({ type: "assistant", timestamp: ts, message: { id: `msg_${tag}_${i}`, model: "claude-sonnet-4-5", stop_reason: "tool_use", usage: { input_tokens: 12, cache_read_input_tokens: 40000 + i, cache_creation_input_tokens: 300, output_tokens: 150 }, content: [{ type: "text", text: "Let me read the tokenizer first and then edit the failing branch." }, { type: "tool_use", id: `tu_${tag}_${i}`, name: i % 8 === 1 ? "Edit" : "Bash", input: i % 8 === 1 ? { file_path: `/bench/Projects/x/src/file-${i % 20}.js`, old_string: "a".repeat(200), new_string: "b".repeat(220) } : { command: `npm test -- case-${i}`, description: "Run tests" } }] } }) + "\n";
    else if (i % 4 === 2) s += JSON.stringify({ type: "user", timestamp: ts, message: { role: "user", content: [{ type: "tool_result", tool_use_id: `tu_${tag}_${i - 1}`, content: "ok ".repeat(150), is_error: false }] } }) + "\n";
    else s += JSON.stringify({ type: "assistant", timestamp: ts, message: { id: `msg_${tag}_${i}`, model: "claude-sonnet-4-5", stop_reason: "end_turn", usage: { input_tokens: 5, cache_read_input_tokens: 41000 + i, cache_creation_input_tokens: 100, output_tokens: 80 }, content: [{ type: "text", text: "The tests pass now. The tokenizer handled escaped quotes wrong; it reads them as one token again." }] } }) + "\n";
  }
  return s;
}
function codexLines(n, t0) {
  let s = JSON.stringify({ type: "session_meta", timestamp: iso(t0), payload: { id: "x" } }) + "\n";
  let total = 0;
  for (let i = 0; i < n; i++) {
    const ts = iso(t0 + i * 1000);
    total += 3000;
    if (i % 5 === 0) s += JSON.stringify({ type: "event_msg", timestamp: ts, payload: { type: "task_started", model_context_window: 258400 } }) + "\n";
    else if (i % 5 === 1) s += JSON.stringify({ type: "event_msg", timestamp: ts, payload: { type: "token_count", info: { total_token_usage: { total_tokens: total, output_tokens: 300 }, last_token_usage: { total_tokens: 30000 }, model_context_window: 258400 }, rate_limits: { primary: { used_percent: 12, window_minutes: 300 } } } }) + "\n";
    else if (i % 5 === 2) s += JSON.stringify({ type: "event_msg", timestamp: ts, payload: { type: "item_completed", item: { type: "CommandExecution", command: ["/bin/zsh", "-lc", `pytest -k case_${i}`], exit_code: 0, status: "completed" } } }) + "\n";
    else if (i % 5 === 3) s += JSON.stringify({ type: "event_msg", timestamp: ts, payload: { type: "item_completed", item: { type: "FileChange", changes: { [`/bench/Projects/y/f${i % 9}.py`]: { type: "update", unified_diff: "@@ -1 +1 @@\n-a\n+b" } } } } }) + "\n";
    else s += JSON.stringify({ type: "event_msg", timestamp: ts, payload: { type: "task_complete", last_agent_message: "Done; the tests pass." } }) + "\n";
  }
  return s;
}
const uuid = (a, i) => `${String(a).padStart(8, "0")}-0000-4000-8000-${String(i).padStart(12, "0")}`;

// The shape of a busy machine: 120 Claude project folders with 600
// transcripts (80 written today, about 1 MB each), and 90 Codex day
// folders with 5,400 rollouts (12 today).
function writeTree(root) {
  const claudeRoot = path.join(root, "claude", "projects");
  const codexRoot = path.join(root, "codex", "sessions");
  const now = Date.now();
  const old = new Date(now - 3 * 86400e3);
  const recent = { claude: [], codex: [] };
  for (let p = 0; p < 120; p++) {
    const dir = path.join(claudeRoot, `-bench-Projects-p${p}`);
    fs.mkdirSync(dir, { recursive: true });
    for (let k = 0; k < 5; k++) {
      const id = uuid(p, k);
      const file = path.join(dir, `${id}.jsonl`);
      const isRecent = p < 16;
      fs.writeFileSync(file, isRecent ? claudeLines(1200, now - 3 * 3600e3, id) : claudeLines(4, old.getTime(), id));
      if (isRecent) recent.claude.push(file);
      else fs.utimesSync(file, old, old);
      // A subagents folder for some sessions, like Claude Code makes.
      if (k < 2) {
        const sub = path.join(dir, id, "subagents");
        fs.mkdirSync(sub, { recursive: true });
        const sf = path.join(sub, `agent-a${k}.jsonl`);
        fs.writeFileSync(sf, claudeLines(4, old.getTime(), `${id}-sub`));
        fs.utimesSync(sf, old, old);
      }
    }
  }
  for (let d = 0; d < 90; d++) {
    const day = new Date(now - (90 - d) * 86400e3);
    const dir = path.join(codexRoot, String(day.getFullYear()), String(day.getMonth() + 1).padStart(2, "0"), String(day.getDate()).padStart(2, "0"));
    fs.mkdirSync(dir, { recursive: true });
    for (let k = 0; k < 60; k++) {
      const file = path.join(dir, `rollout-${day.toISOString().slice(0, 10)}T10-00-00-${uuid(9000 + d, k)}.jsonl`);
      const isRecent = d === 89 && k < 12;
      fs.writeFileSync(file, isRecent ? codexLines(1500, now - 2 * 3600e3) : codexLines(3, old.getTime()));
      if (isRecent) recent.codex.push(file);
      else fs.utimesSync(file, old, old);
    }
  }
  return { claudeRoot, codexRoot, recent };
}

async function benchTail() {
  const { LiveTelemetry } = await imp("core/live.js");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-bench-tail-"));
  try {
    const tree = writeTree(root);
    const bytes = [...tree.recent.claude, ...tree.recent.codex].reduce((a, f) => a + fs.statSync(f).size, 0);
    const result = { recentFiles: tree.recent.claude.length + tree.recent.codex.length, recentMB: Math.round(bytes / 1e5) / 10 };

    // Start: find every transcript, read the recent ones silently.
    const starts = [];
    for (let i = 0; i < 3; i++) {
      const live = new LiveTelemetry({ claudeRoot: tree.claudeRoot, codexRoot: tree.codexRoot, claudeDesktopDir: null });
      starts.push(await timeIt(() => live.start()));
      live.stop();
    }
    result.startMs = summary(starts);

    const live = new LiveTelemetry({ claudeRoot: tree.claudeRoot, codexRoot: tree.codexRoot, claudeDesktopDir: null });
    await live.start();
    // A sweep when nothing changed: the safety net's cost every 2 seconds.
    const sweeps = [];
    for (let i = 0; i < 20; i++) sweeps.push(await timeIt(() => live.sweep({ discover: false })));
    result.sweepMs = summary(sweeps);
    // The sweep every 30 seconds also looks for new transcripts.
    const walks = [];
    for (let i = 0; i < 10; i++) walks.push(await timeIt(() => live.sweep({ discover: true })));
    result.discoverSweepMs = summary(walks);

    // An appended line, from write to event.
    const lat = [];
    const file = tree.recent.claude[0];
    for (let i = 0; i < 30; i++) {
      const id = `tu_bench_${i}`;
      const t = performance.now();
      const seen = new Promise((resolve) => {
        const on = (e) => e.kind === "tool" && e.detail === `bench ${i}` && (live.off("activity", on), resolve(performance.now() - t));
        live.on("activity", on);
        setTimeout(() => (live.off("activity", on), resolve(null)), 6000);
      });
      fs.appendFileSync(file, JSON.stringify({ type: "assistant", timestamp: iso(Date.now()), message: { id: `msg_bench_${i}`, model: "m", stop_reason: "tool_use", content: [{ type: "tool_use", id, name: "Bash", input: { command: `bench ${i}` } }] } }) + "\n");
      lat.push(await seen);
      await sleep(120);
    }
    result.appendToEventMs = summary(lat.filter((v) => v != null));
    result.appendMissed = lat.filter((v) => v == null).length;

    // A new session's first line, from write to event.
    const fresh = [];
    for (let i = 0; i < 10; i++) {
      const dir = path.join(tree.claudeRoot, `-bench-Projects-p${i}`);
      const f = path.join(dir, `${uuid(77777, i)}.jsonl`);
      const t = performance.now();
      const seen = new Promise((resolve) => {
        const on = (e) => e.kind === "prompt" && e.detail === `new ${i}` && (live.off("activity", on), resolve(performance.now() - t));
        live.on("activity", on);
        setTimeout(() => (live.off("activity", on), resolve(null)), 8000);
      });
      fs.writeFileSync(f, JSON.stringify({ type: "user", timestamp: iso(Date.now()), message: { role: "user", content: `new ${i}` } }) + "\n");
      fresh.push(await seen);
      await sleep(150);
    }
    result.newFileToEventMs = summary(fresh.filter((v) => v != null));
    result.newFileMissed = fresh.filter((v) => v == null).length;
    live.stop();
    out("transcript tailing", result);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

// ---------- server start to first state (demo) ----------

async function benchServer() {
  const runs = [];
  for (let i = 0; i < 5; i++) {
    const port = await freePort();
    const t0 = performance.now();
    const proc = spawn(process.execPath, [path.join(repo, "server", "index.js"), "--demo"], { cwd: repo, env: { ...process.env, XENONET_PORT: String(port) }, stdio: ["ignore", "ignore", "inherit"] });
    let listening = null;
    for (const until = t0 + 30000; performance.now() < until; await sleep(10)) {
      try {
        const res = await fetch(`http://127.0.0.1:${port}/api/health`);
        if (res.ok) {
          listening = performance.now() - t0;
          break;
        }
      } catch {}
    }
    const g = await (await fetch(`http://127.0.0.1:${port}/api/graph`)).json();
    runs.push({ listening, graph: performance.now() - t0, nodes: g.nodes.length });
    proc.kill("SIGTERM");
    await new Promise((r) => proc.once("exit", r));
    removeWorld(port);
  }
  out("server start to first state, demo (ms)", { listening: summary(runs.map((r) => r.listening)), firstGraph: summary(runs.map((r) => r.graph)), nodes: runs[0].nodes });
}

// ---------- the page: load and frame time (hidden XenoNet Dev) ----------

async function benchPage() {
  const name = "page load and frame time, demo (ms)";
  const { xenonet } = await imp("core/control.js");
  // A test instance takes one lock whatever its port, so a second one would
  // quit at once. Leave a running one alone.
  if (await xenonet("test").health()) return out(name, { skipped: "a test instance is already running; quit it and run again" });
  const port = await freePort();
  const s = xenonet("test", { port });
  const t0 = performance.now();
  await s.launch({ show: "hidden", timeout: 120000 });
  const appStart = performance.now() - t0;
  // Waits for the page that replaces the one with this time origin.
  const nextPage = async (origin) => {
    for (const until = Date.now() + 90000; Date.now() < until; await sleep(20)) {
      const p = await s.eval("({ origin: performance.timeOrigin, ready: !!window.__xenonet?.driver })", { timeout: 2000 }).catch(() => null);
      if (p && p.origin !== origin && p.ready) return;
    }
    throw new Error("the page didn't come back");
  };
  try {
    // A hidden window gets no animation frames, so the page runs with
    // ?raf-shim, which keeps the scene drawing on a 16 ms timer.
    const first = await s.eval("performance.timeOrigin");
    await s.eval("(setTimeout(() => { location.search = '?raf-shim'; }, 0), true)");
    await nextPage(first);
    if (!(await s.eval("location.search")).includes("raf-shim")) throw new Error("the page isn't running with ?raf-shim");

    // Page load, on the page's own clock: DOM ready, the graph fetched, the
    // driver up (polled every 20 ms) and the view first settled.
    const loads = [];
    for (let i = 0; i < 6; i++) {
      const origin = await s.eval("performance.timeOrigin");
      await s.call({ op: "reload" });
      await nextPage(origin);
      loads.push(
        await s.eval(
          `return (async () => {
            const ready = performance.now();
            await window.__xenonet.driver.settled({ timeout: 30000 });
            const settled = performance.now();
            const nav = performance.getEntriesByType("navigation")[0];
            const graph = performance.getEntriesByType("resource").find((e) => /\\/api\\/graph(\\?|$)/.test(e.name));
            return { dom: nav.domContentLoadedEventEnd, graph: graph ? graph.responseEnd : null, ready, settled };
          })()`,
          { timeout: 45000 },
        ),
      );
    }
    const loadOf = (k) => summary(loads.slice(1).map((l) => l[k]).filter((v) => v != null));

    // Frame time: the time spent in animation-frame callbacks, summed per
    // 16 ms frame, over 20 s of the demo world at work.
    const frames = await s.eval(
      `return await new Promise((resolve) => {
        const orig = window.requestAnimationFrame;
        const slots = new Map();
        let calls = 0;
        window.requestAnimationFrame = (cb) => orig((t) => {
          const a = performance.now();
          try { cb(t); } finally {
            const k = Math.floor(a / 16);
            slots.set(k, (slots.get(k) ?? 0) + (performance.now() - a));
            calls++;
          }
        });
        setTimeout(() => {
          window.requestAnimationFrame = orig;
          resolve({ frames: [...slots.values()], calls, heapMB: performance.memory ? Math.round(performance.memory.usedJSHeapSize / 1e5) / 10 : null });
        }, 20000);
      })`,
      { timeout: 40000 },
    );
    const busy = frames.frames.reduce((a, b) => a + b, 0);
    out(name, {
      appStart: Math.round(appStart),
      load: { dom: loadOf("dom"), graphFetched: loadOf("graph"), ready: loadOf("ready"), settled: loadOf("settled") },
      frame: summary(frames.frames),
      framesPerSecond: Math.round(frames.frames.length / 2) / 10,
      busyMsPerSecond: Math.round(busy / 2) / 10,
      heapMB: frames.heapMB,
      note: "dev server (unbundled modules), hidden window with ?raf-shim; the first load is left out",
    });
  } finally {
    await s.quit().catch(() => {});
    removeWorld(port);
  }
}

// ---------- switching themes (hidden XenoNet Dev) ----------

// Every theme in turn, five times over, in the page as the page part runs
// it (the test instance, ?raf-shim): how long a switch takes until it's
// drawn and the view has settled, and the frame time, the JS heap after
// garbage collection and the GPU's geometries, textures and shader programs
// before and after. None of them should grow, and the page never reloads.
async function benchThemes() {
  const name = "theme switching, demo (ms)";
  const { xenonet } = await imp("core/control.js");
  if (await xenonet("test").health()) return out(name, { skipped: "a test instance is already running; quit it and run again" });
  const port = await freePort();
  const s = xenonet("test", { port });
  await s.launch({ show: "hidden", timeout: 120000 });
  try {
    const first = await s.eval("performance.timeOrigin");
    await s.eval("(setTimeout(() => { location.search = '?raf-shim'; }, 0), true)");
    let origin = null;
    for (const until = Date.now() + 90000; !origin; await sleep(20)) {
      const p = await s.eval("({ origin: performance.timeOrigin, ready: !!window.__xenonet?.driver })", { timeout: 2000 }).catch(() => null);
      if (p && p.origin !== first && p.ready) origin = p.origin;
      else if (Date.now() > until) throw new Error("the page didn't come back");
    }
    await s.driver("settled", { timeout: 30000 });
    // Frame time as the page part measures it, over 8 s, then memory.
    const measure = async () => {
      const frames = await s.eval(
        `return await new Promise((resolve) => {
          const orig = window.requestAnimationFrame;
          const slots = new Map();
          window.requestAnimationFrame = (cb) => orig((t) => {
            const a = performance.now();
            try { cb(t); } finally {
              const k = Math.floor(a / 16);
              slots.set(k, (slots.get(k) ?? 0) + (performance.now() - a));
            }
          });
          setTimeout(() => ((window.requestAnimationFrame = orig), resolve([...slots.values()])), 8000);
        })`,
        { timeout: 30000 },
      );
      await s.call({ op: "cdp", method: "HeapProfiler.collectGarbage" });
      const heap = await s.call({ op: "cdp", method: "Runtime.getHeapUsage" });
      const gpu = await s.eval("return (() => { const r = window.__xenonet.scene.graph.renderer(); return { geometries: r.info.memory.geometries, textures: r.info.memory.textures, programs: r.info.programs?.length ?? null }; })()");
      return { frame: summary(frames), heapMB: Math.round(heap.usedSize / 1e5) / 10, ...gpu };
    };
    const before = await measure();
    const { themes, current } = await s.driver("themes");
    const order = [...themes.filter((t) => t.id !== current), themes.find((t) => t.id === current)];
    const switches = [];
    for (let round = 0; round < 5; round++) {
      for (const t of order) {
        const t0 = performance.now();
        await s.driver("theme", t.id, { timeout: 30000 });
        switches.push(performance.now() - t0);
      }
    }
    const reloaded = (await s.eval("performance.timeOrigin")) !== origin;
    const after = await measure();
    out(name, { themes: themes.length, switches: switches.length, switchToSettled: summary(switches), reloaded, before, after, note: "each switch until drawn and settled, through the control channel; memory after garbage collection" });
  } finally {
    await s.quit().catch(() => {});
    removeWorld(port);
  }
}

// ---------- frame time in each theme (hidden XenoNet Dev) ----------

// Each theme drawn in turn, in the page as the page part runs it (the test
// instance, ?raf-shim), and its frame time over a few seconds, against
// XenoNet's own measured before and after them: a theme's stars, floor,
// sweep, sonar and lines should cost about what XenoNet's do.
async function benchLooks() {
  const name = "frame time in each theme, demo (ms)";
  const { xenonet } = await imp("core/control.js");
  if (await xenonet("test").health()) return out(name, { skipped: "a test instance is already running; quit it and run again" });
  const port = await freePort();
  const s = xenonet("test", { port });
  await s.launch({ show: "hidden", timeout: 120000 });
  try {
    const first = await s.eval("performance.timeOrigin");
    await s.eval("(setTimeout(() => { location.search = '?raf-shim'; }, 0), true)");
    for (const until = Date.now() + 90000; ; await sleep(20)) {
      const p = await s.eval("({ origin: performance.timeOrigin, ready: !!window.__xenonet?.driver })", { timeout: 2000 }).catch(() => null);
      if (p && p.origin !== first && p.ready) break;
      if (Date.now() > until) throw new Error("the page didn't come back");
    }
    await s.driver("settled", { timeout: 30000 });
    // Time in animation-frame callbacks per 16 ms frame, as the page part
    // measures it, over 5 s of the demo world at work in one theme.
    const frames = async (id) => {
      await s.driver("theme", id, { timeout: 30000 });
      await sleep(1000);
      return s.eval(
        `return await new Promise((resolve) => {
          const orig = window.requestAnimationFrame;
          const slots = new Map();
          window.requestAnimationFrame = (cb) => orig((t) => {
            const a = performance.now();
            try { cb(t); } finally {
              const k = Math.floor(a / 16);
              slots.set(k, (slots.get(k) ?? 0) + (performance.now() - a));
            }
          });
          setTimeout(() => ((window.requestAnimationFrame = orig), resolve([...slots.values()])), 5000);
        })`,
        { timeout: 30000 },
      );
    };
    const { themes, current } = await s.driver("themes");
    const ids = themes.map((t) => t.id).filter((id) => id !== "xenonet");
    const xenonetFrames = await frames("xenonet");
    const each = {};
    for (const id of ids) each[id] = summary(await frames(id));
    xenonetFrames.push(...(await frames("xenonet")));
    const base = summary(xenonetFrames);
    await s.driver("theme", current, { timeout: 30000 });
    const vsXenoNet = Object.fromEntries(Object.entries(each).map(([id, f]) => [id, Math.round((f.median / base.median) * 100) / 100]));
    out(name, { xenonet: base, ...each, vsXenoNet, note: "median and p90 of the time in animation-frame callbacks per 16 ms frame, 5 s in each theme; vsXenoNet is the median against XenoNet's" });
  } finally {
    await s.quit().catch(() => {});
    removeWorld(port);
  }
}

// ---------- the observers' looks (Hermes Agent, Grok Bot) ----------

// Each observer's look on a made-up machine: a process table of 1,100
// processes, a Hermes install whose session database holds 200 sessions
// and 20,000 messages, and Grok Bot's app with a few commands running.
// The process table is the test harness's, so this times XenoNet's own work
// on what ps, pgrep and lsof say, not those commands.
async function benchObservers() {
  let mods;
  try {
    mods = {
      ...(await imp("core/actors/index.js")),
      ...(await imp("core/actors/hermes.js")),
      ...(await imp("core/actors/grok-bot.js")),
      ...(await imp("core/procs.js")),
      ...(await imp("test/harness/procs.js")),
      fixture: await imp("test/fixtures/hermes.js"),
    };
  } catch {
    return out("observers' looks (ms)", { skipped: "this checkout has no observers" });
  }
  const { Actors, HermesObserver, GrokBotObserver, createProcs, fakeProcesses, fixture } = mods;
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-bench-observers-"));
  try {
    const clock = { t: Date.parse("2026-10-05T12:00:00Z"), now: () => clock.t, setTimeout, clearTimeout };
    const m = fakeProcesses({ clock });
    for (let i = 0; i < 1100; i++) m.add({ ppid: 1, program: i % 10 ? `/usr/libexec/daemon-${i}` : "/usr/local/bin/node", args: [`/opt/tool-${i}.js`] });
    const root = fixture.hermesRoot(path.join(home, ".hermes"));
    const app = m.add({ ppid: 1, program: `${root}/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app/Contents/MacOS/Hermes` });
    const host = m.add({ ppid: app, program: `${root}/tools/python-3.14.7/bin/python3`, args: ["-m", "hermes_cli.main", "serve"] });
    fixture.setLedger(root, [{ purpose: "serve", pid: host, createTime: m.table.get(host).start / 1000, spawner_pid: app }]);
    fixture.setTicker(root, { at: clock.t, pid: host });
    const db = fixture.stateDb(path.join(root, "state.db"));
    for (let i = 0; i < 200; i++) db.session({ id: `s${i}`, source: "tui", cwd: `/bench/Projects/p${i % 40}`, startedAt: clock.t - (200 - i) * 60e3, activityAt: clock.t - (200 - i) * 30e3, activity: "executing tool: terminal" });
    db.db.exec("BEGIN");
    for (let i = 0; i < 20000; i++) db.message(i % 4 === 1 ? { session: `s${i % 200}`, at: clock.t - (20000 - i) * 300, calls: [{ name: i % 8 === 1 ? "write_file" : "terminal", args: { path: `src/f${i % 50}.ts` } }] } : { session: `s${i % 200}`, role: i % 4 === 2 ? "tool" : "user", tool: i % 4 === 2 ? "terminal" : null, at: clock.t - (20000 - i) * 300 });
    db.db.exec("COMMIT");
    const grok = m.add({ ppid: 1, program: "/Applications/Grok Bot.app/Contents/MacOS/Grok Bot" });
    const helper = m.add({ ppid: grok, program: "/Applications/Grok Bot.app/Contents/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper" });
    for (let i = 0; i < 3; i++) m.add({ ppid: helper, program: "/bin/zsh", cwd: `/bench/Projects/p${i}` });
    const places = new Map(Array.from({ length: 40 }, (_, i) => [`/bench/projects/p${i}`, `project:/bench/projects/p${i}`]));
    const map = { ready: () => true, has: (id) => [...places.values()].includes(id), describe: (id) => ({ kind: "project", name: id.split("/").pop(), path: null }), placeOf: (p) => places.get(String(p).toLowerCase().split("/").slice(0, 4).join("/")) ?? null, sessions: () => [], recentSession: () => null };
    const actors = new Actors({ now: () => clock.t, map });
    const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
    const hermes = new HermesObserver({ actors, procs, home, env: {}, clock });
    // Grok Bot's app as its observer reads it: whether it's there, and the
    // version in its Info.plist (a made-up one, as XML), read once and then
    // looked at for changes, so its looks measure the version read too.
    const plist = "/Applications/Grok Bot.app/Contents/Info.plist";
    const info = `<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0">\n<dict>\n\t<key>CFBundleIdentifier</key>\n\t<string>com.example.made-up-bundle</string>\n\t<key>CFBundleShortVersionString</key>\n\t<string>0.70.4</string>\n</dict>\n</plist>\n`;
    const written = clock.t - 86400e3;
    const missing = () => Object.assign(new Error("no such file"), { code: "ENOENT" });
    const grokFiles = {
      existsSync: () => true,
      statSync: (p) => {
        if (p !== plist) throw missing();
        return { mtimeMs: written, size: Buffer.byteLength(info) };
      },
      readFileSync: (p) => {
        if (p !== plist) throw missing();
        return Buffer.from(info);
      },
    };
    const bot = new GrokBotObserver({ actors, procs, home, apps: ["/Applications/Grok Bot.app"], files: grokFiles, alive: (pid) => m.table.has(pid), clock });
    const step = async (fn) => {
      clock.t += 5000;
      return timeIt(fn);
    };
    const first = await step(() => hermes.tick(clock.t));
    const quiet = [];
    for (let i = 0; i < 30; i++) quiet.push(await step(() => hermes.tick(clock.t)));
    const busy = [];
    for (let i = 0; i < 30; i++) {
      for (let k = 0; k < 20; k++) db.message({ session: `s${k}`, role: "tool", tool: "terminal", at: clock.t });
      busy.push(await step(() => hermes.tick(clock.t)));
    }
    const looks = [];
    for (let i = 0; i < 30; i++) looks.push(await step(() => bot.tick(clock.t)));
    const polls = [];
    for (let i = 0; i < 30; i++) polls.push(await timeIt(() => bot.poll(clock.t)));
    db.close();
    actors.stop();
    out("observers' looks (ms)", { hermesFirstLook: Math.round(first * 100) / 100, hermesQuiet: summary(quiet), hermes20NewRows: summary(busy), grokBotLook: summary(looks), grokBotPoll: summary(polls), processes: m.table.size });
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
}

// ---------- the actors layer: frame time with it and without (hidden XenoNet Dev) ----------

// What the actors layer (src/actors.js) adds to a frame, in the page as the
// page part runs it (the test instance, ?raf-shim). For each cast of actors,
// windows of a few seconds with the layer on and off take turns, so a busy
// machine weighs on both alike: the time in animation-frame callbacks per
// 16 ms frame in each, and the layer's own step each frame while it's on.
// The casts: the demo's actors as the server sends them; six actors with
// 24 beams, the budget's (docs/design/next-phase.md: at most 0.5 ms median
// and 1.0 ms p95 a frame), made of the demo's four and two made up, held
// still so every window draws the same while the demo's events still reach
// them; and none at all, which should cost close to nothing.
async function benchActors() {
  const name = "actors layer, frame time (ms)";
  const { xenonet } = await imp("core/control.js");
  if (await xenonet("test").health()) return out(name, { skipped: "a test instance is already running; quit it and run again" });
  const port = await freePort();
  const s = xenonet("test", { port });
  await s.launch({ show: "hidden", timeout: 120000 });
  try {
    const first = await s.eval("performance.timeOrigin");
    await s.eval("(setTimeout(() => { location.search = '?raf-shim'; }, 0), true)");
    for (const until = Date.now() + 90000; ; await sleep(20)) {
      const p = await s.eval("({ origin: performance.timeOrigin, ready: !!window.__xenonet?.driver })", { timeout: 2000 }).catch(() => null);
      if (p && p.origin !== first && p.ready) break;
      if (Date.now() > until) throw new Error("the page didn't come back");
    }
    await s.driver("settled", { timeout: 30000 });
    // The layer times its own step from here on.
    await s.eval(`return (() => {
      const layer = window.__xenonet.scene.actorLayer;
      if (!layer.timedStep) {
        const step = layer.step;
        layer.times = [];
        layer.step = (f) => { const a = performance.now(); step(f); if (layer.enabled) layer.times.push(performance.now() - a); };
        layer.timedStep = true;
      }
      return true;
    })()`);
    // One window: the layer on or off, then the frames and its own steps.
    const measure = (on, ms) =>
      s.eval(
        `return await new Promise((resolve) => {
          const layer = window.__xenonet.scene.actorLayer;
          layer.setEnabled(${on});
          setTimeout(() => {
            layer.times = [];
            const orig = window.requestAnimationFrame;
            const slots = new Map();
            window.requestAnimationFrame = (cb) => orig((t) => {
              const a = performance.now();
              try { cb(t); } finally {
                const k = Math.floor(a / 16);
                slots.set(k, (slots.get(k) ?? 0) + (performance.now() - a));
              }
            });
            setTimeout(() => {
              window.requestAnimationFrame = orig;
              resolve({ frames: [...slots.values()], steps: layer.times.slice(), ...layer.stats() });
            }, ${ms});
          }, 400);
        })`,
        { timeout: 60000 },
      );
    const run = async (cast) => {
      await s.eval(cast);
      await sleep(1500);
      const on = { frames: [], steps: [] }, off = { frames: [] };
      let seen = null;
      for (let round = 0; round < 6; round++) {
        const a = await measure(true, 2500);
        on.frames.push(...a.frames);
        on.steps.push(...a.steps);
        seen = { actors: a.actors, beams: a.beams };
        off.frames.push(...(await measure(false, 2500)).frames);
      }
      const fOn = summary(on.frames), fOff = summary(off.frames);
      const mean = (xs) => Math.round((xs.reduce((x, y) => x + y, 0) / Math.max(1, xs.length)) * 1000) / 1000;
      return { ...seen, layerStep: { ...summary(on.steps), mean: mean(on.steps) }, frameOn: fOn, frameOff: fOff, added: { median: Math.round((fOn.median - fOff.median) * 100) / 100, p95: Math.round((fOn.p95 - fOff.p95) * 100) / 100 } };
    };
    const demo = await run("return (window.__xenonet.scene.actorLayer.setCast(null), true)");
    // Six working actors with four fresh beams each: the demo's four (you
    // included) and two made up, on the demo's own sessions and projects.
    const six = await run(`return (() => {
      const layer = window.__xenonet.scene.actorLayer;
      layer.setCast(null);
      const now = Date.now();
      const nodes = window.__xenonet.scene.nodes().filter((n) => (n.kind === "session" || n.kind === "project") && Number.isFinite(n.x)).map((n) => n.id);
      const made = ["finch", "heron"].map((slug) => ({ schema: "actor/snapshot/v1", id: "actor:" + slug, kind: "agent", name: slug[0].toUpperCase() + slug.slice(1), product: null, host: null, via: ["demo"], presence: "working", presenceSince: now - 6e5, lastSeen: now, heartbeatMs: 60000, status: "Made up for the benchmark", capabilities: [], targets: [], recent: [], counts: { events24h: 0, spawned24h: 0, messages24h: 0 }, identity: null }));
      const cast = [...window.__xenonet.scene.actors().map((r) => ({ ...r.snapshot, presence: "working" })), ...made].slice(0, 6);
      cast.forEach((a, k) => { a.targets = Array.from({ length: 4 }, (_, i) => ({ id: nodes[(k * 4 + i) % nodes.length], relation: i % 2 ? "spawned" : "works-on", since: now - 6e4, lastAt: now - i * 1000 })); });
      layer.setCast(cast);
      return cast.length;
    })()`);
    const none = await run("return (window.__xenonet.scene.actorLayer.setCast([]), true)");
    await s.eval("return (window.__xenonet.scene.actorLayer.setCast(null), window.__xenonet.scene.actorLayer.setEnabled(true), true)");
    const within = six.layerStep.median <= 0.5 && six.layerStep.p95 <= 1 && six.added.median <= 0.5 && six.added.p95 <= 1;
    out(name, { demo, six, none, budget: { median: 0.5, p95: 1, within }, note: "per 16 ms frame: frameOn and frameOff are the time in animation-frame callbacks with the layer on and off, in windows that take turns; added is on less off; layerStep is the layer's own step. The page's clock is coarsened to about 0.1 ms, so layerStep's mean says more than its median" });
  } finally {
    await s.quit().catch(() => {});
    removeWorld(port);
  }
}

// ---------- hosts: frame time with other machines drawn and not (hidden XenoNet Dev) ----------

// What other machines on the map add to a frame (docs/design/hosts.md), in
// the page as the page part runs it (the test instance, ?raf-shim), with
// hosts shown (XENONET_HOSTS=1). Two demo XenoNets on spare ports stand in for
// two machines the demo's size, added with XenoNet Dev's host-add simulation
// as the end-to-end test adds one. Windows of a few seconds take turns, so
// a busy machine weighs on all alike, with the machines' chips picked
// three ways: only this machine's, so no host is drawn; none, as the map
// starts, which draws every host node and at most the cap of the hosts'
// nodes beyond what you're looking at (src/hosts.js); and every machine's,
// which is looking at all of them, so they're drawn in full. The budget
// (docs/design/hosts.md): the hosts you're not looking at add at most 2 ms
// median and 3 ms p95 a frame. At the end of Wave 2, these two hosts drawn
// in full added about 3.2 ms.
async function benchHosts() {
  const name = "hosts, frame time (ms)";
  const { xenonet } = await imp("core/control.js");
  if (await xenonet("test").health()) return out(name, { skipped: "a test instance is already running; quit it and run again" });
  const ports = [];
  while (ports.length < 3) {
    const p = await freePort();
    if (!ports.includes(p)) ports.push(p);
  }
  const [port, ...hostPorts] = ports;
  const ids = ["demo-b", "demo-c"];
  const home = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-bench-hosts-home-"));
  // The other machines: plain demo XenoNets without the flag, as any host
  // runs, with a home of their own.
  const others = hostPorts.map((p) =>
    spawn(process.execPath, [path.join(repo, "server", "index.js"), "--demo"], {
      cwd: repo,
      env: { ...process.env, HOME: home, USERPROFILE: home, XENONET_PORT: String(p), XENONET_HOSTS: "", XENONET_INSTANCE: "", XENONET_CONTROL: "", XENONET_EXEC: "", XENONET_HOT: "0" },
      stdio: ["ignore", "ignore", "inherit"],
    }),
  );
  const s = xenonet("test", { port });
  try {
    // The test instance takes the flag from the environment it starts in.
    const was = process.env.XENONET_HOSTS;
    process.env.XENONET_HOSTS = "1";
    try {
      await s.launch({ show: "hidden", timeout: 120000 });
    } finally {
      if (was === undefined) delete process.env.XENONET_HOSTS;
      else process.env.XENONET_HOSTS = was;
    }
    const first = await s.eval("performance.timeOrigin");
    await s.eval("(setTimeout(() => { location.search = '?raf-shim'; }, 0), true)");
    for (const until = Date.now() + 90000; ; await sleep(20)) {
      const p = await s.eval("({ origin: performance.timeOrigin, ready: !!window.__xenonet?.driver })", { timeout: 2000 }).catch(() => null);
      if (p && p.origin !== first && p.ready) break;
      if (Date.now() > until) throw new Error("the page didn't come back");
    }
    // Each other XenoNet up with its map, then added as a host, and both live
    // on the map.
    for (const [i, p] of hostPorts.entries()) {
      for (const until = Date.now() + 60000; ; await sleep(250)) {
        const g = await fetch(`http://127.0.0.1:${p}/api/graph`).then((r) => r.json()).catch(() => null);
        if (g?.nodes?.some((n) => n.kind === "session")) break;
        if (Date.now() > until) throw new Error(`the demo XenoNet on port ${p} didn't start`);
      }
      const res = await s.simulate("host-add", { a: ids[i], b: String(p) });
      if (!res?.ok) throw new Error(`couldn't add ${ids[i]}: ${JSON.stringify(res)}`);
    }
    for (const until = Date.now() + 60000; ; await sleep(250)) {
      const { hosts } = await s.driver("hosts");
      if (ids.every((id) => hosts.some((x) => x.id === id && x.state === "live" && x.drawn))) break;
      if (Date.now() > until) throw new Error(`the hosts didn't come up live: ${JSON.stringify(hosts)}`);
    }
    await s.driver("settled", { timeout: 30000 });
    // One window: the chips picked, a moment for what that draws, then the
    // frames, and how many of the hosts' nodes are drawn of those on the map.
    const measure = async (keys, ms) => {
      await s.driver("filterHosts", keys, { timeout: 30000 });
      return s.eval(
        `return await new Promise((resolve) => {
          setTimeout(() => {
            const orig = window.requestAnimationFrame;
            const slots = new Map();
            window.requestAnimationFrame = (cb) => orig((t) => {
              const a = performance.now();
              try { cb(t); } finally {
                const k = Math.floor(a / 16);
                slots.set(k, (slots.get(k) ?? 0) + (performance.now() - a));
              }
            });
            setTimeout(() => {
              window.requestAnimationFrame = orig;
              const remote = window.__xenonet.scene.nodes().filter((n) => n.host);
              resolve({ frames: [...slots.values()], drawn: remote.filter((n) => n.__threeObj).length, nodes: remote.length });
            }, ${ms});
          }, 400);
        })`,
        { timeout: 60000 },
      );
    };
    const ways = { none: ["@here"], capped: [], full: ["@here", ...ids] };
    const got = Object.fromEntries(Object.keys(ways).map((k) => [k, { frames: [], drawn: null, nodes: null }]));
    for (let round = 0; round < 6; round++) {
      for (const [k, keys] of Object.entries(ways)) {
        const m = await measure(keys, 2500);
        got[k].frames.push(...m.frames);
        Object.assign(got[k], { drawn: m.drawn, nodes: m.nodes });
      }
    }
    await s.driver("filterHosts", []);
    const frame = Object.fromEntries(Object.entries(got).map(([k, v]) => [k, summary(v.frames)]));
    const added = (k) => ({ median: Math.round((frame[k].median - frame.none.median) * 100) / 100, p95: Math.round((frame[k].p95 - frame.none.p95) * 100) / 100 });
    const within = added("capped").median <= 2 && added("capped").p95 <= 3;
    const way = (k) => ({ frame: frame[k], drawn: got[k].drawn, of: got[k].nodes });
    out(name, {
      hosts: ids.length,
      none: way("none"),
      capped: way("capped"),
      full: way("full"),
      added: { capped: added("capped"), full: added("full") },
      budget: { median: 2, p95: 3, within },
      note: "per 16 ms frame, the time in animation-frame callbacks, in windows that take turns: none draws no host, capped the hosts as the map draws them when you look at neither, full all of both; added is against none; drawn counts the hosts' nodes drawn (host nodes included), of those on the map",
    });
  } finally {
    await s.quit().catch(() => {});
    for (const child of others) {
      if (child.exitCode !== null) continue;
      child.kill("SIGTERM");
      await new Promise((r) => child.once("exit", r));
    }
    for (const p of ports) removeWorld(p);
    fs.rmSync(home, { recursive: true, force: true });
  }
}

try {
  if (want("graph")) await benchGraph();
  if (want("refresh")) await benchRefresh();
  if (want("tail")) await benchTail();
  if (want("server")) await benchServer();
  if (want("observers")) await benchObservers();
  if (want("page")) await benchPage();
  if (want("themes")) await benchThemes();
  if (want("looks")) await benchLooks();
  if (want("actors")) await benchActors();
  if (want("hosts")) await benchHosts();
} finally {
  removeWorld(ownPort);
}
process.exit(0);
