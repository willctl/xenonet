// A made-up history of Claude Code and Codex sessions, for measuring the
// service at scale: transcripts in the apps' own formats, in a home folder
// of their own, with long sessions, subagents and a few files that keep
// growing. Nothing in it is anyone's real work.
//
//   node scripts/bench-history.mjs make --home <dir> [--sessions 200] [--mb 150] [--growing 6] [--seed 1]
//   node scripts/bench-history.mjs grow --home <dir> [--every 1000] [--seconds 0]
//
// make fills <dir> with ~/.claude/projects, ~/.codex/sessions and a
// Projects folder of small git repos the sessions ran in, and writes
// <dir>/bench-history.json, which says what's there. Older sessions' files
// keep the time they were last written, so the tailer reads only the last
// day's as history, as it would on a real machine.
//
// grow appends a turn to each growing file every --every ms, until it's
// stopped (or for --seconds), as sessions at work would.
//
// Then run a service on it, on a port of its own (never 47840-47842):
//   HOME=<dir> XENONET_PORT=47991 XENONET_STATE_DIR=<dir>/state XENONET_CACHE_DIR=<dir>/cache target/release/xenonet serve
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const DAY = 24 * 3600e3;
const MANIFEST = "bench-history.json";

function args(argv) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith("--")) out[argv[i].slice(2)] = argv[i + 1]?.startsWith("--") || argv[i + 1] === undefined ? true : argv[++i];
    else out._.push(argv[i]);
  }
  return out;
}

// ---------- made-up words ----------

// mulberry32: the same seed makes the same history.
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const COMMON = (
  "the a to of and in is it that for on with as this be at by from or an not are we can if will should when then but so do it's let's " +
  "add fix test run build check update remove rename move write read open make use try keep show find call pass return handle parse " +
  "file files function module test tests error errors build config server client request response route handler query index cache " +
  "store state session sessions user users token tokens order orders refund refunds invoice payment payments api schema migration " +
  "table column row value field key map list array string number type struct trait method class component page view layout style " +
  "branch commit merge diff patch review change changes release version deploy log logs metric metrics timer retry timeout lock"
).split(" ");

function vocabulary(r, n) {
  const sy = ["ka", "lo", "mi", "ra", "ten", "vo", "zu", "pe", "an", "sil", "dor", "ex", "qua", "nim", "bel", "tor", "ga", "fen", "ri", "os", "ul", "ce", "ny", "har"];
  const out = new Set();
  while (out.size < n) {
    let w = "";
    const k = 2 + Math.floor(r() * 3);
    for (let i = 0; i < k; i++) w += sy[Math.floor(r() * sy.length)];
    out.add(w);
  }
  return [...out];
}

class Words {
  constructor(r) {
    this.r = r;
    this.rare = vocabulary(r, 6000);
    // Chunks of prose and code, reused: a line is a few of them after a
    // few words of its own, so no two texts begin alike.
    this.prose = Array.from({ length: 600 }, () => this.sentence(20 + Math.floor(r() * 120)));
    this.code = Array.from({ length: 400 }, () => this.codeBlock(8 + Math.floor(r() * 60)));
  }
  pick(list) {
    return list[Math.floor(this.r() * list.length)];
  }
  word() {
    return this.r() < 0.7 ? this.pick(COMMON) : this.pick(this.rare);
  }
  sentence(n) {
    const w = [];
    for (let i = 0; i < n; i++) w.push(this.word());
    return w.join(" ") + ".";
  }
  codeBlock(lines) {
    const out = [];
    for (let i = 0; i < lines; i++) {
      const ind = "  ".repeat(Math.floor(this.r() * 4));
      const a = this.pick(this.rare);
      const b = this.pick(COMMON);
      out.push(this.r() < 0.5 ? `${ind}const ${a}_${b} = ${this.pick(this.rare)}(${b}, ${Math.floor(this.r() * 1000)});` : `${ind}// ${this.sentence(4 + Math.floor(this.r() * 8))}`);
    }
    return out.join("\n");
  }
  // About `bytes` of prose, starting with a few words of its own.
  text(bytes) {
    let s = this.sentence(4 + Math.floor(this.r() * 6));
    while (s.length < bytes) s += " " + this.pick(this.prose);
    return s.slice(0, Math.max(bytes, 12));
  }
  codeText(bytes) {
    let s = `// ${this.sentence(5)}\n`;
    while (s.length < bytes) s += this.pick(this.code) + "\n";
    return s.slice(0, Math.max(bytes, 12));
  }
}

function uuid(r) {
  const h = () => Math.floor(r() * 16).toString(16);
  let s = "";
  for (let i = 0; i < 32; i++) s += h();
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-4${s.slice(13, 16)}-a${s.slice(17, 20)}-${s.slice(20, 32)}`;
}

const hex = (r, n) => Array.from({ length: n }, () => Math.floor(r() * 16).toString(16)).join("");
const iso = (ms) => new Date(ms).toISOString();

// ---------- writing a file a line at a time ----------

class Out {
  constructor(file) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    this.file = file;
    this.fd = fs.openSync(file, "a");
    this.buf = [];
    this.pending = 0;
    this.bytes = fs.fstatSync(this.fd).size;
  }
  line(obj) {
    const s = JSON.stringify(obj) + "\n";
    this.buf.push(s);
    this.pending += s.length;
    this.bytes += Buffer.byteLength(s);
    if (this.pending > 1 << 20) this.flush();
  }
  flush() {
    if (this.buf.length) fs.writeSync(this.fd, this.buf.join(""));
    this.buf = [];
    this.pending = 0;
  }
  close(mtime) {
    this.flush();
    fs.closeSync(this.fd);
    if (mtime) fs.utimesSync(this.file, new Date(mtime), new Date(mtime));
  }
}

// ---------- Claude Code ----------

const CLAUDE_VERSION = "2.1.30";
const CLAUDE_MODEL = "claude-sonnet-4-5-20250929";
const TOOLS = ["Read", "Bash", "Edit", "Grep", "Write", "Glob"];

// A Claude Code session, its lines written as the CLI writes them: each
// entry chained to the one before by parentUuid.
class ClaudeSession {
  constructor(w, s, file, { sidechain = false, agentId = null } = {}) {
    this.w = w;
    this.s = s;
    this.out = new Out(file);
    this.parent = s.parent ?? null;
    this.sidechain = sidechain;
    this.agentId = agentId;
  }
  base(type, ts) {
    const u = uuid(this.w.r);
    const e = { parentUuid: this.parent, isSidechain: this.sidechain, userType: "external", cwd: this.s.cwd, sessionId: this.s.id, version: CLAUDE_VERSION, gitBranch: this.s.branch };
    if (this.agentId) e.agentId = this.agentId;
    Object.assign(e, { type, uuid: u, timestamp: iso(ts) });
    this.parent = u;
    return e;
  }
  usage() {
    const r = this.w.r;
    return { input_tokens: 4 + Math.floor(r() * 40), cache_creation_input_tokens: Math.floor(r() * 3000), cache_read_input_tokens: 20000 + Math.floor(r() * 120000), output_tokens: 50 + Math.floor(r() * 1500), service_tier: "standard" };
  }
  assistant(content, ts, stop) {
    const e = this.base("assistant", ts);
    e.message = { id: `msg_${hex(this.w.r, 24)}`, type: "message", role: "assistant", model: CLAUDE_MODEL, content, stop_reason: stop, stop_sequence: null, usage: this.usage() };
    e.requestId = `req_${hex(this.w.r, 24)}`;
    this.out.line(e);
  }
  prompt(ts) {
    const e = this.base("user", ts);
    e.message = { role: "user", content: this.w.text(60 + Math.floor(this.w.r() * 400)) };
    this.out.line(e);
  }
  // A turn: a prompt, some thinking, tool calls and their results, a reply.
  // `big`: how large a tool result runs.
  turn(ts, big) {
    const r = this.w.r;
    if (!this.sidechain || this.out.bytes === 0) this.prompt(ts);
    ts += 2000;
    const calls = 1 + Math.floor(r() * 6);
    for (let i = 0; i < calls; i++) {
      if (r() < 0.4) this.assistant([{ type: "thinking", thinking: this.w.text(200 + Math.floor(r() * 1500)), signature: hex(r, 120) }], ts, null);
      const id = `toolu_${hex(r, 24)}`;
      const tool = this.w.pick(TOOLS);
      const file = `${this.s.cwd}/src/${this.w.pick(this.w.rare)}.ts`;
      const input = tool === "Bash" ? { command: `npm test -- ${this.w.pick(this.w.rare)}`, description: this.w.sentence(5) } : tool === "Edit" ? { file_path: file, old_string: this.w.codeText(200), new_string: this.w.codeText(240) } : tool === "Write" ? { file_path: file, content: this.w.codeText(1500) } : { file_path: file };
      this.assistant([{ type: "tool_use", id, name: tool, input }], ts, "tool_use");
      ts += 1500 + Math.floor(r() * 8000);
      const e = this.base("user", ts);
      const size = Math.floor(400 + r() * big);
      e.message = { role: "user", content: [{ tool_use_id: id, type: "tool_result", content: tool === "Bash" ? this.w.text(size) : this.w.codeText(size), is_error: r() < 0.05 }] };
      e.toolUseResult = tool === "Read" ? { type: "text", file: { filePath: file, numLines: 120, startLine: 1, totalLines: 120 } } : { stdout: "ok", stderr: "", interrupted: false };
      this.out.line(e);
      ts += 500;
    }
    this.assistant([{ type: "text", text: this.w.text(150 + Math.floor(r() * 1200)) }], ts, "end_turn");
    return ts + 1000;
  }
}

// ---------- Codex ----------

const CODEX_VERSION = "0.150.0";

class CodexSession {
  constructor(w, s, file) {
    this.w = w;
    this.s = s;
    this.out = new Out(file);
    this.total = 0;
  }
  ev(ts, payload) {
    this.out.line({ timestamp: iso(ts), type: "event_msg", payload });
  }
  item(ts, payload) {
    this.out.line({ timestamp: iso(ts), type: "response_item", payload });
  }
  meta(ts) {
    const s = this.s;
    this.out.line({
      timestamp: iso(ts),
      type: "session_meta",
      payload: { id: s.id, timestamp: iso(ts), cwd: s.cwd, originator: "codex_cli_rs", cli_version: CODEX_VERSION, instructions: null, source: "cli", model_provider: "openai", git: { commit_hash: hex(this.w.r, 40), branch: s.branch } },
    });
    this.item(ts, { type: "message", role: "user", content: [{ type: "input_text", text: `<environment_context>\n  <cwd>${s.cwd}</cwd>\n  <approval_policy>on-request</approval_policy>\n  <sandbox_mode>workspace-write</sandbox_mode>\n  <shell>zsh</shell>\n</environment_context>` }] });
  }
  tokens(ts) {
    const r = this.w.r;
    const last = { input_tokens: 20000 + Math.floor(r() * 90000), cached_input_tokens: 18000 + Math.floor(r() * 80000), output_tokens: 100 + Math.floor(r() * 2000), reasoning_output_tokens: Math.floor(r() * 800), total_tokens: 0 };
    last.total_tokens = last.input_tokens + last.output_tokens;
    this.total += last.total_tokens;
    this.ev(ts, {
      type: "token_count",
      info: { total_token_usage: { input_tokens: this.total, cached_input_tokens: Math.floor(this.total * 0.8), output_tokens: Math.floor(this.total / 40), reasoning_output_tokens: 0, total_tokens: this.total }, last_token_usage: last, model_context_window: 258400 },
      rate_limits: { primary: { used_percent: 20 + Math.floor(r() * 50), window_minutes: 300, resets_in_seconds: 9000 }, secondary: { used_percent: 30, window_minutes: 10080, resets_in_seconds: 300000 } },
    });
  }
  turn(ts, big) {
    const r = this.w.r;
    const s = this.s;
    this.out.line({ timestamp: iso(ts), type: "turn_context", payload: { cwd: s.cwd, approval_policy: "on-request", sandbox_policy: { mode: "workspace-write" }, model: "gpt-5-codex", effort: "medium", summary: "auto" } });
    this.ev(ts, { type: "task_started", model_context_window: 258400 });
    const prompt = this.w.text(60 + Math.floor(r() * 400));
    this.item(ts, { type: "message", role: "user", content: [{ type: "input_text", text: prompt }] });
    this.ev(ts, { type: "user_message", message: prompt, images: [] });
    ts += 2000;
    const calls = 1 + Math.floor(r() * 6);
    for (let i = 0; i < calls; i++) {
      const thought = this.w.text(100 + Math.floor(r() * 600));
      this.item(ts, { type: "reasoning", summary: [{ type: "summary_text", text: thought }], content: null, encrypted_content: `gAAAAA${hex(r, 400)}` });
      this.ev(ts, { type: "agent_reasoning", text: thought });
      const id = `call_${hex(r, 24)}`;
      const cmd = ["bash", "-lc", `rg -n ${this.w.pick(this.w.rare)} src`];
      this.item(ts, { type: "function_call", name: "shell", arguments: JSON.stringify({ command: cmd, workdir: s.cwd, timeout_ms: 120000 }), call_id: id });
      ts += 1000 + Math.floor(r() * 6000);
      const output = this.w.codeText(Math.floor(400 + r() * big));
      this.item(ts, { type: "function_call_output", call_id: id, output: JSON.stringify({ output, metadata: { exit_code: 0, duration_seconds: 0.4 } }) });
      this.tokens(ts);
    }
    const reply = this.w.text(150 + Math.floor(r() * 1200));
    this.item(ts, { type: "message", role: "assistant", content: [{ type: "output_text", text: reply }] });
    this.ev(ts, { type: "agent_message", message: reply });
    this.tokens(ts);
    this.ev(ts, { type: "task_complete", last_agent_message: reply });
    return ts + 1000;
  }
}

// ---------- the history ----------

function projectsOf(home, r, w, n) {
  const names = new Set();
  while (names.size < n) names.add(`${w.pick(w.rare)}-${w.pick(["api", "web", "cli", "core", "app", "lib", "tools", "site"])}`);
  const dirs = [...names].map((name) => path.join(home, "Projects", name));
  for (const dir of dirs) {
    fs.mkdirSync(path.join(dir, "src"), { recursive: true });
    fs.writeFileSync(path.join(dir, "README.md"), `# ${path.basename(dir)}\n\n${w.sentence(20)}\n`);
    fs.writeFileSync(path.join(dir, "src", "index.ts"), w.codeText(800));
    try {
      execFileSync("git", ["init", "-q", "-b", "main"], { cwd: dir, stdio: "ignore" });
    } catch {
      // A folder without git is a project too.
    }
  }
  return dirs;
}

// Sizes with a long tail, adding up to about `total`: most sessions are
// small, and the longest Claude Code and Codex sessions run to hundreds of
// megabytes once the history is in gigabytes. Session i is Claude Code's
// when i % 5 < 3.
function sizes(r, n, total) {
  const raw = Array.from({ length: n }, () => Math.exp(1.7 * Math.sqrt(-2 * Math.log(r() || 1e-9)) * Math.cos(2 * Math.PI * r())));
  const long = [
    [raw.reduce((best, x, i) => (i % 5 < 3 && x > raw[best] ? i : best), 0), Math.min(300 << 20, total * 0.12)],
    [raw.reduce((best, x, i) => (i % 5 >= 3 && x > raw[best] ? i : best), Math.min(3, n - 1)), Math.min(150 << 20, total * 0.05)],
  ];
  const rest = total - long.reduce((k, [, size]) => k + size, 0);
  const sum = raw.reduce((a, b, i) => (long.some(([j]) => j === i) ? a : a + b), 0);
  const out = raw.map((x) => Math.max(6000, Math.round((x / sum) * rest)));
  for (const [i, size] of long) out[i] = Math.round(size);
  return { sizes: out, longest: long[0][0] };
}

// Claude Code names a project's folder after its path.
const claudeFolder = (cwd) => cwd.replace(/[^a-zA-Z0-9]/g, "-");

function make(o) {
  const home = path.resolve(o.home);
  const n = Number(o.sessions ?? 200);
  const total = Number(o.mb ?? 150) * 1024 * 1024;
  const growing = Number(o.growing ?? 6);
  const r = rng(Number(o.seed ?? 1));
  const w = new Words(r);
  if (fs.existsSync(path.join(home, MANIFEST))) throw new Error(`${home} already has a history; make it in an empty folder`);
  fs.mkdirSync(home, { recursive: true });
  const now = Date.now();
  const projects = projectsOf(home, r, w, Math.max(12, Math.round(n / 25)));
  const claudeRoot = path.join(home, ".claude", "projects");
  const codexRoot = path.join(home, ".codex", "sessions");
  // The longest Claude Code session is one of the growing ones, so a long
  // file grows.
  const { sizes: sz, longest: biggest } = sizes(r, n, total);
  const list = [];
  let bytes = 0;
  for (let i = 0; i < n; i++) {
    const grows = i === biggest || (i >= n - growing + 1 && growing > 1);
    const app = i % 5 < 3 ? "claude" : "codex";
    // When it was last written: the growing ones now, a few in the last
    // hours, the rest over the last two months.
    const ago = grows ? 0 : r() < 0.04 ? r() * 0.9 * DAY : DAY + r() * 60 * DAY;
    const end = now - ago;
    const s = { id: uuid(r), app, cwd: w.pick(projects), branch: w.pick(["main", "main", "fix/" + w.pick(w.rare), "feat/" + w.pick(w.rare)]) };
    const budget = sz[i];
    // A turn's tool results run larger in a long session.
    const big = Math.min(60000, 2000 + budget / 200);
    // Timestamps are written forwards from a start that the session's
    // length puts far enough back to end near `end`.
    const turnsGuess = Math.max(1, Math.round(budget / (big * 2.2 + 6000)));
    let ts = end - turnsGuess * 45e3;
    let file;
    const subs = [];
    if (app === "claude") {
      file = path.join(claudeRoot, claudeFolder(s.cwd), `${s.id}.jsonl`);
      const c = new ClaudeSession(w, s, file);
      // A quarter of the sessions ran subagents, which have their own files.
      const subShare = r() < 0.25 ? 0.2 : 0;
      while (c.out.bytes < budget * (1 - subShare)) ts = c.turn(ts, big);
      if (subShare) {
        const k = 1 + Math.floor(r() * 4);
        for (let j = 0; j < k; j++) {
          const agentId = hex(r, 16);
          const subFile = path.join(path.dirname(file), s.id, "subagents", `agent-${agentId}.jsonl`);
          const sub = new ClaudeSession(w, { ...s, parent: null }, subFile, { sidechain: true, agentId });
          let st = ts - 30 * 60e3;
          while (sub.out.bytes < (budget * subShare) / k) st = sub.turn(st, big);
          sub.out.close(Math.min(st, end));
          bytes += sub.out.bytes;
          subs.push(subFile);
        }
      }
      c.out.close(grows ? null : Math.min(ts, end));
      bytes += c.out.bytes;
      list.push({ ...s, file, subagents: subs, grows, parent: c.parent });
    } else {
      const start = new Date(ts);
      const day = path.join(codexRoot, String(start.getFullYear()), String(start.getMonth() + 1).padStart(2, "0"), String(start.getDate()).padStart(2, "0"));
      // Codex names a rollout by its local start time, as its folders are.
      const p2 = (x) => String(x).padStart(2, "0");
      const local = `${start.getFullYear()}-${p2(start.getMonth() + 1)}-${p2(start.getDate())}T${p2(start.getHours())}-${p2(start.getMinutes())}-${p2(start.getSeconds())}`;
      file = path.join(day, `rollout-${local}-${s.id}.jsonl`);
      const c = new CodexSession(w, s, file);
      c.meta(ts);
      while (c.out.bytes < budget) ts = c.turn(ts, big);
      c.out.close(grows ? null : Math.min(ts, end));
      bytes += c.out.bytes;
      list.push({ ...s, file, subagents: [], grows });
    }
  }
  const manifest = {
    made: iso(now),
    seed: Number(o.seed ?? 1),
    sessions: list.length,
    claude: list.filter((s) => s.app === "claude").length,
    codex: list.filter((s) => s.app === "codex").length,
    subagentFiles: list.reduce((k, s) => k + s.subagents.length, 0),
    bytes,
    projects: projects.length,
    growing: list.filter((s) => s.grows).map(({ id, app, cwd, branch, file, parent }) => ({ id, app, cwd, branch, file, parent })),
  };
  fs.writeFileSync(path.join(home, MANIFEST), JSON.stringify(manifest, null, 2) + "\n");
  return manifest;
}

// ---------- growing ----------

async function grow(o) {
  const home = path.resolve(o.home);
  const m = JSON.parse(fs.readFileSync(path.join(home, MANIFEST), "utf8"));
  const every = Number(o.every ?? 1000);
  const until = o.seconds ? Date.now() + Number(o.seconds) * 1000 : Infinity;
  const r = rng(Date.now() & 0xffffffff);
  const w = new Words(r);
  const sessions = m.growing.map((s) => {
    if (s.app === "claude") {
      const c = new ClaudeSession(w, s, s.file);
      c.parent = s.parent;
      return c;
    }
    return new CodexSession(w, s, s.file);
  });
  let appended = 0;
  while (Date.now() < until) {
    const t0 = Date.now();
    for (const c of sessions) {
      const before = c.out.bytes;
      c.turn(Date.now(), 3000);
      c.out.flush();
      appended += c.out.bytes - before;
    }
    await new Promise((res) => setTimeout(res, Math.max(0, every - (Date.now() - t0))));
  }
  for (const c of sessions) c.out.close();
  // The next grow chains on to where this one left each Claude Code session.
  m.growing.forEach((s, i) => {
    if (s.app === "claude") s.parent = sessions[i].parent;
  });
  fs.writeFileSync(path.join(home, MANIFEST), JSON.stringify(m, null, 2) + "\n");
  return { appended };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const o = args(process.argv.slice(2));
  const cmd = o._[0];
  if (!o.home || !["make", "grow"].includes(cmd)) {
    console.error("usage: node scripts/bench-history.mjs make|grow --home <dir> [options] (see the top of the file)");
    process.exit(2);
  }
  if (cmd === "make") {
    const t0 = Date.now();
    const m = make(o);
    console.log(JSON.stringify({ sessions: m.sessions, claude: m.claude, codex: m.codex, subagentFiles: m.subagentFiles, mb: Math.round(m.bytes / 1048576), projects: m.projects, growing: m.growing.length, seconds: Math.round((Date.now() - t0) / 100) / 10 }));
  } else {
    console.log(JSON.stringify(await grow(o)));
  }
}
