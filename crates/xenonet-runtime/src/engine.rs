//! The engine's collaborators in the binary (`Side`, one `Collaborators`
//! for a call): the clock, the processes, the map's readers, the actors,
//! the router and the policy, the repos, the sessions listening after a
//! turn, the waits an inbox and a listening Stop hook hold, the events and
//! the network. An engine call blocks on its waits and on git, so each
//! runs on a blocking thread (`EngineDriver`).

use crate::delivery::{self, RouterDriver};
use crate::runtime::Runtime;
use crate::runtime::{alive, time_of};
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use xenonet_api::cores::{CoreError, Engine, Kind, R};
use xenonet_api::From;
use xenonet_engine::engine::{Fail, ACTIVE_MS};
use xenonet_engine::host::{self, Collaborators, IndexEntry, Repo, StatusSet};
use xenonet_os::sync::Locked;
use xenonet_text::text::{display, to_number};
use xenonet_text::time::now_ms;

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
        self.listening.locked().iter().map(|(a, u)| (a.clone(), xenonet_text::json::number(*u))).collect()
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
    cache: Mutex<Vec<(String, f64, Option<Repo>)>>,
    running: Mutex<usize>,
    room: Condvar,
    /// Each folder's repo key across the network (its remote), kept as
    /// long as its repo; asked only while this machine is in a network or
    /// for a handoff.
    remotes: Mutex<Vec<(String, f64, Option<String>)>>,
}

const REPO_TTL_MS: f64 = 60e3;
const REPOS_KEPT: usize = 512;
const GIT_AT_ONCE: usize = 8;

impl Repos {
    fn read(&self, cwd: &str) -> Option<Repo> {
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
        out.ok().and_then(|stdout| xenonet_engine::engine::parse_repo(&stdout))
    }

    /// A folder's repo key across the network: its repo's remote (origin,
    /// else the first), normalized; None outside git or with no remote.
    pub fn remote_key_of(&self, cwd: &str) -> Option<String> {
        if cwd.is_empty() {
            return None;
        }
        let now = now_ms();
        if let Some((_, _, v)) = self.remotes.locked().iter().find(|(k, at, _)| k == cwd && now - at < REPO_TTL_MS) {
            return v.clone();
        }
        let args: Vec<String> = ["-C", cwd, "config", "--get-regexp", r"^remote\..*\.url$"].iter().map(ToString::to_string).collect();
        let v = crate::exec::run(&crate::exec::Spec::new("git", &args, 3000))
            .ok()
            .and_then(|out| xenonet_engine::network::first_remote(&out))
            .and_then(|u| xenonet_engine::network::repo_key_of(&u));
        let mut c = self.remotes.locked();
        c.retain(|(k, at, _)| k != cwd && now - at < REPO_TTL_MS);
        c.push((cwd.to_string(), now, v.clone()));
        let over = c.len().saturating_sub(REPOS_KEPT);
        c.drain(..over);
        v
    }

    pub fn of(&self, cwd: &str) -> Option<Repo> {
        if cwd.is_empty() {
            return None;
        }
        let now = now_ms();
        if let Some((_, _, v)) = self.cache.locked().iter().find(|(k, at, _)| k == cwd && now - at < REPO_TTL_MS) {
            return v.clone();
        }
        let v = self.read(cwd);
        let mut c = self.cache.locked();
        // Read again, it goes to the end, the last to be let go.
        c.retain(|(k, _, _)| k != cwd);
        c.push((cwd.to_string(), now_ms(), v.clone()));
        if c.len() > REPOS_KEPT {
            let t = now_ms();
            c.retain(|(_, at, _)| t - at < REPO_TTL_MS);
            let over = c.len().saturating_sub(REPOS_KEPT);
            c.drain(..over);
        }
        v
    }
}

/// The engine's collaborators for one call, over the world. `signal`: a
/// listening hook's request.
pub struct Side<'a> {
    w: &'a Runtime,
    signal: Option<Arc<Signal>>,
    /// Live delivery, which holds the world: None only while the world is
    /// going.
    router: Option<RouterDriver>,
}

impl<'a> Side<'a> {
    pub fn new(w: &'a Runtime, signal: Option<Arc<Signal>>) -> Side<'a> {
        Side { w, signal, router: w.arc().map(RouterDriver) }
    }

    pub fn collaborators(&self) -> Collaborators<'_> {
        Collaborators {
            store: &self.w.store,
            clock: self,
            processes: self,
            map: self,
            actors: self.w.actors.as_ref().map(|_| self as &dyn host::ActorsView),
            delivery: self.router.as_ref().map(|r| r as &dyn xenonet_engine::delivery::router::Delivery),
            policy: Some(&self.w.policy),
            repos: self,
            listening: self,
            waits: self,
            events: self,
            net: Some(self),
        }
    }

    fn wait(&self, me: &str, ms: f64, listen: bool) {
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

    /// A call on the actors' registry, with what it told told after.
    fn actors<T>(
        &self,
        f: impl FnOnce(&mut xenonet_actors::registry::Actors, &mut crate::actors::ActorHost<'_>) -> xenonet_actors::registry::Result<T>,
    ) -> xenonet_engine::engine::Result<T> {
        let Some(a) = &self.w.actors else { return Err(Fail::Error("actors is not an object".into())) };
        a.with(self.w, f).map_err(Fail::from)
    }
}

impl host::Clock for Side<'_> {
    fn now(&self) -> f64 {
        now_ms()
    }
    fn time_of(&self, ms: f64) -> String {
        time_of(ms)
    }
}

impl host::Processes for Side<'_> {
    fn alive(&self, pid: f64) -> bool {
        alive(pid)
    }
}

impl host::MapReader for Side<'_> {
    fn live_ids(&self) -> xenonet_engine::engine::Result<Vec<(String, f64)>> {
        self.w.map.live_ids()
    }
    fn index(&self) -> xenonet_engine::engine::Result<Vec<IndexEntry>> {
        self.w.map.index()
    }
    fn nodes_version(&self) -> Option<u64> {
        self.w.map.version()
    }
    fn node(&self, id: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        self.w.map.node(id)
    }
    fn live(&self, id: &str, now: f64) -> xenonet_engine::engine::Result<Option<Value>> {
        self.w.map.live(id, now)
    }
    fn observed_live(&self, id: &str) -> xenonet_engine::engine::Result<bool> {
        self.w.map.observed_live(id)
    }
    fn home_of(&self, id: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        self.w.map.home_of(id)
    }
    fn identities(&self) -> xenonet_engine::engine::Result<Vec<Value>> {
        self.w.map.identities()
    }
}

impl host::ActorsView for Side<'_> {
    fn has(&self, id: &str) -> xenonet_engine::engine::Result<bool> {
        self.actors(|r, _| Ok(r.has(id)))
    }
    fn is_registered(&self, id: &str) -> xenonet_engine::engine::Result<bool> {
        self.actors(|r, _| Ok(r.is_registered(id)))
    }
    fn place_of(&self, dir: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        self.actors(|r, h| Ok(r.place_of(&Value::String(dir.to_string()), h))).map(|v| (!v.is_null()).then_some(v))
    }
    fn engine_ids(&self, since: f64) -> xenonet_engine::engine::Result<Vec<String>> {
        self.actors(|r, _| Ok(r.engine_ids(since, now_ms())))
    }
    fn engine_view(&self, id: &str, now: f64) -> xenonet_engine::engine::Result<Option<Value>> {
        self.actors(|r, h| r.engine_view(id, now, None, h)).map(|v| (!v.is_null()).then_some(v))
    }
    fn set_status(&self, id: &str, status: &str) -> xenonet_engine::engine::Result<StatusSet> {
        let set = self.actors(|r, _| r.set_status(id, status, None, now_ms()))?;
        Ok(match set {
            Ok(status) => StatusSet::Ok(status),
            Err(reason) => StatusSet::Refused(reason.to_string()),
        })
    }
    fn info(&self, id: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        self.actors(|r, _| Ok(r.info(id))).map(|v| (!v.is_null()).then_some(v))
    }
}

impl host::Repos for Side<'_> {
    fn of(&self, dir: &str) -> xenonet_engine::engine::Result<Option<Repo>> {
        Ok(self.w.repos.of(dir))
    }
    /// Git for several folders at once.
    fn of_each(&self, dirs: &[String]) -> Vec<xenonet_engine::engine::Result<Option<Repo>>> {
        if dirs.len() < 2 {
            return dirs.iter().map(|d| Ok(self.w.repos.of(d))).collect();
        }
        let repos = &self.w.repos;
        std::thread::scope(|s| {
            let hs: Vec<_> = dirs.iter().map(|d| s.spawn(move || repos.of(d))).collect();
            hs.into_iter().map(|h| Ok(h.join().unwrap_or(None))).collect()
        })
    }
}

impl host::Listening for Side<'_> {
    fn set(&self, id: &str, until: f64) {
        self.w.mail.set_listening(id, until);
    }
    fn delete(&self, id: &str) {
        self.w.mail.stop_listening(id);
    }
    fn has(&self, id: &str) -> bool {
        self.w.mail.listening_has(id)
    }
}

impl host::Waits for Side<'_> {
    fn wake_waiters(&self, id: &str) {
        self.w.mail.wake(id);
    }
    fn wait(&self, me: &str, ms: f64) {
        Side::wait(self, me, ms, false);
    }
    fn listen_wait(&self, me: &str, ms: f64) {
        Side::wait(self, me, ms, true);
    }
    fn aborted(&self) -> bool {
        self.signal.as_ref().is_some_and(|s| s.aborted.load(Ordering::SeqCst))
    }
}

impl host::Events for Side<'_> {
    fn emit(&self, name: &str, payload: Value) {
        event(self.w, name, &payload);
    }
}

impl host::Net for Side<'_> {
    fn agents(&self) -> xenonet_engine::engine::Result<Vec<Value>> {
        Ok(crate::network::remote_views(self.w, false))
    }
    fn console(&self) -> xenonet_engine::engine::Result<Vec<Value>> {
        Ok(crate::network::remote_views(self.w, true))
    }
    fn view(&self, id: &str) -> xenonet_engine::engine::Result<Value> {
        Ok(crate::network::remote_view(self.w, id))
    }
    fn repo_key(&self, cwd: &str) -> xenonet_engine::engine::Result<Option<String>> {
        Ok(self.w.repos.remote_key_of(cwd))
    }
    fn sent(&self, message: &Value) -> xenonet_engine::engine::Result<()> {
        crate::network::sent(self.w, message)
    }
    fn claim_hits(&self, me: &str, files: &[String]) -> xenonet_engine::engine::Result<Vec<Value>> {
        crate::network::claim_hits(self.w, me, files)
    }
    fn handoffs(&self, view: &Value) -> xenonet_engine::engine::Result<Vec<Value>> {
        crate::network::offers(self.w, view)
    }
    fn receipt(&self, message_id: f64, agent: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        crate::network::receipt_of(self.w, message_id, agent)
    }
}

/// What the engine failed with, as the routes answer it.
fn error(f: Fail) -> CoreError {
    match f {
        Fail::Error(m) => CoreError::new(m),
        Fail::Bad(m) => CoreError::of(Kind::Error, 400, "bad-request", m),
        Fail::Refused { why, code } => CoreError::of(Kind::Error, 429, code, why),
        Fail::Thrown(t) => CoreError { code: t.code, ..CoreError::new(t.message) },
    }
}

/// What the engine tells: a message to the page, the actors, the router and
/// the wakes; mail taken to the router and the listening hooks; a hook's
/// contact to the router.
pub fn event(w: &Runtime, name: &str, payload: &Value) {
    match name {
        "message" => {
            w.emit(From::Engine, "message", payload.clone());
            // A message from another machine to a session that holds them
            // waits until the session asks.
            if xenonet_text::text::truthy(Some(&payload["held"])) {
                return;
            }
            // Agents on other hosts get it there, through the network.
            let to: Vec<Value> = payload["to"].as_array().cloned().unwrap_or_default();
            let mut m = payload.clone();
            if to.iter().any(|r| xenonet_engine::network::is_remote(&display(r))) {
                if let Value::Object(f) = &mut m {
                    f.retain(|k, _| k != "to");
                    f.insert("to".into(), Value::Array(to.iter().filter(|r| !xenonet_engine::network::is_remote(&display(r))).cloned().collect()));
                }
            }
            // Who is listening now, before the route below runs.
            let here: Vec<Value> = m["to"].as_array().cloned().unwrap_or_default();
            let listening: Vec<String> = here.iter().map(display).filter(|id| w.mail.listening_has(id)).collect();
            w.spawn_blocking(move |w| {
                delivery::route(w, &m, &listening);
                crate::network::delivered(w, &m);
            });
            if let Some(wakes) = &w.wakes {
                for r in here {
                    wakes.arrived(w, &display(&r));
                }
            }
        }
        "seen" => {
            let ids: Vec<Value> = payload["ids"].as_array().cloned().unwrap_or_default();
            let me = display(&payload["agent"]);
            w.mail.seen(&me);
            delivery::seen(w, &me, &ids);
            crate::network::seen(w, &me, &ids);
        }
        "contact" if payload["via"].as_str() == Some("hook") => {
            delivery::contact(w, &display(&payload["agent"]));
        }
        _ => {}
    }
}

/// The engine, as the routes call it.
pub struct EngineDriver(pub Arc<Runtime>);

impl EngineDriver {
    /// An engine call on a blocking thread.
    async fn run<T: Send + 'static>(
        &self,
        signal: Option<Arc<Signal>>,
        f: impl FnOnce(&xenonet_engine::engine::Engine, &Collaborators<'_>) -> Result<T, Fail> + Send + 'static,
    ) -> R<T> {
        let w = self.0.clone();
        let out = tokio::task::spawn_blocking(move || {
            let side = Side::new(&w, signal);
            f(&w.engine, &side.collaborators()).map_err(error)
        })
        .await;
        out.map_err(|e| CoreError::new(e.to_string()))?
    }

    fn now<T>(&self, f: impl FnOnce(&xenonet_engine::engine::Engine, &Collaborators<'_>) -> Result<T, Fail>) -> R<T> {
        let side = Side::new(&self.0, None);
        f(&self.0.engine, &side.collaborators()).map_err(error)
    }
}

/// An object with the fields that have a value, in this order.
fn fields(fields: Vec<(&str, Option<Value>)>) -> Value {
    Value::Object(fields.into_iter().filter_map(|(k, v)| v.map(|v| (k.to_string(), v))).collect::<Map<String, Value>>())
}

fn or_null(v: Option<&Value>) -> Value {
    v.cloned().unwrap_or(Value::Null)
}

#[async_trait]
impl Engine for EngineDriver {
    async fn hook(&self, app: &str, event: Option<&str>, body: &Value) -> R<Option<Value>> {
        let (app, event, input) = (app.to_string(), event.map(str::to_string), body.clone());
        self.run(None, move |e, c| e.hook(c, &app, event.as_deref(), &input)).await
    }
    async fn snapshot(&self) -> R<Value> {
        self.run(None, xenonet_engine::engine::Engine::snapshot).await
    }
    async fn agents(&self) -> R<Value> {
        self.run(None, |e, c| e.agents(c, ACTIVE_MS)).await.map(Value::Array)
    }
    async fn who_has(&self, me: Option<&str>, path: &str) -> R<Value> {
        let (me, path) = (me.map(str::to_string), path.to_string());
        self.run(None, move |e, c| e.who_has(c, me.as_deref(), &path)).await.map(Value::Array)
    }
    async fn whoami(&self, me: &str) -> R<Value> {
        let me = me.to_string();
        self.run(None, move |e, c| e.whoami(c, &me)).await
    }
    async fn peers(&self, me: &str, scope: &str) -> R<Value> {
        let (me, scope) = (me.to_string(), scope.to_string());
        self.run(None, move |e, c| e.peers(c, &me, &scope, ACTIVE_MS)).await.map(Value::Array)
    }
    async fn send(&self, me: &str, msg: Value, extra: Option<Value>) -> R<Value> {
        let me = me.to_string();
        // { to, text, replyTo = null }, and { envelope = null }.
        let ask = fields(vec![("to", msg.get("to").cloned()), ("text", msg.get("text").cloned()), ("replyTo", Some(or_null(msg.get("replyTo"))))]);
        let envelope = extra.as_ref().and_then(|e| e.get("envelope")).cloned().unwrap_or(Value::Null);
        self.run(None, move |e, c| e.send(c, &me, &ask, &envelope)).await
    }
    async fn inbox(&self, me: &str, wait: Duration) -> R<Value> {
        let me = me.to_string();
        let ms = wait.as_secs_f64() * 1000.0;
        self.run(None, move |e, c| e.inbox(c, &me, ms)).await.map(Value::Array)
    }
    async fn claim(&self, me: &str, args: Value) -> R<Value> {
        let me = me.to_string();
        let minutes = args.get("minutes").cloned().unwrap_or_else(|| xenonet_text::json::number(CLAIM_MINUTES));
        let ask = fields(vec![("paths", args.get("paths").cloned()), ("note", Some(or_null(args.get("note")))), ("minutes", Some(minutes))]);
        self.run(None, move |e, c| e.claim(c, &me, &ask)).await
    }
    async fn release(&self, me: &str, paths: Value) -> R<Value> {
        let me = me.to_string();
        self.run(None, move |e, c| e.release(c, &me, &paths)).await
    }
    async fn set_focus(&self, me: &str, text: Value) -> R<Value> {
        let me = me.to_string();
        self.run(None, move |e, c| e.set_focus(c, &me, &text)).await
    }
    fn touch(&self, me: &str, info: Value) -> R<()> {
        let ask = fields(vec![("app", info.get("app").cloned()), ("cwd", info.get("cwd").cloned()), ("via", info.get("via").cloned()), ("pid", Some(or_null(info.get("pid"))))]);
        self.now(|e, c| e.touch(c, me, &ask))
    }
    async fn view(&self, me: &str) -> R<Option<Value>> {
        let me = me.to_string();
        self.run(None, move |e, c| e.view(c, &me)).await.map(Some)
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
        let me = me.to_string();
        let ms = wait.as_secs_f64() * 1000.0;
        let out = self.run(Some(signal), move |e, c| e.listen(c, &me, ms)).await;
        watching.abort();
        out
    }
    async fn handoff(&self, me: &str, args: Value, extra: Option<Value>) -> R<Value> {
        let me = me.to_string();
        let envelope = extra.as_ref().and_then(|e| e.get("envelope")).cloned().unwrap_or(Value::Null);
        let w = self.0.clone();
        let out = tokio::task::spawn_blocking(move || crate::network::handoff(&w, &me, &args, &envelope).map_err(error)).await;
        out.map_err(|e| CoreError::new(e.to_string()))?
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
    async fn permission(&self, me: &str, title: Option<String>, body: Value, by: xenonet_api::cores::door::Attestation) -> R<Value> {
        let (w, me) = (self.0.clone(), me.to_string());
        let by = match by {
            xenonet_api::cores::door::Attestation::Unchecked => xenonet_authz::Attestation::Unchecked,
            xenonet_api::cores::door::Attestation::ProcessChecked => xenonet_authz::Attestation::ProcessChecked,
        };
        let out = tokio::task::spawn_blocking(move || crate::asks::ask(&w, &me, title.as_deref(), &body, by)).await.map_err(|e| CoreError::new(e.to_string()))?;
        out.map_err(|(status, why)| CoreError::status(status, why))
    }
    async fn asks(&self) -> R<Value> {
        Ok(crate::asks::list(&self.0))
    }
    async fn answer(&self, id: &str, decision: &str) -> R<Value> {
        let host = crate::asks::answer(&self.0, id, decision).map_err(|why| CoreError::status(409, why))?;
        Ok(serde_json::json!({ "host": host }))
    }
    async fn answer_as(&self, token: &str, id: &str, decision: &str) -> R<Value> {
        let host = crate::asks::answer_as_actor(&self.0, token, id, decision).map_err(|why| CoreError::status(409, why))?;
        Ok(serde_json::json!({ "host": host }))
    }
    async fn actor_allows(&self, slug: &str, scope: Value, at_window: bool) -> R<Value> {
        let scope = (!scope.is_null()).then_some(scope);
        crate::grants::set_allows(&self.0, at_window, slug, scope.as_ref(), &format!("let actor:{slug} allow permission prompts")).map_err(|why| CoreError::status(409, why))?;
        Ok(serde_json::json!({ "ok": true, "actor": format!("actor:{slug}"), "allows": scope }))
    }
    async fn answer_sent(&self, id: &str, decision: &str) -> R<()> {
        crate::asks::answer_sent(&self.0, id, decision);
        Ok(())
    }
    async fn forks(&self) -> R<Value> {
        Ok(
            serde_json::json!({ "forks": crate::starts::forks(&self.0).into_iter().map(|(child, parent)| serde_json::json!({ "child": child, "parent": parent })).collect::<Vec<_>>() }),
        )
    }
    fn mark_unseen(&self, me: &str, ids: &Value) -> R<()> {
        // The ids listed, or a text's characters.
        let ids: Vec<f64> = match ids {
            Value::Array(a) => a.iter().map(|id| to_number(Some(id))).collect(),
            Value::String(s) => s.chars().map(|c| to_number(Some(&Value::String(c.to_string())))).collect(),
            _ => return Err(CoreError::new("that isn't a list")),
        };
        self.0.store.with(|s| s.mark_unseen(me, &ids)).map_err(|e| CoreError::new(e.message().to_string()))
    }
    fn listening(&self) -> R<Vec<(String, Value)>> {
        Ok(self.0.mail.listening())
    }
}
