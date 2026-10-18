//! Connecting your agents (core/engine/connect.js over
//! crates/xenonet-engine/src/connect.rs): what XenoNet adds to Claude
//! Code's and Codex's own settings, shown first and taken out again. The
//! files, the agents' CLIs (claude mcp, codex mcp) and the table of
//! XenoNet's instances are here; the hooks and servers run the xenonet
//! command the launcher names, as the JS service's do.

use crate::exec::Spec;
use crate::js::{at, no_method, text};
use async_trait::async_trait;
use serde_json::Value;
use xenonet_engine::connect::{Connect as Core, Instance, Options};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_service::config::Config;
use xenonet_service::cores::{Connect, CoreError, R};

/// core/instances.js's table.
const INSTANCES: [(&str, &str, f64); 6] = [
    ("dev", "XenoNet Dev", 47842.0),
    ("demo", "XenoNet Dev (demo)", 47841.0),
    ("test", "XenoNet Dev (test)", 47843.0),
    ("film", "XenoNet Dev (film)", 47844.0),
    ("installed", "XenoNet", 47840.0),
    ("showcase", "XenoNet Dev (showcase)", 47845.0),
];

struct ConnectCalls;

impl Calls for ConnectCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("fs", m) => crate::fsx::call(m, &args),
            // A CLI's output, or what it said went wrong.
            ("", "run") => {
                let mut s = Spec::new(&text(&at(&args, 0)), &crate::js::list(at(&args, 1)).iter().map(text).collect::<Vec<_>>(), 20000);
                s.env = vec![("ELECTRON_RUN_AS_NODE".into(), None)];
                crate::exec::run_full(&s).map(|(out, _)| Arg::Text(out)).map_err(|f| {
                    let said = xenonet_base::text::js_trim(&f.stderr).to_string();
                    Thrown::new(if said.is_empty() { f.message } else { said })
                })
            }
            // A CLI on PATH (XenoNet's PATH is your login shell's).
            ("", "cli") => {
                let path = std::env::var("PATH").unwrap_or_default();
                let found = xenonet_base::paths::on_path(&text(&at(&args, 0)), &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut |f: &str| {
                    std::path::Path::new(f).exists()
                });
                Ok(found.map_or(Arg::Null, Arg::Text))
            }
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, _: &str) -> Result<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(if path == "fs" { "object" } else { "function" }.into())
    }
}

/// The runtime the hooks and servers run the xenonet command on: the
/// launcher's, or the app's own, else the Node on PATH.
fn runtime(config: &Config) -> (String, bool) {
    if let Some(e) = &config.launcher_exec {
        return (e.clone(), false);
    }
    if let Some(e) = &config.exec {
        return (e.clone(), true);
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let node = xenonet_base::paths::on_path("node", &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut |f: &str| xenonet_base::paths::can_run(f));
    (node.unwrap_or_else(|| "node".into()), false)
}

pub struct ConnectDriver(std::sync::Arc<Core>);

impl ConnectDriver {
    pub fn new(config: &Config) -> ConnectDriver {
        let (exec, run_as_node) = runtime(config);
        let env = |k: &str| std::env::var(k).ok();
        let options = Options {
            port: Arg::Num(f64::from(config.port)),
            root: config.launcher_root.clone().unwrap_or_else(|| config.root.to_string_lossy().into_owned()),
            demo: Arg::Bool(config.demo),
            exec,
            run_as_node,
            home: config.home.clone(),
            claude_config_dir: env("CLAUDE_CONFIG_DIR"),
            codex_home: env("CODEX_HOME"),
            console_mcp: config.launcher_exec.is_some(),
            platform: crate::js::platform().into(),
            instances: INSTANCES.iter().map(|(id, name, port)| Instance { id: (*id).into(), name: (*name).into(), port: Arg::Num(*port) }).collect(),
        };
        ConnectDriver(std::sync::Arc::new(Core::new(options)))
    }

    async fn run(&self, f: impl FnOnce(&Core, &mut ConnectCalls) -> Result<Value> + Send + 'static) -> R<Value> {
        let core = self.0.clone();
        tokio::task::spawn_blocking(move || f(&core, &mut ConnectCalls))
            .await
            .map_err(|e| CoreError::new(e.to_string()))?
            .map_err(|t| CoreError { code: t.code.as_deref().and_then(Value::as_str).map(str::to_string), ..CoreError::new(t.message) })
    }
}

#[async_trait]
impl Connect for ConnectDriver {
    async fn preview(&self) -> R<Value> {
        self.run(|c, k| c.preview(k)).await
    }
    async fn apply(&self, apps: Value) -> R<Value> {
        // apply(apps = ["claude", "codex"]).
        let apps: Vec<Arg> = match apps {
            Value::Null => vec![Arg::text("claude"), Arg::text("codex")],
            Value::Array(a) => a.iter().map(crate::js::arg).collect(),
            other => vec![crate::js::arg(&other)],
        };
        self.run(move |c, k| c.apply(k, &apps)).await
    }
    async fn remove(&self) -> R<Value> {
        self.run(|c, k| c.remove(k)).await
    }
}
