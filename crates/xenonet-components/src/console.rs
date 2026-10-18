//! core/components/console.js: the console an installed XenoNet offers
//! the console tools, through the service. An MCP process outlives a
//! Desktop restart, so each action finds the service again, and one that
//! was interrupted is never sent twice.
//!
//! Asks { op: "connect" } (connectService: a client and its info),
//! "request" and "close", { op: "spawn" } (Desktop, as it registered
//! itself) and { op: "fetch" } / "json" for the API.

use crate::protocol::{api_value, negotiate, CONSOLE_API};
use crate::runtime::{runtime_dir, Env, Machine};
use serde_json::{json, Value};
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{now_ms, Failure, Io};

pub fn no_desktop() -> Value {
    json!({ "instance": "installed", "name": "XenoNet", "controlled": false, "service": true, "page": { "ready": false }, "error": "XenoNet Desktop is closed or restarting." })
}

pub struct Console {
    pub env: Env,
    pub machine: Machine,
}

fn page_ready(v: &Value) -> bool {
    truthy(v.get("page").and_then(|p| p.get("ready")))
}

impl Console {
    pub fn port(&self) -> String {
        crate::runtime::port_of(&self.env)
    }

    async fn connect(&self, io: &Io) -> Result<Value, Failure> {
        io.ask(json!({ "op": "connect", "env": self.env })).await.map_err(|_| Failure::Rethrow)
    }

    async fn close(&self, io: &Io, client: &Value) -> Result<(), Failure> {
        io.ask(json!({ "op": "close", "client": client["client"] })).await.map(|_| ()).map_err(|_| Failure::Rethrow)
    }

    /// One console request, on a connection of its own.
    pub async fn call(&self, io: &Io, body: &Value) -> Result<Value, Failure> {
        let client = self.connect(io).await?;
        let agreed = negotiate(Some(&api_value(CONSOLE_API)), client.get("info").and_then(|i| i.get("consoleApi")));
        let r = match agreed {
            Err(why) => Err(Failure::msg(why)),
            Ok(_) => io.ask(json!({ "op": "request", "client": client["client"], "method": "console", "params": body })).await.map_err(|_| Failure::Rethrow),
        };
        self.close(io, &client).await?;
        r
    }

    /// Desktop and its window, or no-desktop while it's closed, or null
    /// when there's no service.
    pub async fn ping(&self, io: &Io) -> Result<Value, Failure> {
        let client = match io.ask(json!({ "op": "connect", "env": self.env })).await {
            Ok(c) => c,
            Err(e) => return Self::ping_failed(&e),
        };
        let r = if !truthy(client.get("info").and_then(|i| i.get("desktop"))) {
            Ok(no_desktop())
        } else {
            match io.ask(json!({ "op": "request", "client": client["client"], "method": "console", "params": { "op": "ping" } })).await {
                Ok(v) => Ok(v),
                Err(e) => Self::ping_failed(&e),
            }
        };
        self.close(io, &client).await?;
        r
    }

    fn ping_failed(e: &Value) -> Result<Value, Failure> {
        let message = e.get("message").and_then(Value::as_str).unwrap_or("");
        // Desktop may go between the hello's answer and this probe.
        if message.starts_with("XenoNet Desktop is closed or restarting.") {
            return Ok(no_desktop());
        }
        if e.get("code").and_then(Value::as_str) == Some("ENOENT") || message.contains("disconnect") {
            return Ok(Value::Null);
        }
        Err(Failure::Rethrow)
    }

    pub async fn ready(&self, io: &Io, timeout: f64) -> Result<Value, Failure> {
        let until = now_ms() + timeout;
        while now_ms() < until {
            // A probe the service was too busy to answer in time counts as
            // not ready yet: a ping changes nothing, so asking twice is safe.
            let state = match self.ping(io).await {
                Ok(s) => s,
                Err(Failure::Rethrow) if io.last_message().contains("did not answer") => Value::Null,
                Err(f) => return Err(f),
            };
            if page_ready(&state) {
                return Ok(state);
            }
            io.sleep(200.0).await;
        }
        Err(Failure::msg("XenoNet Desktop did not become ready."))
    }

    pub async fn quit(&self, io: &Io) -> Result<Value, Failure> {
        let state = self.ping(io).await?;
        if !truthy(state.get("page")) {
            return Ok(json!({ "quit": false }));
        }
        if state.get("controlled") == Some(&json!(false)) {
            return Ok(json!({ "quit": false, "service": true }));
        }
        self.call(io, &json!({ "op": "quit" })).await?;
        let until = now_ms() + 20000.0;
        while now_ms() < until {
            let p = self.ping(io).await?;
            if !truthy(Some(&p)) || p.get("controlled") == Some(&json!(false)) {
                return Ok(json!({ "quit": true, "service": true }));
            }
            io.sleep(100.0).await;
        }
        Err(Failure::msg("XenoNet Desktop did not close. The service is still running."))
    }

    pub async fn launch(&self, io: &Io, show: &str, restart: bool, timeout: f64) -> Result<Value, Failure> {
        if restart {
            self.quit(io).await?;
        }
        if page_ready(&self.ping(io).await?) {
            let body = if show == "hidden" { json!({ "op": "window" }) } else { json!({ "op": "window", "show": show }) };
            self.call(io, &body).await?;
            return self.ready(io, timeout).await;
        }
        let file = NATIVE.join(&[&runtime_dir(&self.env, &self.machine), "desktop.json"]);
        let desktop = xenonet_app::fsx::read_text(&file)
            .ok()
            .and_then(|t| xenonet_base::json::parse(&t).ok())
            .ok_or_else(|| Failure::msg("Open XenoNet once to register Desktop, or install the XenoNet bundle."))?;
        let mut env = self.env.clone();
        env.insert("XENONET_SHOW".into(), json!(show));
        env.shift_remove("ELECTRON_RUN_AS_NODE");
        let field = |k: &str| desktop.get(k).cloned().unwrap_or(Value::Null);
        io.ask(json!({ "op": "spawn", "command": field("command"), "args": field("args"), "cwd": field("cwd"), "env": env })).await.map_err(|_| Failure::Rethrow)?;
        self.ready(io, timeout).await
    }

    /// Any read-only API route of the installed XenoNet.
    pub async fn api(&self, io: &Io, route: &str) -> Result<Value, Failure> {
        let res = io.ask(json!({ "op": "fetch", "url": format!("http://127.0.0.1:{}{route}", self.port()) })).await.map_err(|_| Failure::Rethrow)?;
        io.ask(json!({ "op": "json", "res": res["res"] })).await.map_err(|_| Failure::Rethrow)
    }
}
