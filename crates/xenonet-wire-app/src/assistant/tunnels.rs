//! Tunnels XenoNet runs for remote actors' connections through OpenAI's
//! Secure MCP Tunnel (core/assistant/tunnels.js, tunnel-client.js;
//! crates/xenonet-assistant/src/tunnels.rs, tunnel_client.rs). Bindings and
//! their checks, each run's state, its backoff, its log as XenoNet shows it
//! and the client's environment are the Rust's; the processes, timers,
//! credential store, health probes and files are here.
//!
//! The key goes only into tunnel-client's own environment
//! (CONTROL_PLANE_API_KEY), never into a file, a command line or a log.

use super::connections::Remote;
use crate::fx::{self, now_ms, Locked, RunOptions};
use crate::secrets::Secrets;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xenonet_assistant::tunnel_client as client;
use xenonet_assistant::tunnels::{self as core, Binding};
use xenonet_engine::sql::Arg;
use xenonet_service::cores::{CoreError, Kind, Tunnels, R};
use xenonet_service::glue::From;
use xenonet_service::wire::Emitter;

fn fail(status: u16, code: &str, message: impl Into<String>) -> CoreError {
    CoreError::of(Kind::Tunnel, status, code, message)
}

fn refused(r: core::Refused) -> CoreError {
    fail(r.status, r.code, r.message)
}

fn target_ok(s: &str) -> bool {
    xenonet_actors::secret_store::is_target(s)
}

/// How a tunnel-client runs the adapter: the xenonet binary (installed
/// XenoNet's launcher), on this XenoNet's port.
pub struct Launch {
    pub bin: String,
    pub port: u16,
}

#[derive(Default)]
struct Handles {
    child: Option<(Option<u32>, tokio::sync::oneshot::Sender<()>)>,
    /// Which start of it the child is, so an old one's exit is let go.
    run: u64,
    timer: Option<JoinHandle<()>>,
    poll: Option<JoinHandle<()>>,
}

pub struct Runner {
    remote: Arc<Remote>,
    store: xenonet_engine::store::Shared,
    tdir: String,
    secrets: Arc<Secrets>,
    launch: Launch,
    home: String,
    demo: bool,
    emitter: Emitter,
    core: Mutex<core::Tunnels>,
    live: Mutex<HashMap<String, Handles>>,
    versions: Mutex<HashMap<String, Option<String>>>,
    listeners: Mutex<Vec<mpsc::UnboundedSender<()>>>,
    next: Mutex<u64>,
    me: Weak<Runner>,
}

impl Runner {
    #[expect(clippy::too_many_arguments, reason = "createTunnels' options, one by one")]
    pub fn new(remote: Arc<Remote>, store: xenonet_engine::store::Shared, dir: &str, secrets: Arc<Secrets>, launch: Launch, home: &str, demo: bool, emitter: Emitter) -> Arc<Self> {
        let r = Arc::new_cyclic(|me| Runner {
            remote: remote.clone(),
            store,
            tdir: Path::new(dir).join("tunnels").to_string_lossy().into_owned(),
            secrets,
            launch,
            home: home.into(),
            demo,
            emitter,
            core: Mutex::new(core::Tunnels::default()),
            live: Mutex::new(HashMap::new()),
            versions: Mutex::new(HashMap::new()),
            listeners: Mutex::new(Vec::new()),
            next: Mutex::new(0),
            me: me.clone(),
        });
        // A grant revoked, expired or turned off stops its tunnel; a new
        // credential starts it again, since the adapter holds the old one.
        let mut events = remote.listen();
        let me = Arc::downgrade(&r);
        tokio::spawn(async move {
            while let Some((name, data)) = events.recv().await {
                let Some(t) = me.upgrade() else { return };
                let grant = xenonet_base::text::js_string(&data["grant"]);
                match name.as_str() {
                    "revoked" => t.stop_run(&grant, Some("its connection was revoked")),
                    "expired" => t.stop_run(&grant, Some("its grant expired")),
                    "disabled" => {
                        let grants = t.core.locked().grants();
                        for g in grants {
                            t.stop_run(&g, Some("cloud access is off"));
                        }
                    }
                    "rotated" => {
                        let running = t.live.locked().get(&grant).is_some_and(|h| h.child.is_some());
                        if running {
                            t.stop_run(&grant, None);
                            let again = Arc::downgrade(&t);
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(1500)).await;
                                if let Some(t) = again.upgrade() {
                                    if let Err(e) = t.start_run(&grant).await {
                                        t.log(&format!("tunnel {grant}: didn't start again after a new credential: {}", e.message));
                                    }
                                }
                            });
                        }
                    }
                    _ => {}
                }
            }
        });
        r
    }

    fn log(&self, line: &str) {
        self.emitter.log("info", "assistant.tunnel", line);
    }

    // The window reads how they stand with its key; this says only that
    // it changed.
    fn changed(&self) {
        self.emitter.emit(From::Tunnels, "changed", json!({}));
        self.listeners.locked().retain(|tx| tx.send(()).is_ok());
    }

    fn file_of(&self, grant: &str, ext: &str) -> String {
        Path::new(&self.tdir).join(format!("{grant}.{ext}")).to_string_lossy().into_owned()
    }

    // ---------- what's set up ----------

    fn binding_of(&self, connection: &Value) -> Binding {
        let row = self.store.with(|s| s.connection(super::arg(connection))).ok().flatten();
        let text = row.as_ref().map(|r| xenonet_engine::store::js_text(&xenonet_actors::connections::row_arg(r).field("transport_binding")));
        core::binding_of(text.as_deref().filter(|t| !t.is_empty() && *t != "null"), &target_ok)
    }

    fn tunneled(&self, grant: &str, active: bool) -> R<Value> {
        let v = if xenonet_assistant::credentials::is_grant(grant) { self.remote.view(grant)? } else { Value::Null };
        if v.is_null() || !v.is_object() {
            return Err(fail(404, "not-found", "XenoNet has no such connection"));
        }
        if v["transport_kind"] != json!(core::TUNNEL_TRANSPORT) {
            return Err(fail(400, "bad-request", "that connection doesn't go through a tunnel"));
        }
        if active && v["state"] != json!("active") {
            return Err(fail(409, "not-active", format!("that connection is {}", xenonet_base::text::js_string(&v["state"]))));
        }
        Ok(v)
    }

    async fn has_key(&self, target: Option<&str>) -> bool {
        let Some(target) = target else { return false };
        if let Some(had) = self.core.locked().has_key(target, now_ms()) {
            return had;
        }
        let has = self.secrets.has(target).await.unwrap_or(false);
        self.core.locked().set_has(target, has, now_ms());
        has
    }

    // ---------- the client ----------

    fn find(&self) -> Option<String> {
        if self.demo {
            return fx::env_set("XENONET_TUNNEL_CLIENT");
        }
        client::find_client(&fx::env_map(), fx::platform(), &self.home, &mut |f| Ok(std::fs::metadata(f).is_ok_and(|m| m.is_file())))
    }

    async fn client(&self) -> (Option<String>, Option<String>) {
        let Some(file) = self.find() else { return (None, None) };
        let known = self.versions.locked().get(&file).cloned();
        let version = match known {
            Some(v) => v,
            None => {
                let r = fx::exec_file(&file, &["--version".into()], 8000, RunOptions::default()).await;
                let v = if r.ok() { client::version_in(&r.stdout) } else { None };
                self.versions.locked().insert(file.clone(), v.clone());
                v
            }
        };
        (Some(file), version)
    }

    fn profiles(&self) -> Vec<Value> {
        if self.demo {
            return Vec::new();
        }
        struct Disk;
        impl client::Files for Disk {
            fn list(&mut self, dir: &str) -> Result<Vec<String>, ()> {
                std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).map_err(|_| ())
            }
            fn read(&mut self, file: &str) -> Result<String, ()> {
                std::fs::read_to_string(file).map_err(|_| ())
            }
            fn mtime(&mut self, file: &str) -> Result<f64, ()> {
                std::fs::metadata(file).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).ok_or(())
            }
        }
        let dirs = client::profile_dirs(&fx::env_map(), fx::platform(), &self.home);
        client::find_profiles(&dirs, &mut Disk)
    }

    async fn probe(url: &str, ms: u64) -> Option<String> {
        let r = xenonet_service::net::fetch("GET", url, &[], None, Duration::from_millis(ms)).await.ok()?;
        r.ok().then(|| r.text())
    }

    // A tunnel-client you run yourself for this grant, from a profile of
    // your own with a fixed health address that answers.
    async fn outside(&self, grant: &str) -> Option<String> {
        for p in self.profiles().into_iter().filter(|p| p["grant"] == json!(grant)) {
            let Some(url) = core::readyz_of(p.get("health").and_then(Value::as_str)) else { continue };
            if Self::probe(&url, 1000).await.is_some() {
                return Some(xenonet_base::text::js_string(&p["name"]));
            }
        }
        None
    }

    // ---------- running one ----------

    fn view(&self, grant: &str) -> Value {
        let pid = self.live.locked().get(grant).and_then(|h| h.child.as_ref().and_then(|c| c.0)).map(f64::from);
        self.core.locked().view(grant, pid)
    }

    fn with_grant(&self, grant: &str) -> Value {
        let mut v = self.view(grant);
        v["grant"] = json!(grant);
        // { grant, ...view }: grant first.
        let mut out = serde_json::Map::new();
        out.insert("grant".into(), json!(grant));
        if let Value::Object(m) = v {
            for (k, x) in m {
                if k != "grant" {
                    out.insert(k, x);
                }
            }
        }
        Value::Object(out)
    }

    async fn start_run(self: &Arc<Self>, grant: &str) -> R<Value> {
        if self.remote.disabled()? {
            return Err(fail(409, "disabled", "cloud access is off; turn it on first"));
        }
        let v = self.tunneled(grant, true)?;
        let running = self.live.locked().get(grant).is_some_and(|h| h.child.is_some());
        if self.core.locked().has_run(grant) && running && !self.core.locked().stopping(grant) {
            return Ok(self.with_grant(grant));
        }
        let b = self.binding_of(&v["connection_id"]);
        let Some(tunnel_id) = b.tunnel_id.clone() else { return Err(fail(409, "no-tunnel", "say which tunnel it is first")) };
        let (Some(file), _) = self.client().await else { return Err(fail(409, "no-client", format!("XenoNet can't find tunnel-client; install it from {}", client::RELEASES))) };
        if let Some(profile) = self.outside(grant).await {
            return Err(fail(
                409,
                "running-elsewhere",
                format!("a tunnel-client you started yourself already runs this connection (profile {profile}); stop it first, or keep using it"),
            ));
        }
        let key = match core::key_target_of(&b) {
            Some(t) => self.secrets.get(&t).await.ok().flatten(),
            None => None,
        };
        let Some(key) = key else { return Err(fail(409, "no-key", "XenoNet has no runtime key for this tunnel yet")) };
        if let Some(t) = self.live.locked().entry(grant.into()).or_default().timer.take() {
            t.abort();
        }
        self.launch_run(grant, &v, &tunnel_id, &file, &key)?;
        Ok(self.with_grant(grant))
    }

    fn profile_text(&self, grant: &str, tunnel_id: &str, label: &Value) -> R<String> {
        let argv = client::adapter_argv(&json!(self.launch.bin), &json!(grant), &json!(self.launch.port)).map_err(CoreError::new)?;
        let command = client::command_line(&argv);
        client::profile_yaml(&json!(tunnel_id), &json!(command), Some(label)).map_err(CoreError::new)
    }

    fn write_private(file: &str, text: &str) -> R<()> {
        xenonet_app::fsx::write_file(file, text.as_bytes(), Some(0o600)).map_err(|e| CoreError::new(e.message))
    }

    fn launch_run(self: &Arc<Self>, grant: &str, v: &Value, tunnel_id: &str, file: &str, key: &str) -> R<()> {
        xenonet_app::fsx::mkdir_p(&self.tdir, Some(0o700)).map_err(|e| CoreError::new(e.message))?;
        let profile = self.file_of(grant, "yaml");
        let url_file = self.file_of(grant, "url");
        Self::write_private(&profile, &self.profile_text(grant, tunnel_id, &v["label"])?)?;
        let _ = std::fs::remove_file(&url_file);
        let env = core::child_env(&fx::env_map(), key, &json!(self.launch.port), true);
        if self.core.locked().launching(grant, now_ms()) {
            self.changed();
        }
        let mut cmd = tokio::process::Command::new(file);
        cmd.args(["run", "--profile-file", &profile, "--health.url-file", &url_file])
            .env_clear()
            .envs(fx::env_pairs(&env))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.exited(grant, None, &format!("it couldn't start: {e}"));
                return Ok(());
            }
        };
        let run = {
            let mut n = self.next.locked();
            *n += 1;
            *n
        };
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
        {
            let mut live = self.live.locked();
            let h = live.entry(grant.into()).or_default();
            h.child = Some((child.id(), kill_tx));
            h.run = run;
        }
        let (out, err) = (child.stdout.take(), child.stderr.take());
        for stream in [out.map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), err.map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)]
            .into_iter()
            .flatten()
        {
            let (me, g) = (self.me.clone(), grant.to_string());
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Some(t) = me.upgrade() else { return };
                    let said = t.core.locked().line(&g, &line, now_ms(), &t.home);
                    if let Some(s) = said {
                        t.log(&s);
                    }
                }
            });
        }
        let (me, g) = (self.me.clone(), grant.to_string());
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = &mut kill_rx => {
                    let _ = child.start_kill();
                    child.wait().await.ok()
                }
            };
            let Some(t) = me.upgrade() else { return };
            let why = match status.and_then(|s| s.code()) {
                Some(c) => format!("it exited with code {c}"),
                None => "it exited".to_string(),
            };
            t.exited(&g, Some(run), &why);
        });
        // Its health, now and then, from the address it wrote when it started.
        self.poll_later(grant, run, url_file, core::FIRST_POLL_MS);
        Ok(())
    }

    fn poll_later(self: &Arc<Self>, grant: &str, run: u64, url_file: String, ms: f64) {
        let (me, g) = (self.me.clone(), grant.to_string());
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms as u64)).await;
            let Some(t) = me.upgrade() else { return };
            let current = |t: &Runner| t.live.locked().get(&g).is_some_and(|h| h.run == run && h.child.is_some()) && !t.core.locked().stopping(&g);
            if !current(&t) {
                return;
            }
            let written = std::fs::read_to_string(&url_file).ok();
            let url = written.as_deref().and_then(core::health_url);
            let body = match url {
                Some(u) => Self::probe(&u, 3000).await,
                None => None,
            };
            if !current(&t) {
                return;
            }
            let health = body.and_then(|b| xenonet_base::json::parse(&b).ok());
            if t.core.locked().health(&g, health.as_ref(), now_ms()) {
                t.changed();
            }
            t.poll_later(&g, run, url_file, core::POLL_MS);
        });
        if let Some(old) = self.live.locked().entry(grant.into()).or_default().poll.replace(t) {
            old.abort();
        }
    }

    fn exited(self: &Arc<Self>, grant: &str, run: Option<u64>, why: &str) {
        {
            let mut live = self.live.locked();
            let h = live.entry(grant.into()).or_default();
            if run.is_some_and(|r| r != h.run) {
                return;
            }
            h.child = None;
            if let Some(p) = h.poll.take() {
                p.abort();
            }
        }
        let a = self.core.locked().exited(grant, why, now_ms());
        if let Some(l) = &a.log {
            self.log(l);
        }
        if a.changed {
            self.changed();
        }
        let Some(ms) = a.retry_ms else { return };
        let (me, g) = (self.me.clone(), grant.to_string());
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms as u64)).await;
            let Some(t) = me.upgrade() else { return };
            let running = t.live.locked().get(&g).is_some_and(|h| h.child.is_some());
            if t.core.locked().stopping(&g) || running {
                return;
            }
            if let Err(e) = t.start_run(&g).await {
                if t.core.locked().failed(&g, &e.message, now_ms()) {
                    t.changed();
                }
            }
        });
        if let Some(old) = self.live.locked().entry(grant.into()).or_default().timer.replace(t) {
            old.abort();
        }
    }

    fn stop_run(&self, grant: &str, reason: Option<&str>) {
        if !self.core.locked().has_run(grant) {
            return;
        }
        let child = {
            let mut live = self.live.locked();
            let h = live.entry(grant.into()).or_default();
            for t in [h.timer.take(), h.poll.take()].into_iter().flatten() {
                t.abort();
            }
            h.child.take()
        };
        if self.core.locked().stop(grant, reason, child.is_some(), now_ms()) == Some(true) {
            self.changed();
        }
        if let Some((pid, kill)) = child {
            // On Windows, the whole tree, which a kill of the parent alone
            // would leave.
            if cfg!(windows) {
                if let Some(pid) = pid {
                    fx::start_detached("taskkill", &["/pid".into(), pid.to_string(), "/T".into(), "/F".into()]);
                }
            }
            let _ = kill.send(());
        }
    }
}

/// The Tunnels trait, for the routes and the service's start and stop.
pub struct TunnelsCore(pub Arc<Runner>);

#[async_trait]
impl Tunnels for TunnelsCore {
    async fn status(&self) -> R<Value> {
        let t = &self.0;
        let (path, version) = t.client().await;
        let list = t.remote.view_all()?;
        let found = t.profiles();
        let mut tunnels = Vec::new();
        for x in list.iter().filter(|x| x["transport_kind"] == json!(core::TUNNEL_TRANSPORT) && x["state"] != json!("revoked")) {
            let b = t.binding_of(&x["connection_id"]);
            let target = core::key_target_of(&b);
            let yours = found.iter().find(|p| p["grant"] == x["grant_id"]).cloned();
            let stored = t.has_key(target.as_deref()).await;
            let run = t.view(&xenonet_base::text::js_string(&x["grant_id"]));
            tunnels.push(core::status_entry(x, &b, stored, yours.as_ref(), &run));
        }
        Ok(json!({
            "client": { "found": path.is_some(), "path": path.map(|p| xenonet_base::paths::tilde(&p, &t.home)), "version": version },
            "store": { "kind": t.secrets.kind(), "available": t.secrets.available() },
            "links": { "releases": client::RELEASES, "tunnels": client::TUNNELS_PAGE, "guide": core::GUIDE },
            "tunnels": tunnels,
        }))
    }

    fn stop(&self, grant: &str) -> R<Value> {
        self.0.stop_run(grant, None);
        Ok(self.0.with_grant(grant))
    }

    async fn forget_key(&self, grant: &str) -> R<Value> {
        let t = &self.0;
        let v = t.tunneled(grant, false)?;
        let b = t.binding_of(&v["connection_id"]);
        let Some(tunnel_id) = &b.tunnel_id else { return Ok(json!({ "grant": grant, "key": "missing" })) };
        if b.key_target.is_some() {
            return Err(fail(409, "yours", "XenoNet didn't make that item in your credential store; remove it there yourself, or name another"));
        }
        let target = core::key_target_for(tunnel_id);
        let _ = t.secrets.remove(&target).await;
        t.core.locked().set_has(&target, false, now_ms());
        t.changed();
        Ok(json!({ "grant": grant, "key": "missing" }))
    }

    async fn start(&self, grant: &str) -> R<Value> {
        self.0.start_run(grant).await
    }

    // tunnel-client's own checks of the profile, with the key: its lines,
    // with anything like a key taken out.
    async fn check(&self, grant: &str) -> R<Value> {
        let t = &self.0;
        let v = t.tunneled(grant, false)?;
        let b = t.binding_of(&v["connection_id"]);
        let Some(tunnel_id) = b.tunnel_id.clone() else { return Err(fail(409, "no-tunnel", "say which tunnel it is first")) };
        let (Some(file), _) = t.client().await else { return Err(fail(409, "no-client", format!("XenoNet can't find tunnel-client; install it from {}", client::RELEASES))) };
        let key = match core::key_target_of(&b) {
            Some(target) => t.secrets.get(&target).await.ok().flatten(),
            None => None,
        };
        let Some(key) = key else { return Err(fail(409, "no-key", "XenoNet has no runtime key for this tunnel yet")) };
        xenonet_app::fsx::mkdir_p(&t.tdir, Some(0o700)).map_err(|e| CoreError::new(e.message))?;
        let profile = t.file_of(grant, "check.yaml");
        Runner::write_private(&profile, &t.profile_text(grant, &tunnel_id, &v["label"])?)?;
        let env = core::child_env(&fx::env_map(), &key, &json!(t.launch.port), false);
        let r = fx::exec_file(
            &file,
            &["doctor".into(), "--profile-file".into(), profile.clone(), "--explain".into()],
            60000,
            RunOptions { env: Some(fx::env_pairs(&env)), ..Default::default() },
        )
        .await;
        let _ = std::fs::remove_file(&profile);
        Ok(json!({ "grant": grant, "ok": r.ok(), "lines": core::check_lines(&format!("{}\n{}", r.stdout, r.stderr), &t.home) }))
    }

    async fn store_key(&self, grant: &str, key: &Value) -> R<Value> {
        let t = &self.0;
        let v = t.tunneled(grant, false)?;
        let b = t.binding_of(&v["connection_id"]);
        let k = core::runtime_key(&b, (!key.is_null()).then_some(key)).map_err(refused)?;
        if !t.secrets.available() {
            return Err(fail(501, "no-store", format!("XenoNet can't reach {} on this computer", t.secrets.kind())));
        }
        let target = core::key_target_of(&b).unwrap_or_default();
        t.secrets.set(&target, &k, "Runtime API key for an OpenAI Secure MCP Tunnel, kept by XenoNet for a remote actor").await.map_err(|m| fail(500, "store", m))?;
        t.core.locked().set_has(&target, true, now_ms());
        t.changed();
        Ok(json!({ "grant": grant, "key": "stored", "target": target }))
    }

    fn configure(&self, grant: &str, spec: Value) -> R<Value> {
        let t = &self.0;
        let v = t.tunneled(grant, false)?;
        let b = t.binding_of(&v["connection_id"]);
        let some = |k: &str| spec.get(k).filter(|x| !x.is_null());
        let next = core::configure(&b, some("tunnelId"), some("keyTarget"), some("autostart"), &target_ok).map_err(refused)?;
        t.store.with(|s| s.set_transport_binding(super::arg(&v["connection_id"]), Arg::text(next.text()))).map_err(|e| CoreError::new(e.to_string()))?;
        t.changed();
        Ok(json!({ "grant": grant, "tunnel_id": next.tunnel_id, "key_target": core::key_target_of(&next), "autostart": next.autostart }))
    }

    async fn autostart(&self) -> R<()> {
        let t = &self.0;
        if t.remote.disabled()? {
            return Ok(());
        }
        for x in t.remote.view_all()? {
            if x["transport_kind"] != json!(core::TUNNEL_TRANSPORT) || x["state"] != json!("active") || !t.binding_of(&x["connection_id"]).autostart {
                continue;
            }
            let grant = xenonet_base::text::js_string(&x["grant_id"]);
            if let Err(e) = t.start_run(&grant).await {
                t.log(&format!("tunnel {grant}: didn't start with XenoNet: {}", e.message));
            }
        }
        Ok(())
    }

    fn stop_all(&self) {
        let grants = self.0.core.locked().grants();
        for g in grants {
            self.0.stop_run(&g, None);
        }
    }

    fn subscribe(&self) -> Option<mpsc::UnboundedReceiver<()>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.0.listeners.locked().push(tx);
        Some(rx)
    }
}
