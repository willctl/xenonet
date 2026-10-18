//! The private endpoint remote actors' adapters reach XenoNet through
//! (server/assistant-endpoint.js; docs/design/cloud-assistants.md, section
//! 4): `xenonet mcp assistant`, which a tunnel client runs, connects here on
//! 127.0.0.1, says which grant it's for with that grant's credential, and
//! then may ask one thing, assistant.read. It's apart from the component
//! service, whose credential the adapter never has, and from the HTTP API.
//!
//!   hello          {role: "assistant", api, grant_id, token}, and nothing
//!                  else, within five seconds of connecting
//!   assistant.read {request_id, operation, arguments}, once let in
//!
//! Anything else is refused, with nothing more said than that. A grant
//! revoked, expired, given a new credential or turned off lets go of its
//! adapters at once; the tombstones written without the service are looked
//! at every two seconds too. It listens only while a grant could be used,
//! and says where in runtime/assistant-<port>.json, which holds no secret.

use crate::buckets::{now_ms, Bucket, Rate};
use crate::cores::{AssistantService, Audit, Connections};
use crate::rpc::{negotiate, Handler, Rpc};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use xenonet_base::sync::Locked;

const HELLO: Duration = Duration::from_secs(5);
const SWEEP: Duration = Duration::from_secs(2);
pub const MAX_LINE: usize = 320 << 10;
pub const ASSISTANT_API: (u32, u32) = (1, 1);
const HELLO_KEYS: [&str; 4] = ["api", "grant_id", "role", "token"];
const ERROR_CODES: [&str; 13] = [
    "NOT_AUTHORIZED",
    "NOT_FOUND",
    "GRANT_EXPIRED",
    "GRANT_REVOKED",
    "DISABLED",
    "INVALID_ARGUMENT",
    "CURSOR_EXPIRED",
    "SOURCE_UNAVAILABLE",
    "SERVICE_UNAVAILABLE",
    "RATE_LIMITED",
    "INCOMPATIBLE_VERSION",
    "TIMEOUT",
    "INTERNAL_ERROR",
];
// Refused hellos, from anyone: 20 a minute, then none for a minute.
const REFUSALS: Rate = Rate { rate: 20.0 / 60.0, burst: 20.0 };

fn grant_id(v: &Value) -> bool {
    v.as_str().is_some_and(crate::routes::assistant::grant_id)
}

struct Session {
    client: Rpc,
    grant: String,
    actor: Value,
    connection: String,
}

struct Inner {
    connections: Arc<dyn Connections>,
    service: Arc<dyn AssistantService>,
    audit: Option<Arc<dyn Audit>>,
    file: PathBuf,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    server: Mutex<Option<(u16, tokio::task::JoinHandle<()>)>>,
    sessions: Mutex<Vec<Arc<Session>>>,
    refusals: Mutex<Bucket>,
    quiet_until: Mutex<f64>,
}

#[derive(Clone)]
pub struct Endpoint {
    inner: Arc<Inner>,
}

impl Endpoint {
    pub fn new(
        connections: Arc<dyn Connections>,
        service: Arc<dyn AssistantService>,
        audit: Option<Arc<dyn Audit>>,
        file: PathBuf,
        log: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Endpoint {
        let e = Endpoint {
            inner: Arc::new(Inner {
                connections,
                service,
                audit,
                file,
                log,
                server: Mutex::new(None),
                sessions: Mutex::new(Vec::new()),
                refusals: Mutex::new(Bucket::new(REFUSALS, now_ms())),
                quiet_until: Mutex::new(0.0),
            }),
        };
        // A grant revoked, rotated, expired or turned off lets go of its
        // adapters; a change in the grants may start or stop listening.
        if let Some(mut events) = e.inner.connections.subscribe() {
            let me = e.clone();
            tokio::spawn(async move {
                while let Some((event, data)) = events.recv().await {
                    let grant = data["grant"].as_str().unwrap_or("").to_string();
                    match event.as_str() {
                        "revoked" | "expired" => {
                            me.off(&grant);
                            me.sync().await;
                        }
                        "rotated" => me.off(&grant),
                        "disabled" => me.stop_listening(),
                        "enabled" | "changed" => me.sync().await,
                        _ => {}
                    }
                }
            });
        }
        let me = e.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(SWEEP);
            loop {
                every.tick().await;
                me.sweep().await;
            }
        });
        e
    }

    fn drop_where(&self, matches: impl Fn(&Session) -> bool) {
        for s in self.inner.sessions.locked().iter().filter(|s| matches(s)) {
            s.client.close();
        }
    }

    fn off(&self, grant: &str) {
        self.drop_where(|s| s.grant == grant);
        self.inner.service.forget(grant);
    }

    fn hello(&self, client: &Rpc, params: &Value) -> Result<(Value, Arc<Session>), String> {
        if now_ms() < *self.inner.quiet_until.locked() {
            return Err("RATE_LIMITED".into());
        }
        let refused = |code: &str, reason: &str| -> String {
            if let Some(a) = &self.inner.audit {
                a.access(json!({ "grant": if grant_id(&params["grant_id"]) { params["grant_id"].clone() } else { Value::Null }, "operation": "hello", "result": reason }));
            }
            if self.inner.refusals.locked().take(1.0, now_ms()) != 0.0 {
                *self.inner.quiet_until.locked() = now_ms() + 60e3;
            }
            code.to_string()
        };
        let keys: Option<Vec<&str>> = params.as_object().map(|o| {
            let mut k: Vec<&str> = o.keys().map(String::as_str).collect();
            k.sort_unstable();
            k
        });
        if keys.as_deref() != Some(&HELLO_KEYS[..]) || params["role"] != "assistant" {
            return Err(refused("NOT_AUTHORIZED", "bad-hello"));
        }
        if negotiate(ASSISTANT_API, &params["api"]).is_err() {
            return Err(refused("INCOMPATIBLE_VERSION", "version"));
        }
        let s = self.inner.connections.authenticate(&params["grant_id"], &params["token"]);
        if s["ok"] != json!(true) {
            let code = s["code"].as_str().filter(|c| ERROR_CODES.contains(c)).unwrap_or("NOT_AUTHORIZED");
            return Err(refused(code, &xenonet_base::text::js_string(&s["reason"])));
        }
        let grant = s["grant"]["grant_id"].as_str().unwrap_or("").to_string();
        let connection = s["connection"]["connection_id"].as_str().unwrap_or("").to_string();
        let session = Arc::new(Session { client: client.clone(), grant: grant.clone(), actor: s["actorId"].clone(), connection: connection.clone() });
        self.inner.sessions.locked().push(session.clone());
        self.inner.connections.bridge(&connection, 1);
        if let Some(a) = &self.inner.audit {
            a.access(json!({ "grant": grant, "revision": s["revision"], "actor": s["actorId"], "connection": connection, "operation": "hello", "result": "ok" }));
        }
        let p = self.inner.connections.policy(&s).map_err(|e| e.message)?;
        Ok((
            json!({ "assistant_api": 1, "device_id": p["deviceId"], "actor_id": s["actorId"], "connection_id": connection, "grant_revision": s["revision"], "permissions": p["permissions"] }),
            session,
        ))
    }

    fn accept(&self, socket: tokio::net::TcpStream) {
        let me = self.clone();
        let session: Arc<Mutex<Option<Arc<Session>>>> = Arc::new(Mutex::new(None));
        let client: Arc<Mutex<Option<Rpc>>> = Arc::new(Mutex::new(None));
        let (s, c) = (session.clone(), client.clone());
        let handler: Handler = Arc::new(move |method, params| {
            let (me, s, c) = (me.clone(), s.clone(), c.clone());
            Box::pin(async move {
                let current = s.locked().clone();
                if let Some(session) = current {
                    if method != "assistant.read" {
                        return Err("Unsupported request.".into());
                    }
                    let who = json!({ "grantId": session.grant, "actorId": session.actor, "connectionId": session.connection });
                    return me.inner.service.read(&who, &params).await.map_err(|e| e.message);
                }
                if method != "hello" {
                    return Err("XenoNet refused this connection.".into());
                }
                let Some(client) = c.locked().clone() else { return Err("XenoNet refused this connection.".into()) };
                let (answer, session) = me.hello(&client, &params)?;
                *s.locked() = Some(session);
                Ok(answer)
            })
        });
        let (rpc, _) = Rpc::new(socket, handler, &[], MAX_LINE);
        *client.locked() = Some(rpc.clone());
        let me = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(HELLO) => {
                    if session.locked().is_none() {
                        rpc.close();
                    }
                }
                () = rpc.closed() => {}
            }
            rpc.closed().await;
            let gone = session.locked().take();
            if let Some(gone) = gone {
                let mut all = me.inner.sessions.locked();
                let before = all.len();
                all.retain(|x| !Arc::ptr_eq(x, &gone));
                if all.len() < before {
                    me.inner.connections.bridge(&gone.connection, -1);
                }
            }
        });
    }

    fn write_descriptor(&self, port: u16) -> std::io::Result<()> {
        let file = &self.inner.file;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
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
            let doc = json!({ "port": port, "pid": std::process::id(), "api": { "min": ASSISTANT_API.0, "max": ASSISTANT_API.1 }, "state": self.inner.connections.dir() });
            std::fs::write(&tmp, xenonet_base::json::stringify(&doc, 0))?;
            std::fs::rename(&tmp, file)
        })();
        if wrote.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        wrote
    }

    fn remove_descriptor(&self) {
        let mine =
            std::fs::read_to_string(&self.inner.file).ok().and_then(|t| xenonet_base::json::parse(&t).ok()).and_then(|v| v["pid"].as_u64()) == Some(u64::from(std::process::id()));
        if mine {
            let _ = std::fs::remove_file(&self.inner.file);
        }
    }

    async fn start(&self) -> std::io::Result<()> {
        if self.inner.server.locked().is_some() {
            return Ok(());
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        self.write_descriptor(port)?;
        let me = self.clone();
        let task = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    me.accept(socket);
                }
            }
        });
        *self.inner.server.locked() = Some((port, task));
        (self.inner.log)(&format!("remote actors' endpoint on 127.0.0.1:{port}"));
        Ok(())
    }

    fn stop_listening(&self) {
        self.drop_where(|_| true);
        if let Some((_, task)) = self.inner.server.locked().take() {
            task.abort();
            self.remove_descriptor();
        }
    }

    /// Listening while a grant could be used, and not otherwise.
    pub async fn sync(&self) {
        if self.inner.connections.any_active() {
            if let Err(e) = self.start().await {
                (self.inner.log)(&format!("remote actors' endpoint didn't start: {e}"));
            }
        } else {
            self.stop_listening();
        }
    }

    // Sessions whose grant no longer stands, let go; the tombstones written
    // without the service count.
    async fn sweep(&self) {
        let stale: Vec<Arc<Session>> = self.inner.sessions.locked().iter().filter(|s| self.inner.connections.recheck(&s.grant)["ok"] != json!(true)).cloned().collect();
        for s in stale {
            s.client.close();
        }
        self.sync().await;
    }

    pub fn stop(&self) {
        self.stop_listening();
    }

    /// How many adapters are connected, and where it listens.
    pub fn info(&self) -> Value {
        let server = self.inner.server.locked();
        json!({ "listening": server.is_some(), "port": server.as_ref().map(|(p, _)| *p), "sessions": self.inner.sessions.locked().len() })
    }
}
