//! core/demo.js: demo mode, a made-up workspace that runs through
//! XenoNet's real pipeline. It writes projects (real git repos), Claude
//! Code transcripts, Codex rollouts, Grok Build sessions and a small wiki
//! into a temp folder, lists the sessions the way the adapters would, and
//! keeps appending transcript lines so the map stays busy. Your real
//! projects and sessions are never read while it runs.
//!
//! What other parts of XenoNet own is asked through `Around`: whether git
//! can run, the ids identities.js gives accounts, and the local clock.

use crate::fsx;
use crate::jstext::encode_uri_component;
use crate::nodepath::NATIVE;
use crate::spotify::b64url;
use serde_json::{json, Map, Value};
use xenonet_base::json::stringify;
use xenonet_base::time::iso;

const MIN: f64 = 60e3;

/// What the demo asks of the rest of XenoNet.
pub trait Around {
    /// gitAvailable() from core/git.js.
    fn git_available(&mut self) -> bool;
    /// claudeId, githubId and gitId from core/identities.js.
    fn claude_id(&mut self, account: &str, org: &str) -> String;
    fn github_id(&mut self, host: &str, login: &str) -> String;
    fn git_id(&mut self, email: &str) -> String;
    /// The local date and time at `ms`: year, month (1 to 12), day, hour,
    /// minute.
    fn local(&mut self, ms: f64) -> [i64; 5];
    /// Math.random(), crypto.randomUUID() and Date.now(), which a test may
    /// have seeded or fixed.
    fn random(&mut self) -> f64;
    fn uuid(&mut self) -> String;
    fn now(&mut self) -> f64;
}

struct Project {
    name: &'static str,
    readme: &'static str,
    files: &'static [(&'static str, &'static str)],
    dirty: &'static [(&'static str, &'static str)],
    git: bool,
}

const PROJECTS: &[Project] = &[
    Project {
        name: "atlas-api",
        readme: "Orders, inventory and payments API for the Atlas store.",
        files: &[
            ("package.json", "{}"),
            ("tsconfig.json", "{}"),
            ("src/orders/routes.ts", "export const routes = [];\n"),
            ("src/orders/refunds.ts", "export function refund() {}\n"),
        ],
        dirty: &[("src/orders/routes.ts", "export const routes = [refundRoute];\n")],
        git: true,
    },
    Project {
        name: "atlas-api-local-db",
        readme: "Local Postgres and Supabase for atlas-api.",
        files: &[("docker-compose.yml", "services: {}\n"), ("supabase/config.toml", "")],
        dirty: &[],
        git: false,
    },
    Project {
        name: "atlas-web",
        readme: "The Atlas storefront, built with Next.js.",
        files: &[("package.json", "{}"), ("tsconfig.json", "{}"), ("app/checkout/page.tsx", "export default function Checkout() {}\n")],
        dirty: &[],
        git: true,
    },
    Project {
        name: "ledger",
        readme: "Double-entry bookkeeping service with monthly reconciliation.",
        files: &[("pyproject.toml", "[project]\nname = \"ledger\"\n"), ("ledger/reconcile.py", "def reconcile(): ...\n")],
        dirty: &[("ledger/reconcile.py", "def reconcile(rounding='half-even'): ...\n")],
        git: true,
    },
    Project {
        name: "pathfinder",
        readme: "Route planner for delivery vans.",
        files: &[("Cargo.toml", "[package]\nname = \"pathfinder\"\n"), ("src/solver.rs", "pub fn solve() {}\n")],
        dirty: &[],
        git: true,
    },
    Project {
        name: "infra",
        readme: "Terraform modules and deploy scripts for every environment.",
        files: &[("Dockerfile", "FROM node:22\n"), ("compose.yaml", "services: {}\n")],
        dirty: &[],
        git: true,
    },
    Project { name: "docs", readme: "Docs site for Atlas and Ledger.", files: &[("package.json", "{}"), ("guides/refunds.md", "# Refunds\n")], dirty: &[], git: true },
    // A folder of experiments: XenoNet shows it as a group around the two
    // repos in it, and leaves the loose snippets out.
    Project {
        name: "labs",
        readme: "Experiments that aren't projects yet.",
        files: &[("ideas.md", "- geofences per postcode\n- simulate a day of routes\n")],
        dirty: &[],
        git: false,
    },
    Project {
        name: "labs/geofence",
        readme: "Delivery zones drawn from postcodes.",
        files: &[("package.json", "{}"), ("src/zones.ts", "export const zones = [];\n")],
        dirty: &[("src/zones.ts", "export const zones = loadPostcodes();\n")],
        git: true,
    },
    Project {
        name: "labs/route-sim",
        readme: "Simulates a day of van routes against pathfinder.",
        files: &[("pyproject.toml", "[project]\nname = \"route-sim\"\n"), ("sim/day.py", "def run(): ...\n")],
        dirty: &[],
        git: true,
    },
    Project { name: "labs/snippets", readme: "Odds and ends.", files: &[("pyproject.toml", "[project]\nname = \"snippets\"\n"), ("csv_to_json.py", "")], dirty: &[], git: false },
];

// A second projects folder, ~/Clients, for trying another one: client
// work in a few repos, and a labs folder whose loose snippets folder has
// the same path in it as ~/Projects' has. No session runs here.
const CLIENTS: &[Project] = &[
    Project {
        name: "harbor-app",
        readme: "Berth bookings for the Harbor marina, on the web and in the app.",
        files: &[("package.json", "{}"), ("tsconfig.json", "{}"), ("src/bookings.ts", "export const bookings = [];\n")],
        dirty: &[],
        git: true,
    },
    Project {
        name: "harbor-api",
        readme: "Berths, bookings and invoices for the Harbor marina.",
        files: &[("go.mod", "module harbor-api\n"), ("berths.go", "package main\n")],
        dirty: &[("berths.go", "package main\n\n// Tides decide which berths are free.\n")],
        git: true,
    },
    Project {
        name: "kiln",
        readme: "Firing schedules for a pottery studio's kilns.",
        files: &[("Cargo.toml", "[package]\nname = \"kiln\"\n"), ("src/main.rs", "fn main() {}\n")],
        dirty: &[],
        git: true,
    },
    Project { name: "labs", readme: "Experiments for clients.", files: &[], dirty: &[], git: false },
    Project {
        name: "labs/glaze",
        readme: "Glaze recipes, scaled to the batch.",
        files: &[("pyproject.toml", "[project]\nname = \"glaze\"\n"), ("glaze/scale.py", "def scale(): ...\n")],
        dirty: &[],
        git: true,
    },
    Project { name: "labs/snippets", readme: "Odds and ends.", files: &[("pyproject.toml", "[project]\nname = \"snippets\"\n"), ("tides.py", "")], dirty: &[], git: false },
];

/// git's options for the demo's commits, before each command's own.
pub const GIT: [&str; 6] = ["-c", "user.name=XenoNet Demo", "-c", "user.email=demo@example.invalid", "-c", "core.autocrlf=false"];

fn git(dir: &str, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(GIT)
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What makes a folder a repo, after its files: the commands, then what
/// it changes without committing.
pub fn repo_steps() -> Vec<Vec<String>> {
    let with = |args: &[&str]| GIT.iter().chain(args).map(ToString::to_string).collect::<Vec<_>>();
    vec![with(&["init", "-q", "-b", "main"]), with(&["add", "-A"]), with(&["commit", "-q", "-m", "chore: initial commit"])]
}

const MODEL_CLAUDE: &str = "claude-sonnet-4-5";
const MODEL_CODEX: &str = "gpt-5-codex";
const MODEL_GROK: &str = "grok-build";
/// The context window Grok Build reports in its usage updates.
const GROK_WINDOW: u32 = 256_000;

fn model(app: &str) -> &'static str {
    match app {
        "claude" => MODEL_CLAUDE,
        "grok" => MODEL_GROK,
        _ => MODEL_CODEX,
    }
}

// The sessions: each has a script of turns, whose steps are read, edit,
// run, mcp, agent and ask. Live sessions keep looping their turns.
fn sessions(today: &str) -> Vec<Value> {
    vec![
        json!({ "app": "claude", "project": "atlas-api", "title": "Add partial refunds to the orders API", "branch": "feat/partial-refunds", "live": true, "desktop": true, "turns": [
            { "prompt": "Add partial refunds to the orders API. Keep the existing full-refund path working.", "steps": [["read", "src/orders/refunds.ts"], ["edit", "src/orders/routes.ts", "routes = []", "routes = [refundRoute]"], ["run", "npm test -- orders", true], ["edit", "src/orders/refunds.ts", "refund()", "refund(amount?: number)"]], "reply": "Partial refunds work. `POST /orders/:id/refunds` takes an optional amount, and the full-refund path is unchanged. All 48 order tests pass." },
            { "prompt": "Log the refund rules in the wiki.", "steps": [["mcp", "agent-wiki", "wiki_log", { "pages": ["atlas-api", "refunds"] }], ["run", "npm run lint", true]], "reply": "Logged the refund rules on the atlas-api and refunds pages." }
        ] }),
        json!({ "app": "codex", "project": "atlas-api", "title": "Rate-limit the public orders endpoint", "branch": "feat/rate-limit", "live": true, "native": true, "turns": [
            { "prompt": "Rate-limit GET /orders to 60 requests a minute per API key.", "steps": [["read", "src/orders/routes.ts"], ["edit", "src/orders/routes.ts", "routes = [", "routes = [rateLimit(60), "], ["run", "npm test -- rate-limit", true]], "reply": "Added a token-bucket limiter at 60 requests a minute per key. Over the limit, clients get 429 with a Retry-After header." }
        ] }),
        json!({ "app": "claude", "project": "atlas-web", "title": "Move checkout to the new payments SDK", "branch": "feat/payments-v3", "desktop": true, "ask": true, "turns": [
            { "prompt": "Move checkout to version 3 of the payments SDK.", "steps": [["read", "app/checkout/page.tsx"], ["edit", "app/checkout/page.tsx", "payments.v2", "payments.v3"], ["run", "npm run build", true], ["ask", "Version 3 drops saved cards from version 2. Should I migrate them, or ask customers to re-enter them?"]] }
        ] }),
        json!({ "app": "codex", "project": "ledger", "title": "Fix rounding in monthly reconciliation", "branch": "fix/rounding", "live": true, "stuck": true, "turns": [
            { "prompt": "Reconciliation is off by a cent on some months. Find out why and fix it.", "steps": [["read", "ledger/reconcile.py"], ["edit", "ledger/reconcile.py", "round(x, 2)", "Decimal(x).quantize(CENT, ROUND_HALF_EVEN)"], ["run", "pytest tests/test_reconcile.py", false], ["run", "pytest tests/test_reconcile.py", false], ["run", "pytest tests/test_reconcile.py", false]] }
        ] }),
        json!({ "app": "claude", "project": "pathfinder", "title": "Speed up the route solver for 500+ stops", "branch": "perf/solver", "live": true, "desktop": true, "turns": [
            { "prompt": "The solver takes 40 seconds for 500 stops. Get it under 5.", "steps": [["agent", "Profile the solver on the 500-stop fixture"], ["read", "src/solver.rs"], ["edit", "src/solver.rs", "fn solve()", "fn solve_with_cache()"], ["run", "cargo bench solver", true]], "reply": "The solver now caches distance lookups. 500 stops take 3.8 seconds, down from 40." }
        ] }),
        json!({ "app": "codex", "project": "docs", "title": "Write the refunds guide", "review": true, "turns": [
            { "prompt": "Write a guide for the new partial refunds.", "steps": [["read", "guides/refunds.md"], ["edit", "guides/refunds.md", "# Refunds", "# Refunds\n\nRefund all or part of an order."]], "reply": "The refunds guide covers full and partial refunds, with request and response examples." }
        ] }),
        json!({ "app": "claude", "project": "infra", "title": "Move staging to the new cluster", "ago": 190, "desktop": true, "turns": [{ "prompt": "Move staging to the new cluster.", "steps": [["run", "terraform plan -var-file=staging.tfvars", true]], "reply": "Staging runs on the new cluster. The old one can go after a week." }] }),
        json!({ "app": "codex", "project": "atlas-web", "title": "Add product image zoom", "ago": 300, "native": true, "turns": [{ "prompt": "Add zoom on product images.", "steps": [["edit", "app/product/zoom.tsx", "", "export function Zoom() {}"]], "reply": "Product images zoom on hover and pinch." }] }),
        json!({ "app": "claude", "project": "ledger", "title": "Draft the Q4 close checklist", "ago": 420, "turns": [{ "prompt": "Draft a checklist for the Q4 close.", "steps": [["read", "ledger/reconcile.py"]], "reply": "The checklist has 14 steps, grouped by team." }] }),
        json!({ "app": "codex", "cwd": ["Documents", "Codex", today, "regex-help"], "title": "Explain this regex", "ago": 140, "turns": [{ "prompt": "What does ^(?=.*\\d)(?=.*[a-z]).{8,}$ match?", "steps": [], "reply": "It matches strings of eight or more characters with at least one digit and one lowercase letter." }] }),
        json!({ "app": "claude", "project": "atlas-api", "title": "Review the partial refunds change", "ago": 25, "desktop": true, "parent": 0, "turns": [{ "prompt": "Review the partial refunds change before I open a PR.", "steps": [["read", "src/orders/refunds.ts"]], "reply": "Two notes: refunds over the order total aren't rejected yet, and the tests don't cover a second partial refund." }] }),
        json!({ "app": "codex", "project": "pathfinder", "title": "Benchmark solver variants", "ago": 60, "parent": 4, "subagent": true, "turns": [{ "prompt": "Benchmark the cached solver against the old one.", "steps": [["run", "cargo bench", true]], "reply": "The cached solver is 10.5 times faster on 500 stops." }] }),
        json!({ "app": "codex", "project": "labs/geofence", "title": "Draw delivery zones from postcodes", "ago": 35, "turns": [{ "prompt": "Turn the postcode list into delivery zones.", "steps": [["read", "src/zones.ts"], ["edit", "src/zones.ts", "zones = []", "zones = loadPostcodes()"], ["run", "npm test", true]], "reply": "Zones now load from the postcode list. Each one is the convex hull of its postcodes." }] }),
        json!({ "app": "claude", "project": "labs", "title": "Sort out which experiments to keep", "ago": 240, "turns": [{ "prompt": "Which of these experiments are worth keeping?", "steps": [["read", "ideas.md"]], "reply": "Keep geofence and route-sim. Snippets is turning into a tool of its own." }] }),
        // Grok Build's, after the rest so the others keep their ids: one at
        // work in atlas-web, the subagent it runs to check contrast, and an
        // older one in pathfinder.
        json!({ "app": "grok", "project": "atlas-web", "title": "Add dark mode to the storefront", "branch": "feat/dark-mode", "live": true, "turns": [
            { "prompt": "Add a dark mode to the storefront. Follow the system setting, with a toggle in the header.", "steps": [["read", "app/layout.tsx"], ["edit", "app/theme.css", ":root {", ":root, [data-theme=dark] {"], ["run", "npm run build", true]], "reply": "Dark mode follows the system setting, and the header has a toggle that remembers your choice. Every page passes the contrast check." }
        ] }),
        json!({ "app": "grok", "project": "atlas-web", "title": "Check contrast on every page", "ago": 3, "parent": 14, "subagent": true, "turns": [{ "prompt": "Check contrast on every page in dark mode.", "steps": [["run", "npm run test:contrast", true]], "reply": "All 23 pages pass. The sale badge was closest, at 4.6 to 1." }] }),
        json!({ "app": "grok", "project": "pathfinder", "title": "Map the depot loading rules", "ago": 75, "turns": [{ "prompt": "Write down the depot's loading rules as constraints the solver can use.", "steps": [["read", "src/solver.rs"], ["mcp", "agent-wiki", "wiki_log", { "pages": ["pathfinder"] }]], "reply": "The loading rules are on the pathfinder wiki page as seven constraints. Two need data the depot doesn't send yet." }] }),
    ]
}

// Stable ids, so a saved layout survives a restart of the demo.
fn id_of(i: usize) -> String {
    format!("0190d3e0-0000-7000-8000-{:012}", i + 1)
}

fn desktop_id_of(i: usize) -> String {
    format!("local_0190d3e0-0000-7000-9000-{:012}", i + 1)
}

fn line(o: &Value) -> String {
    format!("{}\n", stringify(o, 0))
}

fn n(x: f64) -> Value {
    xenonet_base::json::number(x)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn flag(v: &Value, k: &str) -> bool {
    crate::jsv::truthy(v.get(k))
}

/// Where the made-up world lives.
#[derive(Clone, Debug)]
pub struct World {
    pub root: String,
    pub home: String,
    pub projects_dir: String,
    pub clients_dir: String,
    pub claude_root: String,
    pub claude_desktop_dir: String,
    pub codex_home: String,
    pub grok_home: String,
    pub wiki_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
}

impl World {
    fn at(root: &str) -> Self {
        let home = NATIVE.join(&[root, "home"]);
        World {
            root: root.into(),
            projects_dir: NATIVE.join(&[&home, "Projects"]),
            clients_dir: NATIVE.join(&[&home, "Clients"]),
            claude_root: NATIVE.join(&[&home, ".claude", "projects"]),
            claude_desktop_dir: NATIVE.join(&[&home, "claude-desktop"]),
            codex_home: NATIVE.join(&[&home, ".codex"]),
            grok_home: NATIVE.join(&[&home, ".grok"]),
            wiki_dir: NATIVE.join(&[&home, "wiki"]),
            state_dir: NATIVE.join(&[root, "state"]),
            cache_dir: NATIVE.join(&[root, "cache"]),
            home,
        }
    }

    pub fn to_value(&self) -> Value {
        json!({ "root": self.root, "home": self.home, "projectsDir": self.projects_dir, "clientsDir": self.clients_dir, "claudeRoot": self.claude_root, "claudeDesktopDir": self.claude_desktop_dir, "codexHome": self.codex_home, "grokHome": self.grok_home, "wikiDir": self.wiki_dir, "stateDir": self.state_dir, "cacheDir": self.cache_dir })
    }
}

/// A session's transcript as it's written.
struct Writer {
    s: Value,
    app: String,
    id: String,
    cwd: String,
    file: String,
    // A Grok Build session's folder, and when its summary says it started.
    dir: String,
    started: Option<f64>,
    ctx: f64,
    tokens: f64,
    turn: usize,
    step: i64,
    entries: Vec<Value>,
}

impl Writer {
    fn new(s: &Value, i: usize, world: &World, around: &mut dyn Around) -> Result<Self, fsx::FsError> {
        let app = s_of(s, "app");
        let id = id_of(i);
        let cwd = cwd_of(s, &world.home);
        let now = around.now();
        let mut dir = String::new();
        let file = if app == "claude" {
            NATIVE.join(&[&world.claude_root, &format!("demo-{}", NATIVE.basename(&cwd, None)), &format!("{id}.jsonl")])
        } else if app == "grok" {
            // As Grok Build keeps a session: a folder of its own, in its
            // working folder's group, with its summary and its updates.
            dir = NATIVE.join(&[&world.grok_home, "sessions", &encode_uri_component(&cwd), &id]);
            NATIVE.join(&[&dir, "updates.jsonl"])
        } else {
            let [y, m, d, _, _] = around.local(now);
            let day = NATIVE.join(&[&world.codex_home, "sessions", &y.to_string(), &format!("{m:02}"), &format!("{d:02}")]);
            NATIVE.join(&[&day, &format!("rollout-{}-{id}.jsonl", iso(now)[..19].replace(':', "-"))])
        };
        fsx::mkdir_p(&NATIVE.dirname(&file), None)?;
        Ok(Writer { s: s.clone(), app, id, cwd, file, dir, started: None, ctx: 18000.0 + i as f64 * 2100.0, tokens: 0.0, turn: 0, step: -1, entries: Vec::new() })
    }

    fn write(&self, objs: &[Value]) {
        use std::io::Write;
        let text: String = objs.iter().map(line).collect();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.file) {
            let _ = f.write_all(text.as_bytes());
        }
    }

    fn usage(&mut self, a: &mut dyn Around) -> Value {
        self.ctx += 1800.0 + (a.random() * 2600.0).floor();
        let out = 300.0 + (a.random() * 900.0).floor();
        self.tokens += self.ctx + out;
        json!({ "input_tokens": 40, "cache_read_input_tokens": n(self.ctx - 40.0), "cache_creation_input_tokens": 0, "output_tokens": n(out) })
    }

    fn codex_tokens(&mut self, ts: f64, a: &mut dyn Around) {
        let u = self.usage(a);
        let used = (34.0 + self.tokens / 60000.0).min(92.0);
        let reset = (ts / 1000.0).floor() + 2.4 * 3600.0;
        let out = u["output_tokens"].as_f64().unwrap_or(0.0);
        // As Codex writes it: cached input is part of input_tokens.
        let last = json!({ "input_tokens": n(self.ctx), "cached_input_tokens": u["cache_read_input_tokens"], "output_tokens": n(out), "reasoning_output_tokens": n(xenonet_base::units::js_round(out * 0.4)), "total_tokens": n(self.ctx + out) });
        self.write(&[json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "token_count", "info": { "total_token_usage": { "total_tokens": n(self.tokens), "output_tokens": n(out) }, "last_token_usage": last, "model_context_window": 258400 }, "rate_limits": { "primary": { "used_percent": n(xenonet_base::units::js_round(used)), "window_minutes": 300, "resets_at": n(reset) }, "secondary": { "used_percent": 58, "window_minutes": 10080, "resets_at": n(reset + 3.0 * 86400.0) } } } })]);
    }

    fn turns(&self) -> Vec<Value> {
        self.s.get("turns").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    /// Writes the next step of the script at `ts`; false when the session
    /// has nothing more to do this turn (a question waiting, or a loop).
    fn next(&mut self, ts: f64, a: &mut dyn Around) -> bool {
        let turns = self.turns();
        if turns.is_empty() {
            return false;
        }
        let turn = turns[self.turn % turns.len()].clone();
        let steps = turn.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
        if self.step == -1 {
            self.prompt(s(&turn, "prompt"), ts, a);
            self.step = 0;
            return true;
        }
        if (self.step as usize) < steps.len() {
            self.act(&steps[self.step as usize], ts, a);
            self.step += 1;
            return true;
        }
        let Some(reply) = turn.get("reply").and_then(Value::as_str).filter(|r| !r.is_empty()) else {
            // Scripts without a reply loop on their last step (the stuck
            // session) or wait on their question.
            if let Some(last) = steps.last() {
                if last.get(0).and_then(Value::as_str) == Some("run") {
                    self.act(last, ts, a);
                }
            }
            return false;
        };
        self.reply(reply, ts, a);
        self.turn += 1;
        self.step = -1;
        true
    }

    fn prompt(&mut self, text: &str, ts: f64, a: &mut dyn Around) {
        self.entries.push(json!({ "id": a.uuid(), "ts": n(ts), "role": "user", "text": text }));
        if self.app == "claude" {
            return self.write(&[json!({ "type": "user", "timestamp": iso(ts), "message": { "role": "user", "content": text } })]);
        }
        if self.app == "grok" {
            return self.grok(ts, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": text } }));
        }
        self.write(&[
            json!({ "type": "turn_context", "timestamp": iso(ts), "payload": { "model": MODEL_CODEX } }),
            json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "task_started", "model_context_window": 258400 } }),
            json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "item_completed", "item": { "type": "UserMessage", "content": [{ "type": "text", "text": text }] } } }),
        ]);
    }

    fn reply(&mut self, text: &str, ts: f64, a: &mut dyn Around) {
        self.entries.push(json!({ "id": a.uuid(), "ts": n(ts), "role": "assistant", "text": text }));
        if self.app == "claude" {
            // In the JS's order: the message's id, then its usage.
            let id = format!("msg_{}", a.uuid());
            let usage = self.usage(a);
            return self.write(&[json!({ "type": "assistant", "timestamp": iso(ts), "message": { "id": id, "model": MODEL_CLAUDE, "stop_reason": "end_turn", "usage": usage, "content": [{ "type": "text", "text": text }] } })]);
        }
        if self.app == "grok" {
            self.grok(ts, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } }));
            self.usage(a);
            self.grok(ts, json!({ "sessionUpdate": "usage_update", "used": n(self.ctx), "size": GROK_WINDOW }));
            return self.grok_summary(ts);
        }
        self.codex_tokens(ts, a);
        self.write(&[json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "task_complete", "last_agent_message": text, "duration_ms": 95000 } })]);
    }

    fn act(&mut self, step: &Value, ts: f64, around: &mut dyn Around) {
        let at = |i: usize| step.get(i).cloned();
        let kind = at(0).and_then(|k| k.as_str().map(String::from)).unwrap_or_default();
        let a = at(1);
        let a_s = a.as_ref().map_or_else(|| "undefined".into(), xenonet_base::text::js_string);
        let b = at(2);
        let b_s = b.as_ref().map_or_else(|| "undefined".into(), xenonet_base::text::js_string);
        let c = at(3);
        let c_s = c.as_ref().map_or_else(|| "undefined".into(), xenonet_base::text::js_string);
        let file = matches!(kind.as_str(), "read" | "edit").then(|| NATIVE.join(&[&self.cwd, &a_s]));
        let title = match kind.as_str() {
            "read" => format!("Read {a_s}"),
            "edit" => format!("Edit {a_s}"),
            "run" | "agent" => a_s.clone(),
            "mcp" => format!("{a_s}.{b_s}"),
            "ask" => "AskUserQuestion".into(),
            _ => "undefined".into(),
        };
        let text = match kind.as_str() {
            "edit" => format!("- {b_s}\n+ {c_s}"),
            "ask" => a_s.clone(),
            _ => String::new(),
        };
        let failed = kind == "run" && b == Some(Value::Bool(false));
        self.entries.push(
            json!({ "id": around.uuid(), "ts": n(ts), "role": "tool", "kind": "tool_use", "title": title, "text": text, "status": if failed { "failed" } else { "completed" } }),
        );
        if self.app == "claude" {
            return self.claude_act(&kind, &a_s, b.as_ref(), c.as_ref(), file.as_deref(), ts, around);
        }
        if self.app == "grok" {
            return self.grok_act(&kind, (&a_s, &b_s), b.as_ref(), c.as_ref(), file.as_deref(), ts, around);
        }
        match kind.as_str() {
            "read" => self.codex_tokens(ts, around),
            "edit" => {
                let mut changes = Map::new();
                changes.insert(file.unwrap_or_default(), json!({ "type": "update", "unified_diff": format!("@@ -1 +1 @@\n-{b_s}\n+{c_s}") }));
                self.write(&[json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "item_completed", "item": { "type": "FileChange", "status": "completed", "changes": changes } } })]);
            }
            "run" => {
                self.codex_tokens(ts, around);
                let ok = crate::jsv::truthy(b.as_ref());
                self.write(&[json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "item_completed", "item": { "type": "CommandExecution", "command": ["pwsh", "-Command", a.clone().unwrap_or(Value::Null)], "exit_code": i32::from(!ok), "status": if ok { "completed" } else { "failed" }, "aggregatedOutput": if ok { "ok" } else { "1 failed, 23 passed" } } } })]);
            }
            "mcp" => self.write(&[json!({ "type": "event_msg", "timestamp": iso(ts), "payload": { "type": "item_completed", "item": { "type": "McpToolCall", "server": a.clone().unwrap_or(Value::Null), "tool": b.clone().unwrap_or(Value::Null), "arguments": c.clone().unwrap_or(Value::Null), "status": "completed" } } })]),
            _ => {}
        }
    }

    #[expect(clippy::too_many_arguments, reason = "a script step's kind and arguments, as next took them apart, with its file, time and around")]
    fn claude_act(&mut self, kind: &str, a: &str, b: Option<&Value>, c: Option<&Value>, file: Option<&str>, ts: f64, around: &mut dyn Around) {
        let call_id = format!("toolu_{}", &around.uuid().replace('-', "")[..24]);
        let (name, input) = match kind {
            "read" => ("Read".to_string(), json!({ "file_path": file })),
            "edit" => ("Edit".to_string(), json!({ "file_path": file, "old_string": b, "new_string": c })),
            "run" => ("Bash".to_string(), json!({ "command": a })),
            "mcp" => {
                (format!("mcp__{a}__{}", b.map_or_else(|| "undefined".into(), xenonet_base::text::js_string)), c.filter(|x| !x.is_null()).cloned().unwrap_or_else(|| json!({})))
            }
            "agent" => ("Task".to_string(), json!({ "description": a, "prompt": a })),
            _ => ("AskUserQuestion".to_string(), json!({ "questions": [{ "question": a }] })),
        };
        let id = format!("msg_{}", around.uuid());
        let usage = self.usage(around);
        self.write(&[json!({ "type": "assistant", "timestamp": iso(ts), "message": { "id": id, "model": MODEL_CLAUDE, "stop_reason": "tool_use", "usage": usage, "content": [{ "type": "tool_use", "id": call_id, "name": name, "input": input }] } })]);
        // A question stays pending: the session needs you.
        if kind == "ask" {
            return;
        }
        if kind == "agent" {
            self.subagent(a, ts, around);
        }
        let ok = kind != "run" || b != Some(&Value::Bool(false));
        self.write(&[json!({ "type": "user", "timestamp": iso(ts + 4000.0), "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": call_id, "is_error": !ok, "content": if ok { "ok" } else { "1 failed" } }] } })]);
    }

    // ----- Grok Build: session updates in their envelope, and a summary -----

    fn grok(&self, ts: f64, update: Value) {
        self.write(&[json!({ "timestamp": n((ts / 1000.0).floor()), "method": "session/update", "params": { "sessionId": self.id, "update": update } })]);
    }

    /// A tool call as Grok Build writes it, named as it names its own tools
    /// (an MCP tool by its title), and how it went. Its scripts read, edit,
    /// run and call MCP tools. `names` are the step's first two arguments
    /// as text.
    #[expect(clippy::too_many_arguments, reason = "a script step's kind and arguments, as next took them apart, with its file, time and around")]
    fn grok_act(&mut self, kind: &str, names: (&str, &str), b: Option<&Value>, c: Option<&Value>, file: Option<&str>, ts: f64, around: &mut dyn Around) {
        let (a, b_s) = names;
        let own = |name: &str, k: &str| json!({ "x.ai/tool": { "version": 1, "name": name, "kind": k } });
        let or_null = |v: Option<&Value>| v.cloned().unwrap_or(Value::Null);
        let (title, call_kind, raw, locations, meta) = match kind {
            "read" => (format!("Read {a}"), "read", json!({ "target_file": file }), Some(json!([{ "path": file }])), Some(own("read_file", "read"))),
            "edit" => (
                format!("Edit {a}"),
                "edit",
                json!({ "file_path": file, "old_string": or_null(b), "new_string": or_null(c) }),
                Some(json!([{ "path": file }])),
                Some(own("search_replace", "edit")),
            ),
            "run" => (a.to_string(), "execute", json!({ "command": a }), None, Some(own("run_terminal_cmd", "execute"))),
            "mcp" => (format!("{a}__{b_s}"), "other", c.filter(|x| !x.is_null()).cloned().unwrap_or_else(|| json!({})), None, None),
            _ => return,
        };
        let id = format!("call_{}", &around.uuid()[..8]);
        let mut call = Map::new();
        call.insert("sessionUpdate".into(), json!("tool_call"));
        call.insert("toolCallId".into(), json!(id));
        call.insert("status".into(), json!("pending"));
        call.insert("title".into(), json!(title));
        call.insert("kind".into(), json!(call_kind));
        call.insert("rawInput".into(), raw);
        if let Some(l) = locations {
            call.insert("locations".into(), l);
        }
        if let Some(m) = meta {
            call.insert("_meta".into(), m);
        }
        self.grok(ts, Value::Object(call));
        let ok = kind != "run" || b != Some(&Value::Bool(false));
        let content = if kind == "edit" {
            json!([{ "type": "diff", "path": file, "oldText": or_null(b), "newText": or_null(c) }])
        } else {
            json!([{ "type": "content", "content": { "type": "text", "text": if ok { "ok" } else { "1 failed" } } }])
        };
        self.grok(ts + 3000.0, json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": if ok { "completed" } else { "failed" }, "content": content }));
    }

    /// Its summary beside the updates, written when the session starts and
    /// after each turn. A subagent's names the session that runs it.
    fn grok_summary(&mut self, ts: f64) {
        let started = *self.started.get_or_insert(ts);
        let mut summary = Map::new();
        summary.insert("info".into(), json!({ "id": self.id, "cwd": self.cwd }));
        summary.insert("session_summary".into(), json!(""));
        summary.insert("generated_title".into(), self.s.get("title").cloned().unwrap_or(Value::Null));
        summary.insert("title_is_manual".into(), json!(false));
        summary.insert("created_at".into(), json!(iso(started)));
        summary.insert("updated_at".into(), json!(iso(ts)));
        summary.insert("current_model_id".into(), json!(MODEL_GROK));
        summary.insert("head_branch".into(), self.s.get("branch").cloned().unwrap_or(Value::Null));
        if let Some(p) = self.s.get("parent").and_then(Value::as_u64) {
            summary.insert("parent_session_id".into(), json!(id_of(p as usize)));
            summary.insert("session_kind".into(), json!(if flag(&self.s, "subagent") { "subagent" } else { "fork" }));
        }
        let _ = fsx::write_file(&NATIVE.join(&[&self.dir, "summary.json"]), format!("{}\n", stringify(&Value::Object(summary), 2)).as_bytes(), None);
    }

    fn subagent(&self, task: &str, ts: f64, around: &mut dyn Around) {
        let file = NATIVE.join(&[&NATIVE.dirname(&self.file), &self.id, "subagents", &format!("agent-{}.jsonl", &around.uuid()[..8])]);
        let _ = fsx::mkdir_p(&NATIVE.dirname(&file), None);
        let u = json!({ "input_tokens": 30, "cache_read_input_tokens": 9000, "cache_creation_input_tokens": 0, "output_tokens": 400 });
        let msg = format!("msg_{}", around.uuid());
        let text = line(&json!({ "type": "user", "timestamp": iso(ts), "isSidechain": true, "message": { "role": "user", "content": task } }))
            + &line(
                &json!({ "type": "assistant", "timestamp": iso(ts + 2000.0), "isSidechain": true, "message": { "id": msg, "model": MODEL_CLAUDE, "stop_reason": "tool_use", "usage": u, "content": [{ "type": "tool_use", "id": "toolu_sub1", "name": "Bash", "input": { "command": "cargo flamegraph --bench solver" } }] } }),
            );
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

fn s_of(v: &Value, k: &str) -> String {
    s(v, k).to_string()
}

fn cwd_of(s: &Value, home: &str) -> String {
    match s.get("cwd").and_then(Value::as_array) {
        Some(parts) => {
            let parts: Vec<String> = parts.iter().map(xenonet_base::text::js_string).collect();
            let mut all = vec![home];
            all.extend(parts.iter().map(String::as_str));
            NATIVE.join(&all)
        }
        None => NATIVE.join(&[home, "Projects", self::s(s, "project")]),
    }
}

/// A repo of the second folder, made the first time something asks for
/// that folder: its folder, and what it changes without committing.
#[derive(Clone, Debug)]
pub struct ClientRepo {
    pub dir: String,
    pub dirty: Vec<(String, String)>,
}

/// The demo: its world, and its sessions' writers once it's made.
pub struct Demo {
    pub world: World,
    today: String,
    sessions: Vec<Value>,
    writers: Vec<Writer>,
    pub client_repos: Vec<ClientRepo>,
}

const CLAUDE_PERSONAL: (&str, &str) = ("0190d3e0-0000-7000-a000-000000000001", "0190d3e0-0000-7000-b000-000000000001");
const CLAUDE_WORK: (&str, &str) = ("0190d3e0-0000-7000-a000-000000000002", "0190d3e0-0000-7000-b000-000000000002");
const ATLAS: [&str; 4] = ["atlas-api", "atlas-web", "docs", "infra"];

/// The Claude account a project's desktop sessions run as: Nova's work
/// account for Atlas's, else the personal one.
pub fn demo_claude_account(project: Option<&str>) -> Value {
    let (account, org) = if project.is_some_and(|p| ATLAS.contains(&p)) { CLAUDE_WORK } else { CLAUDE_PERSONAL };
    json!({ "account": account, "org": org })
}

// Each repo's origin, and the email its commits get when it isn't the
// global one. pathfinder pushes over HTTPS with no user name set, so it
// signs in as gh's active account, nova-dev, not orbitworks, and XenoNet
// flags it.
const REMOTES: [(&str, &str, Option<&str>); 8] = [
    ("atlas-api", "https://github.com/atlas-co/atlas-api.git", Some("nova@atlas.example")),
    ("atlas-web", "https://github.com/atlas-co/atlas-web.git", Some("nova@atlas.example")),
    ("docs", "https://github.com/atlas-co/docs.git", Some("nova@atlas.example")),
    ("infra", "git@github.com:atlas-co/infra.git", Some("nova@atlas.example")),
    ("ledger", "https://github.com/nova-dev/ledger.git", None),
    ("pathfinder", "https://github.com/orbitworks/pathfinder.git", Some("hello@orbitworks.example")),
    ("labs/geofence", "https://github.com/orbitworks/geofence.git", None),
    ("labs/route-sim", "git@gh-orbit:orbitworks/route-sim.git", None),
];

impl Demo {
    /// One world per port, so demo servers running side by side don't
    /// wipe each other's. `today` is the local date, as en-CA writes it.
    pub fn new(tmpdir: &str, port: &str, today: &str) -> Self {
        let root = NATIVE.join(&[tmpdir, &format!("xenonet-demo-{port}")]);
        Demo { world: World::at(&root), today: today.into(), sessions: sessions(today), writers: Vec::new(), client_repos: Vec::new() }
    }

    fn p(&self, rel: &str) -> String {
        NATIVE.join(&[&self.world.home, "Projects", rel])
    }

    // Writes each project's files, and returns the ones that are repos.
    fn write_files(list: &[Project], at: &dyn Fn(&str) -> String, git_ok: bool) -> Result<Vec<ClientRepo>, fsx::FsError> {
        for p in list {
            let dir = at(p.name);
            let readme = format!("# {}\n\n{}\n", NATIVE.basename(p.name, None), p.readme);
            let mut files: Vec<(&str, &str)> = vec![("README.md", readme.as_str())];
            files.extend(p.files.iter().copied());
            for (rel, text) in files {
                let f = NATIVE.join(&[&dir, rel]);
                fsx::mkdir_p(&NATIVE.dirname(&f), None)?;
                fsx::write_file(&f, text.as_bytes(), None)?;
            }
        }
        if !git_ok {
            return Ok(Vec::new());
        }
        Ok(list.iter().filter(|p| p.git).map(|p| ClientRepo { dir: at(p.name), dirty: p.dirty.iter().map(|(r, t)| (r.to_string(), t.to_string())).collect() }).collect())
    }

    /// Builds the demo workspace from scratch. Synchronous, so config can
    /// point at it before anything else starts.
    pub fn create_world(&mut self, now: f64, around: &mut dyn Around, platform: &str) -> Result<Value, fsx::FsError> {
        let root = fresh_root(&self.world.root, platform)?;
        if root != self.world.root {
            self.world = World::at(&root);
        }
        fsx::mkdir_p(&self.world.projects_dir, None)?;
        let git_ok = around.git_available();
        let home = self.world.home.clone();
        let projects = |rel: &str| NATIVE.join(&[&home, "Projects", rel]);
        for repo in Self::write_files(PROJECTS, &projects, git_ok)? {
            let ok = git(&repo.dir, &["init", "-q", "-b", "main"]) && git(&repo.dir, &["add", "-A"]) && git(&repo.dir, &["commit", "-q", "-m", "chore: initial commit"]);
            // No git: the projects still show, without repo status.
            if ok {
                for (rel, text) in &repo.dirty {
                    let _ = fsx::write_file(&NATIVE.join(&[&repo.dir, rel]), text.as_bytes(), None);
                }
            }
        }
        let clients = |rel: &str| NATIVE.join(&[&home, "Clients", rel]);
        self.client_repos = Self::write_files(CLIENTS, &clients, git_ok)?;
        self.sessions = sessions(&self.today);
        self.write_history(now, around)?;
        self.write_wiki(now, around)?;
        self.write_claude_limits(now)?;
        self.write_identities(now, around, git_ok)?;
        Ok(self.world.to_value())
    }

    // History: every session gets its past turns, spread over the hours
    // before now, so token totals, the replay and the day's log have
    // something in them.
    fn write_history(&mut self, now: f64, around: &mut dyn Around) -> Result<(), fsx::FsError> {
        let mut writers = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            writers.push(Writer::new(s, i, &self.world, around)?);
        }
        for w in &mut writers {
            let s = w.s.clone();
            // A rollout says the Codex that wrote it, as real ones do.
            if w.app == "codex" {
                w.write(&[json!({ "type": "session_meta", "timestamp": iso(now - 8.0 * 3600e3), "payload": { "id": w.id, "cwd": w.cwd, "cli_version": "0.150.0" } })]);
            }
            // Live sessions start a little in the past and catch up to now.
            let live = flag(&s, "live");
            let end = if live || flag(&s, "ask") {
                now - MIN
            } else if flag(&s, "review") {
                now - 9.0 * MIN
            } else {
                now - s.get("ago").and_then(Value::as_f64).unwrap_or(60.0) * MIN
            };
            let per: usize = w.turns().iter().map(|t| t.get("steps").and_then(Value::as_array).map_or(0, Vec::len) + 2).sum();
            let steps = per * if live { 2 } else { 1 };
            let mut ts = end - steps as f64 * 50e3;
            if w.app == "grok" {
                w.grok_summary(ts);
            }
            for _ in 0..steps {
                if !w.next(ts, around) && !flag(&s, "stuck") {
                    break;
                }
                ts += 50e3;
            }
        }
        self.writers = writers;
        Ok(())
    }

    fn write_wiki(&self, now: f64, around: &mut dyn Around) -> Result<(), fsx::FsError> {
        let wiki = &self.world.wiki_dir;
        let page = |slug: &str, title: &str, body: &str, tags: &[&str]| {
            fsx::write_file(
                &NATIVE.join(&[wiki, "pages", &format!("{slug}.md")]),
                format!("---\ntitle: \"{title}\"\ntype: project\ntags: {}\nupdated: {}\n---\n\n{body}\n", stringify(&json!(tags), 0), iso(now - 30.0 * MIN)).as_bytes(),
                None,
            )
        };
        let [year, ..] = around.local(now);
        for d in ["pages".to_string(), "inbox".to_string(), NATIVE.join(&["log", &year.to_string()])] {
            fsx::mkdir_p(&NATIVE.join(&[wiki, &d]), None)?;
        }
        page("atlas-api", "Atlas API", "Orders, inventory and payments. Refund rules live in [[refunds]]. Runs on [[staging-cluster]].", &["atlas-api"])?;
        page("refunds", "Refunds", "Full and partial refunds. See [[atlas-api]] and the guide in Projects/docs.", &[])?;
        page("staging-cluster", "Staging cluster", "Moved to the new cluster this week. Deploys come from Projects/infra.", &["infra"])?;
        page("ledger", "Ledger", "Monthly reconciliation rounds half-even. See [[q4-close]].", &["ledger"])?;
        page("q4-close", "Q4 close", "The checklist for closing the year in [[ledger]].", &[])?;
        let t = now - 40.0 * MIN;
        let [y, m, d, h, min] = around.local(t);
        fsx::write_file(
            &NATIVE.join(&[wiki, "log", &y.to_string(), &format!("{y:04}-{m:02}-{d:02}.md")]),
            format!("## {h}:{min:02} · claude-code · Recorded the refund rules\n\npages: [[refunds]], [[atlas-api]]\n").as_bytes(),
            None,
        )?;
        fsx::write_file(&NATIVE.join(&[wiki, "inbox", "note-1.md"]), b"---\ntitle: Solver benchmark results\napp: codex\n---\n", None)
    }

    fn write_claude_limits(&self, now: f64) -> Result<(), fsx::FsError> {
        fsx::mkdir_p(&self.world.claude_desktop_dir, None)?;
        let samples: Vec<Value> = (0..30)
            .map(|k| json!({ "t": n(now - f64::from(29 - k) * 10.0 * MIN), "u": { "fh": n((12.0 + f64::from(k) * 2.1).min(95.0)), "sd": n(44.0 + f64::from(k) * 0.4) } }))
            .collect();
        fsx::write_file(&NATIVE.join(&[&self.world.claude_desktop_dir, "plan-usage-history.json"]), stringify(&json!({ "samples": samples }), 0).as_bytes(), None)
    }

    // Who the demo's work runs as, all made up. Nova works at Atlas (the
    // atlas-co organization on GitHub, a work Claude account in the Atlas
    // team) and has a side identity, orbitworks, for pathfinder and the
    // experiments in labs. XenoNet's state remembers the work Claude
    // account from before Nova signed in to the personal one.
    fn write_identities(&self, now: f64, around: &mut dyn Around, git_ok: bool) -> Result<(), fsx::FsError> {
        let home_file = |rel: &str, text: &str| -> Result<(), fsx::FsError> {
            let f = NATIVE.join(&[&self.world.home, rel]);
            fsx::mkdir_p(&NATIVE.dirname(&f), None)?;
            fsx::write_file(&f, text.as_bytes(), None)
        };
        home_file(
            ".claude.json",
            &stringify(
                &json!({ "oauthAccount": { "accountUuid": CLAUDE_PERSONAL.0, "organizationUuid": CLAUDE_PERSONAL.1, "emailAddress": "nova@example.com", "organizationName": "nova@example.com's Organization", "organizationType": "claude_max" } }),
                0,
            ),
        )?;
        // An ID token in the shape Codex keeps, unsigned, and no real tokens.
        let jwt = |claims: &Value| format!("e30.{}.demo", b64url(stringify(claims, 0).as_bytes()));
        let signed_in = ((now - 20.0 * 86400e3) / 1000.0).floor();
        home_file(
            &NATIVE.join(&[".codex", "auth.json"]),
            &stringify(
                &json!({ "auth_mode": "chatgpt", "OPENAI_API_KEY": null, "tokens": { "id_token": jwt(&json!({ "email": "nova@example.com", "auth_time": n(signed_in), "https://api.openai.com/auth": { "chatgpt_account_id": "demo-workspace", "chatgpt_plan_type": "plus" } })), "access_token": "demo", "refresh_token": "demo", "account_id": "demo-workspace" } }),
                0,
            ),
        )?;
        home_file(&NATIVE.join(&[".config", "gh", "hosts.yml"]), "github.com:\n    git_protocol: https\n    users:\n        nova-dev:\n        orbitworks:\n    user: nova-dev\n")?;
        home_file(&NATIVE.join(&[".ssh", "config"]), "Host gh-orbit\n    HostName github.com\n    IdentityFile ~/.ssh/orbitworks\n")?;
        home_file(".gitconfig", &format!("[user]\n\tname = Nova\n\temail = nova@example.com\n[credential \"https://github.com\"]\n\thelper = !gh auth git-credential\n[includeIf \"gitdir:{}/\"]\n\tpath = ~/.gitconfig-orbit\n", self.p("labs").replace('\\', "/")))?;
        home_file(".gitconfig-orbit", "[user]\n\tname = Orbit\n\temail = hello@orbitworks.example\n[credential \"https://github.com\"]\n\tusername = orbitworks\n")?;
        if git_ok {
            for (rel, origin, email) in REMOTES {
                let dir = self.p(rel);
                if !git(&dir, &["remote", "add", "origin", origin]) {
                    continue;
                }
                if let Some(e) = email {
                    if !git(&dir, &["config", "user.email", e]) {
                        continue;
                    }
                    if e.ends_with("orbitworks.example") {
                        git(&dir, &["config", "user.name", "Orbit"]);
                    }
                }
            }
        }
        // What XenoNet would remember: the names Nova gave, and the work
        // account signed in until this morning.
        let work = around.claude_id(CLAUDE_WORK.0, CLAUDE_WORK.1);
        let personal = around.claude_id(CLAUDE_PERSONAL.0, CLAUDE_PERSONAL.1);
        let mut names = Map::new();
        names.insert(around.github_id("github.com", "atlas-co"), json!("work"));
        names.insert(around.git_id("nova@atlas.example"), json!("work"));
        names.insert(work.clone(), json!("work"));
        names.insert(around.github_id("github.com", "orbitworks"), json!("orbit"));
        names.insert(around.git_id("hello@orbitworks.example"), json!("orbit"));
        let mut known = Map::new();
        known.insert(work.clone(), json!({ "kind": "claude", "handle": "nova@atlas.example", "detail": { "plan": "Team", "org": "Atlas" }, "firstSeen": n(now - 30.0 * 86400e3), "lastSeen": n(now - 8.0 * 3600e3) }));
        let signins = json!({ "claude": [{ "id": work, "from": n(now - 30.0 * 86400e3), "to": n(now - 8.0 * 3600e3) }, { "id": personal, "from": n(now - 8.0 * 3600e3), "to": n(now - 3600e3) }] });
        fsx::mkdir_p(&self.world.state_dir, None)?;
        fsx::write_file(
            &NATIVE.join(&[&self.world.state_dir, "state.json"]),
            stringify(&json!({ "identities": { "names": names, "known": known, "signins": signins } }), 2).as_bytes(),
            None,
        )
    }

    /// The sessions of an app as its adapter would return them.
    pub fn sessions_of(&self, app: &str) -> Result<Vec<Value>, fsx::FsError> {
        let mut out = Vec::new();
        for (i, s) in self.sessions.iter().enumerate() {
            if s_of(s, "app") != app {
                continue;
            }
            let Some(w) = self.writers.get(i) else { return Err(fsx::said("Cannot read properties of undefined (reading 'file')")) };
            let meta = std::fs::metadata(&w.file).map_err(|e| fsx::FsError::of(e, "stat", &w.file))?;
            let updated = xenonet_base::fs::mtime_ms_of(&meta).unwrap_or(0.0);
            let parent = s.get("parent").and_then(Value::as_u64).map(|p| format!("{}:{}", s_of(&self.sessions[p as usize], "app"), id_of(p as usize)));
            let subagent = flag(s, "subagent");
            let desktop = flag(s, "desktop");
            out.push(json!({
                "id": format!("{app}:{}", id_of(i)),
                "app": app,
                "rawId": id_of(i),
                "desktopId": if desktop { Some(desktop_id_of(i)) } else { None },
                "inDesktop": desktop,
                "title": s.get("title"),
                "preview": s["turns"][0]["prompt"],
                "cwd": w.cwd,
                "worktree": null,
                "createdAt": n(updated - 3.0 * 3600e3),
                "updatedAt": n(updated),
                "archived": false,
                "starred": i == 0,
                "model": model(app),
                "branch": s.get("branch").cloned().unwrap_or(Value::Null),
                "status": null,
                // A subagent's session says it's one as its app does.
                "source": if subagent { json!(if app == "grok" { "subagent" } else { "subAgent" }) } else { Value::Null },
                "nativeProjectId": if flag(s, "native") { json!("demo-atlas") } else { Value::Null },
                "parentId": parent,
                "parentKind": parent.as_ref().map(|_| if subagent { "subagent" } else { "spawn" }),
                "claudeAccount": if app == "claude" && desktop { demo_claude_account(s.get("project").and_then(Value::as_str)) } else { Value::Null },
            }));
        }
        Ok(out)
    }

    /// Each session's id, app, title and project, without the world.
    pub fn session_list(&self) -> Vec<Value> {
        self.sessions.iter().enumerate().map(|(i, s)| json!({ "id": format!("{}:{}", s_of(s, "app"), id_of(i)), "app": s.get("app"), "title": s.get("title"), "project": s.get("project").cloned().unwrap_or(Value::Null) })).collect()
    }

    pub fn native_projects(&self) -> Value {
        json!([{ "id": "demo-atlas", "name": "Atlas", "roots": [self.p("atlas-api"), self.p("atlas-web")] }])
    }

    /// A session's transcript as its preview shows it: the last 120 entries.
    pub fn transcript(&self, raw_id: &str) -> Result<Value, String> {
        let i = (0..self.sessions.len()).find(|i| id_of(*i) == raw_id);
        let Some(w) = i.and_then(|i| self.writers.get(i)) else { return Err("unknown demo session".into()) };
        let from = w.entries.len().saturating_sub(120);
        Ok(json!({ "entries": w.entries[from..], "cursor": null, "total": w.entries.len() }))
    }

    /// How many live sessions keep moving.
    pub fn live_count(&self) -> usize {
        self.writers.iter().filter(|w| flag(&w.s, "live")).count()
    }

    /// The k-th step of the round robin over the live sessions, at `now`.
    pub fn step_live(&mut self, k: usize, now: f64, around: &mut dyn Around) {
        let live: Vec<usize> = (0..self.writers.len()).filter(|i| flag(&self.writers[*i].s, "live")).collect();
        if live.is_empty() {
            return;
        }
        let i = live[k % live.len()];
        self.writers[i].next(now, around);
    }

    pub fn today(&self) -> &str {
        &self.today
    }
}

// The old workspace taken out, or (held open on Windows) a fresh one made
// beside it.
fn fresh_root(root: &str, platform: &str) -> Result<String, fsx::FsError> {
    let mut attempt = 0;
    let e = loop {
        match fsx::rm_rf(root) {
            Ok(()) => return Ok(root.into()),
            Err(e) => {
                let retry = matches!(e.code.as_deref(), Some("EBUSY" | "EMFILE" | "ENFILE" | "ENOTEMPTY" | "EPERM"));
                if !retry || attempt == crate::demo_root::REMOVE_RETRIES {
                    break e;
                }
                std::thread::sleep(std::time::Duration::from_millis(u64::from(crate::demo_root::REMOVE_RETRY_DELAY_MS) * u64::from(attempt + 1)));
                attempt += 1;
            }
        }
    };
    if !crate::demo_root::use_sibling(platform, e.code.as_deref()) {
        return Err(e);
    }
    fsx::mkdtemp(&crate::demo_root::sibling_prefix(root))
}

/// What "Ask Codex which folders are projects" decides in the demo.
pub fn demo_picks(outline: &Value) -> Value {
    let hit = outline
        .as_array()
        .is_some_and(|a| a.iter().any(|f| f.get("path").and_then(Value::as_str) == Some("labs/snippets") && f.get("xenonet").and_then(Value::as_str) == Some("folder")));
    if hit {
        json!([{ "path": "labs/snippets", "as": "project", "reason": "Its own pyproject.toml and code, apart from the other experiments." }])
    } else {
        json!([])
    }
}

pub const NARRATION: &str = "While you were away, five sessions worked across four projects. Partial refunds landed in atlas-api, and the solver in pathfinder got ten times faster. The checkout session is waiting on your answer about saved cards, and the ledger session keeps failing the same test.";

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Around for Fake {
        fn git_available(&mut self) -> bool {
            false
        }
        fn claude_id(&mut self, a: &str, o: &str) -> String {
            format!("claude:{a}:{o}")
        }
        fn github_id(&mut self, h: &str, l: &str) -> String {
            format!("github:{h}/{l}")
        }
        fn git_id(&mut self, e: &str) -> String {
            format!("git:{e}")
        }
        fn local(&mut self, ms: f64) -> [i64; 5] {
            let (y, m, d) = xenonet_base::time::civil_from_days((ms / 86400e3).floor() as i64);
            [y, i64::from(m), i64::from(d), 12, 5]
        }
        fn random(&mut self) -> f64 {
            0.5
        }
        fn uuid(&mut self) -> String {
            "01234567-89ab-4cde-8f01-23456789abcd".into()
        }
        fn now(&mut self) -> f64 {
            1_800_000_000_000.0
        }
    }

    #[test]
    fn a_made_up_world_with_sessions_to_list() {
        let tmp = std::env::temp_dir().join(format!("xenonet-demo-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut demo = Demo::new(&tmp.to_string_lossy(), "1", "2026-10-17");
        let w = demo.create_world(1_800_000_000_000.0, &mut Fake, "darwin").unwrap();
        assert!(std::path::Path::new(w["projectsDir"].as_str().unwrap()).join("atlas-api/README.md").exists());
        let claude = demo.sessions_of("claude").unwrap();
        assert_eq!(claude[0]["rawId"], id_of(0));
        assert_eq!(claude[0]["claudeAccount"]["account"], CLAUDE_WORK.0);
        let t = demo.transcript(&id_of(0)).unwrap();
        assert!(t["total"].as_u64().unwrap() > 0);
        assert_eq!(demo.live_count(), 5);
        // Grok Build's, in its folders, the subagent's under the session
        // that runs it.
        let grok = demo.sessions_of("grok").unwrap();
        assert_eq!(
            grok.iter().map(|s| s["title"].as_str().unwrap()).collect::<Vec<_>>(),
            ["Add dark mode to the storefront", "Check contrast on every page", "Map the depot loading rules"]
        );
        assert_eq!((grok[1]["source"].as_str(), grok[1]["parentId"].as_str()), (Some("subagent"), Some(format!("grok:{}", id_of(14)).as_str())));
        let folder = std::path::Path::new(w["grokHome"].as_str().unwrap()).join("sessions").join(encode_uri_component(grok[1]["cwd"].as_str().unwrap())).join(id_of(15));
        let summary: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("summary.json")).unwrap()).unwrap();
        assert_eq!((summary["parent_session_id"].as_str(), summary["session_kind"].as_str()), (Some(id_of(14).as_str()), Some("subagent")));
        let first: Value = serde_json::from_str(std::fs::read_to_string(folder.join("updates.jsonl")).unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(first["params"]["update"]["sessionUpdate"], "user_message_chunk");
        assert!(demo.transcript("nope").is_err());
        assert_eq!(demo_picks(&json!([{ "path": "labs/snippets", "xenonet": "folder" }]))[0]["as"], "project");
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
