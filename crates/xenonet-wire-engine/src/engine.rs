//! The engine (core/engine/engine.js over crates/xenonet-engine/src/engine.rs):
//! its collaborators as the twin hands them over, natively. "engine" is
//! the engine itself (its store, actors, policy, delivery, repos and
//! listening hooks); the map's state is read through the State trait; and
//! the waits an inbox and a listening Stop hook hold, the events, the
//! processes and the clock are here. An engine call blocks on its waits
//! and on git, so each runs on a blocking thread.

use crate::delivery::{self, policy_thrown};
use crate::js::{alive, at, json, list, no_method, now_ms, obj, text, time_of, Locked};
use crate::world::World;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use xenonet_engine::engine::{Fail, ACTIVE_MS};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_service::cores::{CoreError, Engine, Kind, R};
use xenonet_service::glue::From;

const CLAIM_MINUTES: f64 = 60.0;

/// One wait: an inbox's, or a listening hook's.
#[derive(Default)]
struct Waiter {
    done: Mutex<bool>,
    woke: Condvar,
    /// A listening hook's wait also ends when the session's main Stop hook
    /// takes its mail.
    listen: bool,
}

impl Waiter {
    fn wake(&self) {
        *self.done.locked() = true;
        self.woke.notify_all();
    }

    /// Until woken or `ms` pass.
    fn wait(&self, ms: f64) {
        let until = Instant::now() + Duration::from_secs_f64(ms.max(0.0) / 1000.0);
        let mut done = self.done.locked();
        while !*done {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            done = self.woke.wait_timeout(done, left).map_or_else(|e| e.into_inner().0, |(g, _)| g);
        }
    }
}

/// A listening hook's request: aborted once the hook stops waiting.
#[derive(Default)]
pub struct Signal {
    aborted: AtomicBool,
    waiting: Mutex<Option<Arc<Waiter>>>,
}

impl Signal {
    fn abort(&self) {
        self.aborted.store(true, Ordering::SeqCst);
        if let Some(w) = self.waiting.locked().clone() {
            w.wake();
        }
    }
}

/// The waits on the engine (inboxes and listening hooks, by agent) and the
/// sessions listening after a turn, until when.
#[derive(Default)]
pub struct Mail {
    waiters: Mutex<HashMap<String, Vec<Arc<Waiter>>>>,
    listening: Mutex<Vec<(String, f64)>>,
}

impl Mail {
    fn add(&self, me: &str, w: &Arc<Waiter>) {
        self.waiters.locked().entry(me.to_string()).or_default().push(w.clone());
    }

    fn drop_waiter(&self, me: &str, w: &Arc<Waiter>) {
        let mut all = self.waiters.locked();
        if let Some(list) = all.get_mut(me) {
            list.retain(|x| !Arc::ptr_eq(x, w));
            if list.is_empty() {
                all.remove(me);
            }
        }
    }

    /// A message arrived for `id`: every wait on it ends.
    pub fn wake(&self, id: &str) {
        let list = self.waiters.locked().get(id).cloned().unwrap_or_default();
        for w in list {
            w.wake();
        }
    }

    /// The main Stop hook took `id`'s mail: its listening hooks stop.
    fn seen(&self, id: &str) {
        let list = self.waiters.locked().get(id).cloned().unwrap_or_default();
        for w in list.iter().filter(|w| w.listen) {
            w.wake();
        }
    }

    pub fn listening_has(&self, id: &str) -> bool {
        self.listening.locked().iter().any(|(a, _)| a == id)
    }

    pub fn listening(&self) -> Vec<(String, Value)> {
        self.listening.locked().iter().map(|(a, u)| (a.clone(), crate::js::number(*u))).collect()
    }

    fn set_listening(&self, id: &str, until: f64) {
        let mut l = self.listening.locked();
        match l.iter_mut().find(|(a, _)| a == id) {
            Some(e) => e.1 = until,
            None => l.push((id.to_string(), until)),
        }
    }

    fn stop_listening(&self, id: &str) {
        self.listening.locked().retain(|(a, _)| a != id);
    }
}

/// The git repository a folder belongs to: its working tree's top and the
/// shared git dir, which is the same for every worktree of a repo. Each
/// folder's answer is kept for a minute, 512 folders at most, and only a
/// few git commands run at once.
#[derive(Default)]
pub struct Repos {
    cache: Mutex<Vec<(String, f64, Arg)>>,
    running: Mutex<usize>,
    room: Condvar,
    /// Each folder's repo key across the network (its remote), kept as
    /// long as its repo; asked only while this machine is in a network or
    /// for a handoff.
    remotes: Mutex<Vec<(String, f64, Arg)>>,
}

const REPO_TTL_MS: f64 = 60e3;
const REPOS_KEPT: usize = 512;
const GIT_AT_ONCE: usize = 8;

impl Repos {
    fn read(&self, cwd: &str) -> Arg {
        {
            let mut n = self.running.locked();
            while *n >= GIT_AT_ONCE {
                n = self.room.wait(n).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            *n += 1;
        }
        let args: Vec<String> =
            ["-C", cwd, "rev-parse", "--path-format=absolute", "--show-toplevel", "--git-common-dir", "--abbrev-ref", "HEAD"].iter().map(ToString::to_string).collect();
        let out = crate::exec::run(&crate::exec::Spec::new("git", &args, 3000));
        *self.running.locked() -= 1;
        self.room.notify_one();
        match out {
            Ok(stdout) => xenonet_engine::engine::parse_repo(&stdout).unwrap_or(Arg::Null),
            Err(_) => Arg::Null,
        }
    }

    /// A folder's repo key across the network: its repo's remote (origin,
    /// else the first), normalized; null outside git or with no remote.
    pub fn remote_key(&self, cwd: &Arg) -> Arg {
        if !cwd.truthy() {
            return Arg::Null;
        }
        let cwd = text(cwd);
        let now = now_ms();
        if let Some((_, _, v)) = self.remotes.locked().iter().find(|(k, at, _)| *k == cwd && now - at < REPO_TTL_MS) {
            return v.clone();
        }
        let args: Vec<String> = ["-C", &cwd, "config", "--get-regexp", r"^remote\..*\.url$"].iter().map(ToString::to_string).collect();
        let v = match crate::exec::run(&crate::exec::Spec::new("git", &args, 3000)) {
            Ok(out) => xenonet_engine::network::first_remote(&out).and_then(|u| xenonet_engine::network::repo_key_of(&u)).map_or(Arg::Null, Arg::Text),
            Err(_) => Arg::Null,
        };
        let mut c = self.remotes.locked();
        c.retain(|(k, at, _)| *k != cwd && now - at < REPO_TTL_MS);
        c.push((cwd, now, v.clone()));
        let over = c.len().saturating_sub(REPOS_KEPT);
        c.drain(..over);
        v
    }

    pub fn of(&self, cwd: &Arg) -> Arg {
        if !cwd.truthy() {
            return Arg::Null;
        }
        let cwd = text(cwd);
        let now = now_ms();
        if let Some((_, _, v)) = self.cache.locked().iter().find(|(k, at, _)| *k == cwd && now - at < REPO_TTL_MS) {
            return v.clone();
        }
        let v = self.read(&cwd);
        let mut c = self.cache.locked();
        // Read again, it goes to the end, the last to be let go.
        c.retain(|(k, _, _)| *k != cwd);
        c.push((cwd, now_ms(), v.clone()));
        if c.len() > REPOS_KEPT {
            let t = now_ms();
            c.retain(|(_, at, _)| t - at < REPO_TTL_MS);
            let over = c.len().saturating_sub(REPOS_KEPT);
            c.drain(..over);
        }
        v
    }
}

/// The engine's collaborators, for one call. `signal`: a listening hook's
/// request. What the policy refused is kept by its handle, with its status.
pub struct EngineCalls<'a> {
    pub w: &'a World,
    signal: Option<Arc<Signal>>,
    refused: Vec<(u64, u16)>,
    next: u64,
}

impl<'a> EngineCalls<'a> {
    pub fn new(w: &'a World, signal: Option<Arc<Signal>>) -> EngineCalls<'a> {
        EngineCalls { w, signal, refused: Vec::new(), next: 0 }
    }

    /// `engine.view(id)`, as a collaborator asks for it.
    pub fn view(&mut self, id: &Arg) -> Result<Arg> {
        let w = self.w;
        w.engine.view(self, id).map_err(thrown_of)
    }

    fn policy(&mut self, method: &str, args: &[Arg]) -> Result<Arg> {
        self.w.policy.call(method, args).map_err(|f| {
            let (mut t, status) = policy_thrown(f);
            if let Some(s) = status {
                self.next += 1;
                t.handle = Some(self.next);
                self.refused.push((self.next, s));
            }
            t
        })
    }

    /// What the engine failed with, as the routes answer it.
    fn error(&self, f: Fail) -> CoreError {
        match f {
            Fail::Error(m) => CoreError::new(m),
            Fail::Bad(m) => CoreError::of(Kind::Error, 400, "bad-request", m),
            Fail::Thrown(t) => {
                let status = t.handle.and_then(|h| self.refused.iter().find(|(k, _)| *k == h)).map(|(_, s)| *s);
                let code = t.code.as_deref().and_then(Value::as_str).map(str::to_string);
                match status {
                    Some(s) => CoreError { status: Some(s), code, ..CoreError::new(t.message) },
                    None => CoreError { code, ..CoreError::new(t.message) },
                }
            }
        }
    }

    fn wait(&mut self, me: &str, ms: f64, listen: bool) {
        let waiter = Arc::new(Waiter { listen, ..Waiter::default() });
        self.w.mail.add(me, &waiter);
        if listen {
            if let Some(s) = &self.signal {
                *s.waiting.locked() = Some(waiter.clone());
                if s.aborted.load(Ordering::SeqCst) {
                    waiter.wake();
                }
            }
        }
        waiter.wait(ms);
        if let (true, Some(s)) = (listen, &self.signal) {
            *s.waiting.locked() = None;
        }
        self.w.mail.drop_waiter(me, &waiter);
    }

    fn actors(&self) -> Option<&crate::actors::ActorSide> {
        self.w.actors.as_ref()
    }
}

fn thrown_of(f: Fail) -> Thrown {
    match f {
        Fail::Error(m) | Fail::Bad(m) => Thrown::new(m),
        Fail::Thrown(t) => t,
    }
}

/// What the engine tells: a message to the page, the actors, the router and
/// the wakes; mail taken to the router and the listening hooks; a hook's
/// contact to the router.
pub fn told(w: &World, name: &str, payload: &Arg) {
    match name {
        "message" => {
            w.emit(From::Engine, "message", json(payload));
            // A message from another machine to a session that holds them
            // waits until the session asks.
            if payload.field("held").truthy() {
                return;
            }
            // Agents on other hosts get it there, through the network.
            let to = list(payload.field("to"));
            let mut m = payload.clone();
            if to.iter().any(xenonet_engine::network::is_remote) {
                if let Arg::Obj(f) = &mut m {
                    f.retain(|(k, _)| k != "to");
                    f.push(("to".into(), Arg::List(to.iter().filter(|r| !xenonet_engine::network::is_remote(r)).cloned().collect())));
                }
            }
            // Who is listening now, before the route below runs.
            let listening: Vec<String> = list(m.field("to")).iter().map(text).filter(|id| w.mail.listening_has(id)).collect();
            let here = list(m.field("to"));
            w.spawn_blocking(move |w| {
                delivery::route(w, &m, &listening);
                crate::network::delivered(w, &m);
            });
            if let Some(wakes) = &w.wakes {
                for r in here {
                    wakes.arrived(w, &r);
                }
            }
        }
        "seen" => {
            let agent = payload.field("agent");
            w.mail.seen(&text(&agent));
            delivery::seen(w, &agent, &list(payload.field("ids")));
            crate::network::seen(w, &agent, &list(payload.field("ids")));
        }
        "contact" if payload.field("via").as_str() == Some("hook") => {
            delivery::contact(w, &payload.field("agent"));
        }
        _ => {}
    }
}

impl Calls for EngineCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let w = self.w;
        let x = |i| at(&args, i);
        match (path, method) {
            ("engine", "now") => Ok(Arg::Num(now_ms())),
            ("engine.store", m) => crate::store::call(&w.store, m, &args),
            ("engine.actors", m) => match self.actors() {
                Some(a) => a.call(w, m, &args),
                None => Err(no_method("engine.actors", m)),
            },
            ("engine.delivery", m) => delivery::router_call(w, m, &args),
            ("engine.policy", m) => self.policy(m, &args),
            ("engine.repos", "of") => Ok(w.repos.of(&x(0))),
            ("engine.net", m) => crate::network::call(w, self, m, &args),
            ("engine.listening", "set") => {
                w.mail.set_listening(&text(&x(0)), crate::js::num(&x(1)));
                Ok(Arg::Undef)
            }
            ("engine.listening", "delete") => {
                w.mail.stop_listening(&text(&x(0)));
                Ok(Arg::Undef)
            }
            ("engine.listening", "has") => Ok(Arg::Bool(w.mail.listening_has(&text(&x(0))))),
            ("", "liveIds") => w.map.live_ids(),
            ("", "index") => w.map.index(),
            ("", "node") => w.map.node(&x(0)),
            ("", "live") => w.map.live(&x(0), crate::js::num(&x(1))),
            ("", "observedLive") => w.map.observed_live(&x(0)).map(Arg::Bool),
            ("", "homeOf") => w.map.home_of(&x(0)),
            ("", "identities") => w.map.identities(),
            ("", "alive") => Ok(Arg::Bool(x(0).as_f64().is_some_and(alive))),
            ("", "emit") => {
                told(w, &text(&x(0)), &x(1));
                Ok(Arg::Undef)
            }
            ("", "wakeWaiters") => {
                w.mail.wake(&text(&x(0)));
                Ok(Arg::Undef)
            }
            ("", "wait") => {
                self.wait(&text(&x(0)), crate::js::num(&x(1)), false);
                Ok(Arg::Undef)
            }
            ("", "listenWait") => {
                self.wait(&text(&x(0)), crate::js::num(&x(1)), true);
                Ok(Arg::Undef)
            }
            ("", "wall") => Ok(Arg::Num(now_ms())),
            ("", "tick") => Ok(Arg::Undef),
            ("", "timeOf") => Ok(Arg::Text(time_of(crate::js::num(&x(0))))),
            _ => Err(no_method(path, method)),
        }
    }

    fn call_all(&mut self, calls: Vec<(&str, &str, Vec<Arg>)>) -> Vec<Result<Arg>> {
        // Git for several folders at once, as Promise.all started them.
        if calls.len() > 1 && calls.iter().all(|(p, m, _)| *p == "engine.repos" && *m == "of") {
            let repos = &self.w.repos;
            return std::thread::scope(|s| {
                let hs: Vec<_> = calls.iter().map(|(_, _, a)| s.spawn(move || repos.of(&at(a, 0)))).collect();
                hs.into_iter().map(|h| Ok(h.join().unwrap_or(Arg::Null))).collect()
            });
        }
        calls.into_iter().map(|(p, m, a)| self.call(p, m, a)).collect()
    }

    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "signal.aborted" => Arg::Bool(self.signal.as_ref().is_some_and(|s| s.aborted.load(Ordering::SeqCst))),
            _ => Arg::Undef,
        })
    }

    fn kind(&mut self, path: &str) -> Result<String> {
        let actors = self.actors().is_some();
        Ok(match path {
            "signal" => {
                if self.signal.is_some() {
                    "object"
                } else {
                    "null"
                }
            }
            "engine.actors" => {
                if actors {
                    "object"
                } else {
                    "null"
                }
            }
            "engine.store" | "engine.policy" | "engine.delivery" | "engine.repos" | "engine.listening" | "engine.net" => "object",
            p => match p.strip_prefix("engine.delivery.") {
                Some(m) => delivery::router_kind(m),
                None => match p.strip_prefix("engine.store.") {
                    Some(m) => crate::store::kind(m),
                    None => match p.strip_prefix("engine.actors.") {
                        Some(m) if actors => crate::actors::kind(m),
                        _ => "undefined",
                    },
                },
            },
        }
        .into())
    }

    fn now(&mut self) -> f64 {
        now_ms()
    }
}

/// The engine, as the routes call it.
pub struct EngineDriver(pub Arc<World>);

impl EngineDriver {
    /// An engine call on a blocking thread.
    async fn run<T: Send + 'static>(&self, signal: Option<Arc<Signal>>, f: impl FnOnce(&World, &mut EngineCalls<'_>) -> std::result::Result<T, Fail> + Send + 'static) -> R<T> {
        let w = self.0.clone();
        let out = tokio::task::spawn_blocking(move || {
            let mut c = EngineCalls::new(&w, signal);
            f(&w, &mut c).map_err(|e| c.error(e))
        })
        .await;
        out.map_err(|e| CoreError::new(e.to_string()))?
    }

    fn now<T>(&self, f: impl FnOnce(&World, &mut EngineCalls<'_>) -> std::result::Result<T, Fail>) -> R<T> {
        let mut c = EngineCalls::new(&self.0, None);
        f(&self.0, &mut c).map_err(|e| c.error(e))
    }
}

fn some(v: Arg) -> Option<Value> {
    (!v.is_nullish()).then(|| json(&v))
}

#[async_trait]
impl Engine for EngineDriver {
    async fn hook(&self, app: &str, event: Option<&str>, body: &Value) -> R<Option<Value>> {
        let (app, event, input) = (Arg::text(app), event.map_or(Arg::Undef, Arg::text), crate::js::arg(body));
        self.run(None, move |w, c| w.engine.hook(c, &app, &event, &input)).await.map(some)
    }
    async fn snapshot(&self) -> R<Value> {
        self.run(None, |w, c| w.engine.snapshot(c)).await.map(|a| json(&a))
    }
    async fn agents(&self) -> R<Value> {
        self.run(None, |w, c| w.engine.agents(c, ACTIVE_MS)).await.map(|l| Value::Array(l.iter().map(json).collect()))
    }
    async fn who_has(&self, me: Option<&str>, path: &str) -> R<Value> {
        let (me, path) = (me.map_or(Arg::Null, Arg::text), Arg::text(path));
        self.run(None, move |w, c| w.engine.who_has(c, &me, &path)).await.map(|l| Value::Array(l.iter().map(json).collect()))
    }
    async fn whoami(&self, me: &str) -> R<Value> {
        let me = Arg::text(me);
        self.run(None, move |w, c| w.engine.whoami(c, &me)).await.map(|a| json(&a))
    }
    async fn peers(&self, me: &str, scope: &str) -> R<Value> {
        let (me, scope) = (Arg::text(me), Arg::text(scope));
        self.run(None, move |w, c| w.engine.peers(c, &me, &scope, ACTIVE_MS)).await.map(|l| Value::Array(l.iter().map(json).collect()))
    }
    async fn send(&self, me: &str, msg: Value, extra: Option<Value>) -> R<Value> {
        let me = Arg::text(me);
        let m = crate::js::arg(&msg);
        // { to, text, replyTo = null }, and { envelope = null }.
        let ask = obj(vec![("to", m.field("to")), ("text", m.field("text")), ("replyTo", m.field("replyTo").or(Arg::Null))]);
        let envelope = extra.as_ref().and_then(|e| e.get("envelope")).map_or(Arg::Null, crate::js::arg);
        self.run(None, move |w, c| w.engine.send(c, &me, &ask, &envelope)).await.map(|a| json(&a))
    }
    async fn inbox(&self, me: &str, wait: Duration) -> R<Value> {
        let me = Arg::text(me);
        let ms = Arg::Num(wait.as_secs_f64() * 1000.0);
        self.run(None, move |w, c| w.engine.inbox(c, &me, &ms)).await.map(|l| Value::Array(l.iter().map(json).collect()))
    }
    async fn claim(&self, me: &str, args: Value) -> R<Value> {
        let me = Arg::text(me);
        let a = crate::js::arg(&args);
        let minutes = match a.field("minutes") {
            Arg::Undef => Arg::Num(CLAIM_MINUTES),
            m => m,
        };
        let ask = obj(vec![("paths", a.field("paths")), ("note", a.field("note").or(Arg::Null)), ("minutes", minutes)]);
        self.run(None, move |w, c| w.engine.claim(c, &me, &ask)).await.map(|a| json(&a))
    }
    async fn release(&self, me: &str, paths: Value) -> R<Value> {
        let (me, paths) = (Arg::text(me), crate::js::arg(&paths));
        self.run(None, move |w, c| w.engine.release(c, &me, &paths)).await.map(|a| json(&a))
    }
    async fn set_focus(&self, me: &str, text: Value) -> R<Value> {
        let (me, t) = (Arg::text(me), crate::js::arg(&text));
        self.run(None, move |w, c| w.engine.set_focus(c, &me, &t)).await.map(|a| json(&a))
    }
    fn touch(&self, me: &str, info: Value) -> R<()> {
        let i = crate::js::arg(&info);
        let ask = obj(vec![("app", i.field("app")), ("cwd", i.field("cwd")), ("via", i.field("via")), ("pid", i.field("pid").or(Arg::Null))]);
        self.now(|w, c| w.engine.touch(c, &Arg::text(me), &ask))
    }
    async fn view(&self, me: &str) -> R<Option<Value>> {
        let me = Arg::text(me);
        self.run(None, move |w, c| w.engine.view(c, &me)).await.map(some)
    }
    async fn listen(&self, me: &str, wait: Duration, mut gone: watch::Receiver<bool>) -> R<Option<Value>> {
        let signal = Arc::new(Signal::default());
        let watching = {
            let s = signal.clone();
            tokio::spawn(async move {
                // The hook's request went: changed to true, or its sender gone.
                while !*gone.borrow_and_update() {
                    if gone.changed().await.is_err() {
                        break;
                    }
                }
                s.abort();
            })
        };
        let me = Arg::text(me);
        let ms = Arg::Num(wait.as_secs_f64() * 1000.0);
        let out = self.run(Some(signal), move |w, c| w.engine.listen(c, &me, &ms)).await;
        watching.abort();
        out.map(some)
    }
    async fn handoff(&self, me: &str, args: Value, extra: Option<Value>) -> R<Value> {
        let me = Arg::text(me);
        let envelope = extra.as_ref().and_then(|e| e.get("envelope")).cloned().unwrap_or(Value::Null);
        self.run(None, move |w, c| crate::network::handoff(w, c, &me, &args, &envelope)).await
    }
    async fn start(&self, by: Value, req: Value, strong: bool) -> R<Value> {
        let w = self.0.clone();
        let out = tokio::task::spawn_blocking(move || crate::starts::ask(&w, by, &req, strong)).await.map_err(|e| CoreError::new(e.to_string()))?;
        out.map_err(|(status, why)| CoreError::status(status, why))
    }
    async fn runs_sign_in(&self, action: &str, token: &str) -> R<Value> {
        let (w, action, token) = (self.0.clone(), action.to_string(), token.to_string());
        let out = tokio::task::spawn_blocking(move || crate::starts::sign_in(&w, &action, &token)).await.map_err(|e| CoreError::new(e.to_string()))?;
        out.map_err(|(status, why)| CoreError::status(status, why))
    }
    async fn forks(&self) -> R<Value> {
        Ok(serde_json::json!({ "forks": crate::starts::forks(&self.0).into_iter().map(|(child, parent)| serde_json::json!({ "child": child, "parent": parent })).collect::<Vec<_>>() }))
    }
    fn mark_unseen(&self, me: &str, ids: &Value) -> R<()> {
        crate::store::call(&self.0.store, "markUnseen", &[Arg::text(me), crate::js::arg(ids)]).map(|_| ()).map_err(|t| CoreError::new(t.message))
    }
    fn listening(&self) -> R<Vec<(String, Value)>> {
        Ok(self.0.mail.listening())
    }
}
