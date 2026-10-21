// Demo mode: a made-up workspace that runs through XenoNet's real pipeline.
// It writes projects (real git repos), Claude Code transcripts, Codex rollouts
// and a small wiki into a temp folder, lists the sessions the way the adapters
// would, and keeps appending transcript lines so the map stays busy. Your real
// projects and sessions are never read while it runs.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { execFile, execFileSync } from "node:child_process";
import { gitAvailable } from "./git.js";
import { freshDemoRoot } from "./demo-root.js";
import { claudeId, gitId, githubId } from "./identities.js";

// One world per port, so demo servers running side by side (XenoNet Dev's demo
// and test instances, say) don't wipe each other's.
let ROOT = path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${process.env.XENONET_PORT ?? 47841}`);
let HOME = path.join(ROOT, "home");
const P = (...parts) => path.join(HOME, "Projects", ...parts);
const C = (...parts) => path.join(HOME, "Clients", ...parts);

export const world = {
  root: ROOT,
  home: HOME,
  projectsDir: P(),
  // A second projects folder, to try another (see CLIENTS).
  clientsDir: C(),
  claudeRoot: path.join(HOME, ".claude", "projects"),
  claudeDesktopDir: path.join(HOME, "claude-desktop"),
  codexHome: path.join(HOME, ".codex"),
  grokHome: path.join(HOME, ".grok"),
  wikiDir: path.join(HOME, "wiki"),
  stateDir: path.join(ROOT, "state"),
  cacheDir: path.join(ROOT, "cache"),
};

const MIN = 60e3;
const iso = (ts) => new Date(ts).toISOString();
const line = (o) => JSON.stringify(o) + "\n";

// ---------- projects ----------

const PROJECTS = [
  { name: "atlas-api", readme: "Orders, inventory and payments API for the Atlas store.", files: { "package.json": "{}", "tsconfig.json": "{}", "src/orders/routes.ts": "export const routes = [];\n", "src/orders/refunds.ts": "export function refund() {}\n" }, dirty: { "src/orders/routes.ts": "export const routes = [refundRoute];\n" } },
  { name: "atlas-api-local-db", readme: "Local Postgres and Supabase for atlas-api.", files: { "docker-compose.yml": "services: {}\n", "supabase/config.toml": "" }, git: false },
  { name: "atlas-web", readme: "The Atlas storefront, built with Next.js.", files: { "package.json": "{}", "tsconfig.json": "{}", "app/checkout/page.tsx": "export default function Checkout() {}\n" } },
  { name: "ledger", readme: "Double-entry bookkeeping service with monthly reconciliation.", files: { "pyproject.toml": "[project]\nname = \"ledger\"\n", "ledger/reconcile.py": "def reconcile(): ...\n" }, dirty: { "ledger/reconcile.py": "def reconcile(rounding='half-even'): ...\n" } },
  { name: "pathfinder", readme: "Route planner for delivery vans.", files: { "Cargo.toml": "[package]\nname = \"pathfinder\"\n", "src/solver.rs": "pub fn solve() {}\n" } },
  { name: "infra", readme: "Terraform modules and deploy scripts for every environment.", files: { "Dockerfile": "FROM node:22\n", "compose.yaml": "services: {}\n" } },
  { name: "docs", readme: "Docs site for Atlas and Ledger.", files: { "package.json": "{}", "guides/refunds.md": "# Refunds\n" } },
  // A folder of experiments: XenoNet shows it as a group around the two repos
  // in it, and leaves the loose snippets out.
  { name: "labs", readme: "Experiments that aren't projects yet.", files: { "ideas.md": "- geofences per postcode\n- simulate a day of routes\n" }, git: false },
  { name: "labs/geofence", readme: "Delivery zones drawn from postcodes.", files: { "package.json": "{}", "src/zones.ts": "export const zones = [];\n" }, dirty: { "src/zones.ts": "export const zones = loadPostcodes();\n" } },
  { name: "labs/route-sim", readme: "Simulates a day of van routes against pathfinder.", files: { "pyproject.toml": "[project]\nname = \"route-sim\"\n", "sim/day.py": "def run(): ...\n" } },
  { name: "labs/snippets", readme: "Odds and ends.", files: { "pyproject.toml": "[project]\nname = \"snippets\"\n", "csv_to_json.py": "" }, git: false },
];

// A second projects folder, ~/Clients, for trying another one (Change the
// projects folder… in the palette, or xenonet root): client work in a few
// repos, and a labs folder whose loose snippets folder has the same path in
// it as ~/Projects' has, so Codex's picks for one folder can be told from
// the other's. No session runs here, so with it in use, every session is a
// folder outside it.
const CLIENTS = [
  { name: "harbor-app", readme: "Berth bookings for the Harbor marina, on the web and in the app.", files: { "package.json": "{}", "tsconfig.json": "{}", "src/bookings.ts": "export const bookings = [];\n" } },
  { name: "harbor-api", readme: "Berths, bookings and invoices for the Harbor marina.", files: { "go.mod": "module harbor-api\n", "berths.go": "package main\n" }, dirty: { "berths.go": "package main\n\n// Tides decide which berths are free.\n" } },
  { name: "kiln", readme: "Firing schedules for a pottery studio's kilns.", files: { "Cargo.toml": "[package]\nname = \"kiln\"\n", "src/main.rs": "fn main() {}\n" } },
  { name: "labs", readme: "Experiments for clients.", files: {}, git: false },
  { name: "labs/glaze", readme: "Glaze recipes, scaled to the batch.", files: { "pyproject.toml": "[project]\nname = \"glaze\"\n", "glaze/scale.py": "def scale(): ...\n" } },
  { name: "labs/snippets", readme: "Odds and ends.", files: { "pyproject.toml": "[project]\nname = \"snippets\"\n", "tides.py": "" }, git: false },
];

const GIT = ["-c", "user.name=XenoNet Demo", "-c", "user.email=demo@example.invalid", "-c", "core.autocrlf=false"];
function git(dir, ...args) {
  execFileSync("git", [...GIT, ...args], { cwd: dir, stdio: "ignore" });
}

// Writes each project's files, and returns the ones that are repos.
function writeFiles(list, at) {
  for (const p of list) {
    const dir = at(p.name);
    for (const [rel, text] of Object.entries({ "README.md": `# ${path.basename(p.name)}\n\n${p.readme}\n`, ...p.files })) {
      fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
      fs.writeFileSync(path.join(dir, rel), text);
    }
  }
  return gitAvailable() ? list.filter((p) => p.git !== false).map((p) => ({ dir: at(p.name), dirty: p.dirty ?? {} })) : [];
}

function writeProjects() {
  for (const { dir, dirty } of writeFiles(PROJECTS, P)) {
    try {
      git(dir, "init", "-q", "-b", "main");
      git(dir, "add", "-A");
      git(dir, "commit", "-q", "-m", "chore: initial commit");
      for (const [rel, text] of Object.entries(dirty)) fs.writeFileSync(path.join(dir, rel), text);
    } catch {} // No git: the projects still show, without repo status.
  }
}

// The second folder's files are written with the rest, but its repos are
// made the first time something asks for that folder (XenoNetState's checks
// and changes of folder wait for clientsReady()), side by side, so the demo
// starts as fast as it did without them.
let clientRepos = [];
let clients = null;
export function clientsReady() {
  const run = (dir, ...args) => new Promise((resolve, reject) => execFile("git", [...GIT, ...args], { cwd: dir }, (err) => (err ? reject(err) : resolve())));
  clients ??= Promise.all(
    clientRepos.map(async ({ dir, dirty }) => {
      try {
        await run(dir, "init", "-q", "-b", "main");
        await run(dir, "add", "-A");
        await run(dir, "commit", "-q", "-m", "chore: initial commit");
        for (const [rel, text] of Object.entries(dirty)) fs.writeFileSync(path.join(dir, rel), text);
      } catch {} // No git: the projects still show, without repo status.
    }),
  );
  return clients;
}
function writeClients() {
  clientRepos = writeFiles(CLIENTS, C);
  clients = null;
}

// ---------- sessions ----------

// Each session has a script of turns. Steps: read, edit, run, mcp, agent, ask.
// Live sessions keep looping their turns while the demo runs.
const SESSIONS = [
  {
    app: "claude", project: "atlas-api", title: "Add partial refunds to the orders API", branch: "feat/partial-refunds", live: true, desktop: true,
    turns: [
      { prompt: "Add partial refunds to the orders API. Keep the existing full-refund path working.", steps: [["read", "src/orders/refunds.ts"], ["edit", "src/orders/routes.ts", "routes = []", "routes = [refundRoute]"], ["run", "npm test -- orders", true], ["edit", "src/orders/refunds.ts", "refund()", "refund(amount?: number)"]], reply: "Partial refunds work. `POST /orders/:id/refunds` takes an optional amount, and the full-refund path is unchanged. All 48 order tests pass." },
      { prompt: "Log the refund rules in the wiki.", steps: [["mcp", "agent-wiki", "wiki_log", { pages: ["atlas-api", "refunds"] }], ["run", "npm run lint", true]], reply: "Logged the refund rules on the atlas-api and refunds pages." },
    ],
  },
  {
    app: "codex", project: "atlas-api", title: "Rate-limit the public orders endpoint", branch: "feat/rate-limit", live: true, native: true,
    turns: [
      { prompt: "Rate-limit GET /orders to 60 requests a minute per API key.", steps: [["read", "src/orders/routes.ts"], ["edit", "src/orders/routes.ts", "routes = [", "routes = [rateLimit(60), "], ["run", "npm test -- rate-limit", true]], reply: "Added a token-bucket limiter at 60 requests a minute per key. Over the limit, clients get 429 with a Retry-After header." },
    ],
  },
  {
    app: "claude", project: "atlas-web", title: "Move checkout to the new payments SDK", branch: "feat/payments-v3", desktop: true, ask: true,
    turns: [
      { prompt: "Move checkout to version 3 of the payments SDK.", steps: [["read", "app/checkout/page.tsx"], ["edit", "app/checkout/page.tsx", "payments.v2", "payments.v3"], ["run", "npm run build", true], ["ask", "Version 3 drops saved cards from version 2. Should I migrate them, or ask customers to re-enter them?"]] },
    ],
  },
  {
    app: "codex", project: "ledger", title: "Fix rounding in monthly reconciliation", branch: "fix/rounding", live: true, stuck: true,
    turns: [
      { prompt: "Reconciliation is off by a cent on some months. Find out why and fix it.", steps: [["read", "ledger/reconcile.py"], ["edit", "ledger/reconcile.py", "round(x, 2)", "Decimal(x).quantize(CENT, ROUND_HALF_EVEN)"], ["run", "pytest tests/test_reconcile.py", false], ["run", "pytest tests/test_reconcile.py", false], ["run", "pytest tests/test_reconcile.py", false]] },
    ],
  },
  {
    app: "claude", project: "pathfinder", title: "Speed up the route solver for 500+ stops", branch: "perf/solver", live: true, desktop: true,
    turns: [
      { prompt: "The solver takes 40 seconds for 500 stops. Get it under 5.", steps: [["agent", "Profile the solver on the 500-stop fixture"], ["read", "src/solver.rs"], ["edit", "src/solver.rs", "fn solve()", "fn solve_with_cache()"], ["run", "cargo bench solver", true]], reply: "The solver now caches distance lookups. 500 stops take 3.8 seconds, down from 40." },
    ],
  },
  {
    app: "codex", project: "docs", title: "Write the refunds guide", review: true,
    turns: [{ prompt: "Write a guide for the new partial refunds.", steps: [["read", "guides/refunds.md"], ["edit", "guides/refunds.md", "# Refunds", "# Refunds\n\nRefund all or part of an order."]], reply: "The refunds guide covers full and partial refunds, with request and response examples." }],
  },
  { app: "claude", project: "infra", title: "Move staging to the new cluster", ago: 190, desktop: true, turns: [{ prompt: "Move staging to the new cluster.", steps: [["run", "terraform plan -var-file=staging.tfvars", true]], reply: "Staging runs on the new cluster. The old one can go after a week." }] },
  { app: "codex", project: "atlas-web", title: "Add product image zoom", ago: 300, native: true, turns: [{ prompt: "Add zoom on product images.", steps: [["edit", "app/product/zoom.tsx", "", "export function Zoom() {}"]], reply: "Product images zoom on hover and pinch." }] },
  { app: "claude", project: "ledger", title: "Draft the Q4 close checklist", ago: 420, turns: [{ prompt: "Draft a checklist for the Q4 close.", steps: [["read", "ledger/reconcile.py"]], reply: "The checklist has 14 steps, grouped by team." }] },
  { app: "codex", cwd: ["Documents", "Codex", new Date().toLocaleDateString("en-CA"), "regex-help"], title: "Explain this regex", ago: 140, turns: [{ prompt: "What does ^(?=.*\\d)(?=.*[a-z]).{8,}$ match?", steps: [], reply: "It matches strings of eight or more characters with at least one digit and one lowercase letter." }] },
  { app: "claude", project: "atlas-api", title: "Review the partial refunds change", ago: 25, desktop: true, parent: 0, turns: [{ prompt: "Review the partial refunds change before I open a PR.", steps: [["read", "src/orders/refunds.ts"]], reply: "Two notes: refunds over the order total aren't rejected yet, and the tests don't cover a second partial refund." }] },
  { app: "codex", project: "pathfinder", title: "Benchmark solver variants", ago: 60, parent: 4, subagent: true, turns: [{ prompt: "Benchmark the cached solver against the old one.", steps: [["run", "cargo bench", true]], reply: "The cached solver is 10.5 times faster on 500 stops." }] },
  { app: "codex", project: "labs/geofence", title: "Draw delivery zones from postcodes", ago: 35, turns: [{ prompt: "Turn the postcode list into delivery zones.", steps: [["read", "src/zones.ts"], ["edit", "src/zones.ts", "zones = []", "zones = loadPostcodes()"], ["run", "npm test", true]], reply: "Zones now load from the postcode list. Each one is the convex hull of its postcodes." }] },
  { app: "claude", project: "labs", title: "Sort out which experiments to keep", ago: 240, turns: [{ prompt: "Which of these experiments are worth keeping?", steps: [["read", "ideas.md"]], reply: "Keep geofence and route-sim. Snippets is turning into a tool of its own." }] },
];

// Stable IDs, so a saved layout survives a restart of the demo.
const ids = SESSIONS.map((_, i) => `0190d3e0-0000-7000-8000-${String(i + 1).padStart(12, "0")}`);
const desktopIds = SESSIONS.map((_, i) => `local_0190d3e0-0000-7000-9000-${String(i + 1).padStart(12, "0")}`);
const cwdOf = (s) => (s.cwd ? path.join(HOME, ...s.cwd) : P(s.project));

// ---------- transcript writers ----------

const MODELS = { claude: "claude-sonnet-4-5", codex: "gpt-5-codex" };

class Writer {
  constructor(s, i) {
    this.s = s;
    this.id = ids[i];
    this.cwd = cwdOf(s);
    this.ctx = 18000 + i * 2100;
    this.tokens = 0;
    this.turn = 0;
    this.step = -1; // -1: next is the prompt
    this.entries = []; // for the transcript preview
    const now = new Date();
    if (s.app === "claude") {
      this.file = path.join(world.claudeRoot, `demo-${path.basename(this.cwd)}`, `${this.id}.jsonl`);
    } else {
      const day = path.join(world.codexHome, "sessions", String(now.getFullYear()), String(now.getMonth() + 1).padStart(2, "0"), String(now.getDate()).padStart(2, "0"));
      this.file = path.join(day, `rollout-${iso(now).slice(0, 19).replace(/:/g, "-")}-${this.id}.jsonl`);
    }
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
  }

  write(...objs) {
    fs.appendFileSync(this.file, objs.map(line).join(""));
  }

  usage() {
    this.ctx += 1800 + Math.floor(Math.random() * 2600);
    const out = 300 + Math.floor(Math.random() * 900);
    this.tokens += this.ctx + out;
    return { input_tokens: 40, cache_read_input_tokens: this.ctx - 40, cache_creation_input_tokens: 0, output_tokens: out };
  }

  codexTokens(ts) {
    const u = this.usage();
    const used = Math.min(92, 34 + this.tokens / 60000);
    const reset = Math.floor(ts / 1000) + 2.4 * 3600;
    // As Codex writes it: cached input is part of input_tokens.
    const last = { input_tokens: this.ctx, cached_input_tokens: u.cache_read_input_tokens, output_tokens: u.output_tokens, reasoning_output_tokens: Math.round(u.output_tokens * 0.4), total_tokens: this.ctx + u.output_tokens };
    this.write({ type: "event_msg", timestamp: iso(ts), payload: { type: "token_count", info: { total_token_usage: { total_tokens: this.tokens, output_tokens: u.output_tokens }, last_token_usage: last, model_context_window: 258400 }, rate_limits: { primary: { used_percent: Math.round(used), window_minutes: 300, resets_at: reset }, secondary: { used_percent: 58, window_minutes: 10080, resets_at: reset + 3 * 86400 } } } });
  }

  // Writes the next step of the script at time ts. Returns false when the
  // session has nothing more to do in this turn (a pending question or a loop).
  next(ts) {
    const turn = this.s.turns[this.turn % this.s.turns.length];
    if (this.step === -1) {
      this.prompt(turn.prompt, ts);
      this.step = 0;
      return true;
    }
    if (this.step < turn.steps.length) {
      this.act(turn.steps[this.step], ts);
      this.step++;
      return true;
    }
    if (!turn.reply) {
      // Scripts without a reply loop on their last step (the stuck session)
      // or wait on their question.
      const last = turn.steps[turn.steps.length - 1];
      if (last?.[0] === "run") this.act(last, ts);
      return false;
    }
    this.reply(turn.reply, ts);
    this.turn++;
    this.step = -1;
    return true;
  }

  prompt(text, ts) {
    this.entries.push({ id: randomUUID(), ts, role: "user", text });
    if (this.s.app === "claude") return this.write({ type: "user", timestamp: iso(ts), message: { role: "user", content: text } });
    this.write(
      { type: "turn_context", timestamp: iso(ts), payload: { model: MODELS.codex } },
      { type: "event_msg", timestamp: iso(ts), payload: { type: "task_started", model_context_window: 258400 } },
      { type: "event_msg", timestamp: iso(ts), payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } } },
    );
  }

  reply(text, ts) {
    this.entries.push({ id: randomUUID(), ts, role: "assistant", text });
    if (this.s.app === "claude") {
      return this.write({ type: "assistant", timestamp: iso(ts), message: { id: `msg_${randomUUID()}`, model: MODELS.claude, stop_reason: "end_turn", usage: this.usage(), content: [{ type: "text", text }] } });
    }
    this.codexTokens(ts);
    this.write({ type: "event_msg", timestamp: iso(ts), payload: { type: "task_complete", last_agent_message: text, duration_ms: 95000 } });
  }

  act([kind, a, b, c], ts) {
    const file = kind === "read" || kind === "edit" ? path.join(this.cwd, a) : null;
    const title = { read: `Read ${a}`, edit: `Edit ${a}`, run: a, mcp: `${a}.${b}`, agent: a, ask: "AskUserQuestion" }[kind];
    this.entries.push({ id: randomUUID(), ts, role: "tool", kind: "tool_use", title, text: kind === "edit" ? `- ${b}\n+ ${c}` : kind === "ask" ? a : "", status: kind === "run" && b === false ? "failed" : "completed" });
    if (this.s.app === "claude") return this.claudeAct(kind, a, b, c, file, ts);
    if (kind === "read") return this.codexTokens(ts);
    if (kind === "edit") return this.write({ type: "event_msg", timestamp: iso(ts), payload: { type: "item_completed", item: { type: "FileChange", status: "completed", changes: { [file]: { type: "update", unified_diff: `@@ -1 +1 @@\n-${b}\n+${c}` } } } } });
    if (kind === "run") {
      this.codexTokens(ts);
      return this.write({ type: "event_msg", timestamp: iso(ts), payload: { type: "item_completed", item: { type: "CommandExecution", command: ["pwsh", "-Command", a], exit_code: b ? 0 : 1, status: b ? "completed" : "failed", aggregatedOutput: b ? "ok" : "1 failed, 23 passed" } } });
    }
    if (kind === "mcp") return this.write({ type: "event_msg", timestamp: iso(ts), payload: { type: "item_completed", item: { type: "McpToolCall", server: a, tool: b, arguments: c, status: "completed" } } });
  }

  claudeAct(kind, a, b, c, file, ts) {
    const callId = `toolu_${randomUUID().replace(/-/g, "").slice(0, 24)}`;
    const tool = {
      read: ["Read", { file_path: file }],
      edit: ["Edit", { file_path: file, old_string: b, new_string: c }],
      run: ["Bash", { command: a }],
      mcp: [`mcp__${a}__${b}`, c ?? {}],
      agent: ["Task", { description: a, prompt: a }],
      ask: ["AskUserQuestion", { questions: [{ question: a }] }],
    }[kind];
    this.write({ type: "assistant", timestamp: iso(ts), message: { id: `msg_${randomUUID()}`, model: MODELS.claude, stop_reason: "tool_use", usage: this.usage(), content: [{ type: "tool_use", id: callId, name: tool[0], input: tool[1] }] } });
    if (kind === "ask") return; // stays pending: the session needs you
    if (kind === "agent") this.subagent(a, ts);
    const ok = kind !== "run" || b !== false;
    this.write({ type: "user", timestamp: iso(ts + 4000), message: { role: "user", content: [{ type: "tool_result", tool_use_id: callId, is_error: !ok, content: ok ? "ok" : "1 failed" }] } });
  }

  subagent(task, ts) {
    const file = path.join(path.dirname(this.file), this.id, "subagents", `agent-${randomUUID().slice(0, 8)}.jsonl`);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const u = { input_tokens: 30, cache_read_input_tokens: 9000, cache_creation_input_tokens: 0, output_tokens: 400 };
    fs.appendFileSync(
      file,
      line({ type: "user", timestamp: iso(ts), isSidechain: true, message: { role: "user", content: task } }) +
        line({ type: "assistant", timestamp: iso(ts + 2000), isSidechain: true, message: { id: `msg_${randomUUID()}`, model: MODELS.claude, stop_reason: "tool_use", usage: u, content: [{ type: "tool_use", id: "toolu_sub1", name: "Bash", input: { command: "cargo flamegraph --bench solver" } }] } }),
    );
  }
}

let writers = [];

// History: every session gets its past turns, spread over the hours before
// now, so token totals, the replay and the day's log have something in them.
function writeHistory(now) {
  writers = SESSIONS.map((s, i) => new Writer(s, i));
  for (const w of writers) {
    const s = w.s;
    // A rollout says the Codex that wrote it, as real ones do (a made-up
    // version).
    if (s.app === "codex") w.write({ type: "session_meta", timestamp: iso(now - 8 * 3600e3), payload: { id: w.id, cwd: w.cwd, cli_version: "0.150.0" } });
    // Live sessions start a little in the past and catch up to now.
    const end = s.live || s.ask ? now - 1 * MIN : s.review ? now - 9 * MIN : now - (s.ago ?? 60) * MIN;
    const steps = s.turns.reduce((n, t) => n + t.steps.length + 2, 0) * (s.live ? 2 : 1);
    let ts = end - steps * 50e3;
    for (let k = 0; k < steps; k++, ts += 50e3) if (!w.next(ts) && !s.stuck) break;
  }
}

function writeWiki(now) {
  const page = (slug, title, body, tags = []) => fs.writeFileSync(path.join(world.wikiDir, "pages", `${slug}.md`), `---\ntitle: "${title}"\ntype: project\ntags: ${JSON.stringify(tags)}\nupdated: ${iso(now - 30 * MIN)}\n---\n\n${body}\n`);
  for (const d of ["pages", "inbox", path.join("log", String(new Date(now).getFullYear()))]) fs.mkdirSync(path.join(world.wikiDir, d), { recursive: true });
  page("atlas-api", "Atlas API", "Orders, inventory and payments. Refund rules live in [[refunds]]. Runs on [[staging-cluster]].", ["atlas-api"]);
  page("refunds", "Refunds", "Full and partial refunds. See [[atlas-api]] and the guide in Projects/docs.");
  page("staging-cluster", "Staging cluster", "Moved to the new cluster this week. Deploys come from Projects/infra.", ["infra"]);
  page("ledger", "Ledger", "Monthly reconciliation rounds half-even. See [[q4-close]].", ["ledger"]);
  page("q4-close", "Q4 close", "The checklist for closing the year in [[ledger]].");
  const t = new Date(now - 40 * MIN);
  const hm = `${t.getHours()}:${String(t.getMinutes()).padStart(2, "0")}`;
  fs.writeFileSync(path.join(world.wikiDir, "log", String(t.getFullYear()), `${t.toLocaleDateString("en-CA")}.md`), `## ${hm} · claude-code · Recorded the refund rules\n\npages: [[refunds]], [[atlas-api]]\n`);
  fs.writeFileSync(path.join(world.wikiDir, "inbox", "note-1.md"), "---\ntitle: Solver benchmark results\napp: codex\n---\n");
}

function writeClaudeLimits(now) {
  fs.mkdirSync(world.claudeDesktopDir, { recursive: true });
  const samples = Array.from({ length: 30 }, (_, k) => ({ t: now - (29 - k) * 10 * MIN, u: { fh: Math.min(95, 12 + k * 2.1), sd: 44 + k * 0.4 } }));
  fs.writeFileSync(path.join(world.claudeDesktopDir, "plan-usage-history.json"), JSON.stringify({ samples }));
}

// Builds the demo workspace from scratch. Synchronous, so config can point at
// it before anything else starts.
export function createWorld(now = Date.now()) {
  const root = freshDemoRoot(ROOT);
  if (root !== ROOT) {
    for (const key of Object.keys(world)) world[key] = root + world[key].slice(ROOT.length);
    ROOT = root;
    HOME = path.join(root, "home");
  }
  fs.mkdirSync(world.projectsDir, { recursive: true });
  writeProjects();
  writeClients();
  writeHistory(now);
  writeWiki(now);
  writeClaudeLimits(now);
  writeIdentities(now);
  return world;
}

// ---------- identities ----------

// Who the demo's work runs as, all made up (see identities.js). Nova works at
// Atlas: the atlas-co organization on GitHub and a work Claude account in the
// Atlas team. Nova also has a side identity, orbitworks, for pathfinder and
// the experiments in labs. The home folder holds what the real apps would
// leave there, and XenoNet's state remembers the work Claude account from
// before Nova signed in to the personal one.
const CLAUDE = {
  personal: { account: "0190d3e0-0000-7000-a000-000000000001", org: "0190d3e0-0000-7000-b000-000000000001" },
  work: { account: "0190d3e0-0000-7000-a000-000000000002", org: "0190d3e0-0000-7000-b000-000000000002" },
};
const ATLAS = new Set(["atlas-api", "atlas-web", "docs", "infra"]);
export const demoClaudeAccount = (project) => (ATLAS.has(project) ? CLAUDE.work : CLAUDE.personal);

// Each repo's origin, and the email its commits get when it isn't the global
// one. pathfinder pushes over HTTPS with no user name set, so it signs in as
// gh's active account, nova-dev, not orbitworks, and XenoNet flags it.
const REMOTES = {
  "atlas-api": ["https://github.com/atlas-co/atlas-api.git", "nova@atlas.example"],
  "atlas-web": ["https://github.com/atlas-co/atlas-web.git", "nova@atlas.example"],
  docs: ["https://github.com/atlas-co/docs.git", "nova@atlas.example"],
  infra: ["git@github.com:atlas-co/infra.git", "nova@atlas.example"],
  ledger: ["https://github.com/nova-dev/ledger.git"],
  pathfinder: ["https://github.com/orbitworks/pathfinder.git", "hello@orbitworks.example"],
  "labs/geofence": ["https://github.com/orbitworks/geofence.git"],
  "labs/route-sim": ["git@gh-orbit:orbitworks/route-sim.git"],
};

function writeIdentities(now) {
  const home = (rel, text) => {
    fs.mkdirSync(path.dirname(path.join(HOME, rel)), { recursive: true });
    fs.writeFileSync(path.join(HOME, rel), text);
  };
  home(".claude.json", JSON.stringify({ oauthAccount: { accountUuid: CLAUDE.personal.account, organizationUuid: CLAUDE.personal.org, emailAddress: "nova@example.com", organizationName: "nova@example.com's Organization", organizationType: "claude_max" } }));
  // An ID token in the shape Codex keeps, unsigned, and no real tokens.
  const jwt = (claims) => ["e30", Buffer.from(JSON.stringify(claims)).toString("base64url"), "demo"].join(".");
  const signedIn = Math.floor((now - 20 * 86400e3) / 1000);
  home(path.join(".codex", "auth.json"), JSON.stringify({ auth_mode: "chatgpt", OPENAI_API_KEY: null, tokens: { id_token: jwt({ email: "nova@example.com", auth_time: signedIn, "https://api.openai.com/auth": { chatgpt_account_id: "demo-workspace", chatgpt_plan_type: "plus" } }), access_token: "demo", refresh_token: "demo", account_id: "demo-workspace" } }));
  home(path.join(".config", "gh", "hosts.yml"), "github.com:\n    git_protocol: https\n    users:\n        nova-dev:\n        orbitworks:\n    user: nova-dev\n");
  home(path.join(".ssh", "config"), "Host gh-orbit\n    HostName github.com\n    IdentityFile ~/.ssh/orbitworks\n");
  home(".gitconfig", `[user]\n\tname = Nova\n\temail = nova@example.com\n[credential "https://github.com"]\n\thelper = !gh auth git-credential\n[includeIf "gitdir:${P("labs").replace(/\\/g, "/")}/"]\n\tpath = ~/.gitconfig-orbit\n`);
  home(".gitconfig-orbit", `[user]\n\tname = Orbit\n\temail = hello@orbitworks.example\n[credential "https://github.com"]\n\tusername = orbitworks\n`);
  if (gitAvailable()) {
    for (const [rel, [origin, email]] of Object.entries(REMOTES)) {
      try {
        git(P(rel), "remote", "add", "origin", origin);
        if (email) git(P(rel), "config", "user.email", email);
        if (email?.endsWith("orbitworks.example")) git(P(rel), "config", "user.name", "Orbit");
      } catch {}
    }
  }
  // What XenoNet would remember: the names Nova gave, and the work account
  // signed in until this morning.
  const work = claudeId(CLAUDE.work.account, CLAUDE.work.org);
  const personal = claudeId(CLAUDE.personal.account, CLAUDE.personal.org);
  const names = { [githubId("github.com", "atlas-co")]: "work", [gitId("nova@atlas.example")]: "work", [work]: "work", [githubId("github.com", "orbitworks")]: "orbit", [gitId("hello@orbitworks.example")]: "orbit" };
  const known = { [work]: { kind: "claude", handle: "nova@atlas.example", detail: { plan: "Team", org: "Atlas" }, firstSeen: now - 30 * 86400e3, lastSeen: now - 8 * 3600e3 } };
  const signins = { claude: [{ id: work, from: now - 30 * 86400e3, to: now - 8 * 3600e3 }, { id: personal, from: now - 8 * 3600e3, to: now - 3600e3 }] };
  fs.mkdirSync(world.stateDir, { recursive: true });
  fs.writeFileSync(path.join(world.stateDir, "state.json"), JSON.stringify({ identities: { names, known, signins } }, null, 2));
}

// ---------- what the adapters would return ----------

export function demoSessions(app) {
  return SESSIONS.flatMap((s, i) => {
    if (s.app !== app) return [];
    const w = writers[i];
    const updatedAt = fs.statSync(w.file).mtimeMs;
    const parent = s.parent != null ? `${SESSIONS[s.parent].app}:${ids[s.parent]}` : null;
    return [{
      id: `${app}:${ids[i]}`,
      app,
      rawId: ids[i],
      desktopId: s.desktop ? desktopIds[i] : null,
      inDesktop: !!s.desktop,
      title: s.title,
      preview: s.turns[0].prompt,
      cwd: w.cwd,
      worktree: null,
      createdAt: updatedAt - 3 * 3600e3,
      updatedAt,
      archived: false,
      starred: i === 0,
      model: MODELS[app],
      branch: s.branch ?? null,
      status: null,
      source: s.subagent ? "subAgent" : null,
      nativeProjectId: s.native ? "demo-atlas" : null,
      parentId: parent,
      parentKind: parent ? (s.subagent ? "subagent" : "spawn") : null,
      claudeAccount: app === "claude" && s.desktop ? demoClaudeAccount(s.project) : null,
    }];
  });
}

// Each session's id, app, title and project, without the world: what the
// demo's actors (core/actors/demo.js) work on.
export const demoSessionList = () => SESSIONS.map((s, i) => ({ id: `${s.app}:${ids[i]}`, app: s.app, title: s.title, project: s.project ?? null }));

export const demoNativeProjects = () => [{ id: "demo-atlas", name: "Atlas", roots: [P("atlas-api"), P("atlas-web")] }];

export function demoTranscript(rawId) {
  const w = writers[ids.indexOf(rawId)];
  if (!w) throw new Error("unknown demo session");
  return { entries: w.entries.slice(-120), cursor: null, total: w.entries.length };
}

// What "Ask Codex which folders are projects" decides in the demo.
export const demoPicks = (outline) =>
  outline.some((f) => f.path === "labs/snippets" && f.xenonet === "folder") ? [{ path: "labs/snippets", as: "project", reason: "Its own pyproject.toml and code, apart from the other experiments." }] : [];

export const demoNarration = () =>
  "While you were away, five sessions worked across four projects. Partial refunds landed in atlas-api, and the solver in pathfinder got ten times faster. The checkout session is waiting on your answer about saved cards, and the ledger session keeps failing the same test.";

// Keeps the live sessions moving: one step every couple of seconds, round robin.
export function startDriver() {
  const live = writers.filter((w) => w.s.live);
  let k = 0;
  const timer = setInterval(() => {
    const w = live[k++ % live.length];
    w.next(Date.now());
  }, 2200);
  return () => clearInterval(timer);
}
