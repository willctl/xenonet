//! A network across machines, made up, for XenoNet Dev
//! (docs/design/network.md; POST /api/dev/simulate with the net-… kinds).
//! It's a network/state/v1 and the other hosts' agents, so the console's
//! Network panel, Add a machine, the attention strip and the message box
//! can be checked without a second machine. It lives in memory, for the
//! service that made it, and `clear` ends it.
//!
//! While a simulation runs, and only then, this answers for it:
//!
//! - GET /api/net/state, through the core's route (routes/net.rs), which
//!   asks `simulated_net_state` first;
//! - the network's writes from the window (ticket, replace, remove,
//!   settings, create, join, leave), on the simulated state;
//! - GET /api/engine/state and GET /api/engine/agents, with the simulated
//!   hosts' agents after this machine's own;
//! - POST /api/actors/you/message to a simulated host's agent, which goes
//!   into the simulated outbox while its host is away;
//! - GET /api/system?hosts=1, with a made-up reading for each other host,
//!   each its own machine, so the Machines panel's sums aren't this one
//!   counted again.
//!
//! With nothing simulated, every one of these is the real route's.

use crate::borrowed::random_uuid;
use crate::cores::{ok, reason, CoreError};
use crate::ctx::Ctx;
use crate::http::{Body, Reply, Req};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_base::ids::{is_host_id, parse_qualified};
use xenonet_base::sync::Locked;

/// The simulations, for the console tools' list.
pub const KINDS: [&str; 8] = ["net-none", "net-founder", "net-member", "net-gateway-down", "net-ticket", "net-mismatch", "net-remote-waiting", "net-approval"];

const NETWORK_ID: &str = "net_5d1e0c7a9b3f4e2d8c6a1b0f9e7d3c25";
const ADDRESS: &str = "gw.example.net";
const ROSTER_HEAD: &str = "c2ltdWxhdGVkLXJvc3Rlci1oZWFkLTA0";
const PROPOSAL_ID: &str = "c2ltdWxhdGVkLWFkZC1kZW4tYXMtd2luZG93";
const TICKET_MS: f64 = 10.0 * 60e3;
const MAX_TICKETS: usize = 5;
const MAX_TEXT: usize = 16000;
const MINUTE: f64 = 60e3;

struct Sim {
    state: Value,
    agents: Vec<Value>,
    /// Made-up permission prompts of runs on the other machines.
    asks: Vec<Value>,
}

fn sims() -> &'static Mutex<HashMap<usize, Sim>> {
    static SIMS: OnceLock<Mutex<HashMap<usize, Sim>>> = OnceLock::new();
    SIMS.get_or_init(Mutex::default)
}

// A simulation belongs to the service that made it (tests make several).
fn key(ctx: &Ctx) -> usize {
    std::ptr::from_ref(ctx).addr()
}

/// network/state/v1 for a machine on its own.
pub fn alone() -> Value {
    json!({
        "schema": "network/state/v1",
        "network": null,
        "self": null,
        "gateway": null,
        "hosts": [],
        "tickets": [],
        "outbox": { "frames": 0, "bytes": 0, "oldest": null },
        "rosterHead": null,
        "rosterMismatch": [],
        "titlesOnly": false,
        "shareLoad": true,
    })
}

/// The hook the core's GET /api/net/state asks first: the simulated
/// state while a simulation runs (its lapsed tickets gone), else None.
pub fn simulated_net_state(ctx: &Ctx) -> Option<Value> {
    let now = ctx.now();
    let mut all = sims().locked();
    let sim = all.get_mut(&key(ctx))?;
    if let Some(t) = sim.state["tickets"].as_array_mut() {
        t.retain(|t| t["expires"].as_f64().is_some_and(|e| e > now));
    }
    Some(sim.state.clone())
}

/// The simulated hosts' agents, as the engine lists other hosts' agents
/// (`@<host>:<id>`, with host, seen and waitsOnYou), while a simulation
/// runs.
pub fn simulated_remote_agents(ctx: &Ctx) -> Option<Vec<Value>> {
    sims().locked().get(&key(ctx)).map(|s| s.agents.clone())
}

/// Puts the network into one of KINDS: what it made, or None for a kind
/// that isn't one. `you_at` is the host the person is at (this one by
/// default), for net-founder and net-remote-waiting; `variant`
/// "ask", for net-founder, has a run on laptop wait for your Allow or Deny
/// on a permission prompt, and "ask-env" one whose command may read the
/// run's environment, with the Allow card's warning; "titles-only" has the other machines share
/// titles only, so their
/// agents carry no question's text, last message or stuck reason, and
/// "relay", for net-founder, has laptop drawn from a relay snapshot: this
/// machine's own map, served as laptop's by a relay over a network inside
/// this process (the hub's net-relay), whose transcripts open through the
/// window as another machine's do.
pub fn simulate(ctx: &Ctx, kind: &str, you_at: Option<&str>, variant: Option<&str>) -> Option<Value> {
    let (state, agents) = made_up_with(kind, you_at, variant == Some("titles-only"), ctx.now())?;
    let mut out = json!({
        "ok": true,
        "kind": kind,
        "network": state["network"]["name"],
        "self": state["self"]["id"],
        "hosts": state["hosts"].as_array().map_or(0, Vec::len),
        "agents": agents.len(),
    });
    // "ask": a run XenoNet started on laptop asks to use a tool, for Needs
    // you's Allow and Deny; "ask-env", a tool call that may read the run's
    // environment, with the warning.
    let asks = match variant {
        Some("ask") => vec![made_up_ask(ctx.now(), false)],
        Some("ask-env") => vec![made_up_ask(ctx.now(), true)],
        _ => Vec::new(),
    };
    sims().locked().insert(key(ctx), Sim { state: state.clone(), agents, asks: asks.clone() });
    for a in &asks {
        ctx.broadcast("ask", a);
    }
    let relay = kind == "net-founder" && variant == Some("relay");
    if let Some(h) = &ctx.cores.hub {
        if let Ok(Some(r)) = h.simulate("net-relay", &json!({ "id": "laptop", "off": !relay })) {
            if relay {
                out["relay"] = r;
            }
        }
    }
    ctx.broadcast("net", &state);
    Some(out)
}

/// Ends the simulation; whether one ran.
pub fn clear(ctx: &Ctx) -> bool {
    if let Some(h) = &ctx.cores.hub {
        let _ = h.simulate("net-relay", &json!({ "id": "laptop", "off": true }));
    }
    let had = sims().locked().remove(&key(ctx)).is_some();
    if had {
        ctx.broadcast("net", &alone());
    }
    had
}

/// What a simulation answers while one runs in a development instance.
/// None: nothing is simulated, or this isn't one of its routes, and the
/// real route answers.
pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let (m, p) = (req.method.clone(), req.path.clone());
    if !ctx.config.dev || simulated_net_state(ctx).is_none() {
        return Ok(None);
    }
    match (m.as_str(), p.as_str()) {
        ("GET", "/api/engine/agents") => {
            let mut list = ctx.cores.engine.agents().await.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
            list.extend(simulated_remote_agents(ctx).unwrap_or_default());
            Ok(Some(Reply::json(200, Value::Array(list))))
        }
        // The engine's state lists other hosts' agents after this one's.
        ("GET", "/api/engine/state") => {
            let mut state = ctx.cores.engine.snapshot().await.ok().filter(Value::is_object).unwrap_or_else(|| json!({ "agents": [], "messages": [] }));
            let mut list = state["agents"].as_array().cloned().unwrap_or_default();
            list.extend(simulated_remote_agents(ctx).unwrap_or_default());
            state["agents"] = Value::Array(list);
            Ok(Some(Reply::json(200, state)))
        }
        ("POST", "/api/actors/you/message") => message(ctx, req).await,
        ("POST", "/api/start") => start(ctx, req).await,
        ("GET", "/api/runs/asks") => asks(ctx, req, false).await,
        ("POST", "/api/runs/answer") => asks(ctx, req, true).await,
        // This machine's own reading, and a made-up one for each other host
        // the simulation has: one that's away has its last, from when it
        // was last heard from.
        ("GET", "/api/system") if req.param("hosts") == Some("1") => {
            let mut reading = ctx.cores.system.read().await?;
            let Some(state) = simulated_net_state(ctx) else { return Ok(None) };
            let me = state["self"]["id"].clone();
            let now = ctx.now();
            let hosts: Vec<Value> = state["hosts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|h| h["id"] != me)
                .filter_map(|h| {
                    let id = h["id"].as_str()?;
                    let at = if h["online"] == true { now } else { h["seen"].as_f64().unwrap_or(now) };
                    let mut r = made_up_reading(id, at)?;
                    r["host"] = json!(id);
                    Some(r)
                })
                .collect();
            if reading.is_object() {
                reading["hosts"] = Value::Array(hosts);
            }
            Ok(Some(Reply::json(200, reading)))
        }
        ("POST", "/api/net/ticket" | "/api/net/replace" | "/api/net/remove" | "/api/net/create" | "/api/net/join" | "/api/net/leave" | "/api/net/approve")
        | ("PUT", "/api/net/settings") => {
            if let Some(why) = ctx.from_window(req).await {
                return Ok(Some(Reply::coded(403, "yours", why)));
            }
            let body = req.json(16 << 10).await?;
            let now = ctx.now();
            // The lock is let go before every page hears of the change.
            let (reply, changed) = {
                let mut all = sims().locked();
                let Some(sim) = all.get_mut(&key(ctx)) else { return Ok(None) };
                let reply = apply(sim, &p, &body, now);
                let changed = (reply.status < 300).then(|| sim.state.clone());
                (reply, changed)
            };
            if let Some(state) = changed {
                ctx.broadcast("net", &state);
            }
            Ok(Some(reply))
        }
        _ => Ok(None),
    }
}

fn fail(status: u16, code: &str, error: impl Into<String>) -> Reply {
    Reply::coded(status, code, error)
}

/// A simulated state with this machine's own agents counted on its own
/// row, as the core counts them.
pub async fn with_own_agents(ctx: &Ctx, mut state: Value) -> Value {
    let own = ctx
        .cores
        .engine
        .agents()
        .await
        .ok()
        .and_then(|v| v.as_array().map(|l| l.iter().filter(|a| !a["id"].as_str().unwrap_or("@").starts_with('@') && a["app"] != "actor").count()));
    let me = state["self"]["id"].clone();
    if let (Some(n), Some(hosts)) = (own, state["hosts"].as_array_mut()) {
        for h in hosts.iter_mut().filter(|h| h["id"] == me) {
            h["agents"] = json!(n);
        }
    }
    state
}

// ---------- the made-up network ----------

struct Machine {
    id: &'static str,
    window: bool,
    founder: bool,
    gateway: bool,
    platform: &'static str,
}

const STUDIO: Machine = Machine { id: "studio", window: true, founder: true, gateway: true, platform: "darwin" };
const LAPTOP: Machine = Machine { id: "laptop", window: true, founder: false, gateway: false, platform: "darwin" };
const RACK: Machine = Machine { id: "rack", window: false, founder: false, gateway: false, platform: "linux" };

/// A made-up key id: xnk1_ and 32 base32 letters, the same for a host each time.
pub fn key_id(host: &str) -> String {
    const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let b = host.as_bytes();
    let mut acc: u32 = 7;
    let mut out = String::from("xnk1_");
    for i in 0..32 {
        acc = acc.wrapping_mul(31).wrapping_add(u32::from(b.get(i % b.len().max(1)).copied().unwrap_or(b'x'))).wrapping_add(i as u32);
        out.push(char::from(B32[(acc % 32) as usize]));
    }
    out
}

// A host's agents, as another host's presence would list them, with what
// the console's state adds: each in a clone of a repo (its key) or in a
// folder that isn't one, and what it said last.
fn agents_of(host: &str, now: f64, seen: f64) -> Vec<Value> {
    struct A<'a> {
        local: &'a str,
        app: &'a str,
        title: &'a str,
        status: &'a str,
        project: &'a str,
        repo: bool,
        branch: Option<&'a str>,
        focus: Option<&'a str>,
        said: Option<&'a str>,
        ago: f64,
    }
    let agent = |a: A<'_>| {
        let cwd = if a.repo { format!("~/src/{}", a.project) } else { format!("~/jobs/{}", a.project) };
        json!({
            "id": format!("@{host}:{}", a.local),
            "host": host,
            "machine": host,
            "app": a.app,
            "title": a.title,
            "status": a.status,
            "project": a.project,
            "branch": a.branch,
            "cwd": cwd,
            "top": if a.repo { json!(cwd) } else { Value::Null },
            "repoKey": if a.repo { json!(format!("github.com/acme/{}", a.project)) } else { Value::Null },
            "focus": a.focus,
            "lastActive": now - a.ago,
            "seen": seen,
            "waitsOnYou": false,
            "needsYou": null,
            "question": null,
            "lastMessage": a.said,
            "stuck": null,
            "editing": [],
            "claims": [],
            "reachable": true,
        })
    };
    match host {
        "studio" => vec![agent(A {
            local: "claude:3b9e1d44-5a0c-4f6e-8d21-7c4b9a0e1f52",
            app: "claude",
            title: "Write the release notes",
            status: "working",
            project: "atlas-api",
            repo: true,
            branch: Some("release/2.4"),
            focus: Some("Listing what changed since 2.3"),
            said: Some("The notes for 2.4 are drafted. The migration section is next."),
            ago: 15e3,
        })],
        "laptop" => vec![
            agent(A {
                local: "claude:6f1c2b9e-0d4a-4c1e-9b7a-3e5d2a1f0c11",
                app: "claude",
                title: "Fix the login redirect",
                status: "working",
                project: "atlas-web",
                repo: true,
                branch: Some("fix/login"),
                focus: Some("Moving the redirect after the session check"),
                said: Some("The redirect now waits for the session check."),
                ago: 20e3,
            }),
            agent(A {
                local: "codex:019a7c55-1e2f-7a3b-8c4d-5e6f7a8b9c0d",
                app: "codex",
                title: "Tidy the billing tests",
                status: "idle",
                project: "ledger",
                repo: true,
                branch: Some("main"),
                focus: None,
                said: Some("Merged the two invoice fixtures."),
                ago: 12.0 * MINUTE,
            }),
        ],
        "rack" => vec![agent(A {
            local: "claude:8d0f6a21-7b3c-4e9d-a1f2-5c6e7d8b9a03",
            app: "claude",
            title: "Nightly import of the price lists",
            status: "idle",
            project: "importer",
            repo: false,
            branch: None,
            focus: None,
            said: None,
            ago: 2.0 * 3600e3,
        })],
        _ => vec![],
    }
}

fn host_row(m: &Machine, online: bool, seen: Option<f64>, agents: usize, you: bool) -> Value {
    json!({
        "id": m.id,
        "keyId": key_id(m.id),
        "window": m.window,
        "founder": m.founder,
        "gateway": m.gateway,
        "online": online,
        "seen": seen,
        "agents": agents,
        "waitsOnYou": 0,
        "you": you,
        "platform": m.platform,
        // rack runs an older XenoNet, which says neither: starts there are
        // refused as too old.
        "reach": if m.id == "rack" { Value::Null } else { json!({ "start": "you", "wake": "consent", "pending": [] }) },
        "can": if m.id == "rack" { json!([]) } else { json!(xenonet_base::network::CAN) },
    })
}

/// The state and the other hosts' agents for `kind`, at `now`.
fn made_up(kind: &str, you_at: Option<&str>, now: f64) -> Option<(Value, Vec<Value>)> {
    made_up_with(kind, you_at, false, now)
}

/// The same, with the other machines sharing titles only when `titles_only`.
fn made_up_with(kind: &str, you_at: Option<&str>, titles_only: bool, now: f64) -> Option<(Value, Vec<Value>)> {
    if kind == "net-none" {
        let mut s = alone();
        s["simulated"] = json!(true);
        return Some((s, vec![]));
    }
    let member = matches!(kind, "net-member" | "net-gateway-down");
    if !member && !matches!(kind, "net-founder" | "net-ticket" | "net-mismatch" | "net-remote-waiting" | "net-approval") {
        return None;
    }
    let down = kind == "net-gateway-down";
    let me = if member { &LAPTOP } else { &STUDIO };
    let you = you_at.filter(|h| is_host_id(h)).unwrap_or(me.id);
    let dropped = now - 25.0 * MINUTE;
    // From this host: who's online, and when each was last heard from.
    let rack_seen = now - 2.0 * 3600e3;
    let (studio_on, studio_seen) = if down { (false, dropped) } else { (true, now - 3e3) };
    let laptop_seen = now - 4e3;
    let mut agents = Vec::new();
    let mut hosts = Vec::new();
    for m in [&STUDIO, &LAPTOP, &RACK] {
        let (online, seen) = match m.id {
            _ if m.id == me.id => (true, now),
            "studio" => (studio_on, studio_seen),
            "laptop" => (true, laptop_seen),
            _ => (false, rack_seen),
        };
        let theirs = agents_of(m.id, now, seen);
        let count = theirs.len();
        if m.id != me.id {
            agents.extend(theirs);
        }
        let mut row = host_row(m, online, Some(seen), if m.id == me.id { 0 } else { count }, m.id == you);
        // Where you last worked: a moment ago where you are, and a while
        // ago at the other window hosts, so what waits sounds only there.
        row["youAt"] = if m.id == you {
            json!(now - 20e3)
        } else if m.window {
            json!(now - 40.0 * MINUTE)
        } else {
            Value::Null
        };
        hosts.push(row);
    }
    let gateway = json!({
        "address": ADDRESS,
        "keyId": key_id(STUDIO.id),
        "kind": "tunnel",
        "link": if down { "offline" } else { "live" },
        "since": if down { dropped } else if member { now - 40.0 * MINUTE } else { now - 3.0 * 3600e3 },
        "reason": if down { json!("timed out") } else { Value::Null },
        "skewMs": if down { Value::Null } else if member { json!(120) } else { json!(0) },
    });
    let mut state = json!({
        "schema": "network/state/v1",
        "network": { "id": NETWORK_ID, "name": "home" },
        "self": { "id": me.id, "keyId": key_id(me.id), "window": me.window, "founder": me.founder, "gateway": me.gateway },
        "gateway": gateway,
        "hosts": hosts,
        "tickets": [],
        "outbox": if down { json!({ "frames": 3, "bytes": 2816, "oldest": now - 22.0 * MINUTE }) } else { json!({ "frames": 0, "bytes": 0, "oldest": null }) },
        "rosterHead": ROSTER_HEAD,
        "rosterMismatch": [],
        "titlesOnly": false,
        "shareLoad": true,
        "start": "you",
        "wake": "consent",
        "simulated": true,
    });
    match kind {
        "net-ticket" => state["tickets"] = json!([{ "host": "desk", "window": false, "expires": now + 8.5 * MINUTE }]),
        "net-mismatch" => state["rosterMismatch"] = json!(["laptop"]),
        // laptop, a window host but not the founder, signed den in as a
        // window host: that waits for the founder or a second window host.
        "net-approval" => state["proposals"] = json!([{ "id": PROPOSAL_ID, "op": "add", "host": "den", "window": true, "gateway": null, "signers": ["laptop"], "at": now - 40e3 }]),
        // laptop's session waits on you with a question; at net-founder its
        // other one seems stuck too.
        "net-remote-waiting" | "net-founder" => {
            if let Some(a) = agents.iter_mut().find(|a| a["host"] == "laptop" && a["app"] == "claude") {
                a["status"] = json!("waiting");
                a["waitsOnYou"] = json!(true);
                a["needsYou"] = json!("question");
                a["question"] = json!("Should the redirect in <LoginForm> keep the old query string, or drop it?");
                a["since"] = json!(now - 90e3);
                a["focus"] = Value::Null;
            }
            if kind == "net-founder" {
                if let Some(a) = agents.iter_mut().find(|a| a["host"] == "laptop" && a["app"] == "codex") {
                    a["status"] = json!("working");
                    a["lastActive"] = json!(now - 40e3);
                    a["stuck"] = json!(["ran cargo test 3 times in a row"]);
                }
            }
            if let Some(h) = state["hosts"].as_array_mut().and_then(|l| l.iter_mut().find(|h| h["id"] == "laptop")) {
                h["waitsOnYou"] = json!(1);
            }
        }
        _ => {}
    }
    // The other machines share titles only: their agents carry no detail.
    if titles_only {
        for a in &mut agents {
            for k in ["question", "lastMessage", "stuck"] {
                a[k] = Value::Null;
            }
        }
    }
    Some((state, agents))
}

// ---------- each machine's load, made up ----------

/// A made-up machine's reading at `at`, as its relay would send it (no
/// history, no cores), breathing slowly as the demo's own does, each on
/// its own phase: studio an Apple M2 Ultra, laptop an M3 Pro, rack a Linux
/// box with two NVIDIA cards. None for a host the simulation doesn't have.
pub fn made_up_reading(host: &str, at: f64) -> Option<Value> {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    struct Box {
        platform: &'static str,
        arch: &'static str,
        model: &'static str,
        threads: u32,
        ghz: f64,
        ram: f64,
        gpus: &'static [(&'static str, f64, bool)],
        phase: f64,
        cpu: f64,
        gpu: f64,
    }
    let b = match host {
        "studio" => Box {
            platform: "darwin",
            arch: "arm64",
            model: "Apple M2 Ultra",
            threads: 24,
            ghz: 3.5,
            ram: 128.0,
            gpus: &[("Apple M2 Ultra · 76-core GPU", 0.0, true)],
            phase: 0.4,
            cpu: 0.22,
            gpu: 0.31,
        },
        "laptop" => Box {
            platform: "darwin",
            arch: "arm64",
            model: "Apple M3 Pro",
            threads: 12,
            ghz: 4.1,
            ram: 36.0,
            gpus: &[("Apple M3 Pro · 18-core GPU", 0.0, true)],
            phase: 1.9,
            cpu: 0.58,
            gpu: 0.18,
        },
        "rack" => Box {
            platform: "linux",
            arch: "x64",
            model: "AMD EPYC 7313P",
            threads: 32,
            ghz: 3.0,
            ram: 128.0,
            gpus: &[("NVIDIA RTX A4000", 16.0, false), ("NVIDIA RTX A4000", 16.0, false)],
            phase: 3.1,
            cpu: 0.12,
            gpu: 0.64,
        },
        _ => return None,
    };
    let wave = |period: f64, phase: f64| ((at / period) * std::f64::consts::PI * 2.0 + b.phase + phase).sin();
    let share = |v: f64| (v.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
    let cpu = share(b.cpu + 0.12 * wave(37e3, 0.0) + 0.05 * wave(8.3e3, 1.1));
    let mem = share(0.48 + 0.06 * wave(71e3, 0.5));
    let ram = b.ram * GB;
    let gpus: Vec<Value> = b
        .gpus
        .iter()
        .enumerate()
        .map(|(i, (name, vram, shared))| {
            let usage = share(b.gpu + 0.2 * wave(51e3, i as f64 * 1.7) + 0.06 * wave(5.9e3, 0.3));
            let (used, total) = if *shared { ((0.08 + 0.1 * usage) * ram, ram) } else { ((0.3 + 0.4 * usage) * vram * GB, vram * GB) };
            let temp = if *shared { Value::Null } else { json!((48.0 + 30.0 * usage).round()) };
            json!({ "index": i, "name": name, "usage": usage, "memUsed": used.round(), "memTotal": total, "temp": temp, "shared": shared })
        })
        .collect();
    let mut r = json!({
        "at": at,
        "platform": b.platform,
        "arch": b.arch,
        "uptime": 9 * 86400 + 2 * 3600,
        "cpu": { "model": b.model, "threads": b.threads, "ghz": b.ghz, "usage": cpu },
        "memory": { "total": ram, "used": (mem * ram).round() },
        "gpus": gpus,
        "gpu": if b.platform == "darwin" { "ioreg" } else { "nvidia-smi" },
    });
    r["hardware"] = xenonet_app::system_stats::hardware_of(&r);
    Some(r)
}

// ---------- the network's writes, on the simulated state ----------

fn host_arg(body: &Value) -> &str {
    body.get("host").and_then(Value::as_str).unwrap_or("")
}

fn ticket_words() -> String {
    const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut b = [0u8; 10];
    let _ = getrandom::fill(&mut b);
    let t: String = b.iter().map(|x| char::from(B32[usize::from(x % 32)])).collect();
    format!("{}-{}-{}", &t[0..4], &t[4..8], &t[8..10])
}

// A ticket for `host`, kept as waiting, and the answer that shows it once.
fn ticket_for(sim: &mut Sim, host: &str, window: bool, now: f64) -> Value {
    let ticket = ticket_words();
    let address = sim.state["gateway"]["address"].as_str().unwrap_or(ADDRESS).to_string();
    let expires = now + TICKET_MS;
    if let Some(t) = sim.state["tickets"].as_array_mut() {
        t.push(json!({ "host": host, "window": window, "expires": expires }));
    }
    json!({ "ticket": ticket, "line": format!("xenonet net join {address} {ticket}"), "address": address, "host": host, "window": window, "expires": expires, "simulated": true })
}

fn has_host(state: &Value, host: &str) -> bool {
    state["hosts"].as_array().is_some_and(|l| l.iter().any(|h| h["id"] == host))
}

fn apply(sim: &mut Sim, path: &str, body: &Value, now: f64) -> Reply {
    let in_network = !sim.state["network"].is_null();
    let me = sim.state["self"]["id"].as_str().unwrap_or("").to_string();
    let host = host_arg(body).to_string();
    let outside = || fail(409, "no-network", "This machine isn't in a network yet: start one, or join one, first.");
    match path {
        "/api/net/create" => {
            if in_network {
                return fail(409, "in-network", "This machine is in a network already.");
            }
            let name = body.get("name").and_then(Value::as_str).map_or("", str::trim);
            if name.is_empty() || name.chars().count() > 64 {
                return fail(400, "bad-request", "Give the network a name, up to 64 characters.");
            }
            if !is_host_id(&host) {
                return fail(400, "bad-request", "A machine's id is 2 to 32 lowercase letters, digits and dashes, starting with a letter.");
            }
            let Some((mut state, _)) = made_up("net-founder", None, now) else { return fail(500, "error", "") };
            state["network"]["name"] = json!(name);
            state["self"] = json!({ "id": host, "keyId": key_id(&host), "window": true, "founder": true, "gateway": true });
            let mut row = host_row(&STUDIO, true, Some(now), 0, true);
            row["id"] = json!(host);
            row["keyId"] = json!(key_id(&host));
            state["hosts"] = json!([row]);
            // No tunnel or LAN address yet: the person sets one, as the
            // Actors panel's gateway does.
            state["gateway"] = Value::Null;
            sim.state = state;
            sim.agents.clear();
            Reply::json(200, sim.state.clone())
        }
        "/api/net/join" => {
            if in_network {
                return fail(409, "in-network", "This machine is in a network already.");
            }
            let address = body.get("address").and_then(Value::as_str).map_or("", str::trim);
            let ticket = body.get("ticket").and_then(Value::as_str).map_or("", str::trim);
            let shape = ticket.replace('-', "");
            if address.is_empty() || shape.len() != 10 || !shape.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return fail(400, "bad-request", "Give the address and the ticket from the line Add a machine showed.");
            }
            let Some((mut state, agents)) = made_up("net-member", None, now) else { return fail(500, "error", "") };
            state["gateway"]["address"] = json!(address);
            sim.state = state;
            sim.agents = agents;
            Reply::json(200, sim.state.clone())
        }
        "/api/net/leave" => {
            if !in_network {
                return outside();
            }
            sim.state = alone();
            sim.state["simulated"] = json!(true);
            sim.agents.clear();
            Reply::json(200, sim.state.clone())
        }
        "/api/net/ticket" => {
            if !in_network {
                return outside();
            }
            if sim.state["gateway"].is_null() {
                return fail(409, "no-address", "The gateway has no address yet. Give it its tailnet or LAN address first.");
            }
            if !is_host_id(&host) {
                return fail(400, "bad-request", "A machine's id is 2 to 32 lowercase letters, digits and dashes, starting with a letter.");
            }
            if has_host(&sim.state, &host) {
                return fail(409, "taken", format!("There's a machine called {host} in the network already. If it was set up again, use Replace on its row."));
            }
            let now_waiting = sim.state["tickets"].as_array().map_or(0, |t| t.iter().filter(|t| t["expires"].as_f64().is_some_and(|e| e > now)).count());
            if now_waiting >= MAX_TICKETS {
                return fail(429, "too-many", "Five tickets wait already. Use one, or let them lapse.");
            }
            let window = body.get("window") == Some(&Value::Bool(true));
            Reply::json(200, ticket_for(sim, &host, window, now))
        }
        "/api/net/replace" => {
            if !in_network {
                return outside();
            }
            if host == me {
                return fail(400, "bad-request", "This is the machine you're at. Replace a machine from another one's console.");
            }
            let Some(window) = sim.state["hosts"].as_array().and_then(|l| l.iter().find(|h| h["id"] == host.as_str())).map(|h| h["window"] == true) else {
                return fail(404, "not-found", format!("There's no machine called {host} in the network."));
            };
            let mut out = ticket_for(sim, &host, window, now);
            out["replaced"] = json!(host);
            Reply::json(200, out)
        }
        "/api/net/remove" => {
            if !in_network {
                return outside();
            }
            if host == me {
                return fail(400, "bad-request", "To take this machine out of the network, run xenonet net leave on it.");
            }
            if !has_host(&sim.state, &host) {
                return fail(404, "not-found", format!("There's no machine called {host} in the network."));
            }
            if let Some(l) = sim.state["hosts"].as_array_mut() {
                l.retain(|h| h["id"] != host.as_str());
            }
            sim.agents.retain(|a| a["host"] != host.as_str());
            Reply::json(200, json!({ "ok": true, "removed": host }))
        }
        "/api/net/approve" => {
            let id = body.get("id").and_then(Value::as_str).unwrap_or("");
            let Some(list) = sim.state["proposals"].as_array_mut() else { return fail(404, "nothing", "nothing waits for approval by that id") };
            let Some(i) = list.iter().position(|p| p["id"] == id) else { return fail(404, "nothing", "nothing waits for approval by that id") };
            let p = list.remove(i);
            let host = p["host"].as_str().unwrap_or("").to_string();
            if p["op"] == "add" {
                let mut row = host_row(&LAPTOP, true, Some(now), 0, false);
                row["id"] = json!(host);
                row["keyId"] = json!(key_id(&host));
                if let Some(h) = sim.state["hosts"].as_array_mut() {
                    h.push(row);
                }
            }
            Reply::json(200, sim.state.clone())
        }
        "/api/net/settings" => {
            if !in_network {
                return outside();
            }
            if let Some(on) = body.get("titlesOnly").and_then(Value::as_bool) {
                sim.state["titlesOnly"] = json!(on);
            }
            if let Some(on) = body.get("shareLoad").and_then(Value::as_bool) {
                sim.state["shareLoad"] = json!(on);
            }
            match set_reach(&mut sim.state, body, now) {
                Ok(()) => Reply::json(200, sim.state.clone()),
                Err((status, code, why)) => fail(status, code, why),
            }
        }
        _ => fail(404, "not-found", "not found"),
    }
}

/// A permission prompt of a made-up run on laptop, as its machine would
/// send it (asks.rs); `env`, one that may read the run's environment.
fn made_up_ask(now: f64, env: bool) -> Value {
    let (input, warning) = if env { ("printenv | grep -i token", json!(xenonet_engine::delivery::starts::ENV_WARNING)) } else { ("cargo test --package billing", Value::Null) };
    json!({
        "id": format!("ask_{}", random_uuid()),
        "host": "laptop",
        "here": false,
        "session": "claude:0190d3e0-0000-7000-8000-00000000a5c1",
        "run": format!("run_{}", random_uuid()),
        "title": "laptop · Tidy the billing tests",
        "tool": "Bash",
        "input": input,
        "warning": warning,
        "at": (now - 20e3) as i64,
        "until": (now + 580e3) as i64,
        "simulated": true,
    })
}

/// The permission prompts waiting, with the made-up ones; and your answer
/// to a made-up one, which goes nowhere but takes it off. Anything else is
/// the real route's, with its body put back.
async fn asks(ctx: &Arc<Ctx>, req: &mut Req, answer: bool) -> Result<Option<Reply>, CoreError> {
    if !answer {
        if let Some(why) = ctx.from_window(req).await {
            return Ok(Some(Reply::coded(403, "yours", why)));
        }
        let mut real = ctx.cores.engine.asks().await.unwrap_or_else(|_| json!({ "asks": [] }));
        let made: Vec<Value> = sims().locked().get(&key(ctx)).map(|s| s.asks.clone()).unwrap_or_default();
        if let Some(list) = real["asks"].as_array_mut() {
            list.extend(made);
        }
        return Ok(Some(Reply::json(200, real)));
    }
    let bytes = req.bytes(4 << 10).await?;
    let body = xenonet_base::json::parse(&String::from_utf8_lossy(&bytes)).unwrap_or(Value::Null);
    let id = body.get("ask").and_then(Value::as_str).unwrap_or("").to_string();
    let decision = body.get("decision").and_then(Value::as_str).unwrap_or("").to_string();
    let made = sims().locked().get(&key(ctx)).is_some_and(|s| s.asks.iter().any(|a| a["id"] == id.as_str()));
    if !made {
        req.body = Body::Bytes(bytes);
        return Ok(None);
    }
    if let Some(why) = ctx.from_window(req).await {
        return Ok(Some(Reply::coded(403, "yours", why)));
    }
    if !matches!(decision.as_str(), "allow" | "deny") {
        return Ok(Some(Reply::coded(400, "bad-request", "the answer is allow or deny")));
    }
    if let Some(s) = sims().locked().get_mut(&key(ctx)) {
        s.asks.retain(|a| a["id"] != id.as_str());
    }
    ctx.broadcast("ask-done", &json!({ "op": "done", "id": id, "decision": decision, "simulated": true }));
    Ok(Some(Reply::json(200, json!({ "ok": true, "ask": id, "decision": decision, "host": "laptop", "simulated": true }))))
}

/// A start on a made-up machine, from the window, answered as the real
/// one would be: refused at once when the machine is away or its XenoNet
/// too old (rack's), else sent; then, as the start event, started a
/// moment later and ended a few seconds after. A start here, or anything
/// that isn't the window's, goes on to the real route, with its body put
/// back.
async fn start(ctx: &Arc<Ctx>, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let bytes = req.bytes(64 << 10).await?;
    let body = xenonet_base::json::parse(&String::from_utf8_lossy(&bytes)).unwrap_or(Value::Null);
    let machine = body.get("machine").and_then(Value::as_str).unwrap_or("").to_string();
    let Some(state) = simulated_net_state(ctx) else { return Ok(None) };
    let row = state["hosts"].as_array().and_then(|l| l.iter().find(|h| h["id"] == machine.as_str() && h["self"] != true).cloned());
    let Some(row) = row.filter(|_| !machine.is_empty() && state["self"]["id"] != machine.as_str()) else {
        req.body = Body::Bytes(bytes);
        return Ok(None);
    };
    if let Some(why) = ctx.from_window(req).await {
        return Ok(Some(Reply::coded(403, "yours", why)));
    }
    let prompt = body.get("prompt").and_then(Value::as_str).map_or("", str::trim);
    if prompt.is_empty() {
        return Ok(Some(Reply::coded(400, "bad-request", "say what the new session should do: a prompt")));
    }
    if row["online"] != true {
        return Ok(Some(Reply::coded(409, "refused", format!("{machine} is offline, so nothing was started: a start is never kept for later"))));
    }
    if !row["can"].as_array().is_some_and(|c| c.iter().any(|x| x == "start")) {
        return Ok(Some(Reply::coded(409, "refused", format!("{machine}'s XenoNet is too old to start sessions: update it there"))));
    }
    let app = body.get("app").and_then(Value::as_str).filter(|a| ["claude", "codex", "grok"].contains(a)).unwrap_or("claude").to_string();
    let folder = body.get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()).unwrap_or("~").to_string();
    let request = format!("msg_{}", random_uuid());
    let session = (app != "codex").then(|| format!("{app}:{}", random_uuid()));
    {
        let c = ctx.clone();
        let (machine, folder, session) = (machine.clone(), folder.clone(), session.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let started = json!({ "session": session, "folder": folder });
            let words = xenonet_engine::delivery::starts::answer_words("started", &machine, &started);
            c.broadcast(
                "start",
                &json!({ "state": "started", "host": machine, "session": session, "folder": folder, "words": words, "at": xenonet_base::json::number(c.now()), "simulated": true }),
            );
            tokio::time::sleep(std::time::Duration::from_millis(4000)).await;
            let ended = json!({ "session": session, "exit": 0, "minutes": 0 });
            let words = xenonet_engine::delivery::starts::answer_words("ended", &machine, &ended);
            c.broadcast("start", &json!({ "state": "ended", "host": machine, "session": session, "exit": 0, "minutes": 0, "words": words, "at": xenonet_base::json::number(c.now()), "simulated": true }));
        });
    }
    Ok(Some(Reply::json(
        200,
        json!({ "request": request, "host": machine, "state": "sent", "way": format!("sent to {machine}: the answer comes as a message, started or refused, and again when the session ends"), "simulated": true }),
    )))
}

/// Who may start and wake sessions, set in the simulated state: this
/// machine's at once; another's (`host`) as that machine would take it
/// (xenonet_net::operator): narrowing at once, widening to network at once
/// from the founder, else waiting for a second window host, which the
/// made-up laptop is when this machine is studio.
fn set_reach(state: &mut Value, body: &Value, now: f64) -> Result<(), (u16, &'static str, String)> {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let me = state["self"]["id"].as_str().unwrap_or("").to_string();
    let bad = |k: &str, v: &str, allowed: &[&str]| (!allowed.contains(&v)).then(|| (400, "bad-request", format!("{k} is one of {}", allowed.join(", "))));
    for (k, allowed) in [("start", &xenonet_base::network::STARTS[..]), ("wake", &xenonet_base::network::WAKES[..])] {
        if let Some(e) = text(k).and_then(|v| bad(k, &v, allowed)) {
            return Err(e);
        }
    }
    let host = text("host").filter(|h| *h != me);
    let Some(host) = host else {
        for k in ["start", "wake"] {
            if let Some(v) = text(k) {
                state[k] = json!(v);
                if let Some(row) = state["hosts"].as_array_mut().and_then(|l| l.iter_mut().find(|h| h["id"] == me.as_str())) {
                    row["reach"][k] = json!(v);
                }
            }
        }
        return Ok(());
    };
    if state["self"]["window"] != true {
        return Err((403, "not-window", "another machine's settings are changed from a window host".into()));
    }
    let founder = state["self"]["founder"] == true;
    let Some(row) = state["hosts"].as_array_mut().and_then(|l| l.iter_mut().find(|h| h["id"] == host.as_str())) else {
        return Err((404, "no-host", format!("{host} isn't in the network")));
    };
    if row["reach"].is_null() {
        // An older XenoNet ignores the frame: nothing changes there.
        return Ok(());
    }
    if let Some(id) = text("approve") {
        let pending = row["reach"]["pending"].as_array().cloned().unwrap_or_default();
        if let Some(w) = pending.iter().find(|w| w["id"] == id.as_str()) {
            let setting = w["setting"].as_str().unwrap_or("start").to_string();
            row["reach"][setting.as_str()] = json!("network");
            row["reach"]["pending"] = json!(pending.into_iter().filter(|w| w["id"] != id.as_str()).collect::<Vec<_>>());
        }
        return Ok(());
    }
    for k in ["start", "wake"] {
        let Some(v) = text(k) else { continue };
        let mut pending: Vec<Value> = row["reach"]["pending"].as_array().cloned().unwrap_or_default().into_iter().filter(|w| w["setting"] != k).collect();
        if v != "network" || founder {
            row["reach"][k] = json!(v);
        } else if row["reach"][k] != "network" {
            pending.push(json!({ "id": format!("wid_{k}_{}", now as i64), "setting": k, "value": "network", "by": me, "at": now as i64 }));
        }
        row["reach"]["pending"] = json!(pending);
    }
    Ok(())
}

// You, writing to a simulated host's agent from the window: checked as
// the real route checks the window, then kept in the simulated outbox
// while its host or the gateway is away. Anything else goes on to the
// real route, with its body put back.
async fn message(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let bytes = req.bytes(16 << 10).await?;
    let body = xenonet_base::json::parse(&String::from_utf8_lossy(&bytes)).unwrap_or(Value::Null);
    let to = body.get("to").and_then(Value::as_str).unwrap_or("").to_string();
    let host = parse_qualified(&to).and_then(|(h, _)| h).map(str::to_string);
    let known = simulated_remote_agents(ctx).is_some_and(|l| l.iter().any(|a| a["id"] == to.as_str()));
    let Some(host) = host.filter(|_| known) else {
        req.body = Body::Bytes(bytes);
        return Ok(None);
    };
    if !ctx.window_only(req) {
        return Ok(Some(Reply::coded(403, "yours", "this is for XenoNet's own window")));
    }
    let seen = ctx.cores.verify.window(req.peer).await;
    if !ok(&seen) {
        return Ok(Some(Reply::coded(403, "yours", reason(&seen))));
    }
    let text = body.get("text").and_then(Value::as_str).map_or("", str::trim);
    if text.is_empty() || text.chars().count() > MAX_TEXT {
        return Ok(Some(Reply::coded(400, "bad-request", "say what to tell it: text, up to 16,000 characters")));
    }
    let now = ctx.now();
    let state = {
        let mut all = sims().locked();
        let Some(sim) = all.get_mut(&key(ctx)) else { return Ok(None) };
        let reachable = sim.state["gateway"]["link"] == "live" && sim.state["hosts"].as_array().is_some_and(|l| l.iter().any(|h| h["id"] == host.as_str() && h["online"] == true));
        if !reachable {
            let o = &mut sim.state["outbox"];
            o["frames"] = json!(o["frames"].as_u64().unwrap_or(0) + 1);
            o["bytes"] = json!(o["bytes"].as_u64().unwrap_or(0) + text.len() as u64);
            if o["oldest"].is_null() {
                o["oldest"] = json!(now);
            }
        }
        (reachable, sim.state.clone())
    };
    ctx.broadcast("net", &state.1);
    Ok(Some(Reply::json(
        200,
        json!({ "ok": true, "mid": format!("msg_{}", random_uuid()), "to": to, "host": host, "state": if state.0 { "stored" } else { "queued" }, "simulated": true }),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cores::{Cores, Verify};
    use crate::http::Peer;
    use async_trait::async_trait;

    const HOST_KEYS: [&str; 11] = ["id", "keyId", "window", "founder", "gateway", "online", "seen", "agents", "waitsOnYou", "you", "platform"];

    fn key_ok(k: &Value) -> bool {
        k.as_str().is_some_and(|s| s.len() == 37 && s.starts_with("xnk1_") && s[5..].bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)))
    }

    // What network/state/v1 requires, as far as these states use it.
    fn well_formed(s: &Value) {
        for k in ["schema", "network", "self", "gateway", "hosts", "tickets", "outbox", "rosterHead", "rosterMismatch", "titlesOnly"] {
            assert!(s.get(k).is_some(), "{k} is there");
        }
        assert_eq!(s["schema"], "network/state/v1");
        if s["network"].is_null() {
            assert!(s["self"].is_null() && s["hosts"].as_array().unwrap().is_empty());
            return;
        }
        let id = s["network"]["id"].as_str().unwrap();
        assert!(id.len() == 36 && id.starts_with("net_") && id[4..].bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert!(key_ok(&s["self"]["keyId"]));
        let me = &s["self"]["id"];
        let hosts = s["hosts"].as_array().unwrap();
        assert_eq!(hosts.iter().filter(|h| &h["id"] == me).count(), 1, "this host is listed once");
        for h in hosts {
            for k in HOST_KEYS {
                assert!(h.get(k).is_some(), "a host's {k}");
            }
            assert!(is_host_id(h["id"].as_str().unwrap()) && key_ok(&h["keyId"]));
        }
        if !s["gateway"].is_null() {
            let g = &s["gateway"];
            assert!(key_ok(&g["keyId"]));
            assert!(["off", "connecting", "live", "offline", "refused", "removed"].contains(&g["link"].as_str().unwrap()));
            assert!(["tunnel", "lan", "loopback"].contains(&g["kind"].as_str().unwrap()));
            for k in ["since", "reason", "skewMs"] {
                assert!(g.get(k).is_some());
            }
        }
        for k in ["frames", "bytes", "oldest"] {
            assert!(s["outbox"].get(k).is_some());
        }
    }

    #[test]
    fn every_kind_is_a_well_formed_state_with_other_hosts_agents() {
        for kind in KINDS {
            let (s, agents) = made_up(kind, None, 1.8e12).unwrap();
            well_formed(&s);
            let me = s["self"]["id"].as_str().unwrap_or("");
            for a in &agents {
                let (host, local) = parse_qualified(a["id"].as_str().unwrap()).unwrap();
                assert_eq!(host, a["host"].as_str());
                assert!(host != Some(me) && has_host(&s, host.unwrap()), "{kind}: {a}");
                assert!(local.starts_with("claude:") || local.starts_with("codex:"));
            }
        }
        assert!(made_up("net-nope", None, 0.0).is_none());
    }

    #[test]
    fn each_kind_shows_what_it_says() {
        let now = 1.8e12;
        let (f, _) = made_up("net-founder", None, now).unwrap();
        assert_eq!(f["self"], json!({ "id": "studio", "keyId": key_id("studio"), "window": true, "founder": true, "gateway": true }));
        let row = |s: &Value, id: &str| s["hosts"].as_array().unwrap().iter().find(|h| h["id"] == id).cloned().unwrap();
        assert_eq!((row(&f, "rack")["online"].clone(), row(&f, "studio")["you"].clone()), (json!(false), json!(true)));
        let (m, agents) = made_up("net-member", None, now).unwrap();
        assert_eq!((m["self"]["id"].clone(), m["self"]["gateway"].clone(), row(&m, "laptop")["you"].clone()), (json!("laptop"), json!(false), json!(true)));
        assert!(agents.iter().any(|a| a["host"] == "studio"));
        let (d, _) = made_up("net-gateway-down", None, now).unwrap();
        assert_eq!((d["gateway"]["link"].clone(), d["gateway"]["reason"].clone(), d["outbox"]["frames"].clone()), (json!("offline"), json!("timed out"), json!(3)));
        assert_eq!(row(&d, "studio")["online"], false);
        assert_eq!(made_up("net-ticket", None, now).unwrap().0["tickets"][0]["host"], "desk");
        assert_eq!(made_up("net-mismatch", None, now).unwrap().0["rosterMismatch"], json!(["laptop"]));
        let (w, agents) = made_up("net-remote-waiting", Some("laptop"), now).unwrap();
        let waiting: Vec<&Value> = agents.iter().filter(|a| a["waitsOnYou"] == true).collect();
        assert_eq!(waiting.len(), 1);
        assert!(waiting[0]["id"].as_str().unwrap().starts_with("@laptop:claude:") && waiting[0]["question"].is_string());
        assert_eq!((row(&w, "laptop")["waitsOnYou"].clone(), row(&w, "laptop")["you"].clone(), row(&w, "studio")["you"].clone()), (json!(1), json!(true), json!(false)));
        assert!(row(&w, "laptop")["youAt"].as_f64().unwrap() > row(&w, "studio")["youAt"].as_f64().unwrap(), "you worked at laptop last");
        // Repo keys for the map's projects, and a folder that isn't a repo.
        assert!(agents.iter().any(|a| a["repoKey"] == "github.com/acme/atlas-web") && agents.iter().any(|a| a["host"] == "rack" && a["repoKey"].is_null()));
        // net-founder has a question and a stuck session on laptop; with
        // titles only, neither's text.
        let (_, agents) = made_up("net-founder", None, now).unwrap();
        assert!(agents.iter().any(|a| a["question"].is_string()) && agents.iter().any(|a| a["stuck"].is_array()));
        let (_, agents) = made_up_with("net-founder", None, true, now).unwrap();
        assert!(agents.iter().all(|a| a["question"].is_null() && a["lastMessage"].is_null() && a["stuck"].is_null()));
        assert!(agents.iter().any(|a| a["waitsOnYou"] == true), "it still waits");
        assert_eq!(made_up("net-none", None, now).unwrap().0["network"], Value::Null);
        let (a, _) = made_up("net-approval", None, now).unwrap();
        assert_eq!((a["proposals"][0]["host"].clone(), a["proposals"][0]["signers"].clone()), (json!("den"), json!(["laptop"])));
    }

    #[test]
    fn an_approval_by_its_id_adds_the_window_host() {
        let now = 1.8e12;
        let mut s = sim("net-approval");
        assert_eq!(apply(&mut s, "/api/net/approve", &json!({ "id": "nope" }), now).status, 404);
        let r = apply(&mut s, "/api/net/approve", &json!({ "id": PROPOSAL_ID }), now);
        assert_eq!(r.status, 200);
        well_formed(&s.state);
        assert_eq!(s.state["proposals"], json!([]));
        assert!(has_host(&s.state, "den"));
    }

    fn sim(kind: &str) -> Sim {
        let (state, agents) = made_up(kind, None, 1.8e12).unwrap();
        Sim { state, agents, asks: Vec::new() }
    }

    #[test]
    fn tickets_once_each_and_never_for_a_machine_thats_there() {
        let now = 1.8e12;
        let mut s = sim("net-founder");
        let r = apply(&mut s, "/api/net/ticket", &json!({ "host": "desk", "window": true }), now);
        assert_eq!(r.status, 200);
        let t = r.body_json();
        let ticket = t["ticket"].as_str().unwrap();
        assert!(ticket.len() == 12 && ticket.as_bytes()[4] == b'-' && ticket.as_bytes()[9] == b'-');
        assert_eq!(t["line"], format!("xenonet net join gw.example.net {ticket}"));
        assert_eq!(s.state["tickets"], json!([{ "host": "desk", "window": true, "expires": now + TICKET_MS }]));
        assert!(!s.state.to_string().contains(ticket), "the state never holds the ticket");
        assert_eq!(apply(&mut s, "/api/net/ticket", &json!({ "host": "laptop" }), now).status, 409);
        assert_eq!(apply(&mut s, "/api/net/ticket", &json!({ "host": "Desk!" }), now).status, 400);
        for h in ["a1", "a2", "a3", "a4"] {
            assert_eq!(apply(&mut s, "/api/net/ticket", &json!({ "host": h }), now).status, 200);
        }
        assert_eq!(apply(&mut s, "/api/net/ticket", &json!({ "host": "a5" }), now).status, 429);
        let mut alone = sim("net-none");
        assert_eq!(apply(&mut alone, "/api/net/ticket", &json!({ "host": "desk" }), now).status, 409);
    }

    #[test]
    fn start_and_wake_here_and_on_another_machine_as_it_would_take_them() {
        let now = 1_000_000.0;
        let reach = |s: &Sim, host: &str| s.state["hosts"].as_array().unwrap().iter().find(|h| h["id"] == host).unwrap()["reach"].clone();
        // Here, at once.
        let mut s = sim("net-founder");
        let st = apply(&mut s, "/api/net/settings", &json!({ "start": "network", "wake": "network" }), now).body_json();
        assert_eq!((st["start"].clone(), st["wake"].clone()), (json!("network"), json!("network")));
        assert_eq!(apply(&mut s, "/api/net/settings", &json!({ "start": "everyone" }), now).status, 400);
        // studio is the founder: laptop's start widens at once.
        apply(&mut s, "/api/net/settings", &json!({ "host": "laptop", "start": "network" }), now);
        assert_eq!(reach(&s, "laptop")["start"], "network");
        // rack's XenoNet says nothing of its reach: it takes none.
        assert_eq!(apply(&mut s, "/api/net/settings", &json!({ "host": "rack", "start": "off" }), now).status, 200);
        assert!(reach(&s, "rack").is_null());
        // At laptop, a window host but not the founder, a widening of
        // studio's waits, and a narrowing doesn't.
        let mut m = sim("net-member");
        apply(&mut m, "/api/net/settings", &json!({ "host": "studio", "wake": "network" }), now);
        assert_eq!((reach(&m, "studio")["wake"].clone(), reach(&m, "studio")["pending"][0]["by"].clone()), (json!("consent"), json!("laptop")));
        apply(&mut m, "/api/net/settings", &json!({ "host": "studio", "start": "off" }), now);
        assert_eq!(reach(&m, "studio")["start"], "off");
        let id = reach(&m, "studio")["pending"][0]["id"].as_str().unwrap().to_string();
        apply(&mut m, "/api/net/settings", &json!({ "host": "studio", "approve": id }), now);
        assert_eq!((reach(&m, "studio")["wake"].clone(), reach(&m, "studio")["pending"].clone()), (json!("network"), json!([])));
    }

    #[test]
    fn remove_replace_settings_create_join_and_leave() {
        let now = 1.8e12;
        let mut s = sim("net-founder");
        assert_eq!(apply(&mut s, "/api/net/remove", &json!({ "host": "studio" }), now).status, 400);
        assert_eq!(apply(&mut s, "/api/net/remove", &json!({ "host": "nope" }), now).status, 404);
        assert_eq!(apply(&mut s, "/api/net/remove", &json!({ "host": "rack" }), now).status, 200);
        assert!(!has_host(&s.state, "rack") && !s.agents.iter().any(|a| a["host"] == "rack"));
        let r = apply(&mut s, "/api/net/replace", &json!({ "host": "laptop" }), now).body_json();
        assert_eq!((r["replaced"].clone(), r["window"].clone()), (json!("laptop"), json!(true)));
        assert_eq!(apply(&mut s, "/api/net/settings", &json!({ "titlesOnly": true }), now).body_json()["titlesOnly"], true);
        assert_eq!(apply(&mut s, "/api/net/create", &json!({ "name": "x", "host": "desk" }), now).status, 409);
        assert_eq!(apply(&mut s, "/api/net/leave", &json!({}), now).body_json()["network"], Value::Null);
        let made = apply(&mut s, "/api/net/create", &json!({ "name": "home", "host": "desk" }), now).body_json();
        well_formed(&made);
        assert_eq!((made["self"]["id"].clone(), made["gateway"].clone(), made["hosts"].as_array().unwrap().len()), (json!("desk"), Value::Null, 1));
        assert_eq!(apply(&mut s, "/api/net/ticket", &json!({ "host": "laptop" }), now).status, 409, "no address yet");
        apply(&mut s, "/api/net/leave", &json!({}), now);
        assert_eq!(apply(&mut s, "/api/net/join", &json!({ "address": "gw.home.lan:47850", "ticket": "nope" }), now).status, 400);
        let joined = apply(&mut s, "/api/net/join", &json!({ "address": "gw.home.lan:47850", "ticket": "KX7Q-49MZ-T2" }), now).body_json();
        well_formed(&joined);
        assert_eq!(joined["gateway"]["address"], "gw.home.lan:47850");
    }

    struct Window;
    #[async_trait]
    impl Verify for Window {
        async fn window(&self, _peer: Peer) -> Value {
            json!({ "ok": true })
        }
    }

    fn dev_ctx() -> Arc<Ctx> {
        let mut cores = Cores::pending(false, false, false);
        cores.verify = Arc::new(Window);
        let home = |k: &str| (k == "HOME" || k == "USERPROFILE").then(|| "/home/nova".to_string());
        let mut config = crate::config::Config::from(&["--demo".to_string(), "--dev".to_string()], &home);
        config.dev = true;
        let tmp = std::env::temp_dir().join(format!("xenonet-net-sim-test-{}", std::process::id()));
        config.state_dir = tmp.join("state").to_string_lossy().into_owned();
        config.cache_dir = tmp.join("cache").to_string_lossy().into_owned();
        Arc::new(Ctx::new(config, "0.0.0".into(), cores, xenonet_base::log::Log::new(None, "service", None, None, "/home/nova", 1 << 20)))
    }

    #[tokio::test]
    async fn the_state_route_answers_the_simulation_until_clear_and_the_real_state_after() {
        let ctx = dev_ctx();
        let key = ctx.window_key.reveal().to_string();
        let get = |ctx: Arc<Ctx>| {
            let key = key.clone();
            async move {
                let mut req = Req::fake("GET", "/api/net/state", &[("x-xenonet-window", key.as_str())], None);
                assert!(route(&ctx, &mut req).await.unwrap().is_none(), "never this one's");
                crate::routes::net::route(&ctx, &mut req).await.unwrap().unwrap()
            }
        };
        // Nothing simulated: the core's route, here with no network to read.
        assert_eq!(get(ctx.clone()).await.status, 503);
        assert!(simulate(&ctx, "net-founder", None, None).is_some());
        assert_eq!(get(ctx.clone()).await.body_json()["network"]["name"], "home");
        // Simulated or not, the state is the owner's: no window key and no
        // person, no state.
        let mut bare = Req::fake("GET", "/api/net/state", &[], None);
        assert!(route(&ctx, &mut bare).await.unwrap().is_none());
        assert_eq!(crate::routes::net::route(&ctx, &mut bare).await.unwrap().unwrap().status, 403);
        assert!(simulated_net_state(&ctx).is_some());
        // The engine's state and its list have the other hosts' agents.
        for path in ["/api/engine/state", "/api/engine/agents"] {
            let got = route(&ctx, &mut Req::fake("GET", path, &[], None)).await.unwrap().unwrap().body_json();
            let list = if path.ends_with("state") { got["agents"].clone() } else { got };
            assert!(list.as_array().unwrap().iter().any(|a| a["host"] == "laptop"), "{path}");
        }
        // Another service's simulation is its own.
        assert!(simulated_net_state(&dev_ctx()).is_none());
        assert!(clear(&ctx) && !clear(&ctx));
        assert_eq!(get(ctx.clone()).await.status, 503);
        // Nothing simulated: the rest is the real routes'.
        assert!(route(&ctx, &mut Req::fake("POST", "/api/net/ticket", &[], Some("{}"))).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn writes_and_messages_are_the_windows_and_a_message_to_a_host_thats_away_waits() {
        let ctx = dev_ctx();
        simulate(&ctx, "net-gateway-down", None, None).unwrap();
        let key = ctx.window_key.reveal().to_string();
        let with_key = [("x-xenonet-window", key.as_str())];
        let ticket = |headers: &[(&str, &str)]| Req::fake("POST", "/api/net/ticket", headers, Some(r#"{"host":"desk"}"#));
        assert_eq!(route(&ctx, &mut ticket(&[])).await.unwrap().unwrap().status, 403);
        assert_eq!(route(&ctx, &mut ticket(&with_key)).await.unwrap().unwrap().status, 200);
        assert_eq!(simulated_net_state(&ctx).unwrap()["tickets"][0]["host"], "desk");
        let to = simulated_remote_agents(&ctx).unwrap().into_iter().find(|a| a["host"] == "studio").unwrap()["id"].clone();
        let body = json!({ "to": to, "text": "Hold the release until the tests pass" }).to_string();
        let msg = |headers: &[(&str, &str)]| Req::fake("POST", "/api/actors/you/message", headers, Some(&body));
        assert_eq!(route(&ctx, &mut msg(&[])).await.unwrap().unwrap().status, 403);
        let sent = route(&ctx, &mut msg(&with_key)).await.unwrap().unwrap().body_json();
        assert_eq!((sent["state"].clone(), sent["host"].clone()), (json!("queued"), json!("studio")));
        assert!(sent["mid"].as_str().unwrap().starts_with("msg_"));
        assert_eq!(simulated_net_state(&ctx).unwrap()["outbox"]["frames"], 4);
        // A message for anyone else goes on to the real route, its body whole.
        let mut other = Req::fake("POST", "/api/actors/you/message", &with_key, Some(r#"{"to":"actor:wren","text":"hi"}"#));
        assert!(route(&ctx, &mut other).await.unwrap().is_none());
        assert_eq!(other.json(1024).await.unwrap()["to"], "actor:wren");
        clear(&ctx);
    }

    #[test]
    fn each_made_up_machine_has_a_load_of_its_own_that_fits_system_v1() {
        let doc = xenonet_base::json::parse(include_str!("../../../../schemas/system/v1.json")).unwrap();
        let checker = xenonet_schema::check::compile(&[doc]).unwrap();
        let now = 1_800_000_000_000.0;
        let mut threads = Vec::new();
        for host in ["studio", "laptop", "rack"] {
            let r = made_up_reading(host, now).unwrap();
            let c = checker.check(checker.root(0), &xenonet_schema::check::Data::from(&r), true);
            assert!(c.ok(), "{host}: {r}");
            threads.push(r["cpu"]["threads"].as_u64().unwrap());
        }
        assert_eq!(threads, [24, 12, 32], "each its own machine");
        assert_eq!(made_up_reading("rack", now).unwrap()["gpus"].as_array().unwrap().len(), 2);
        assert!(made_up_reading("desk", now).is_none());
    }
}
