//! The component service (server/component-service.js): a socket on
//! 127.0.0.1 that XenoNet Desktop and the Tools connect to, found through
//! `runtime/service-<port>.json`, which holds its port and a token new at each
//! start. JSON lines (rpc.rs): a hello carrying the token, then status,
//! console (relayed to the Desktop), stop, yours (whether the connection
//! comes from you, for the command), and the Desktop's own requests
//! (where its window is, the terminals). The service sends the Desktop
//! `notification` events, and asks it to open things, to show its window
//! (`show`) and, for XenoNet Dev's control channel, to act on its window
//! (`console`).
//!
//! XenoNet's tray app connects as `tray`. Its hello's answer carries the
//! tray's key, for the tray's routes, only when verify.tray can tell it's
//! XenoNet Tray.app's own process; nothing is written down for it. One tray
//! at a time, and the Desktop hears a `tray` event as it comes and goes, so
//! a Mac's Desktop hides its own tray while the tray app runs.
//!
//! What Desktop got over a utility process's port when it started the JS
//! service itself (the window key, ready, open, notify, the control relay)
//! is all here: the hello's answer carries the window key, and the rest
//! are requests and events on this socket (docs/design/rust-core.md).

use crate::cores::{ok, Terminals};
use crate::http::Peer;
use crate::routes::{api_range, CAPABILITIES, CONSOLE_API, SERVICE_API};
use crate::rpc::{negotiate, Handler, Rpc, MAX_LINE};
use crate::window_key::{random_hex, same, TrayKey, WindowKey};
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
    /// Whether a connection comes from you, as the routes that are yours
    /// to do ask (verify.person): from XenoNet's window or your own
    /// terminal, not from under an agent.
    pub on_person: Arc<dyn Fn(Peer) -> Seen + Send + Sync>,
    /// Whether the tray that connected is XenoNet's tray app (verify.tray).
    pub on_tray: Arc<dyn Fn(Peer) -> Seen + Send + Sync>,
    pub tray_key: Arc<TrayKey>,
    pub terminals: Arc<dyn Terminals>,
}

pub struct Components {
    token: String,
    port: u16,
    file: PathBuf,
    desktop: Mutex<Option<Rpc>>,
    tray: Mutex<Option<Rpc>>,
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
            "tray": self.tray_connected(),
        })
    }

    /// Whether XenoNet's tray app is connected.
    pub fn tray_connected(&self) -> bool {
        self.tray.locked().as_ref().is_some_and(|t| !t.is_closed())
    }

    /// Tells the connected Desktop the tray came or went; one from before
    /// the tray ignores it.
    fn tell_tray(&self, connected: bool) {
        if let Some(d) = self.desktop.locked().clone() {
            let _ = d.notify("tray", json!({ "connected": connected }));
        }
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

    /// Has the connected Desktop show its window, at a session or an actor
    /// when `select` names one; false when there's none, or it's too old to.
    pub async fn show(&self, select: Option<&str>) -> bool {
        let Some(d) = self.desktop.locked().clone() else { return false };
        d.request("show", json!({ "select": select }), ASK).await.is_ok()
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
            if role != "desktop" && role != "tools" && role != "tray" {
                return Err("Unknown XenoNet component.".into());
            }
            if role == "tray" {
                return self.tray_hello(conn).await;
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
            // What the command does itself and is yours to do, such as
            // starting the service at login, asks here first.
            "yours" => return Ok((self.hooks.on_person)(conn.peer).await),
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

impl Components {
    /// The tray's hello: its key, only for XenoNet's tray app, and one tray
    /// at a time. A tray that isn't XenoNet's gets no role, so the
    /// connection ends at the hello's deadline.
    async fn tray_hello(self: &Arc<Self>, conn: &Arc<Conn>) -> Result<Value, String> {
        let seen = (self.hooks.on_tray)(conn.peer).await;
        if !ok(&seen) {
            return Err(format!("XenoNet didn't take this as its tray: {}", crate::cores::reason(&seen)));
        }
        {
            let mut tray = self.tray.locked();
            if tray.as_ref().is_some_and(|t| !t.is_closed()) {
                return Err("XenoNet already has a connected tray.".into());
            }
            tray.clone_from(&conn.rpc.locked());
        }
        *conn.role.locked() = Some("tray".into());
        let _ = conn.hello.locked().take().map(|t| t.send(()));
        self.tell_tray(true);
        let mut out = self.info();
        out["trayKey"] = json!(self.hooks.tray_key.reveal());
        Ok(out)
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
    let me = Arc::new(Components { token, port, file, desktop: Mutex::new(None), tray: Mutex::new(None), clients: Mutex::new(Vec::new()), hooks });
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
                let was_tray = conn.role.locked().as_deref() == Some("tray") && {
                    let mut t = s.tray.locked();
                    let gone = t.as_ref().is_some_and(Rpc::is_closed);
                    if gone {
                        *t = None;
                    }
                    gone
                };
                if was_tray {
                    s.tell_tray(false);
                }
            });
        }
    });
    Ok(me)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    /// The component service with every check answered by `tray_ok`
    /// (the tray's) or passed (the Desktop's), its descriptor in a
    /// temporary folder.
    async fn service(name: &str, tray_ok: bool) -> (Arc<Components>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("xenonet-component-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pass = |v: bool| -> Arc<dyn Fn(Peer) -> Seen + Send + Sync> {
            Arc::new(move |_| {
                Box::pin(async move {
                    if v {
                        json!({ "ok": true })
                    } else {
                        json!({ "ok": false, "reason": "it came from under a Claude Code process" })
                    }
                })
            })
        };
        let hooks = Hooks {
            version: "0.0.0".into(),
            window_key: Arc::new(WindowKey::new()),
            window_state: Arc::new(|_| {}),
            shutdown: Arc::new(|| {}),
            on_desktop: Arc::new(|_| Box::pin(async { json!({ "ok": true }) })),
            on_person: pass(true),
            on_tray: pass(tray_ok),
            tray_key: Arc::new(TrayKey::new()),
            terminals: Arc::new(crate::cores::Pending),
        };
        let c = start(dir.join("service-1.json"), hooks).await.unwrap();
        (c, dir)
    }

    struct Client {
        lines: tokio::io::Lines<BufReader<tokio::net::tcp::OwnedReadHalf>>,
        write: tokio::net::tcp::OwnedWriteHalf,
    }

    impl Client {
        async fn hello(c: &Components, role: &str) -> (Client, Value) {
            let (read, write) = TcpStream::connect(("127.0.0.1", c.port())).await.unwrap().into_split();
            let mut me = Client { lines: BufReader::new(read).lines(), write };
            let answer = me.ask(1, "hello", json!({ "token": c.token, "role": role, "api": { "min": 1, "max": 1 } })).await;
            (me, answer)
        }
        async fn ask(&mut self, id: i64, method: &str, params: Value) -> Value {
            self.write.write_all(format!("{}\n", json!({ "id": id, "method": method, "params": params })).as_bytes()).await.unwrap();
            self.next().await
        }
        async fn next(&mut self) -> Value {
            let line = tokio::time::timeout(Duration::from_secs(5), self.lines.next_line()).await.unwrap().unwrap().unwrap();
            xenonet_base::json::parse(&line).unwrap()
        }
    }

    #[tokio::test]
    async fn the_tray_gets_its_key_only_when_its_xenonets_and_desktop_hears_it_come_and_go() {
        let (c, dir) = service("tray", true).await;
        let (mut desktop, d) = Client::hello(&c, "desktop").await;
        assert_eq!(d["result"]["tray"], json!(false));
        assert!(d["result"].get("trayKey").is_none(), "the Desktop never gets the tray's key");
        let (tray, t) = Client::hello(&c, "tray").await;
        assert_eq!(t["result"]["trayKey"], json!(c.hooks.tray_key.reveal()));
        assert_eq!(t["result"]["tray"], json!(true));
        assert!(t["result"].get("windowKey").is_none(), "the tray never gets the window's key");
        assert_eq!(desktop.next().await, json!({ "event": "tray", "value": { "connected": true } }));
        // One tray at a time.
        let (_, again) = Client::hello(&c, "tray").await;
        assert_eq!(again["error"], json!("XenoNet already has a connected tray."));
        assert!(c.tray_connected());
        // The tool's status says so too.
        let (mut tools, _) = Client::hello(&c, "tools").await;
        assert_eq!(tools.ask(2, "status", json!({})).await["result"]["tray"], json!(true));
        drop(tray);
        assert_eq!(desktop.next().await, json!({ "event": "tray", "value": { "connected": false } }));
        assert!(!c.tray_connected());
        c.close();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_tray_that_isnt_xenonets_gets_no_key_and_no_standing() {
        let (c, dir) = service("not-tray", false).await;
        let (mut desktop, _) = Client::hello(&c, "desktop").await;
        let (_, t) = Client::hello(&c, "tray").await;
        assert_eq!(t["error"], json!("XenoNet didn't take this as its tray: it came from under a Claude Code process"));
        assert!(!c.tray_connected());
        // The Desktop hears nothing of it, and a request made then answers.
        assert_eq!(desktop.ask(2, "status", json!({})).await["result"]["tray"], json!(false));
        c.close();
        let _ = std::fs::remove_dir_all(dir);
    }
}
