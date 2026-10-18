//! Waking actors when a message arrives (core/actors/wake.js over
//! crates/xenonet-actors/src/wake.rs): a stream for a program of their own,
//! a webhook you give (its URL and key in your credential store, the
//! demo's in memory), or a notification to you. The timers, the streams'
//! sinks, DNS, fetch and URL parsing are here.

use crate::actors::{collab_call, collab_get, collab_kind};
use crate::exec::Spec;
use crate::js::{arg, at, json, no_method, now_ms, obj, text, Locked};
use crate::world::World;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_actors::secret_store::{self, MemorySecrets, SecretStore};
use xenonet_actors::wake::{Fail, Wakes as Core, TIMEOUT_MS};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;
use xenonet_service::cores::actors::WakeStream;
use xenonet_service::cores::{CoreError, Kind, Wakes, R};
use xenonet_service::glue::From;

/// Your computer's credential store, or the demo's memory.
pub enum Secrets {
    System(SecretStore),
    Memory(Mutex<MemorySecrets>),
}

struct Runner;

impl secret_store::Run for Runner {
    fn run(&mut self, c: secret_store::Command) -> std::result::Result<secret_store::Ran, String> {
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
        let tool = std::env::var("PATH").ok().and_then(|p| xenonet_base::paths::on_path("secret-tool", &p, None, false, &mut |f: &str| xenonet_base::paths::can_run(f)));
        Secrets::System(SecretStore::new(crate::js::platform(), tool))
    }

    fn kind(&self) -> &'static str {
        match self {
            Secrets::System(s) => s.kind(),
            Secrets::Memory(_) => MemorySecrets::KIND,
        }
    }

    fn call(&self, method: &str, args: &[Arg]) -> Result<Arg> {
        let t = text(&at(args, 0));
        let r: std::result::Result<Arg, String> = match self {
            Secrets::Memory(m) => {
                let mut m = m.locked();
                Ok(match method {
                    "available" => Arg::Bool(true),
                    "has" => Arg::Bool(m.has(&t)),
                    "get" => m.get(&t).map_or(Arg::Null, Arg::Text),
                    "set" => Arg::Bool(m.set(&t, &text(&at(args, 1))).map_err(Thrown::new)?),
                    "remove" => Arg::Bool(m.remove(&t)),
                    _ => return Err(no_method("secrets", method)),
                })
            }
            Secrets::System(s) => match method {
                "available" => Ok(Arg::Bool(s.available())),
                "has" => s.has(&mut Runner, &t).map(Arg::Bool),
                "get" => s.get(&mut Runner, &t).map(|v| v.map_or(Arg::Null, Arg::Text)),
                "set" => {
                    let comment = match at(args, 2).field("comment") {
                        Arg::Undef => "Kept by XenoNet".to_string(),
                        c => text(&c),
                    };
                    s.set(&mut Runner, &t, &text(&at(args, 1)), &comment).map(Arg::Bool)
                }
                "remove" => s.remove(&mut Runner, &t).map(Arg::Bool),
                _ => return Err(no_method("secrets", method)),
            },
        };
        r.map_err(Thrown::new)
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

/// The wakes' collaborators.
struct WakeCalls<'a> {
    w: &'a World,
    side: &'a WakeSide,
}

/// A URL as WHATWG's parser reads it, its hash taken off; null for what
/// isn't one.
fn parse_url(s: &str) -> Arg {
    let Ok(mut u) = url::Url::parse(s) else { return Arg::Null };
    u.set_fragment(None);
    let host = match (u.host_str(), u.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => String::new(),
    };
    obj(vec![
        ("username", Arg::text(u.username())),
        ("password", Arg::text(u.password().unwrap_or(""))),
        ("protocol", Arg::text(format!("{}:", u.scheme()))),
        ("hostname", Arg::text(u.host_str().unwrap_or(""))),
        ("host", Arg::text(host)),
        ("href", Arg::text(u.as_str())),
    ])
}

/// The addresses a host name has, as dns.lookup({ all: true }) gives them.
fn resolve(host: &str) -> Arg {
    use std::net::ToSocketAddrs;
    let addrs = (host, 0).to_socket_addrs().map(|a| a.collect::<Vec<_>>()).unwrap_or_default();
    Arg::List(addrs.iter().map(|a| obj(vec![("address", Arg::text(a.ip().to_string())), ("family", Arg::Num(if a.is_ipv4() { 4.0 } else { 6.0 }))])).collect())
}

impl WakeCalls<'_> {
    /// A call as the JS made it: no redirect followed, ten seconds at most,
    /// the body let go. What it threw, as its parts.
    fn fetch(&self, url: &str, init: &Arg) -> Arg {
        let headers: Vec<(String, String)> = match init.field("headers") {
            Arg::Obj(h) => h.iter().map(|(k, v)| (k.clone(), text(v))).collect(),
            _ => Vec::new(),
        };
        let body = init.field("body");
        let body = (!body.is_nullish()).then(|| text(&body).into_bytes());
        let method = match init.field("method") {
            Arg::Text(m) => m,
            _ => "GET".into(),
        };
        match self.w.block_on(fetch_once(&method, url, &headers, body)) {
            Ok(status) => obj(vec![("status", Arg::Num(f64::from(status)))]),
            Err(name) => obj(vec![("failed", obj(vec![("name", Arg::text(name))]))]),
        }
    }

    fn set_timer(&self, kind: &str, id: &str, ms: f64) -> bool {
        let key = id.to_string();
        let k = key.clone();
        let task = if kind == "wake" {
            self.w.after(ms, move |w| {
                let Some(side) = &w.wakes else { return };
                if let Some(t) = side.timers.locked().get_mut(&k) {
                    t.0 = None;
                }
                if side.core.wake_due(&Arg::text(&k)) {
                    if let Err(f) = side.core.fire(&mut WakeCalls { w, side }, &Arg::text(&k)) {
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
                let _ = side.core.notify_due(&mut WakeCalls { w, side }, &Arg::text(&k));
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
async fn fetch_once(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>) -> std::result::Result<u16, String> {
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        let _ = xenonet_service::net::client();
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
        Fail::Thrown(t) => t.message.clone(),
    }
}

impl Calls for WakeCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let x = |i| at(&args, i);
        let w = self.w;
        match (path, method) {
            ("", "now") => Ok(Arg::Num(now_ms())),
            ("", "log") => {
                w.log("warn", &text(&x(0)), &text(&x(1)));
                Ok(Arg::Undef)
            }
            ("", "notify") => {
                w.emit(From::Wakes, "mail", json(&x(0)));
                Ok(Arg::Undef)
            }
            ("", "parseUrl") => Ok(parse_url(&text(&x(0)))),
            ("", "resolve") => Ok(resolve(&text(&x(0)))),
            ("", "fetch") => Ok(self.fetch(&text(&x(0)), &x(1))),
            ("", "setTimer") => Ok(Arg::Bool(self.set_timer(&text(&x(0)), &text(&x(1)), crate::js::num(&x(2))))),
            ("", "clearTimers") => {
                if let Some((a, b)) = self.side.timers.locked().remove(&text(&x(0))) {
                    for t in [a, b].into_iter().flatten() {
                        t.abort();
                    }
                }
                Ok(Arg::Undef)
            }
            ("", "send") => {
                let no = crate::js::num(&x(0)) as u64;
                if let Some(sink) = self.side.sinks.locked().get(&no) {
                    let _ = sink.tx.try_send((text(&x(1)), json(&x(2))));
                }
                Ok(Arg::Undef)
            }
            ("", "end") => {
                if let Some(end) = self.side.sinks.locked().remove(&(crate::js::num(&x(0)) as u64)).and_then(|s| s.end) {
                    let _ = end.send(());
                }
                Ok(Arg::Undef)
            }
            ("secrets", m) => self.side.secrets.call(m, &args),
            (p, m) if p == "actors" || p.starts_with("actors.") => collab_call(w, p, m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "secrets.kind" => Arg::text(self.side.secrets.kind()),
            p => collab_get(self.w, p),
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "secrets" => "object",
            "secrets.kind" => "string",
            p if p.starts_with("secrets.") => "function",
            p => collab_kind(self.w, p),
        }
        .into())
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

fn wake_error(f: Fail) -> CoreError {
    match f {
        Fail::Wake { status, code, message } => CoreError::of(Kind::Wake, status as u16, code, message),
        Fail::Thrown(t) => crate::actors::core_error(t),
    }
}

impl WakeSide {
    pub fn new(store: Shared, demo: bool, version: &str) -> WakeSide {
        WakeSide {
            core: Core::new(store, Arg::text(version), now_ms()),
            secrets: Secrets::new(demo),
            timers: Mutex::new(HashMap::new()),
            sinks: Mutex::new(HashMap::new()),
            next_sink: AtomicU64::new(0),
        }
    }

    /// A message arrived for `id`.
    pub fn arrived(&self, w: &World, id: &Arg) {
        let _ = self.core.arrived(&mut WakeCalls { w, side: self }, id);
    }

    fn run<T>(&self, w: &World, f: impl FnOnce(&Core, &mut WakeCalls) -> std::result::Result<T, Fail>) -> R<T> {
        f(&self.core, &mut WakeCalls { w, side: self }).map_err(wake_error)
    }

    pub fn list(&self, w: &World) -> R<Value> {
        self.run(w, |c, k| c.list(k)).map(|a| json(&a))
    }
}

pub struct WakesDriver(pub Arc<World>);

impl WakesDriver {
    fn side(&self) -> R<&WakeSide> {
        self.0.wakes.as_ref().ok_or_else(|| CoreError::pending("core/actors/wake.js"))
    }

    async fn blocking<T: Send + 'static>(&self, f: impl FnOnce(&World, &WakeSide) -> R<T> + Send + 'static) -> R<T> {
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
        let (slug, spec) = (Arg::text(slug), arg(&spec));
        let ask = obj(vec![("kind", spec.field("kind")), ("url", spec.field("url")), ("key", spec.field("key"))]);
        self.blocking(move |w, s| s.run(w, |c, k| c.set(k, &slug, &ask)).map(|a| json(&a))).await
    }
    async fn test(&self, slug: &str) -> R<Value> {
        let slug = Arg::text(slug);
        self.blocking(move |w, s| s.run(w, |c, k| c.test(k, &slug)).map(|a| json(&a))).await
    }
    async fn forget(&self, slug: &str) -> R<()> {
        let slug = Arg::text(slug);
        self.blocking(move |w, s| s.run(w, |c, k| c.forget(k, &slug))).await
    }
    fn subscribe(&self, id: &str) -> R<WakeStream> {
        let side = self.side()?;
        let no = side.next_sink.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::channel(64);
        let (end_tx, end_rx) = tokio::sync::oneshot::channel::<()>();
        side.sinks.locked().insert(no, Sink { tx: tx.clone(), end: Some(end_tx) });
        let a = Arg::text(id);
        let set = match side.run(&self.0, |c, k| c.subscribe(k, &a, no)) {
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
                _ = tx.closed() => {}
                _ = end_rx => {}
            }
            drop(tx);
            if let Some(side) = &w.wakes {
                side.core.unsubscribe(&Arg::text(&id), set, no);
                side.sinks.locked().remove(&no);
            }
        });
        Ok(rx)
    }
    fn stop(&self) {
        if let Ok(side) = self.side() {
            let _ = side.run(&self.0, |c, k| c.stop(k));
            for (_, (a, b)) in side.timers.locked().drain() {
                for t in [a, b].into_iter().flatten() {
                    t.abort();
                }
            }
        }
    }
}
