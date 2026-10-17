#!/usr/bin/env node
// OpenCode sessions messaging each other by name through XenoNet, for real:
// a XenoNet from this checkout, and two or three OpenCode TUIs in tmux, each
// in a project of its own with a session title of its own. One session asks
// another by words from its title, another asks by selector (app:opencode,
// project:… app:opencode), and each answer comes back the same way. XenoNet
// puts every message into the recipient's TUI through OpenCode's own server
// (prompt_async), so the recipient starts a turn while it sits idle.
//
//   node scripts/demo/opencode-named.mjs [--sessions 3] [--rounds 2]
//        [--model opencode/big-pickle] [--opencode <path>] [--port <n>]
//        [--out <dir>] [--keep] [--seed-consent] [--timeout <seconds>]
//
// Everything runs apart from your own setup: the XenoNet has its own port,
// state and an empty home, so it reads none of your sessions; OpenCode gets
// an empty home and config, so it uses OpenCode's free models with no
// account and no key, and none of your settings, sign-ins or sessions; tmux
// runs a server of its own, with no config. The projects, and everything
// else, live in one folder under the system's temp folder, which goes at
// the end with the tmux server and the XenoNet (--keep leaves them running).
//
// Consent is yours: the script runs xenonet live allow for each session, and
// XenoNet takes that only from you, not from under an agent. Run under an
// agent, it says so and waits for you to run the commands it prints in your
// own terminal. --seed-consent instead writes the same allowance into the
// demo XenoNet's own state before it starts again; the demo's XenoNet is
// always a new one in the demo's folder, so this reaches nothing else.
//
// At the end it prints, for each message, how long from the sender's tool
// call to XenoNet taking it, to OpenCode's server taking it, to the
// recipient's turn starting, and to the answer reaching XenoNet.
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const TOOL = path.join(ROOT, "scripts/demo/opencode-tool.js");

// The projects: a name, the session's title, and what its notes say, which
// the other sessions ask about.
export const PROJECTS = [
  { name: "atlas", title: "Atlas: billing API", codename: "Juniper", version: "2.4.1" },
  { name: "borealis", title: "Borealis: docs site", codename: "Larch", version: "0.9.0" },
  { name: "cirrus", title: "Cirrus: mobile app", codename: "Sorrel", version: "5.1.3" },
];

// What each round asks, as you would type it into a session's TUI: one
// question by title words, one by selector, and with three sessions one to
// a project by selector. `expect` is how many messages that makes, the
// question and its answers.
export function plan(count, round) {
  const ask = (from, to, text, expect) => ({ from, to, text: `${text} (round ${round})`, expect });
  if (count === 2)
    return [
      ask("atlas", "docs site", "What's your release codename?", 2),
      ask("borealis", "project:atlas app:opencode", "Which version are you on?", 2),
    ];
  return [
    ask("atlas", "docs site", "What's your release codename?", 2),
    ask("cirrus", "app:opencode", "Which version are you on?", 3),
    ask("borealis", "project:cirrus app:opencode", "What's your release codename?", 2),
  ];
}

export const kickoff = (step) =>
  `Send one message through XenoNet: call xenonet_send with to "${step.to}" and text "${step.text}". Then stop. The answer comes to you as a message of its own.`;

export function agentsMd(p) {
  return `# ${p.title}

You are the OpenCode session working on ${p.name}. Other sessions on this machine work on other projects, and XenoNet carries messages between you.

- A message from another session arrives as "[XenoNet from …] (message #N; …)". When it asks you something, answer it once with xenonet_reply, using N, from what NOTES.md says, and end the answer with the round the question names, such as "(round 2)". Never answer a reply to a question you asked.
- When the answer to a question you sent arrives, say in one line what it says, and stop.
- When your user asks you to message another session, do exactly that with xenonet_send, then stop.
- Keep every message to one sentence. Don't edit files or run commands.
`;
}

export const notesMd = (p) => `# ${p.name}\n\nRelease codename: ${p.codename}\nCurrent version: ${p.version}\n`;

// OpenCode's project config: the model, nothing but reading and XenoNet's
// tools, and no sharing or updates.
export function openCodeConfig(model) {
  const off = ["bash", "edit", "write", "patch", "apply_patch", "task", "webfetch", "websearch", "todowrite", "question", "skill"];
  return {
    $schema: "https://opencode.ai/config.json",
    model,
    small_model: model,
    share: "disabled",
    autoupdate: false,
    tools: Object.fromEntries(off.map((t) => [t, false])),
    permission: { edit: "deny", bash: "deny", task: "deny", webfetch: "deny", websearch: "deny", external_directory: "deny", question: "deny" },
  };
}

// The demo XenoNet's consent file, as `xenonet live allow` would leave it.
export function seededConsent(ids, { now = Date.now(), hours = 1, previous = null } = {}) {
  const data = { paused: false, grants: {}, consent: {}, ...(previous ?? {}) };
  for (const id of ids) data.consent[id] = { until: now + hours * 3600e3, remote: false, at: now };
  return data;
}

export function parseArgs(argv) {
  const o = { sessions: 3, rounds: 2, model: "opencode/big-pickle", opencode: null, port: null, out: null, keep: false, seedConsent: false, timeout: 240 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => {
      const v = argv[++i];
      if (v == null) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === "--sessions") o.sessions = Number(value());
    else if (a === "--rounds") o.rounds = Number(value());
    else if (a === "--model") o.model = value();
    else if (a === "--opencode") o.opencode = value();
    else if (a === "--port") o.port = Number(value());
    else if (a === "--out") o.out = path.resolve(value());
    else if (a === "--timeout") o.timeout = Number(value());
    else if (a === "--keep") o.keep = true;
    else if (a === "--seed-consent") o.seedConsent = true;
    else if (a === "--help" || a === "-h") o.help = true;
    else throw new Error(`unknown option ${a}; --help lists them`);
  }
  if (![2, 3].includes(o.sessions)) throw new Error("--sessions is 2 or 3");
  if (!(o.rounds >= 1 && o.rounds <= 4)) throw new Error("--rounds is 1 to 4: XenoNet's wake budget is 12 an hour per sender");
  if (!/^[\w.-]+\/[\w.:-]+$/.test(o.model)) throw new Error("--model is provider/model, such as opencode/big-pickle");
  return o;
}

// A word for a POSIX shell.
export const sh = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;

// ---------- timings ----------

const MESSAGE_IN = /\(message #(\d+);/;
const SENT = /Sent as message #(\d+)/;

// The XenoNet tool calls in one OpenCode session's messages:
// [{ tool, input, message (the id XenoNet gave it), start, end }].
export function toolCalls(messages) {
  const out = [];
  for (const m of messages ?? [])
    for (const p of m.parts ?? []) {
      if (p.type !== "tool" || !/^xenonet_(send|reply)$/.test(p.tool ?? "") || p.state?.status !== "completed") continue;
      const id = SENT.exec(p.state.output ?? "")?.[1];
      if (id) out.push({ tool: p.tool, input: p.state.input ?? {}, message: Number(id), start: p.state.time?.start ?? null, end: p.state.time?.end ?? null });
    }
  return out;
}

// The XenoNet messages that came into one OpenCode session, each with when
// OpenCode stored it and when the turn it started began and ended:
// Map(message id → { in, turn, done }).
export function arrivals(messages) {
  const out = new Map();
  const list = messages ?? [];
  for (const [i, m] of list.entries()) {
    if (m.info?.role !== "user") continue;
    const text = (m.parts ?? []).filter((p) => p.type === "text").map((p) => p.text ?? "").join("\n");
    const id = MESSAGE_IN.exec(text)?.[1];
    if (!id) continue;
    const answer = list.find((x) => x.info?.role === "assistant" && x.info.parentID === m.info.id) ?? list.slice(i + 1).find((x) => x.info?.role === "assistant");
    out.set(Number(id), { in: m.info.time?.created ?? null, turn: answer?.info?.time?.created ?? null, done: answer?.info?.time?.completed ?? null });
  }
  return out;
}

// One row per message and recipient. `messages`: XenoNet's (GET
// /api/engine/state), `deliveries`: its router's records (GET
// /api/delivery), `sessions`: { [xenonetId]: { title, messages } }, each
// session's own OpenCode messages.
export function timings({ messages, deliveries, sessions }) {
  const calls = new Map(); // XenoNet message id → { from, call }
  const replies = new Map(); // `${question id}|${who answered}` → the answer's id
  for (const [id, s] of Object.entries(sessions))
    for (const c of toolCalls(s.messages)) {
      calls.set(c.message, { from: id, call: c });
      const q = `${Number(c.input.message)}|${id}`;
      if (c.tool === "xenonet_reply" && Number.isInteger(Number(c.input.message)) && !replies.has(q)) replies.set(q, c.message);
    }
  const into = Object.fromEntries(Object.entries(sessions).map(([id, s]) => [id, arrivals(s.messages)]));
  const byId = new Map(messages.map((m) => [m.id, m]));
  const rows = [];
  for (const m of [...messages].sort((a, b) => a.id - b.id)) {
    const c = calls.get(m.id);
    if (!c || !sessions[m.from]) continue;
    const by = c.call.tool === "xenonet_reply" ? "reply" : /^[a-z]+:\S/.test(c.call.input.to ?? "") ? "selector" : "title words";
    for (const { id: to } of m.to) {
      if (!sessions[to]) continue;
      const d = deliveries.find((r) => r.message === m.id && r.agent === to) ?? null;
      const got = into[to].get(m.id) ?? null;
      const reply = byId.get(replies.get(`${m.id}|${to}`)) ?? null;
      const since = (t) => (t != null && c.call.start != null ? t - c.call.start : null);
      rows.push({
        message: m.id,
        from: sessions[m.from].title,
        to: sessions[to].title,
        by,
        asked: by === "reply" ? null : c.call.input.to,
        text: m.text,
        how: d?.how ?? null,
        state: d?.state ?? null,
        accepted: since(m.at),
        pushed: since(d?.ackedAt),
        stored: since(got?.in),
        turn: since(got?.turn),
        answered: by === "reply" ? null : since(reply?.at),
      });
    }
  }
  return rows;
}

export function stats(values) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return null;
  const mid = v.length >> 1;
  return { n: v.length, median: v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2, min: v[0], max: v[v.length - 1] };
}

const ms = (x) => (x == null ? "–" : x < 1000 ? `${Math.round(x)} ms` : `${(x / 1000).toFixed(1)} s`);

export function report(rows) {
  const lines = ["What they said:"];
  const said = new Map(rows.map((r) => [r.message, r]));
  for (const r of said.values()) lines.push(`  #${r.message} ${r.from}, ${r.by === "reply" ? "replying" : `to "${r.asked}"`}: ${String(r.text ?? "").replace(/\s+/g, " ").slice(0, 90)}`);
  lines.push("");
  const cols = ["#", "from", "to", "by", "XenoNet took it", "OpenCode took it", "turn started", "answer back"];
  const table = rows.map((r) => [`#${r.message}`, r.from, r.to, r.by === "reply" ? "reply" : `${r.by}: ${r.asked}`, ms(r.accepted), ms(r.pushed), ms(r.turn), ms(r.answered)]);
  const width = cols.map((c, i) => Math.max(c.length, ...table.map((t) => t[i].length)));
  const row = (cells) => cells.map((c, i) => c.padEnd(width[i])).join("  ").trimEnd();
  lines.push(row(cols), row(width.map((w) => "-".repeat(w))), ...table.map(row));
  lines.push("", "Times are from the sender's tool call. Median and range over the deliveries above:");
  for (const [key, label] of [["accepted", "XenoNet takes it"], ["pushed", "OpenCode's server takes it"], ["stored", "OpenCode has it in the session"], ["turn", "the recipient's turn starts"], ["answered", "the answer reaches XenoNet"]]) {
    const s = stats(rows.map((r) => r[key]));
    if (s) lines.push(`  ${label.padEnd(32)} ${ms(s.median).padStart(8)}   ${ms(s.min)} to ${ms(s.max)}   (${s.n})`);
  }
  const notLive = rows.filter((r) => r.how !== "opencode");
  if (notLive.length) lines.push("", `${notLive.length} of ${rows.length} didn't go in through OpenCode's server: ${notLive.map((r) => `#${r.message} ${r.state ?? "?"}`).join(", ")}`);
  return lines.join("\n");
}

// ---------- running it ----------

const sleep = (t) => new Promise((r) => setTimeout(r, t));

async function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.unref();
    s.on("error", reject);
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

async function http(base, route, { method = "GET", body, ms = 5000 } = {}) {
  const r = await fetch(`${base}${route}`, {
    method,
    headers: body === undefined ? {} : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(ms),
  });
  const text = await r.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!r.ok) throw Object.assign(new Error(`${method} ${route}: ${r.status} ${typeof data === "string" ? data.slice(0, 200) : (data?.error ?? "")}`), { status: r.status, data });
  return data;
}

async function until(what, fn, { ms = 60e3, every = 500 } = {}) {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (err) {
      last = err;
    }
    await sleep(every);
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ""}`);
}

function findOpenCode(given) {
  const tries = [given, ...String(process.env.PATH ?? "").split(path.delimiter).map((d) => path.join(d, "opencode")), path.join(os.homedir(), ".opencode/bin/opencode")].filter(Boolean);
  for (const t of tries) {
    try {
      fs.accessSync(t, fs.constants.X_OK);
      if (fs.statSync(t).isFile()) return t;
    } catch {}
  }
  return null;
}

// An environment without the caller's XenoNet, Claude Code, Codex and
// OpenCode settings, so nothing the demo starts talks to them.
function cleanEnv(extra) {
  const env = {};
  for (const [k, v] of Object.entries(process.env)) if (!/^(XENONET_|CLAUDE|CODEX_|OPENCODE_|ANTHROPIC_|OPENAI_)/.test(k)) env[k] = v;
  return { ...env, ...extra };
}

const say = (s) => process.stdout.write(`${s}\n`);

export async function main(argv = process.argv.slice(2)) {
  const o = parseArgs(argv);
  if (o.help) {
    say(fs.readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").slice(1, 31).map((l) => l.replace(/^\/\/ ?/, "")).join("\n"));
    return 0;
  }
  if (process.platform === "win32") throw new Error("the demo runs OpenCode in tmux, so it needs macOS or Linux (WSL works)");
  if (spawnSync("tmux", ["-V"]).status !== 0) throw new Error("tmux isn't on PATH: install it (brew install tmux, or your package manager)");
  const opencode = findOpenCode(o.opencode);
  if (!opencode) throw new Error("OpenCode isn't on PATH or in ~/.opencode/bin: install it from https://opencode.ai, or pass --opencode <path>");
  const ocVersion = String(spawnSync(opencode, ["--version"], { env: cleanEnv({}), encoding: "utf8" }).stdout).trim();

  // The real path, as OpenCode reports its folders (macOS's temp folder is
  // under a symlink), so XenoNet places each session in its project.
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xn-demo-")));
  const tag = path.basename(dir).toLowerCase().replace(/[^a-z0-9-]/g, "");
  const sock = tag; // tmux -L: a server of the demo's own
  const home = path.join(dir, "home");
  const xdg = Object.fromEntries(["config", "data", "state", "cache"].map((k) => [k, path.join(dir, "xdg", k)]));
  const xn = { state: path.join(dir, "xenonet/state"), cache: path.join(dir, "xenonet/cache"), log: path.join(dir, "xenonet/server.log") };
  for (const d of [home, ...Object.values(xdg), xn.state, xn.cache, path.join(home, ".claude"), path.join(home, ".codex"), path.join(home, "wiki")]) fs.mkdirSync(d, { recursive: true });
  const port = o.port ?? (await freePort());
  const xbase = `http://127.0.0.1:${port}`;
  const projects = PROJECTS.slice(0, o.sessions).map((p) => ({ ...p, dir: path.join(dir, "projects", p.name) }));

  let server = null;
  const panes = [];
  let socket = null; // the tmux server's socket, which kill-server leaves behind
  let cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    cleaned = true;
    spawnSync("tmux", ["-L", sock, "kill-server"], { stdio: "ignore" });
    if (socket && path.basename(socket) === sock) fs.rmSync(socket, { force: true });
    for (const pid of panes) {
      try {
        process.kill(pid, "SIGTERM");
      } catch {}
    }
    if (server && server.exitCode == null) {
      server.kill("SIGTERM");
      await Promise.race([new Promise((r) => server.once("exit", r)), sleep(3000)]);
      if (server.exitCode == null) server.kill("SIGKILL");
    }
    await sleep(500);
    for (const pid of panes) {
      try {
        process.kill(pid, "SIGKILL");
      } catch {}
    }
    fs.rmSync(dir, { recursive: true, force: true });
  };
  const onSignal = () => void cleanup().then(() => process.exit(130));
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  const startXenoNet = async () => {
    const env = cleanEnv({
      HOME: home,
      XENONET_PORT: String(port),
      XENONET_NAME: "XenoNet (OpenCode demo)",
      XENONET_STATE_DIR: xn.state,
      XENONET_CACHE_DIR: xn.cache,
      XENONET_PROJECTS_DIR: path.join(dir, "projects"),
      XENONET_WIKI_DIR: path.join(home, "wiki"),
      XENONET_IDENTITIES: "off",
      XENONET_CLAUDE_USAGE: "off",
      XENONET_ACTORS: "off",
      XENONET_ASSISTANT: "off",
      XENONET_CODEX_BIN: fs.existsSync("/usr/bin/false") ? "/usr/bin/false" : "/bin/false",
      CLAUDE_CONFIG_DIR: path.join(home, ".claude"),
      CODEX_HOME: path.join(home, ".codex"),
    });
    const log = fs.openSync(xn.log, "a");
    server = spawn(process.execPath, [path.join(ROOT, "server/index.js")], { cwd: ROOT, env, stdio: ["ignore", log, log] });
    fs.closeSync(log);
    await until("the demo XenoNet to answer", async () => (await http(xbase, "/api/health", { ms: 1000 }))?.name, { ms: 60e3 });
  };
  const stopXenoNet = async () => {
    if (!server || server.exitCode != null) return;
    server.kill("SIGTERM");
    await Promise.race([new Promise((r) => server.once("exit", r)), sleep(5000)]);
  };
  const xenonet = (...args) => spawnSync(process.execPath, [path.join(ROOT, "cli/xenonet.js"), ...args, "--port", String(port)], { env: cleanEnv({ NO_COLOR: "1" }), encoding: "utf8" });

  try {
    say(`OpenCode ${ocVersion}, model ${o.model}, ${o.sessions} sessions, ${o.rounds} round${o.rounds > 1 ? "s" : ""}`);
    say(`Everything is in ${dir}`);

    // The projects: notes the others ask about, the session's standing
    // instructions, OpenCode's config, and XenoNet's tools.
    for (const p of projects) {
      fs.mkdirSync(path.join(p.dir, ".opencode/tools"), { recursive: true });
      fs.writeFileSync(path.join(p.dir, "NOTES.md"), notesMd(p));
      fs.writeFileSync(path.join(p.dir, "AGENTS.md"), agentsMd(p));
      fs.writeFileSync(path.join(p.dir, "opencode.json"), `${JSON.stringify(openCodeConfig(o.model), null, 2)}\n`);
      fs.copyFileSync(TOOL, path.join(p.dir, ".opencode/tools/xenonet.js"));
      const git = (...a) => spawnSync("git", ["-c", "user.name=demo", "-c", "user.email=demo@example.invalid", "-c", "init.defaultBranch=main", ...a], { cwd: p.dir, env: cleanEnv({ HOME: home, GIT_CONFIG_NOSYSTEM: "1" }), stdio: "ignore" });
      git("init", "-q");
      git("add", "NOTES.md", "AGENTS.md", "opencode.json");
      git("commit", "-qm", "demo project");
    }

    // The OpenCode TUIs, each with its server on a port of its own.
    for (const p of projects) {
      p.port = await freePort();
      p.base = `http://127.0.0.1:${p.port}`;
      const env = {
        PATH: "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin",
        HOME: home,
        TERM: "xterm-256color",
        LANG: process.env.LANG || "en_US.UTF-8",
        XDG_CONFIG_HOME: xdg.config,
        XDG_DATA_HOME: xdg.data,
        XDG_STATE_HOME: xdg.state,
        XDG_CACHE_HOME: xdg.cache,
        OPENCODE_DISABLE_AUTOUPDATE: "1",
        OPENCODE_DISABLE_CLAUDE_CODE: "1",
        OPENCODE_DISABLE_DEFAULT_PLUGINS: "1",
        XENONET_PORT: String(port),
      };
      const cmd = `exec env -i ${Object.entries(env).map(([k, v]) => `${k}=${sh(v)}`).join(" ")} ${sh(opencode)} --hostname 127.0.0.1 --port ${p.port} ${sh(p.dir)}`;
      // The first starts the server, without your tmux config.
      const r = spawnSync("tmux", ["-L", sock, ...(socket ? [] : ["-f", "/dev/null"]), "new-session", "-d", "-s", `xn-demo-${p.name}`, "-x", "160", "-y", "48", "-c", p.dir, cmd], { encoding: "utf8" });
      if (r.status !== 0) throw new Error(`tmux couldn't start ${p.name}: ${r.stderr}`);
      socket ??= spawnSync("tmux", ["-L", sock, "display-message", "-p", "#{socket_path}"], { encoding: "utf8" }).stdout.trim() || null;
      const pid = Number(spawnSync("tmux", ["-L", sock, "list-panes", "-t", `xn-demo-${p.name}`, "-F", "#{pane_pid}"], { encoding: "utf8" }).stdout.trim());
      if (pid) panes.push(pid);
    }
    for (const p of projects) await until(`OpenCode in ${p.name}`, async () => (await http(p.base, "/global/health", { ms: 1000 }))?.healthy, { ms: 90e3 });

    // The model, and whether it's free.
    const [providerID, modelID] = o.model.split("/");
    const providers = await http(projects[0].base, "/config/providers", { ms: 30e3 });
    const model = providers?.providers?.find((x) => x.id === providerID)?.models?.[modelID];
    if (!model) {
      const free = Object.entries(providers?.providers?.find((x) => x.id === "opencode")?.models ?? {}).filter(([, m]) => !m.cost?.input && !m.cost?.output).map(([id]) => `opencode/${id}`);
      throw new Error(`OpenCode has no model ${o.model} here. Free ones it has now: ${free.join(", ") || "none"}`);
    }
    say(`${o.model} costs ${model.cost?.input || model.cost?.output ? "money: it needs an account" : "nothing: no account or key"}`);

    // A session with its title in each TUI, and XenoNet's tools loaded.
    for (const p of projects) {
      const s = await http(p.base, "/session", { method: "POST", body: { title: p.title } });
      p.session = s.id;
      p.id = `opencode:${s.id.replace(/^ses_/, "ses-")}`;
      await http(p.base, "/tui/select-session", { method: "POST", body: { sessionID: s.id } });
    }
    for (const p of projects) await until(`XenoNet's tools in ${p.name}`, async () => (await http(p.base, "/experimental/tool/ids", { ms: 60e3 }))?.includes?.("xenonet_send"), { ms: 180e3, every: 1000 });

    // XenoNet, and your consent for each session.
    await startXenoNet();
    say(`XenoNet from this checkout answers on port ${port}`);
    const refused = [];
    for (const p of projects) {
      const r = xenonet("live", "allow", p.id, "--hours", "1");
      if (r.status === 0) say(`  allowed ${p.title} (${p.id})`);
      else refused.push([p, `${r.stderr}${r.stdout}`.trim()]);
    }
    if (refused.length) {
      say(`\nXenoNet refused xenonet live allow: ${refused[0][1].split("\n")[0]}`);
      if (o.seedConsent) {
        say("--seed-consent: writing the same allowance into the demo XenoNet's own state, then starting it again");
        await stopXenoNet();
        const file = path.join(xn.state, "delivery.json");
        let previous = null;
        try {
          previous = JSON.parse(fs.readFileSync(file, "utf8"));
        } catch {}
        fs.writeFileSync(file, JSON.stringify(seededConsent(refused.map(([p]) => p.id), { previous }), null, 1), { mode: 0o600 });
        await startXenoNet();
      } else {
        say("Consent is yours to give. In your own terminal, run:");
        for (const [p] of refused) say(`  node ${path.join(ROOT, "cli/xenonet.js")} live allow ${p.id} --hours 1 --port ${port}`);
        say("Waiting up to 10 minutes for it.");
      }
      await until("your consent for each session", async () => {
        const d = await http(xbase, "/api/delivery");
        return projects.every((p) => d.consent?.[p.id]?.until > Date.now());
      }, { ms: 600e3, every: 2000 });
    }

    // Who XenoNet sees: the sessions, by their own titles.
    const agents = await until("XenoNet to list the sessions", async () => {
      const list = await http(xbase, "/api/engine/agents");
      return projects.every((p) => list.some((a) => a.id === p.id)) && list;
    }, { ms: 30e3 });
    say("\nXenoNet sees:");
    for (const a of agents.filter((a) => projects.some((p) => p.id === a.id))) say(`  ${a.title}  ${a.id}  ${a.project ?? ""}  ${a.status}`);

    // The rounds: each question typed into the asking session's TUI, as you
    // would, then wait for the answers and for every session to go idle.
    const byName = Object.fromEntries(projects.map((p) => [p.name, p]));
    const count = async () => (await http(xbase, "/api/engine/state")).messages.length;
    for (let round = 1; round <= o.rounds; round++) {
      for (const step of plan(o.sessions, round)) {
        const from = byName[step.from];
        const before = await count();
        say(`\nRound ${round}: ${from.title} asks ${step.to}`);
        await http(from.base, `/session/${from.session}/prompt_async`, { method: "POST", body: { parts: [{ type: "text", text: kickoff(step) }] } });
        let lastCount = before, lastChange = Date.now();
        await until(`the answers to "${step.text}"`, async () => {
          const n = await count();
          if (n !== lastCount) [lastCount, lastChange] = [n, Date.now()];
          const busy = (await Promise.all(projects.map((p) => http(p.base, "/session/status").catch(() => ({}))))).some((st, i) => st?.[projects[i].session] && st[projects[i].session].type !== "idle");
          return n - before >= step.expect && !busy && Date.now() - lastChange > 5000;
        }, { ms: o.timeout * 1000, every: 1000 }).catch((err) => say(`  ${err.message}; going on`));
        say(`  ${(await count()) - before} messages`);
      }
    }

    // The timings, from XenoNet's records and each session's own.
    const state = await http(xbase, "/api/engine/state");
    const delivery = await http(xbase, "/api/delivery?audit=200");
    const sessions = {};
    for (const p of projects) sessions[p.id] = { title: p.title, messages: await http(p.base, `/session/${p.session}/message`, { ms: 15e3 }) };
    const rows = timings({ messages: state.messages, deliveries: delivery.recent, sessions });
    say(`\n${report(rows)}`);

    if (o.out) {
      fs.mkdirSync(o.out, { recursive: true });
      for (const p of projects) {
        const text = spawnSync("tmux", ["-L", sock, "capture-pane", "-p", "-J", "-S", "-", "-t", `xn-demo-${p.name}`], { encoding: "utf8" }).stdout;
        fs.writeFileSync(path.join(o.out, `pane-${p.name}.txt`), text);
      }
      fs.writeFileSync(path.join(o.out, "timings.json"), JSON.stringify({ opencode: ocVersion, model: o.model, sessions: o.sessions, rounds: o.rounds, rows }, null, 1));
      fs.writeFileSync(path.join(o.out, "xenonet.json"), JSON.stringify({ messages: state.messages, deliveries: delivery.recent, audit: delivery.audit }, null, 1));
      fs.writeFileSync(path.join(o.out, "opencode.json"), JSON.stringify(sessions, null, 1));
      fs.copyFileSync(xn.log, path.join(o.out, "xenonet-server.log"));
      say(`\nThe panes' text and the records are in ${o.out}`);
    }
    if (o.keep) {
      say(`\n--keep: still running. Watch a session with: tmux -L ${sock} attach -t xn-demo-${projects[0].name}`);
      say(`Stop it all with: tmux -L ${sock} kill-server; kill ${server.pid}; rm -rf ${sh(dir)}${socket ? ` ${sh(socket)}` : ""}`);
      cleaned = true;
    }
    return rows.length ? 0 : 1;
  } finally {
    await cleanup();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().then(
    (code) => process.exit(code),
    (err) => {
      process.stderr.write(`${err.message}\n`);
      process.exit(1);
    },
  );
}
