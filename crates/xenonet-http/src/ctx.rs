//! What every route reaches: the service's settings, its cores, the event
//! stream, the API's credentials, the error log, and the parts server/index.js
//! keeps at its top level (the component service, the actor gateway, the
//! assistant's endpoint, A2A and the network, each one of the service's
//! parts behind a trait of parts.rs, and Spotify's poll).

use crate::actor_caller::{ActorCaller, Rate};
use crate::borrowed::now_ms;
use crate::config::Config;
use crate::cores::{CoreError, Cores};
use crate::events::Events;
use crate::http::{BodyError, Reply};
use crate::parts::{A2a, Components, Endpoint, Gateway, Network};
use crate::upstream::Upstream;
use crate::window_key::{ApiKey, SelfKey, TrayKey, WindowKey};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_api::limits::Bucket;
use xenonet_api::wire::DriftLog;
use xenonet_os::log::{ErrInfo, Log};
use xenonet_os::sync::Locked;

pub struct Ctx {
    pub config: Config,
    pub version: String,
    pub cores: Cores,
    pub events: Arc<Events>,
    pub window_key: Arc<WindowKey>,
    /// The tray app's key, for its routes (routes/tray.rs).
    pub tray_key: Arc<TrayKey>,
    /// The API credential this service writes for the programs of this
    /// account (routes/rules.rs).
    pub api_key: Arc<ApiKey>,
    /// The service's own key, for its in-process reads of its own API.
    pub self_key: Arc<SelfKey>,
    pub log: Mutex<Log>,
    pub drift: Arc<Mutex<DriftLog>>,
    pub caller: Option<ActorCaller>,
    pub components: Mutex<Option<Arc<dyn Components>>>,
    /// The actor gateway's switch, made once the service is (it logs to it).
    pub gateway: std::sync::OnceLock<Arc<dyn Gateway>>,
    /// A2A agents' side of the gateway, made with it.
    pub a2a: std::sync::OnceLock<Arc<dyn A2a>>,
    /// Outbound actors, the agents XenoNet calls over A2A.
    pub a2a_out: std::sync::OnceLock<Arc<dyn crate::parts::A2aOut>>,
    pub endpoint: Mutex<Option<Arc<dyn Endpoint>>>,
    pub themes_dir: String,
    pub music: Mutex<Music>,
    /// The window reports you at most once a second; this leaves room for bursts.
    pub you_reports: Mutex<Bucket>,
    pub refusal_notes: Mutex<RefusalNotes>,
    /// XenoNet Dev reading another XenoNet (XENONET_UPSTREAM).
    pub upstream: Option<Upstream>,
    /// The network across machines (the service's gateway/network.rs),
    /// made once the service is: its node, link and LAN listener.
    pub net: std::sync::OnceLock<Arc<dyn Network>>,
}

/// What Spotify's poll keeps between looks (server/index.js's `music`).
#[derive(Default)]
pub struct Music {
    pub last: String,
    pub held: Value,
    pub quiet_until: f64,
    pub again: Option<tokio::sync::mpsc::UnboundedSender<u64>>,
}

/// The notifications refusals made lately: a few an hour, one a session
/// every ten minutes.
#[derive(Default)]
pub struct RefusalNotes {
    pub at: Vec<f64>,
    pub by_session: std::collections::HashMap<String, f64>,
}

pub const YOU_REPORTS: Rate = Rate { rate: 2.0, burst: 10.0 };

pub type Answer = Result<Reply, CoreError>;

impl From<BodyError> for CoreError {
    fn from(e: BodyError) -> Self {
        CoreError::status(e.status, e.message)
    }
}

impl Ctx {
    /// The service's parts around its cores, as the service's serve.rs
    /// makes them: no log folder unless one's given, the drift log in
    /// memory unless it's opened, new keys.
    pub fn new(config: Config, version: String, cores: Cores, log: Log) -> Ctx {
        let caller = cores.registration.clone().map(|r| ActorCaller::new(r, cores.verify.clone()));
        let themes_dir = std::path::Path::new(&config.state_dir).join("themes").to_string_lossy().into_owned();
        Ctx {
            config,
            version,
            cores,
            events: Arc::new(Events::new()),
            window_key: Arc::new(WindowKey::new()),
            tray_key: Arc::new(TrayKey::new()),
            api_key: Arc::new(ApiKey::new()),
            self_key: Arc::new(SelfKey::new()),
            log: Mutex::new(log),
            drift: Arc::new(Mutex::new(DriftLog::default())),
            caller,
            components: Mutex::new(None),
            gateway: std::sync::OnceLock::new(),
            a2a: std::sync::OnceLock::new(),
            a2a_out: std::sync::OnceLock::new(),
            endpoint: Mutex::new(None),
            themes_dir,
            music: Mutex::new(Music::default()),
            you_reports: Mutex::new(Bucket::new(YOU_REPORTS.rate, YOU_REPORTS.burst, now_ms())),
            refusal_notes: Mutex::new(RefusalNotes::default()),
            upstream: None,
            net: std::sync::OnceLock::new(),
        }
    }

    /// A service for tests: the demo's settings, these cores, nothing written.
    pub fn for_tests(cores: Cores) -> Arc<Ctx> {
        let mut config = Config::from(&["--demo".to_string()], &|k: &str| (k == "HOME" || k == "USERPROFILE").then(|| "/home/nova".to_string()));
        let tmp = std::env::temp_dir().join(format!("xenonet-service-test-{}", std::process::id()));
        config.state_dir = tmp.join("state").to_string_lossy().into_owned();
        config.cache_dir = tmp.join("cache").to_string_lossy().into_owned();
        Arc::new(Ctx::new(config, "0.0.0".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)))
    }

    pub fn now(&self) -> f64 {
        now_ms()
    }

    /// A line in the service's error log.
    pub fn log(&self, level: &str, code: &str, message: &str, err: Option<&ErrInfo>, fix: Option<&str>) {
        if let Ok(mut log) = self.log.lock() {
            log.write(level, code, Some(message), err, fix, None, now_ms());
        }
    }

    /// An event for every page.
    pub fn broadcast(&self, event: &str, data: &Value) {
        self.events.send(event, data);
    }

    /// A notification from the desktop app (through the component service).
    pub fn notify_desktop(&self, note: Value) {
        if let Some(c) = self.components.locked().clone() {
            c.notify(note);
        }
    }

    /// An app's link, a folder or a page, opened by the connected Desktop
    /// when there is one, else by the service itself.
    pub async fn open(&self, target: &str) -> Result<(), CoreError> {
        let components = self.components.locked().clone();
        if let Some(c) = components {
            if c.open(target).await {
                return Ok(());
            }
        }
        self.cores.opener.launch(target)
    }
}

/// A failure as `{ error, code, …extra }`, with headers.
pub fn fail(status: u16, code: &str, error: impl Into<String>, headers: &[(String, String)], extra: &Value) -> Reply {
    let mut body = json!({ "error": error.into(), "code": code });
    if let Value::Object(more) = extra {
        for (k, v) in more {
            body[k] = v.clone();
        }
    }
    let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    Reply::send(status, &body, &h)
}

/// What a pending core answers, wherever it's reached.
pub fn pending_reply(e: &CoreError) -> Reply {
    Reply::coded(501, "pending", e.message.clone())
}

/// An object body: null, a list or a number is the client's mistake.
pub fn is_object(v: &Value) -> bool {
    v.is_object()
}
