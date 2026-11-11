// What the xenonet service costs the machine: start-up time, resident
// memory and idle CPU of the release binary, the latency of `xenonet hook`
// and the MCP server's start, on the demo world and on a synthetic pile of
// many large transcripts (made up, in a temp home; nothing real is read).
//
//   npm run rust:bin -- --release
//   node scripts/bench-service.mjs [--pile] [--home dir] [--demo] [--files 600] [--mb 3] [--idle 30] [--json]
//
// Prints one JSON object per scenario; save them (> file.jsonl) to compare
// runs. Run before/after back to back: the machine's other load moves the
// numbers more than most changes do.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { apiCredential } from "../electron/lib/components/runtime.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const flag = (n, d) => (argv.includes(`--${n}`) ? Number(argv[argv.indexOf(`--${n}`) + 1]) : d);
const bin = path.join(root, "target", "release", "xenonet");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const r1 = (v) => Math.round(v * 10) / 10;
const freePort = () => new Promise((res) => { const s = net.createServer().listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => res(port)); }); });

// ps reports cumulative CPU as [dd-]hh:mm:ss.cc; resident KB.
function cpuSeconds(pid) {
  const t = execFileSync("ps", ["-o", "time=", "-p", String(pid)]).toString().trim();
  return t.split(":").reduce((a, v) => a * 60 + Number(v), 0);
}
const rssMB = (pid) => Number(execFileSync("ps", ["-o", "rss=", "-p", String(pid)]).toString()) / 1024;
const childPids = (pid) => { try { return execFileSync("pgrep", ["-P", String(pid)]).toString().split("\n").filter(Boolean).map(Number); } catch { return []; } };

const iso = (t) => new Date(t).toISOString();
function claudeLines(n, t0, tag) {
  let s = "";
  for (let i = 0; i < n; i++) {
    const ts = iso(t0 + i * 1000);
    if (i % 4 === 0) s += JSON.stringify({ type: "user", timestamp: ts, cwd: "/bench/Projects/x", sessionId: tag, message: { role: "user", content: `Please look at the parser and fix case ${i} in the tokenizer, then run the tests again.` } }) + "\n";
    else if (i % 4 === 1) s += JSON.stringify({ type: "assistant", timestamp: ts, message: { id: `msg_${tag}_${i}`, model: "claude-sonnet-4-5", stop_reason: "tool_use", usage: { input_tokens: 12, cache_read_input_tokens: 40000 + i, cache_creation_input_tokens: 300, output_tokens: 150 }, content: [{ type: "text", text: "Let me read the tokenizer first and then edit the failing branch." }, { type: "tool_use", id: `tu_${tag}_${i}`, name: i % 8 === 1 ? "Edit" : "Bash", input: i % 8 === 1 ? { file_path: `/bench/Projects/x/src/file-${i % 20}.js`, old_string: "a".repeat(200), new_string: "b".repeat(220) } : { command: `npm test -- case-${i}`, description: "Run tests" } }] } }) + "\n";
    else if (i % 4 === 2) s += JSON.stringify({ type: "user", timestamp: ts, message: { role: "user", content: [{ type: "tool_result", tool_use_id: `tu_${tag}_${i - 1}`, content: "ok ".repeat(300), is_error: false }] } }) + "\n";
    else s += JSON.stringify({ type: "assistant", timestamp: ts, message: { id: `msg_${tag}_${i}`, model: "claude-sonnet-4-5", stop_reason: "end_turn", usage: { input_tokens: 5, cache_read_input_tokens: 41000 + i, cache_creation_input_tokens: 100, output_tokens: 80 }, content: [{ type: "text", text: "The tests pass now. The tokenizer handled escaped quotes wrong; it reads them as one token again." }] } }) + "\n";
  }
  return s;
}

// A home with `files` Claude transcripts of about `mb` MB in 60 project
// folders, a tenth of them written in the last day.
function writePile(home, files, mb) {
  const base = claudeLines(1000, Date.now() - 86400e3, "seed");
  const per = Math.max(1, Math.round((mb * 1e6) / base.length));
  const body = base.repeat(per);
  const now = Date.now();
  let bytes = 0;
  for (let i = 0; i < files; i++) {
    const dir = path.join(home, ".claude", "projects", `-bench-Projects-p${i % 60}`);
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, `${String(i).padStart(8, "0")}-0000-4000-8000-000000000000.jsonl`);
    fs.writeFileSync(file, body);
    bytes += body.length;
    const t = new Date(i % 10 === 0 ? now - 3600e3 : now - (2 + (i % 40)) * 86400e3);
    fs.utimesSync(file, t, t);
  }
  return bytes;
}

async function until(fn, timeout = 60000) {
  const t0 = performance.now();
  while (performance.now() - t0 < timeout) {
    try { if (await fn()) return performance.now() - t0; } catch {}
    await sleep(20);
  }
  throw new Error("timed out");
}

async function scenario(name, { demo, home, bytes, keep }) {
  const port = await freePort();
  const tmp = home ?? fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-bench-svc-"));
  const env = { ...process.env, XENONET_PORT: String(port), XENONET_RUNTIME_DIR: path.join(tmp, "runtime"), XENONET_CONTROL: "", XENONET_HOT: "0" };
  // The API's credential the service writes at start, read again for each
  // request while it may not be there yet.
  const keyed = () => ({ headers: { "x-xenonet-credential": apiCredential(env, port) ?? "" } });
  if (demo) env.XENONET_DEMO = "1";
  else Object.assign(env, { HOME: tmp, XENONET_STATE_DIR: path.join(tmp, "state"), XENONET_CACHE_DIR: path.join(tmp, "cache") });
  if (!demo && !argv.includes("--keep-state")) for (const d of ["state", "cache"]) fs.rmSync(path.join(tmp, d), { recursive: true, force: true });
  const t0 = performance.now();
  const svc = spawn(bin, ["serve"], { env, stdio: "ignore" });
  try {
  const up = await until(async () => (await fetch(`http://127.0.0.1:${port}/api/graph`, keyed())).ok).then((t) => t + (performance.now() - t0 - t));
  const result = { scenario: name, pileMB: bytes ? Math.round(bytes / 1e6) : 0, startMs: Math.round(up) };
  // Settle: let the first scan, index and tail finish before sampling.
  // Peak resident memory and the CPU spent while it does, and how long the
  // search index says its first pass took.
  let peak = 0;
  const settleUntil = performance.now() + flag("settle", 8) * 1000;
  while (performance.now() < settleUntil) {
    peak = Math.max(peak, rssMB(svc.pid));
    await sleep(100);
  }
  result.peakRssMB = r1(peak);
  result.startCpuS = r1(cpuSeconds(svc.pid));
  try {
    const sessions = await (await fetch(`http://127.0.0.1:${port}/api/sessions`, keyed())).json();
    result.indexMs = Math.round(sessions.search?.ms ?? 0);
    result.indexDocs = sessions.search?.docs ?? 0;
  } catch {}
  result.rssMB = r1(rssMB(svc.pid));
  const idle = flag("idle", 30);
  const c0 = cpuSeconds(svc.pid);
  await sleep(idle * 1000);
  result.idleCpuPct = r1(((cpuSeconds(svc.pid) - c0) / idle) * 100);
  result.rssAfterIdleMB = r1(rssMB(svc.pid));

  // Busy sessions: lines appended to 8 of the recent transcripts, 40 a
  // second in all, for `live` seconds (--live 0 skips it).
  const live = flag("live", 20);
  if (live > 0 && !demo) {
    const dirs = [];
    for (let i = 0; dirs.length < 8; i += 10) {
      const dir = path.join(tmp, ".claude", "projects", `-bench-Projects-p${i % 60}`);
      dirs.push(path.join(dir, `${String(i).padStart(8, "0")}-0000-4000-8000-000000000000.jsonl`));
    }
    const l0 = cpuSeconds(svc.pid);
    const end = performance.now() + live * 1000;
    let n = 0;
    while (performance.now() < end) {
      for (const f of dirs) fs.appendFileSync(f, claudeLines(5, Date.now(), `live${n}`));
      n++;
      await sleep(1000);
    }
    result.liveCpuPct = r1(((cpuSeconds(svc.pid) - l0) / live) * 100);
    result.liveRssMB = r1(rssMB(svc.pid));
  }

  // A hook call, the way an agent's hook runs it: a process per event.
  const hookEnv = { ...env };
  const hook = JSON.stringify({ hook_event_name: "UserPromptSubmit", session_id: "bench", cwd: "/tmp", prompt: "hi" });
  const times = [];
  for (let i = 0; i < 25; i++) {
    const t = performance.now();
    spawnSync(bin, ["hook", "claude"], { env: hookEnv, input: hook, stdio: ["pipe", "ignore", "ignore"] });
    times.push(performance.now() - t);
  }
  result.hookMs = r1(median(times));

  // The MCP server's start: spawn, send initialize, wait for the answer.
  const mcp = [];
  for (let i = 0; i < 10; i++) {
    const t = performance.now();
    const p = spawn(bin, ["mcp"], { env: hookEnv, stdio: ["pipe", "pipe", "ignore"] });
    p.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "bench", version: "0" } } }) + "\n");
    await new Promise((res) => p.stdout.once("data", res));
    mcp.push(performance.now() - t);
    p.kill();
  }
  result.mcpInitMs = r1(median(mcp));
  console.log(JSON.stringify(result));
  if (argv.includes("--hold")) {
    console.error(`holding: pid ${svc.pid} port ${port} home ${tmp}; Ctrl-C to stop`);
    await new Promise((r) => process.on("SIGINT", r));
  }
  } finally {
  svc.kill();
  }
  await sleep(300);
  if (!keep) fs.rmSync(tmp, { recursive: true, force: true });
}

if (!fs.existsSync(bin)) { console.error("build first: npm run rust:bin -- --release"); process.exit(1); }
const any = argv.includes("--demo") || argv.includes("--pile");
if (!any || argv.includes("--demo")) await scenario("demo", { demo: true, home: fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-bench-demo-")) });
if (!any || argv.includes("--pile")) {
  // --home reuses a pile (made there when the folder is new) and keeps it.
  const kept = argv.includes("--home") ? argv[argv.indexOf("--home") + 1] : null;
  const home = kept ?? fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-bench-pile-"));
  const made = kept && fs.existsSync(path.join(home, ".claude"));
  const bytes = made ? 0 : writePile(home, flag("files", 600), flag("mb", 3));
  await scenario("pile", { home, bytes, keep: !!kept });
}
