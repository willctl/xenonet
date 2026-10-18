//! The component service (server/component-service.js): a socket on
//! 127.0.0.1 that XenoNet Desktop and the Tools connect to, found through
//! `runtime/service-<port>.json`, which holds its port and a token new at each
//! start. JSON lines (rpc.rs): a hello carrying the token, then status,
//! console (relayed to the Desktop), stop, and the Desktop's own requests
//! (where its window is, the terminals). The service sends the Desktop
//! `notification` events, and asks it to open things and, for XenoNet Dev's
//! control channel, to act on its window (`console`).
//!
//! What Desktop got over a utility process's port when it started the JS
//! service itself (the window key, ready, open, notify, the control relay)
//! is all here: the hello's answer carries the window key, and the rest
//! are requests and events on this socket (docs/design/rust-core.md).

use crate::cores::{ok, Terminals};
use crate::http::Peer;
use crate::routes::{api_range, CAPABILITIES, CONSOLE_API, SERVICE_API};
use crate::rpc::{negotiate, Handler, Rpc, MAX_LINE};
use crate::window_key::{random_hex, same, WindowKey};
use serde_json::{json, Value};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use xenonet_base::sync::Locked;

const HELLO: Duration = Duration::from_secs(5);
// Desktop's answers to the service's requests: rpc.js's default.
const ASK: Duration = Duration::from_secs(150);

type Seen = Pin<Box<dyn Future<Output = Value> + Send>>;

/// What the component service needs from the rest of the service.
pub struct Hooks {
    pub version: String,
    pub window_key: Arc<WindowKey>,
    /// Where the window is (shown, hidden, closed): your presence.
    pub window_state: Arc<dyn Fn(&Value) + Send + Sync>,
    pub shutdown: Arc<dyn Fn() + Send + Sync>,
    /// Whether the Desktop that connected is XenoNet's app (verify.desktop);
    /// None when it went.
    pub on_desktop: Arc<dyn Fn(Option<Peer>) -> Seen + Send + Sync>,
    pub terminals: Arc<dyn Terminals>,
}

pub struct Components {
    token: String,
    port: u16,
    file: PathBuf,
    desktop: Mutex<Option<Rpc>>,
    clients: Mutex<Vec<Rpc>>,
    hooks: Hooks,
}

impl Components {
    pub fn info(&self) -> Value {
        json!({
            "version": self.hooks.version,
            "pid": std::process::id(),
            "api": api_range(SERVICE_API),
            "consoleApi": api_range(CONSOLE_API),
            "capabilities": CAPABILITIES,
            "desktop": self.desktop.locked().is_some(),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// A notification for the connected Desktop, if there is one.
    pub fn notify(&self, note: Value) {
        if let Some(d) = self.desktop.locked().clone() {
            let _ = d.notify("notification", note);
        }
    }

    /// Has the connected Desktop open an app's link, a folder or a page;
    /// false when there's none, or it's too old to (the service opens it then).
    pub async fn open(&self, target: &str) -> bool {
        let Some(d) = self.desktop.locked().clone() else { return false };
        d.request("open", json!({ "target": target }), ASK).await.is_ok()
    }

    /// XenoNet Dev's control channel: the request, for the Desktop's window.
    pub async fn console(&self, body: Value) -> Result<Value, String> {
        let Some(d) = self.desktop.locked().clone() else {
            return Err("XenoNet Desktop is closed or restarting. The service is still running.".into());
        };
        d.request("console", body, ASK).await
    }

    pub fn close(&self) {
        for c in self.clients.locked().drain(..) {
            c.close();
        }
        self.hooks.terminals.kill_all();
        // Its descriptor, if it's still this process's.
        let mine = std::fs::read_to_string(&self.file).ok().and_then(|t| xenonet_base::json::parse(&t).ok()).and_then(|v| v["pid"].as_u64()) == Some(u64::from(std::process::id()));
        if mine {
            let _ = std::fs::remove_file(&self.file);
        }
    }

    async fn handle(self: &Arc<Self>, conn: &Arc<Conn>, method: String, params: Value) -> Result<Value, String> {
        let role = conn.role.locked().clone();
        let Some(role) = role else {
            let token = params.get("token").and_then(Value::as_str);
            if method != "hello" || !token.is_some_and(|t| same(t.as_bytes(), self.token.as_bytes())) {
                return Err("XenoNet refused this connection.".into());
            }
            negotiate(SERVICE_API, params.get("api").unwrap_or(&Value::Null))?;
            let role = params.get("role").and_then(Value::as_str).unwrap_or("");
            if role != "desktop" && role != "tools" {
                return Err("Unknown XenoNet component.".into());
            }
            if role == "desktop" && self.desktop.locked().is_some() {
                return Err("XenoNet already has a connected Desktop.".into());
            }
            *conn.role.locked() = Some(role.to_string());
            let _ = conn.hello.locked().take().map(|t| t.send(()));
            if role != "desktop" {
                return Ok(self.info());
            }
            let rpc = conn.rpc.locked().clone();
            *self.desktop.locked() = rpc;
            // The window key goes only to a Desktop the service can tell is
            // an app's own process, which the page's requests then come from.
            let seen = (self.hooks.on_desktop)(Some(conn.peer)).await;
            let mut out = self.info();
            if ok(&seen) {
                out["windowKey"] = json!(self.hooks.window_key.reveal());
            }
            return Ok(out);
        };
        match method.as_str() {
            "status" => {
                let mut out = self.info();
                out["terminals"] = self.hooks.terminals.list();
                return Ok(out);
            }
            "console" => return self.console(params).await,
            "stop" => {
                let running = self.hooks.terminals.list().as_array().is_some_and(|l| l.iter().any(|t| !xenonet_base::text::truthy(t.get("exited"))));
                if running && !xenonet_base::text::truthy(params.get("force")) {
                    return Err("XenoNet has running terminals. Close them first, or use --force.".into());
                }
                let shutdown = self.hooks.shutdown.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    shutdown();
                });
                return Ok(json!({ "stopping": true }));
            }
            _ => {}
        }
        if role != "desktop" {
            return Err("This request belongs to XenoNet Desktop.".into());
        }
        let t = &self.hooks.terminals;
        let e = |r: crate::cores::R<Value>| r.map_err(|e| e.message);
        let unit = |r: crate::cores::R<()>| r.map(|()| Value::Null).map_err(|e| e.message);
        match method.as_str() {
            "window" => {
                (self.hooks.window_state)(&params);
                Ok(Value::Null)
            }
            "terminal.profiles" => e(t.profiles()),
            "terminal.list" => Ok(t.list()),
            "terminal.create" => e(t.create(&params)),
            "terminal.read" => e(t.read(&params)),
            "terminal.write" => unit(t.write(&params)),
            "terminal.resize" => unit(t.resize(&params)),
            "terminal.kill" => unit(t.kill(params.get("id").unwrap_or(&Value::Null))),
            other => Err(format!("Unsupported service request: {other}")),
        }
    }
}

struct Conn {
    role: Mutex<Option<String>>,
    rpc: Mutex<Option<Rpc>>,
    hello: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    peer: Peer,
}

/// Writes the descriptor through a temporary file only this account can
/// read, renamed into place.
fn write_descriptor(file: &Path, port: u16, token: &str, version: &str) -> std::io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = PathBuf::from(format!("{}.{}.tmp", file.display(), std::process::id()));
    let wrote = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        drop(opts.open(&tmp)?);
        let mut run = |f: &str, a: &[&str]| xenonet_base::private_file::run_private(f, a);
        xenonet_base::private_file::protect_file(&tmp.to_string_lossy(), cfg!(windows), &mut run).map_err(|e| std::io::Error::other(e.message().to_string()))?;
        std::fs::write(&tmp, xenonet_base::json::stringify(&json!({ "port": port, "token": token, "pid": std::process::id(), "version": version }), 0))?;
        std::fs::rename(&tmp, file)
    })();
    if wrote.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    wrote
}

/// Starts the component service and writes its descriptor.
pub async fn start(file: PathBuf, hooks: Hooks) -> std::io::Result<Arc<Components>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let token = random_hex(32);
    let me = Arc::new(Components { token, port, file, desktop: Mutex::new(None), clients: Mutex::new(Vec::new()), hooks });
    write_descriptor(&me.file, port, &me.token, &me.hooks.version)?;
    let svc = me.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, remote)) = listener.accept().await else { continue };
            let local = socket.local_addr().unwrap_or(remote);
            let (hello_tx, hello_rx) = tokio::sync::oneshot::channel();
            let conn = Arc::new(Conn { role: Mutex::new(None), rpc: Mutex::new(None), hello: Mutex::new(Some(hello_tx)), peer: Peer { remote, local } });
            let (s, c) = (svc.clone(), conn.clone());
            let handler: Handler = Arc::new(move |method, params| {
                let (s, c) = (s.clone(), c.clone());
                Box::pin(async move { s.handle(&c, method, params).await })
            });
            let (rpc, _) = Rpc::new(socket, handler, &[], MAX_LINE);
            *conn.rpc.locked() = Some(rpc.clone());
            svc.clients.locked().push(rpc.clone());
            let s = svc.clone();
            tokio::spawn(async move {
                // Five seconds to say hello.
                tokio::select! {
                    _ = hello_rx => {}
                    () = tokio::time::sleep(HELLO) => rpc.close(),
                }
                rpc.closed().await;
                s.clients.locked().retain(|c| !c.is_closed());
                let was_desktop = {
                    let mut d = s.desktop.locked();
                    if d.as_ref().is_some_and(Rpc::is_closed) && conn.role.locked().as_deref() == Some("desktop") {
                        *d = None;
                        true
                    } else {
                        false
                    }
                };
                if was_desktop {
                    (s.hooks.window_state)(&json!("closed"));
                    (s.hooks.on_desktop)(None).await;
                }
            });
        }
    });
    Ok(me)
}
