//! Waking actors when a message arrives (crates/xenonet-actors/src/wake.rs):
//! a stream for a program of their own, a webhook you give (its URL and
//! key in your credential store, the demo's in memory), or a notification
//! to you. The timers, the streams' sinks, DNS, fetch and URL parsing are
//! here, as the wakes' host, and their rows are the engine store's.

use crate::actors::{rows_json, Rows};
use crate::exec::Spec;
use crate::runtime::Runtime;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_actors::secret_store::{self, MemorySecrets, SecretStore};
use xenonet_actors::store as rows;
use xenonet_actors::wake::{self, Fail, Fetched, Request, Wakes as Core, TIMEOUT_MS};
use xenonet_api::cores::actors::WakeStream;
use xenonet_api::cores::{CoreError, Kind, Wakes, R};
use xenonet_api::From;
use xenonet_engine::store::Shared;
use xenonet_os::sync::Locked;
use xenonet_text::time::now_ms;

/// Your computer's credential store, or the demo's memory.
pub enum Secrets {
    System(SecretStore),
    Memory(Mutex<MemorySecrets>),
}

struct Runner;

impl secret_store::Run for Runner {
    fn run(&mut self, c: secret_store::Command) -> Result<secret_store::Ran, String> {
        let mut s = Spec::new(&c.file, &c.args, 20000);
        s.input = Some(c.input.unwrap_or_default().into_bytes());
        Ok(match crate::exec::run(&s) {
            Ok(stdout) => secret_store::Ran { code: 0.0, stdout },
            Err(f) => secret_store::Ran { code: f.code.as_ref().and_then(Value::as_f64).unwrap_or(-1.0), stdout: f.stdout },
        })
    }
}

impl Secrets {
    pub fn new(demo: bool) -> Secrets {
        if demo {
            return Secrets::Memory(Mutex::new(MemorySecrets::default()));
        }
        let tool = std::env::var("PATH").ok().and_then(|p| xenonet_os::paths::on_path("secret-tool", &p, None, false, &mut |f: &str| xenonet_os::paths::can_run(f)));
        Secrets::System(SecretStore::new(crate::runtime::platform(), tool))
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Secrets::System(s) => s.kind(),
            Secrets::Memory(_) => MemorySecrets::KIND,
        }
    }

    pub(crate) fn available(&self) -> bool {
        match self {
            Secrets::System(s) => s.available(),
            Secrets::Memory(_) => true,
        }
    }

    pub(crate) fn has(&self, target: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().has(target)),
            Secrets::System(s) => s.has(&mut Runner, target),
        }
    }

    pub(crate) fn get(&self, target: &str) -> Result<Option<String>, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().get(target)),
            Secrets::System(s) => s.get(&mut Runner, target),
        }
    }

    pub(crate) fn set(&self, target: &str, value: &str, comment: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => m.locked().set(target, value),
            Secrets::System(s) => s.set(&mut Runner, target, value, comment),
        }
    }

    pub(crate) fn remove(&self, target: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().remove(target)),
            Secrets::System(s) => s.remove(&mut Runner, target),
        }
    }
}

/// An actor's wake timer and notify timer, while set.
type Timers = (Option<tokio::task::JoinHandle<()>>, Option<tokio::task::JoinHandle<()>>);

/// A stream held open: where its events go, and how it's ended.
struct Sink {
    tx: mpsc::Sender<(String, Value)>,
    end: Option<tokio::sync::oneshot::Sender<()>>,
}

pub struct WakeSide {
    core: Core,
    secrets: Secrets,
    timers: Mutex<HashMap<String, Timers>>,
    sinks: Mutex<HashMap<u64, Sink>>,
    next_sink: AtomicU64,
}

/// The wakes' collaborators, over the world.
struct WakeHost<'a> {
    w: &'a Runtime,
    side: &'a WakeSide,
}

/// A URL as WHATWG's parser reads it, its hash taken off; None for what
/// isn't one.
fn parse_url(s: &str) -> Option<wake::Url> {
    let mut u = url::Url::parse(s).ok()?;
    u.set_fragment(None);
    let host = match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    };
    Some(wake::Url {
        username: u.username().to_string(),
        password: u.password().unwrap_or("").to_string(),
        protocol: format!("{}:", u.scheme()),
        hostname: u.host_str().unwrap_or("").to_string(),
        host,
        href: u.as_str().to_string(),
    })
}

/// The addresses a host name has, as the resolver gives them.
fn resolve(host: &str) -> Vec<String> {
    use std::net::ToSocketAddrs;
    let addrs = (host, 0).to_socket_addrs().map(Iterator::collect::<Vec<_>>).unwrap_or_default();
    addrs.iter().map(|a| a.ip().to_string()).collect()
}

impl WakeHost<'_> {
    /// An actor's wake or notify timer, in place of the one it had: when it
    /// comes, the wakes fire or notify from under the world.
    fn arm(&self, kind: &str, id: &str, ms: f64) -> bool {
        let key = id.to_string();
        let k = key.clone();
        let task = if kind == "wake" {
            self.w.after(ms, move |w| {
                let Some(side) = &w.wakes else { return };
                if let Some(t) = side.timers.locked().get_mut(&k) {
                    t.0 = None;
                }
                if side.core.wake_due(&k) {
                    if let Err(f) = side.core.fire(&mut WakeHost { w, side }, &k) {
                        w.log("warn", "actor.wake", &format!("{k}: {}", fail_message(&f)));
                    }
                }
            })
        } else {
            self.w.after(ms, move |w| {
                let Some(side) = &w.wakes else { return };
                if let Some(t) = side.timers.locked().get_mut(&k) {
                    t.1 = None;
                }
                let _ = side.core.notify_due(&mut WakeHost { w, side }, &k);
            })
        };
        let mut timers = self.side.timers.locked();
        let held = timers.entry(key).or_default();
        let slot = if kind == "wake" { &mut held.0 } else { &mut held.1 };
        if let Some(old) = slot.replace(task) {
            old.abort();
        }
        true
    }
}

/// One request, without following a redirect: its status, or the error's
/// name.
async fn fetch_once(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>) -> Result<u16, String> {
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        let _ = xenonet_os::net::client();
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().ok()
    });
    let Some(client) = client else { return Err("TypeError".into()) };
    let Ok(m) = reqwest::Method::from_bytes(method.as_bytes()) else { return Err("TypeError".into()) };
    let mut req = client.request(m, url).timeout(Duration::from_secs_f64(TIMEOUT_MS / 1000.0));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        req = req.body(b);
    }
    match req.send().await {
        Ok(res) => Ok(res.status().as_u16()),
        Err(e) if e.is_timeout() => Err("TimeoutError".into()),
        Err(_) => Err("TypeError".into()),
    }
}

fn fail_message(f: &Fail) -> String {
    match f {
        Fail::Wake { message, .. } => message.clone(),
        Fail::Error(e) => e.message.clone(),
    }
}

impl wake::Host for WakeHost<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn log(&mut self, code: &str, detail: &str) {
        self.w.log("warn", code, detail);
    }
    fn notify(&mut self, mail: Value) {
        self.w.emit(From::Wakes, "mail", mail);
    }
    fn parse_url(&mut self, s: &str) -> Option<wake::Url> {
        parse_url(s)
    }
    fn resolve(&mut self, host: &str) -> Vec<String> {
        resolve(host)
    }
    /// A call as a wake makes it: no redirect followed, ten seconds at
    /// most, the body let go.
    fn fetch(&mut self, url: &str, request: &Request) -> Fetched {
        match self.w.block_on(fetch_once(&request.method, url, &request.headers, Some(request.body.clone().into_bytes()))) {
            Ok(status) => Fetched::Status(f64::from(status)),
            Err(name) => Fetched::Failed(Some(name)),
        }
    }
    fn set_timer(&mut self, kind: &str, id: &str, ms: f64) -> bool {
        self.arm(kind, id, ms)
    }
    fn clear_timers(&mut self, id: &str) {
        if let Some((a, b)) = self.side.timers.locked().remove(id) {
            for t in [a, b].into_iter().flatten() {
                t.abort();
            }
        }
    }
    fn send(&mut self, sink: u64, event: &str, data: Value) {
        if let Some(s) = self.side.sinks.locked().get(&sink) {
            let _ = s.tx.try_send((event.to_string(), data));
        }
    }
    fn end(&mut self, sink: u64) {
        if let Some(end) = self.side.sinks.locked().remove(&sink).and_then(|s| s.end) {
            let _ = end.send(());
        }
    }
    fn secrets_available(&mut self) -> bool {
        self.side.secrets.available()
    }
    fn secrets_kind(&mut self) -> Option<String> {
        Some(self.side.secrets.kind().to_string())
    }
    fn secret_get(&mut self, target: &str) -> Option<String> {
        self.side.secrets.get(target).ok().flatten()
    }
    fn secret_set(&mut self, target: &str, value: &str, comment: &str) -> rows::Result<()> {
        self.side.secrets.set(target, value, comment).map(|_| ()).map_err(rows::Error::new)
    }
    fn secret_remove(&mut self, target: &str) {
        let _ = self.side.secrets.remove(target);
    }
    fn is_registered(&mut self, id: &str) -> bool {
        self.w.actors.as_ref().is_some_and(|a| a.read(|r| r.is_registered(id)))
    }
    fn info(&mut self, id: &str) -> Value {
        self.w.actors.as_ref().map_or(Value::Null, |a| a.read(|r| r.info(id)))
    }
}

impl wake::Store for Rows {
    fn meta(&self, key: &str) -> rows::Result<Value> {
        self.direct(|s| s.meta(key))
    }
    fn set_meta(&self, key: &str, value: &str) -> rows::Result<()> {
        self.direct(|s| s.set_meta(key, value))
    }
    fn has_gateway_token(&self, slug: &str) -> rows::Result<bool> {
        self.direct(|s| s.gateway_token(slug)).map(|r| r.is_some())
    }
    fn unseen_ids(&self, actor: &str) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.unseen(actor))).map(|rs| rs.into_iter().map(|r| r["id"].clone()).collect())
    }
    fn actor_token_times(&self) -> rows::Result<Vec<Value>> {
        self.direct(|s| s.actor_token_times())
    }
}

fn wake_error(f: Fail) -> CoreError {
    match f {
        Fail::Wake { status, code, message } => CoreError::of(Kind::Wake, status as u16, code, message),
        Fail::Error(e) => {
            let mut c = CoreError::new(e.message);
            c.code = e.code;
            c
        }
    }
}

impl WakeSide {
    pub fn new(store: Shared, demo: bool, version: &str) -> WakeSide {
        WakeSide {
            core: Core::new(Arc::new(Rows(store)), version, now_ms()),
            secrets: Secrets::new(demo),
            timers: Mutex::new(HashMap::new()),
            sinks: Mutex::new(HashMap::new()),
            next_sink: AtomicU64::new(0),
        }
    }

    /// A message arrived for `id`.
    pub fn arrived(&self, w: &Runtime, id: &str) {
        let _ = self.core.arrived(&mut WakeHost { w, side: self }, id);
    }

    fn run<T>(&self, w: &Runtime, f: impl FnOnce(&Core, &mut WakeHost<'_>) -> Result<T, Fail>) -> R<T> {
        f(&self.core, &mut WakeHost { w, side: self }).map_err(wake_error)
    }

    pub fn list(&self, w: &Runtime) -> R<Value> {
        self.run(w, |c, k| c.list(k))
    }
}

pub struct WakesDriver(pub Arc<Runtime>);

impl WakesDriver {
    fn side(&self) -> R<&WakeSide> {
        self.0.wakes.as_ref().ok_or_else(|| CoreError::pending("core/actors/wake.js"))
    }

    async fn blocking<T: Send + 'static>(&self, f: impl FnOnce(&Runtime, &WakeSide) -> R<T> + Send + 'static) -> R<T> {
        let w = self.0.clone();
        tokio::task::spawn_blocking(move || match &w.wakes {
            Some(side) => f(&w, side),
            None => Err(CoreError::pending("core/actors/wake.js")),
        })
        .await
        .map_err(|e| CoreError::new(e.to_string()))?
    }
}

#[async_trait]
impl Wakes for WakesDriver {
    fn list(&self) -> R<Value> {
        self.side()?.list(&self.0)
    }
    async fn set(&self, slug: &str, spec: Value) -> R<Value> {
        let slug = slug.to_string();
        // What a wake is set from: its kind, URL and key, each as given.
        let mut ask = Map::new();
        for k in ["kind", "url", "key"] {
            if let Some(v) = spec.get(k) {
                ask.insert(k.to_string(), v.clone());
            }
        }
        let ask = Value::Object(ask);
        self.blocking(move |w, s| s.run(w, |c, k| c.set(k, &slug, &ask))).await
    }
    async fn test(&self, slug: &str) -> R<Value> {
        let slug = slug.to_string();
        self.blocking(move |w, s| s.run(w, |c, k| c.test(k, &slug))).await
    }
    async fn forget(&self, slug: &str) -> R<()> {
        let slug = slug.to_string();
        self.blocking(move |w, s| s.run(w, |c, k| c.forget(k, &slug))).await
    }
    fn subscribe(&self, id: &str) -> R<WakeStream> {
        let side = self.side()?;
        let no = side.next_sink.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(64);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel::<()>();
        side.sinks.locked().insert(no, Sink { tx: tx.clone(), end: Some(end_tx) });
        let set = match side.run(&self.0, |c, k| c.subscribe(k, id, no)) {
            Ok(set) => set,
            Err(e) => {
                side.sinks.locked().remove(&no);
                return Err(e);
            }
        };
        // The stream closed: out of its set, and its sink gone.
        let w = self.0.clone();
        let id = id.to_string();
        self.0.rt.spawn(async move {
            // The stream's reader went, or the wakes ended it.
            tokio::select! {
                () = tx.closed() => {}
                _ = end_rx => {}
            }
            drop(tx);
            if let Some(side) = &w.wakes {
                side.core.unsubscribe(&id, set, no);
                side.sinks.locked().remove(&no);
            }
        });
        Ok(rx)
    }
    fn stop(&self) {
        if let Ok(side) = self.side() {
            side.core.stop(&mut WakeHost { w: &self.0, side });
            for (_, (a, b)) in side.timers.locked().drain() {
                for t in [a, b].into_iter().flatten() {
                    t.abort();
                }
            }
        }
    }
}
