//! The console an installed XenoNet offers the console tools, through the
//! service. An MCP process outlives a Desktop restart, so each action finds
//! the service again, and one that was interrupted is never sent twice.

use crate::host::{Desktop, Spawn};
use crate::protocol::{api_value, negotiate, CONSOLE_API};
use crate::runtime::{runtime_dir, Env, Machine};
use serde_json::{json, Value};
use xenonet_os::error::Failure;
use xenonet_os::nodepath::NATIVE;
use xenonet_text::text::truthy;
use xenonet_text::time::now_ms;

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

    /// One console request, on a connection of its own.
    pub async fn call(&self, host: &dyn Desktop, body: &Value) -> Result<Value, Failure> {
        let client = host.connect(&self.env).await?;
        let agreed = negotiate(Some(&api_value(CONSOLE_API)), client.info.get("consoleApi"));
        let r = match agreed {
            Err(why) => Err(Failure::msg(why)),
            Ok(_) => host.request(client.id, "console", body.clone()).await,
        };
        host.close(client.id).await;
        r
    }

    /// Desktop and its window, or no-desktop while it's closed, or null
    /// when there's no service.
    pub async fn ping(&self, host: &dyn Desktop) -> Result<Value, Failure> {
        let client = match host.connect(&self.env).await {
            Ok(c) => c,
            Err(e) => return Self::ping_failed(e),
        };
        let r = if !truthy(client.info.get("desktop")) {
            Ok(no_desktop())
        } else {
            match host.request(client.id, "console", json!({ "op": "ping" })).await {
                Ok(v) => Ok(v),
                Err(e) => Self::ping_failed(e),
            }
        };
        host.close(client.id).await;
        r
    }

    fn ping_failed(e: Failure) -> Result<Value, Failure> {
        // Desktop may go between the hello's answer and this probe.
        if e.message.starts_with("XenoNet Desktop is closed or restarting.") {
            return Ok(no_desktop());
        }
        if e.code() == Some("ENOENT") || e.message.contains("disconnect") {
            return Ok(Value::Null);
        }
        Err(e)
    }

    pub async fn ready(&self, host: &dyn Desktop, timeout: f64) -> Result<Value, Failure> {
        let until = now_ms() + timeout;
        while now_ms() < until {
            // A probe the service was too busy to answer in time counts as
            // not ready yet: a ping changes nothing, so asking twice is safe.
            let state = match self.ping(host).await {
                Ok(s) => s,
                Err(e) if e.message.contains("did not answer") => Value::Null,
                Err(f) => return Err(f),
            };
            if page_ready(&state) {
                return Ok(state);
            }
            host.sleep(200.0).await;
        }
        Err(Failure::msg("XenoNet Desktop did not become ready."))
    }

    pub async fn quit(&self, host: &dyn Desktop) -> Result<Value, Failure> {
        let state = self.ping(host).await?;
        if !truthy(state.get("page")) {
            return Ok(json!({ "quit": false }));
        }
        if state.get("controlled") == Some(&json!(false)) {
            return Ok(json!({ "quit": false, "service": true }));
        }
        self.call(host, &json!({ "op": "quit" })).await?;
        let until = now_ms() + 20000.0;
        while now_ms() < until {
            let p = self.ping(host).await?;
            if !truthy(Some(&p)) || p.get("controlled") == Some(&json!(false)) {
                return Ok(json!({ "quit": true, "service": true }));
            }
            host.sleep(100.0).await;
        }
        Err(Failure::msg("XenoNet Desktop did not close. The service is still running."))
    }

    pub async fn launch(&self, host: &dyn Desktop, show: &str, restart: bool, timeout: f64) -> Result<Value, Failure> {
        if restart {
            self.quit(host).await?;
        }
        if page_ready(&self.ping(host).await?) {
            let body = if show == "hidden" { json!({ "op": "window" }) } else { json!({ "op": "window", "show": show }) };
            self.call(host, &body).await?;
            return self.ready(host, timeout).await;
        }
        let file = NATIVE.join(&[&runtime_dir(&self.env, &self.machine), "desktop.json"]);
        let desktop = xenonet_os::files::read_text(&file)
            .ok()
            .and_then(|t| xenonet_text::json::parse(&t).ok())
            .ok_or_else(|| Failure::msg("Open XenoNet once to register Desktop, or install the XenoNet bundle."))?;
        let mut env = self.env.clone();
        env.insert("XENONET_SHOW".into(), json!(show));
        env.shift_remove("ELECTRON_RUN_AS_NODE");
        let field = |k: &str| desktop.get(k).cloned().unwrap_or(Value::Null);
        let spawn = Spawn {
            command: xenonet_text::text::display(&field("command")),
            args: field("args").as_array().into_iter().flatten().map(xenonet_text::text::display).collect(),
            cwd: field("cwd").as_str().map(str::to_string),
            env,
            log: None,
        };
        host.spawn(&spawn).await?;
        self.ready(host, timeout).await
    }

    /// Any read-only API route of the installed XenoNet.
    pub async fn api(&self, host: &dyn Desktop, route: &str) -> Result<Value, Failure> {
        host.get_json(&format!("http://127.0.0.1:{}{route}", self.port())).await
    }
}
