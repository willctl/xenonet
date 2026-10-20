//! Delivery (core/delivery/): its policy (policy.js, kept in delivery.json
//! with its audit log beside it), the router that puts messages into live
//! sessions (router.js, with its ways in: a listening Stop hook, a Claude
//! Code channel, OpenCode's server, tmux, a headless resume) and Claude
//! Code's channels (channel.js). The decisions are the Rust cores'; the
//! files, commands, HTTP, timers and the records' objects are here.

use crate::engine::EngineCalls;
use crate::exec::{self, Spec};
use crate::js::Locked;
use crate::js::{arg, at, json, list, no_method, now_ms, obj, text};
use crate::world::World;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use xenonet_engine::delivery::channel::{Channels as ChannelCore, Push};
use xenonet_engine::delivery::opencode::{self as oc, OpenCode};
use xenonet_engine::delivery::policy::{self, Fail, Policy as PolicyCore};
use xenonet_engine::delivery::resume;
use xenonet_engine::delivery::router::{Router as RouterCore, ACK_MS};
use xenonet_engine::delivery::tmux::Tmux;
use xenonet_engine::delivery::usage::plan_usage;
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_service::cores::{Channels, CoreError, Policy, Router, R};
use xenonet_service::glue::From;

// ---------- the policy ----------

pub struct PolicySide {
    core: Mutex<PolicyCore>,
    file: Option<String>,
    audit_file: Option<String>,
    limits: Value,
}

/// The policy's own collaborators: its clock, limits and files, and fs.
struct PolicyCalls<'a> {
    side: &'a PolicySide,
}

impl Calls for PolicyCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("policy", "now") => Ok(Arg::Num(now_ms())),
            ("fs", m) => crate::fsx::call(m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "policy.file" => self.side.file.clone().map_or(Arg::Null, Arg::Text),
            "policy.auditFile" => self.side.audit_file.clone().map_or(Arg::Null, Arg::Text),
            p => match p.strip_prefix("policy.limits.") {
                Some(k) => self.side.limits.get(k).map_or(Arg::Undef, arg),
                None => Arg::Undef,
            },
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        let v = self.get(path)?;
        Ok(crate::js::type_of(&v).into())
    }
}

/// A policy refusal or error, as the JS threw it: a refusal has its code
/// and status 429.
pub fn policy_thrown(f: Fail) -> (Thrown, Option<u16>) {
    match f {
        Fail::Refused { why, code } => (Thrown::coded(why, code), Some(429)),
        Fail::Error(m) => (Thrown::new(m), None),
        Fail::Thrown(t) => (t, None),
    }
}

fn opt(a: Arg) -> Option<Arg> {
    (!matches!(a, Arg::Undef)).then_some(a)
}

fn by(a: Arg) -> Arg {
    match a {
        Arg::Undef => Arg::text("you"),
        a => a,
    }
}

impl PolicySide {
    pub fn new(file: Option<String>, audit_file: Option<String>) -> PolicySide {
        let mut side = PolicySide { core: Mutex::new(PolicyCore::default()), file, audit_file, limits: policy::limits() };
        let loaded = PolicyCore::load(&mut PolicyCalls { side: &side });
        if let Ok(p) = loaded {
            side.core = Mutex::new(p);
        }
        side
    }

    /// `policy.<method>(...args)`, as the DeliveryPolicy object answers.
    pub fn call(&self, method: &str, args: &[Arg]) -> std::result::Result<Arg, Fail> {
        let c = &mut PolicyCalls { side: self };
        let mut p = self.core.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let x = |i| at(args, i);
        match method {
            "paused" => Ok(Arg::Bool(p.paused())),
            "setPaused" => p.set_paused(c, x(0).truthy(), &by(x(1))),
            "grant" => {
                let o = x(2);
                p.grant(c, &x(0), &x(1), opt(o.field("ttlMs")).as_ref(), &by(o.field("by")))
            }
            "revoke" => p.revoke(c, &x(0), &by(x(1))),
            "grants" => p.grants(c),
            "scopeOf" => p.scope_of(c, &x(0)).map(|s| s.to_arg()),
            "visible" => p.visible(c, &x(0), &x(1)).map(Arg::Bool),
            "checkSend" => {
                let o = x(0);
                let depth = match o.field("depth") {
                    Arg::Undef => 0.0,
                    d => crate::js::num(&d),
                };
                p.check_send(c, &o.field("sender"), &list(o.field("recipients")), &o.field("text"), depth).map(|()| Arg::Undef)
            }
            "textFor" => {
                let o = x(0);
                let (t, n) = policy::text_for(&o.field("text"), &o.field("sender"), &o.field("senderFacts"), &o.field("recipientFacts"));
                Ok(obj(vec![("text", t), ("redacted", Arg::Num(n as f64))]))
            }
            "allow" => {
                let o = x(1);
                p.allow(c, &x(0), o.field("remote").truthy(), opt(o.field("ttlMs")).as_ref(), &by(o.field("by")))
            }
            "disallow" => p.disallow(c, &x(0), &by(x(1))).map(|()| Arg::Undef),
            "consented" => p.consented(c, &x(0), x(1).field("remote").truthy()),
            "wakeAllowed" => p.wake_allowed(c, &x(0), &x(1), &x(2).field("usage").or(Arg::Null)),
            "spendWake" => p.spend_wake(c, &x(0), &x(1)).map(|()| Arg::Undef),
            "audit" => p.audit(c, x(0)),
            "readAudit" => p.read_audit(
                c,
                match x(0) {
                    Arg::Undef => 100.0,
                    l => crate::js::num(&l),
                },
            ),
            _ => Err(Fail::Thrown(no_method("policy", method))),
        }
    }

    pub fn data(&self) -> Arg {
        self.core.lock().unwrap_or_else(std::sync::PoisonError::into_inner).data_arg()
    }

    /// A policy call as a collaborator's, its refusals coded.
    pub fn as_call(&self, method: &str, args: &[Arg]) -> Result<Arg> {
        self.call(method, args).map_err(|f| policy_thrown(f).0)
    }
}

fn core_error(f: Fail) -> CoreError {
    match f {
        Fail::Refused { why, code } => CoreError::of(xenonet_service::cores::Kind::Error, 429, code, why),
        Fail::Error(m) => CoreError::new(m),
        Fail::Thrown(t) => CoreError::new(t.message),
    }
}

pub struct PolicyDriver(pub std::sync::Arc<World>);

impl Policy for PolicyDriver {
    fn paused(&self) -> R<bool> {
        Ok(self.0.policy.call("paused", &[]).map_err(core_error)?.truthy())
    }
    fn grants(&self) -> R<Value> {
        self.0.policy.call("grants", &[]).map(|a| json(&a)).map_err(core_error)
    }
    fn consent(&self) -> R<Value> {
        Ok(json(&self.0.policy.data().field("consent")))
    }
    fn read_audit(&self, n: f64) -> R<Value> {
        self.0.policy.call("readAudit", &[Arg::Num(n)]).map(|a| json(&a)).map_err(core_error)
    }
    fn set_paused(&self, paused: bool) -> R<Value> {
        self.0.policy.call("setPaused", &[Arg::Bool(paused)]).map(|a| json(&a)).map_err(core_error)
    }
    fn grant(&self, actor: &str, scope: &str, ttl_ms: f64) -> R<Value> {
        self.0.policy.call("grant", &[Arg::text(actor), Arg::text(scope), obj(vec![("ttlMs", Arg::Num(ttl_ms))])]).map(|a| json(&a)).map_err(core_error)
    }
    fn revoke(&self, actor: &str) -> R<Value> {
        self.0.policy.call("revoke", &[Arg::text(actor)]).map(|a| json(&a)).map_err(core_error)
    }
    fn allow(&self, session: &str, remote: bool, ttl_ms: f64) -> R<Value> {
        self.0.policy.call("allow", &[Arg::text(session), obj(vec![("remote", Arg::Bool(remote)), ("ttlMs", Arg::Num(ttl_ms))])]).map(|a| json(&a)).map_err(core_error)
    }
    fn disallow(&self, session: &str) -> R<()> {
        self.0.policy.call("disallow", &[Arg::text(session)]).map(|_| ()).map_err(core_error)
    }
}

// ---------- Claude Code's channels ----------

/// The poll waiting for each session: its number, and where its items go.
type Polls = HashMap<String, (u64, tokio::sync::oneshot::Sender<Vec<Arg>>)>;

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
    pub fn push(&self, agent: &str, item: Arg) -> bool {
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
    pub async fn poll(&self, agent: &str, wait: Duration) -> Vec<Arg> {
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

pub struct ChannelsDriver(pub std::sync::Arc<World>);

#[async_trait]
impl Channels for ChannelsDriver {
    async fn poll(&self, me: &str, wait: Duration) -> R<Value> {
        Ok(Value::Array(self.0.channels.poll(me, wait).await.iter().map(json).collect()))
    }
}

// ---------- the router and its ways in ----------

/// A resumed turn's processes, by session.
#[derive(Default)]
pub struct Resumes {
    running: Mutex<HashMap<String, u32>>,
    settings: Option<String>,
}

/// The router's records as JS objects, which route() and list() give and
/// the delivery events copy: those listed, in the order they were.
#[derive(Default)]
struct Objs {
    objs: HashMap<String, Arg>,
    records: Vec<String>,
}

pub struct RouterSide {
    pub core: RouterCore,
    objs: Mutex<Objs>,
    timers: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    opencode: OpenCode,
    tmux: Tmux,
    resume: Option<Resumes>,
}

impl RouterSide {
    pub fn new(demo: bool, listen_file: Option<String>) -> RouterSide {
        RouterSide {
            core: RouterCore::default(),
            objs: Mutex::new(Objs::default()),
            timers: Mutex::new(HashMap::new()),
            opencode: OpenCode::new(10e3),
            tmux: Tmux { off: cfg!(windows) },
            // Made-up sessions are never resumed.
            resume: (!demo).then(|| Resumes { running: Mutex::new(HashMap::new()), settings: listen_file }),
        }
    }

    /// The latest deliveries.
    pub fn list(&self, n: usize) -> Vec<Arg> {
        let o = self.objs.locked();
        let from = o.records.len().saturating_sub(n);
        o.records[from..].iter().filter_map(|k| o.objs.get(k).cloned()).collect()
    }
}

/// The router's collaborators: the twin's own (now, usage, notify, ackMs,
/// the records' objects and timers), the engine, the policy, the channels
/// and the ways in.
pub struct RouterCalls<'a> {
    pub w: &'a World,
    /// Who was listening when the message was sent, for a route that runs
    /// after it (route()); None asks the engine now.
    pub listening: Option<&'a [String]>,
}

impl<'a> RouterCalls<'a> {
    pub fn of(w: &'a World) -> Self {
        RouterCalls { w, listening: None }
    }
}

fn windows() -> bool {
    cfg!(windows)
}

impl RouterCalls<'_> {
    fn usage(&self, app: &Arg) -> Arg {
        let limits = match self.w.state().live_snapshot() {
            Ok(s) => s.get("limits").map_or(Arg::Null, arg),
            Err(_) => Arg::Null,
        };
        plan_usage(&limits, app, now_ms()).map_or(Arg::Null, Arg::Num)
    }

    fn engine(&mut self) -> EngineCalls<'_> {
        EngineCalls::new(self.w, None)
    }

    fn resume_deliver(&self, o: &Arg) -> Arg {
        let no = |why: String| obj(vec![("ok", Arg::Bool(false)), ("why", Arg::text(why))]);
        let Some(r) = &self.w.router.resume else { return no("there's no resume here".into()) };
        let agent = o.field("agent");
        let cwd = o.field("cwd");
        let settings = r.settings.clone().map_or(Arg::Null, Arg::Text);
        let mut pc = ResumePlanCalls { r };
        let plan = match resume::plan(&mut pc, &agent, &settings, &cwd, windows()) {
            Ok(p) => p,
            Err(t) => return no(t.message),
        };
        let (command, args) = match plan {
            Err(why) => return no(why.into()),
            Ok(c) => c,
        };
        let env = resume::resume_env(std::env::vars().map(|(k, v)| (k, Arg::Text(v))).collect());
        let mut cmd = std::process::Command::new(&command);
        cmd.args(args.iter().map(text))
            .current_dir(text(&cwd))
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        for (k, v) in env {
            cmd.env(k, text(&v));
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return no(format!("{command} isn't installed here")),
            Err(e) => return no(e.to_string()),
        };
        let pid = child.id();
        let key = text(&agent);
        r.running.locked().insert(key.clone(), pid);
        if let Some(mut stdin) = child.stdin.take() {
            let t = text(&o.field("text"));
            std::thread::spawn(move || {
                let _ = std::io::Write::write_all(&mut stdin, t.as_bytes());
            });
        }
        // One turn, half an hour at most; then it's let go.
        let w = self.w.arc();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(30 * 60);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if std::time::Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(250)),
                }
            }
            if let Some(r) = w.as_ref().and_then(|w| w.router.resume.as_ref()) {
                r.running.locked().remove(&key);
            }
        });
        obj(vec![("ok", Arg::Bool(true)), ("where", Arg::text(format!("pid {pid}")))])
    }
}

struct ResumePlanCalls<'a> {
    r: &'a Resumes,
}

impl Calls for ResumePlanCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("", "running") => Ok(Arg::Bool(self.r.running.locked().contains_key(&text(&at(&args, 0))))),
            ("", "exists") => Ok(Arg::Bool(std::path::Path::new(&text(&at(&args, 0))).exists())),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("function".into())
    }
}

/// A command for the ways in: its output and whether it ran, or "" when it
/// failed, as each of them ran it.
fn run_quiet(cmd: &Arg, args: &Arg, ms: u64, max: usize) -> std::result::Result<String, String> {
    let mut s = Spec::new(&text(cmd), &list(args.clone()).iter().map(text).collect::<Vec<_>>(), ms);
    s.max = max;
    exec::run(&s).map_err(|f| f.message)
}

/// OpenCode's servers, found and asked: commands, files and HTTP.
struct OpenCodeCalls<'a> {
    w: &'a World,
}

impl OpenCodeCalls<'_> {
    fn fetch(&self, method: &str, url: &str, body: Option<String>, ms: f64) -> std::result::Result<xenonet_service::net::Fetched, Thrown> {
        let headers = if body.is_some() { vec![("content-type".to_string(), "application/json".to_string())] } else { Vec::new() };
        let timeout = Duration::from_secs_f64(ms.max(0.0) / 1000.0);
        self.w.block_on(xenonet_service::net::fetch(method, url, &headers, body.map(String::into_bytes), timeout)).map_err(Thrown::new)
    }
}

impl Calls for OpenCodeCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let x = |i| at(&args, i);
        match (path, method) {
            ("", "now") => Ok(Arg::Num(now_ms())),
            ("", "processes") => {
                let ps = oc::open_code_processes(&mut PsCalls, windows())?;
                Ok(Arg::List(ps.iter().map(xenonet_engine::delivery::opencode::Proc::to_arg).collect()))
            }
            ("", "ports") => {
                let ports = oc::listening_ports(&mut PsCalls, crate::js::num(&x(0)), crate::js::platform())?;
                Ok(Arg::List(ports.into_iter().map(Arg::Num).collect()))
            }
            ("", "get") => {
                let r = self.fetch("GET", &text(&x(0)), None, crate::js::num(&x(1)))?;
                if !r.ok() {
                    return Ok(obj(vec![("ok", Arg::Bool(false)), ("status", Arg::Num(f64::from(r.status)))]));
                }
                let v = xenonet_base::json::parse(&r.text()).map_err(|e| Thrown::new(e.to_string()))?;
                Ok(obj(vec![("ok", Arg::Bool(true)), ("status", Arg::Num(f64::from(r.status))), ("json", arg(&v))]))
            }
            ("", "post") => {
                let r = self.fetch("POST", &text(&x(0)), Some(text(&x(1))), 3000.0)?;
                Ok(obj(vec![("ok", Arg::Bool(r.ok())), ("status", Arg::Num(f64::from(r.status)))]))
            }
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("function".into())
    }
}

/// ps and the files of /proc, as openCodeProcesses and listeningPorts read
/// them: "" for a command that failed.
struct PsCalls;

impl Calls for PsCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let x = |i| at(&args, i);
        match (path, method) {
            ("", "ps" | "exec") => Ok(Arg::Text(run_quiet(&x(0), &x(1), 3000, 4 << 20).unwrap_or_default())),
            ("", "readFile") => crate::fsx::call("readFileSync", &args),
            ("", "readdir") => crate::fsx::call("readdirSync", &[x(0)]),
            ("", "readlink") => crate::fsx::call("readlinkSync", &[x(0)]),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("function".into())
    }
}

/// tmux and ps, as { ok, out }.
struct TmuxCalls;

impl Calls for TmuxCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("", "exec") => Ok(match run_quiet(&at(&args, 0), &at(&args, 1), 3000, 1 << 20) {
                Ok(out) => obj(vec![("ok", Arg::Bool(true)), ("out", Arg::Text(out))]),
                Err(_) => obj(vec![("ok", Arg::Bool(false)), ("out", Arg::text(""))]),
            }),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("function".into())
    }
}

/// The router's own methods, as the engine and the hooks call them.
pub fn router_call(w: &World, method: &str, args: &[Arg]) -> Result<Arg> {
    let c = &mut RouterCalls::of(w);
    let r = &w.router.core;
    let x = |i| at(args, i);
    match method {
        "reaches" => r.reaches(c, &x(0)),
        "known" => r.known(c).map(Arg::List),
        "describe" => r.describe(c, &x(0)),
        "resumable" => r.resumable(c, &x(0).or(Arg::Null)),
        "wake" => r.wake(c, &x(0), &x(1)),
        "way" => r.way(c, &x(0), &x(1)).map(|w| w.map_or(Arg::Null, Arg::text)),
        "list" => Ok(Arg::List(w.router.list(match x(0) {
            Arg::Undef => 100,
            n => crate::js::num(&n).max(0.0) as usize,
        }))),
        _ => Err(no_method("engine.delivery", method)),
    }
}

pub fn router_kind(method: &str) -> &'static str {
    match method {
        "" => "object",
        "reaches" | "known" | "describe" | "resumable" | "wake" | "way" | "list" | "route" => "function",
        _ => "undefined",
    }
}

/// A message routed: each of its sessions' way in tried. What failed goes
/// out as the router's error event. `listening` is who was listening when it
/// was sent: the JS router routed in the same turn as the send, and here a
/// listening hook can take the message and go before the route runs.
pub fn route(w: &World, m: &Arg, listening: &[String]) {
    let mut c = RouterCalls { w, listening: Some(listening) };
    if let Err(t) = w.router.core.route(&mut c, m) {
        w.emit(From::Delivery, "error", serde_json::json!({ "message": t.message }));
    }
}

/// Messages the engine says a session has seen.
pub fn seen(w: &World, agent: &Arg, ids: &[Arg]) {
    let _ = w.router.core.seen(&mut RouterCalls::of(w), agent, ids);
}

/// A hook from the session.
pub fn contact(w: &World, agent: &Arg) {
    let _ = w.router.core.contact(&mut RouterCalls::of(w), agent);
}

impl Calls for RouterCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let w = self.w;
        let x = |i| at(&args, i);
        match (path, method) {
            ("router", "now") => Ok(Arg::Num(now_ms())),
            ("router", "usage") => Ok(self.usage(&x(0))),
            ("router", "notify") => {
                w.emit(From::Delivery, "notify", json(&x(0)));
                Ok(Arg::Undef)
            }
            ("router.policy", m) => w.policy.as_call(m, &args),
            ("router.engine", "view") => self.engine().view(&x(0)),
            ("router.engine.listening", "has") => {
                let id = text(&x(0));
                Ok(Arg::Bool(self.listening.map_or_else(|| w.mail.listening_has(&id), |l| l.contains(&id))))
            }
            ("router.engine.store", m) => crate::store::call(&w.store, m, &args),
            ("router.channels", "live") => Ok(Arg::Bool(w.channels.live(&text(&x(0))))),
            ("router.channels", "push") => Ok(Arg::Bool(w.channels.push(&text(&x(0)), x(1)))),
            ("router.opencode", "info") => w.router.opencode.info(&mut OpenCodeCalls { w }, &x(0), windows()),
            ("router.opencode", "deliver") => w.router.opencode.deliver(&mut OpenCodeCalls { w }, &x(0), &x(1), windows()),
            ("router.tmux", "deliver") => {
                let o = x(0);
                w.router.tmux.deliver(&mut TmuxCalls, &o.field("pid"), &o.field("text"))
            }
            ("router.resume", "can") => Ok(Arg::Bool(resume::resume_command(&x(0), &Arg::Null).is_some())),
            ("router.resume", "deliver") => Ok(self.resume_deliver(&x(0))),
            ("", "listed") => {
                let key = text(&x(0));
                let mut o = w.router.objs.locked();
                o.objs.insert(key.clone(), x(1));
                if !o.records.contains(&key) {
                    o.records.push(key);
                }
                Ok(Arg::Undef)
            }
            ("", "unlisted") => {
                let key = text(&x(0));
                w.router.objs.locked().records.retain(|k| *k != key);
                Ok(Arg::Undef)
            }
            ("", "release") => {
                w.router.objs.locked().objs.remove(&text(&x(0)));
                Ok(Arg::Undef)
            }
            ("", "sync") => {
                let key = text(&x(0));
                let mut o = w.router.objs.locked();
                if let (Some(Arg::Obj(have)), Arg::Obj(rec)) = (o.objs.get_mut(&key), x(1)) {
                    for (k, v) in rec {
                        match have.iter_mut().find(|(n, _)| *n == k) {
                            Some(e) => e.1 = v,
                            None => have.push((k, v)),
                        }
                    }
                }
                Ok(Arg::Undef)
            }
            ("", "emit") => {
                let rec = w.router.objs.locked().objs.get(&text(&x(0))).cloned().unwrap_or(Arg::Undef);
                w.emit(From::Delivery, "delivery", json(&rec));
                Ok(Arg::Undef)
            }
            ("", "arm") => {
                let key = text(&x(0));
                let k = key.clone();
                let t = w.after(crate::js::num(&x(1)), move |w| {
                    w.router.timers.locked().remove(&k);
                    let _ = w.router.core.expire(&mut RouterCalls::of(w), &k);
                });
                if let Some(old) = w.router.timers.locked().insert(key, t) {
                    old.abort();
                }
                Ok(Arg::Undef)
            }
            ("", "clear") => {
                if let Some(t) = w.router.timers.locked().remove(&text(&x(0))) {
                    t.abort();
                }
                Ok(Arg::Undef)
            }
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "router.ackMs" => Arg::Num(ACK_MS),
            "router.policy.data.consent" => self.w.policy.data().field("consent"),
            "router.policy.data" => self.w.policy.data(),
            _ => Arg::Undef,
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "router.opencode"
            | "router.tmux"
            | "router.channels"
            | "router.policy"
            | "router.policy.data"
            | "router.engine"
            | "router.engine.listening"
            | "router.engine.store" => "object",
            "router.resume" => {
                if self.w.router.resume.is_some() {
                    "object"
                } else {
                    "null"
                }
            }
            "router.opencode.info"
            | "router.opencode.deliver"
            | "router.tmux.deliver"
            | "router.resume.can"
            | "router.resume.deliver"
            | "router.channels.live"
            | "router.channels.push"
            | "router.engine.listening.has" => "function",
            "router.ackMs" => "number",
            _ => "undefined",
        }
        .into())
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

pub struct RouterDriver(pub std::sync::Arc<World>);

impl Router for RouterDriver {
    fn usage(&self, app: &str) -> R<Value> {
        Ok(json(&RouterCalls::of(&self.0).usage(&Arg::text(app))))
    }
    fn list(&self, n: usize) -> R<Value> {
        Ok(Value::Array(self.0.router.list(n).iter().map(json).collect()))
    }
}

/// node:fs, for the listen settings.
struct FsCalls;

impl Calls for FsCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match path {
            "fs" => crate::fsx::call(method, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("object".into())
    }
}

/// The settings a Claude Code session starts with to listen after its
/// turns (writeListenSettings), written to the state folder: their file.
pub fn listen_settings(dir: &str, port: u16) -> std::result::Result<String, String> {
    xenonet_engine::delivery::setup::write_listen_settings(&mut FsCalls, dir, &Arg::Num(f64::from(port)), &Arg::Num(300.0)).map_err(|t| t.message)
}
