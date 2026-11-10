//! Delivery: its policy (kept in delivery.json with its audit log beside
//! it), the router that puts messages into live sessions (with its ways
//! in: a listening Stop hook, a Claude Code channel, OpenCode's server,
//! tmux, a headless resume) and Claude Code's channels. The decisions are the engine's cores'; the
//! files, commands, HTTP, timers and events are here, behind the cores'
//! host traits.

use crate::engine::Side;
use crate::exec::{self, Spec};
use crate::runs::{Runner, Turn};
use crate::runtime::Runtime;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use xenonet_base::sync::Locked;
use xenonet_base::text::js_number;
use xenonet_base::time::now_ms;
use xenonet_engine::delivery::channel::{Channels as ChannelCore, Push};
use xenonet_engine::delivery::opencode::{self as oc, OpenCode, Posted};
use xenonet_engine::delivery::policy::{self, Fail, Policy as PolicyCore, WakeAllowed};
use xenonet_engine::delivery::reach::Wake;
use xenonet_engine::delivery::resume;
use xenonet_engine::delivery::router::{self, Delivery, Router as RouterCore};
use xenonet_engine::delivery::tmux::Tmux;
use xenonet_engine::delivery::usage::plan_usage;
use xenonet_engine::delivery::{Delivered, Error, Files};
use xenonet_service::cores::{Channels, CoreError, Policy, Router, R};
use xenonet_service::glue::From;

// ---------- files ----------

/// The disk, with errors worded as Node words them.
struct Fs;

impl Files for Fs {
    fn read_file(&mut self, path: &str) -> Result<String, String> {
        crate::fs::read_text(path).map_err(|e| e.message)
    }
    fn mkdir_all(&mut self, dir: &str) -> Result<(), String> {
        crate::fs::mkdir_all(dir).map_err(|e| e.message)
    }
    fn write_file(&mut self, path: &str, body: &str, mode: Option<u32>) -> Result<(), String> {
        crate::fs::write_file(path, body.as_bytes(), mode).map_err(|e| e.message)
    }
    fn append_file(&mut self, path: &str, body: &str, mode: Option<u32>) -> Result<(), String> {
        crate::fs::append_file(path, body.as_bytes(), mode).map_err(|e| e.message)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        crate::fs::rename(from, to).map_err(|e| e.message)
    }
}

// ---------- the policy ----------

impl policy::Host for Fs {
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

pub struct PolicySide {
    core: Mutex<PolicyCore>,
}

/// A refusal's or error's words.
fn fail_message(f: Fail) -> String {
    match f {
        Fail::Refused { why, .. } | Fail::Error(why) => why,
    }
}

fn core_error(f: Fail) -> CoreError {
    match f {
        Fail::Refused { why, code } => CoreError::of(xenonet_service::cores::Kind::Error, 429, code, why),
        Fail::Error(m) => CoreError::new(m),
    }
}

impl PolicySide {
    pub fn new(file: Option<String>, audit_file: Option<String>) -> PolicySide {
        PolicySide { core: Mutex::new(PolicyCore::load(&mut Fs, file, audit_file)) }
    }

    /// The policy, with its host, under its lock.
    pub fn with<T>(&self, f: impl FnOnce(&mut PolicyCore, &mut dyn policy::Host) -> T) -> T {
        let mut p = self.core.locked();
        f(&mut p, &mut Fs)
    }

    /// `policy.data.consent`.
    pub fn consent(&self) -> Value {
        self.with(|p, _| p.consent())
    }
}

impl policy::Guard for PolicySide {
    fn visible(&self, caller: &str, facts: &Value) -> policy::Result<bool> {
        Ok(self.with(|p, h| p.visible(h, caller, facts)))
    }
    fn check_send(&self, sender: &str, recipients: &[Value], body: &str, depth: f64) -> policy::Result<()> {
        self.with(|p, h| p.check_send(h, sender, recipients, body, depth))
    }
    fn text_for(&self, body: &str, sender: &str, sender_facts: &Value, recipient_facts: &Value) -> (String, usize) {
        policy::text_for(body, sender, sender_facts, recipient_facts)
    }
    fn audit(&self, entry: Value) -> policy::Result<Value> {
        self.with(|p, h| p.audit(h, entry))
    }
}

pub struct PolicyDriver(pub std::sync::Arc<Runtime>);

impl Policy for PolicyDriver {
    fn paused(&self) -> R<bool> {
        Ok(self.0.policy.with(|p, _| p.paused()))
    }
    fn grants(&self) -> R<Value> {
        Ok(Value::Array(self.0.policy.with(|p, h| p.grants(h))))
    }
    fn consent(&self) -> R<Value> {
        Ok(self.0.policy.consent())
    }
    fn read_audit(&self, n: f64) -> R<Value> {
        Ok(Value::Array(self.0.policy.with(|p, h| p.read_audit(h, n))))
    }
    fn set_paused(&self, paused: bool) -> R<Value> {
        self.0.policy.with(|p, h| p.set_paused(h, paused, "you")).map_err(core_error)
    }
    fn grant(&self, actor: &str, scope: &str, ttl_ms: f64) -> R<Value> {
        self.0.policy.with(|p, h| p.grant(h, actor, scope, Some(ttl_ms), "you")).map_err(core_error)
    }
    fn revoke(&self, actor: &str) -> R<Value> {
        self.0.policy.with(|p, h| p.revoke(h, actor, "you")).map_err(core_error)
    }
    fn allow(&self, session: &str, remote: bool, ttl_ms: f64) -> R<Value> {
        self.0.policy.with(|p, h| p.allow(h, session, remote, Some(ttl_ms), "you")).map_err(core_error)
    }
    fn disallow(&self, session: &str) -> R<()> {
        self.0.policy.with(|p, h| p.disallow(h, session, "you")).map_err(core_error)
    }
}

// ---------- Claude Code's channels ----------

/// The poll waiting for each session: its number, and where its items go.
type Polls = HashMap<String, (u64, tokio::sync::oneshot::Sender<Vec<Value>>)>;

#[derive(Default)]
pub struct ChannelSide {
    core: Mutex<ChannelCore>,
    waiting: Mutex<Polls>,
    polls: std::sync::atomic::AtomicU64,
}

impl ChannelSide {
    pub fn live(&self, agent: &str) -> bool {
        let waiting = self.waiting.locked().contains_key(agent);
        self.core.locked().live(agent, waiting, &mut now_ms)
    }

    /// Hands an item to the session's poll; false when no channel is live.
    pub fn push(&self, agent: &str, item: Value) -> bool {
        let mut waiting = self.waiting.locked();
        let went = self.core.locked().push(agent, item.clone(), waiting.contains_key(agent), &mut now_ms);
        if went == Push::Waiter {
            if let Some((_, tx)) = waiting.remove(agent) {
                self.core.locked().polled(agent, now_ms());
                let _ = tx.send(vec![item]);
            }
        }
        went != Push::NotLive
    }

    /// Waits up to `wait` for items for `agent`; none on time out.
    pub async fn poll(&self, agent: &str, wait: Duration) -> Vec<Value> {
        if let Some(queued) = self.core.locked().poll(agent, now_ms()) {
            return queued;
        }
        let no = self.polls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        // A poll already waiting for the session ends empty.
        if let Some((_, old)) = self.waiting.locked().insert(agent.to_string(), (no, tx)) {
            let _ = old.send(Vec::new());
        }
        let ms = ChannelCore::wait_ms(wait.as_secs_f64() * 1000.0);
        let ms = if ms.is_nan() { 0.0 } else { ms };
        match tokio::time::timeout(Duration::from_secs_f64(ms / 1000.0), rx).await {
            Ok(Ok(items)) => items,
            _ => {
                let mut w = self.waiting.locked();
                if w.get(agent).is_some_and(|(n, _)| *n == no) {
                    w.remove(agent);
                }
                drop(w);
                self.core.locked().polled(agent, now_ms());
                Vec::new()
            }
        }
    }
}

pub struct ChannelsDriver(pub std::sync::Arc<Runtime>);

#[async_trait]
impl Channels for ChannelsDriver {
    async fn poll(&self, me: &str, wait: Duration) -> R<Value> {
        Ok(Value::Array(self.0.channels.poll(me, wait).await))
    }
}

// ---------- the router and its ways in ----------

/// Headless resume: the shared runner, and the settings file a resumed
/// Claude Code turn listens with.
pub struct Resumes {
    pub runner: std::sync::Arc<Runner>,
    pub settings: Option<String>,
}

pub struct RouterSide {
    pub core: RouterCore,
    timers: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    opencode: OpenCode,
    tmux: Tmux,
    pub resume: Option<Resumes>,
}

impl RouterSide {
    /// `state_dir` is where the runs' logs and records go.
    pub fn new(demo: bool, listen_file: Option<String>, state_dir: &std::path::Path) -> RouterSide {
        RouterSide {
            core: RouterCore::default(),
            timers: Mutex::new(HashMap::new()),
            opencode: OpenCode::new(10e3),
            tmux: Tmux { off: cfg!(windows) },
            // Made-up sessions are never resumed.
            resume: (!demo).then(|| Resumes { runner: Runner::new(Some(state_dir.to_path_buf())), settings: listen_file }),
        }
    }

    /// The latest deliveries, as JSON.
    pub fn list(&self, n: usize) -> Vec<Value> {
        self.core.list(n)
    }
}

/// The router's collaborators: the clock, the plan readings, the network's
/// reach, the events and timers, the engine, the policy, the channels and
/// the ways in.
pub struct RouterHost<'a> {
    pub w: &'a Runtime,
    /// Who was listening when the message was sent, for a route that runs
    /// after it (route()); None asks the engine now.
    pub listening: Option<&'a [String]>,
}

impl<'a> RouterHost<'a> {
    pub fn of(w: &'a Runtime) -> Self {
        RouterHost { w, listening: None }
    }
}

fn windows() -> bool {
    cfg!(windows)
}

/// A provider's plan as the wake budget reads it, from the live snapshot's
/// limits.
fn usage(w: &Runtime, app: &str) -> Option<f64> {
    let limits = match w.state().live_snapshot() {
        Ok(s) => s.get("limits").cloned().unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };
    plan_usage(&limits, app, now_ms())
}

fn store_error(e: xenonet_engine::sql::SqlError) -> Error {
    Error(e.message().to_string())
}

/// A turn resumed for a session (or, `fork`, a copy of it) with the
/// message as its prompt: where it runs, or why it couldn't.
fn resume_deliver(w: &Runtime, agent: &str, cwd: Option<&str>, input: &str, fork: bool) -> Delivered {
    let Some(r) = &w.router.resume else { return Err("there's no resume here".into()) };
    let settings = r.settings.as_deref();
    let mut plans = ResumePlan { r };
    // A copy of a Claude Code session a process holds, or a turn of one
    // whose process has gone.
    let plan = if fork { resume::plan_fork(&mut plans, agent, settings, cwd) } else { resume::plan(&mut plans, agent, settings, cwd) };
    let (app, args) = plan.map_err(str::to_string)?;
    let mut env: Vec<(String, String)> = resume::resume_env(std::env::vars().collect());
    // Claude Code signs in with the token you gave XenoNet's runs, and
    // asks its permission prompts of you through this XenoNet's server,
    // waiting a minute past the hold for your answer, as a start does.
    if app == "claude" {
        crate::starts::signed_env(w, &mut env);
        env.retain(|(k, _)| k != "MCP_TOOL_TIMEOUT");
        env.push(("MCP_TOOL_TIMEOUT".into(), crate::starts::mcp_tool_timeout_ms(w).to_string()));
    }
    let server = xenonet_engine::delivery::setup::mcp_server_name(f64::from(w.setup.port));
    let mut args = resume::with_permission_tool(&app, args, &server);
    let key = if fork { format!("fork:{agent}") } else { agent.to_string() };
    let cwd = cwd.unwrap_or_default();
    let mut input = input.to_string();
    // Grok Build reads its prompt from a file of XenoNet's.
    let prompt_file = if args.last().map(String::as_str) == Some("--prompt-file") {
        let f = crate::starts::prompt_file(w, &input)?;
        args.push(f.clone());
        input.clear();
        Some(f)
    } else {
        None
    };
    let turn = Turn { key: &key, app: &app, args: &args, cwd, env: &env, text: input };
    match crate::starts::run_turn(w, &r.runner, turn) {
        Ok(pid) => {
            crate::starts::after_resume(w, &r.runner, &app, &key, pid, fork.then_some(agent), prompt_file);
            Ok(if fork { format!("pid {pid}, a copy of the session") } else { format!("pid {pid}") })
        }
        Err(why) => {
            if let Some(f) = prompt_file {
                let _ = std::fs::remove_file(f);
            }
            Err(why)
        }
    }
}

/// What a resume's plan asks: the runner, and the disk.
struct ResumePlan<'a> {
    r: &'a Resumes,
}

impl resume::Host for ResumePlan<'_> {
    fn running(&mut self, key: &str) -> bool {
        self.r.runner.running(key)
    }
    fn exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
}

/// A command for the ways in: its output, or Err when it failed to run or
/// failed.
fn run_quiet(cmd: &str, args: &[&str], ms: u64, max: usize) -> Result<String, String> {
    let mut s = Spec::new(cmd, &args.iter().map(|a| (*a).to_string()).collect::<Vec<_>>(), ms);
    s.max = max;
    exec::run(&s).map_err(|f| f.message)
}

/// ps and the files of /proc, as finding OpenCode's servers reads them.
struct Ps;

impl oc::System for Ps {
    fn run(&mut self, cmd: &str, args: &[&str]) -> String {
        run_quiet(cmd, args, 3000, 4 << 20).unwrap_or_default()
    }
    fn read_file(&mut self, path: &str) -> Option<String> {
        std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    fn readdir(&mut self, path: &str) -> Option<Vec<String>> {
        xenonet_base::paths::read_dir_sorted(std::path::Path::new(path)).ok().map(|ents| ents.into_iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    }
    fn readlink(&mut self, path: &str) -> Option<String> {
        std::fs::read_link(path).ok().map(|t| t.to_string_lossy().into_owned())
    }
}

/// OpenCode's servers, found and asked: the clock, the processes, and HTTP.
struct OpenCodeHost<'a> {
    w: &'a Runtime,
}

impl OpenCodeHost<'_> {
    fn fetch(&self, method: &str, url: &str, body: Option<String>, ms: f64) -> Result<xenonet_service::net::Fetched, String> {
        let headers = if body.is_some() { vec![("content-type".to_string(), "application/json".to_string())] } else { Vec::new() };
        let timeout = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
        self.w.block_on(xenonet_service::net::fetch(method, url, &headers, body.map(String::into_bytes), timeout))
    }
}

impl oc::Host for OpenCodeHost<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn processes(&mut self) -> Vec<oc::Proc> {
        oc::open_code_processes(&mut Ps, windows())
    }
    fn ports(&mut self, pid: f64) -> Vec<f64> {
        oc::listening_ports(&mut Ps, pid, crate::runtime::platform())
    }
    fn get(&mut self, url: &str, ms: f64) -> Option<Value> {
        let r = self.fetch("GET", url, None, ms).ok().filter(xenonet_service::net::Fetched::ok)?;
        xenonet_base::json::parse(&r.text()).ok()
    }
    fn post(&mut self, url: &str, body: &str) -> Result<Posted, String> {
        let r = self.fetch("POST", url, Some(body.to_string()), 3000.0)?;
        Ok(Posted { ok: r.ok(), status: f64::from(r.status) })
    }
}

/// tmux and ps.
struct TmuxShell;

impl xenonet_engine::delivery::tmux::Shell for TmuxShell {
    fn exec(&mut self, cmd: &str, args: &[&str]) -> Option<String> {
        run_quiet(cmd, args, 3000, 1 << 20).ok()
    }
}

impl router::Host for RouterHost<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn usage(&mut self, app: &str) -> Option<f64> {
        usage(self.w, app)
    }
    fn wake_setting(&mut self) -> Wake {
        Wake::parse(&crate::network::net(self.w).reach().wake)
    }
    fn notify(&mut self, notice: Value) {
        self.w.emit(From::Delivery, "notify", notice);
    }
    fn emit(&mut self, record: Value) {
        self.w.emit(From::Delivery, "delivery", record);
    }
    fn arm(&mut self, key: &str, ms: f64) {
        let w = self.w;
        let k = key.to_string();
        let t = w.after(ms, move |w| {
            w.router.timers.locked().remove(&k);
            let _ = w.router.core.expire(&mut RouterHost::of(w), &k);
        });
        if let Some(old) = w.router.timers.locked().insert(key.to_string(), t) {
            old.abort();
        }
    }
    fn clear(&mut self, key: &str) {
        if let Some(t) = self.w.router.timers.locked().remove(key) {
            t.abort();
        }
    }

    fn view(&mut self, id: &str) -> Option<Value> {
        self.w.engine.view(&Side::new(self.w, None).collaborators(), id).ok()
    }
    fn listening(&mut self, id: &str) -> bool {
        self.listening.map_or_else(|| self.w.mail.listening_has(id), |l| l.iter().any(|x| x == id))
    }
    fn contact_pid(&mut self, agent: &str) -> Option<f64> {
        self.w.store.with(|s| s.contact(agent)).ok().flatten().and_then(|r| r.num("pid"))
    }
    fn mark_seen(&mut self, agent: &str, ids: &[Value], now: f64) -> xenonet_engine::delivery::Result<()> {
        let ids: Vec<f64> = ids.iter().map(|id| js_number(Some(id))).collect();
        self.w.store.with(|s| s.mark_seen(agent, &ids, now)).map_err(store_error)
    }
    fn mark_unseen(&mut self, agent: &str, ids: &[Value]) -> xenonet_engine::delivery::Result<()> {
        let ids: Vec<f64> = ids.iter().map(|id| js_number(Some(id))).collect();
        self.w.store.with(|s| s.mark_unseen(agent, &ids)).map_err(store_error)
    }

    fn consented(&mut self, session: &str, remote: bool) -> bool {
        self.w.policy.with(|p, h| p.consented(h, session, remote))
    }
    fn consent_sessions(&mut self) -> Vec<String> {
        match self.w.policy.consent() {
            Value::Object(m) => m.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }
    fn wake_allowed(&mut self, sender: &str, target: &str, usage: Option<f64>) -> WakeAllowed {
        self.w.policy.with(|p, h| p.wake_allowed(h, sender, target, usage))
    }
    fn spend_wake(&mut self, sender: &str, target: &str) {
        self.w.policy.with(|p, h| p.spend_wake(h, sender, target));
    }
    fn audit(&mut self, entry: Value) -> xenonet_engine::delivery::Result<()> {
        self.w.policy.with(|p, h| p.audit(h, entry)).map(|_| ()).map_err(|f| Error(fail_message(f)))
    }

    fn channel_live(&mut self, agent: &str) -> bool {
        self.w.channels.live(agent)
    }
    fn channel_push(&mut self, agent: &str, item: Value) -> bool {
        self.w.channels.push(agent, item)
    }
    fn opencode_info(&mut self, id: &str) -> Option<Value> {
        self.w.router.opencode.info(&mut OpenCodeHost { w: self.w }, id, windows())
    }
    fn opencode_deliver(&mut self, id: &str, body: &str) -> Delivered {
        self.w.router.opencode.deliver(&mut OpenCodeHost { w: self.w }, id, body, windows()).unwrap_or_else(|e| Err(e.0))
    }
    fn tmux_deliver(&mut self, pid: f64, body: &str) -> Delivered {
        self.w.router.tmux.deliver(&mut TmuxShell, pid, body).unwrap_or_else(|e| Err(e.0))
    }
    fn resumes(&mut self) -> bool {
        self.w.router.resume.is_some()
    }
    fn resume_deliver(&mut self, agent: &str, cwd: Option<&str>, body: &str, fork: bool) -> Delivered {
        resume_deliver(self.w, agent, cwd, body, fork)
    }
}

/// A message routed: each of its sessions' way in tried. What failed goes
/// out as the router's error event. `listening` is who was listening when it
/// was sent: a listening hook can take the message and go before the route
/// runs.
pub fn route(w: &Runtime, m: &Value, listening: &[String]) {
    let mut h = RouterHost { w, listening: Some(listening) };
    if let Err(e) = w.router.core.route(&mut h, m) {
        w.emit(From::Delivery, "error", serde_json::json!({ "message": e.0 }));
    }
}

/// Messages the engine says a session has seen.
pub fn seen(w: &Runtime, agent: &str, ids: &[Value]) {
    w.router.core.seen(&mut RouterHost::of(w), agent, ids);
}

/// A hook from the session.
pub fn contact(w: &Runtime, agent: &str) {
    w.router.core.contact(&mut RouterHost::of(w), agent);
}

pub struct RouterDriver(pub std::sync::Arc<Runtime>);

impl Router for RouterDriver {
    fn usage(&self, app: &str) -> R<Value> {
        Ok(usage(&self.0, app).map_or(Value::Null, xenonet_base::json::number))
    }
    fn list(&self, n: usize) -> R<Value> {
        Ok(Value::Array(self.0.router.core.list(n)))
    }
}

impl Delivery for RouterDriver {
    fn known(&self) -> Vec<String> {
        self.0.router.core.known(&mut RouterHost::of(&self.0))
    }
    fn reaches(&self, id: &str) -> bool {
        self.0.router.core.reaches(&mut RouterHost::of(&self.0), id)
    }
    fn resumable(&self, view: &Value) -> bool {
        self.0.router.core.resumable(&mut RouterHost::of(&self.0), view)
    }
    fn wake(&self, from: &str, agent: &str) -> WakeAllowed {
        self.0.router.core.wake(&mut RouterHost::of(&self.0), from, agent)
    }
    fn way(&self, me: &str, recipient: &Value) -> Option<&'static str> {
        self.0.router.core.way(&mut RouterHost::of(&self.0), me, recipient)
    }
    fn describe(&self, id: &str) -> Option<Value> {
        self.0.router.core.describe(&mut RouterHost::of(&self.0), id)
    }
}

/// The settings a Claude Code session starts with to listen after its
/// turns (writeListenSettings), written to the state folder: their file.
pub fn listen_settings(dir: &str, port: u16) -> Result<String, String> {
    xenonet_engine::delivery::setup::write_listen_settings(&mut Fs, dir, f64::from(port), 300.0).map_err(|e| e.0)
}
