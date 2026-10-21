//! Connecting your agents (core/engine/connect.js over
//! crates/xenonet-engine/src/connect.rs): what XenoNet adds to Claude
//! Code's and Codex's own settings, shown first and taken out again. The
//! files and the agents' CLIs (claude mcp, codex mcp) are here, with
//! XenoNet's instances from the app's table; the hooks and servers run the
//! xenonet command the launcher names, as the JS service's do.

use crate::exec::Spec;
use crate::js::{at, no_method, text};
use async_trait::async_trait;
use serde_json::Value;
use xenonet_app::instances::INSTANCES;
use xenonet_engine::connect::{Connect as Core, Instance, Options};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_service::config::Config;
use xenonet_service::cores::{Connect, CoreError, R};

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

/// The xenonet binary the hooks and servers run (runtime.js's xenonetBin):
/// XENONET_BIN, installed XenoNet's launcher, which never moves, else this
/// one.
fn bin(config: &Config) -> String {
    let env: xenonet_components::runtime::Env = std::env::vars().map(|(k, v)| (k, Value::String(v))).collect();
    let m = xenonet_components::runtime::Machine { platform: crate::js::platform().into(), arch: std::env::consts::ARCH.into(), home: config.home.clone() };
    let this = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    xenonet_components::runtime::xenonet_bin(&env, &m, &this)
}

pub struct ConnectDriver(std::sync::Arc<Core>);

impl ConnectDriver {
    pub fn new(config: &Config) -> ConnectDriver {
        let env = |k: &str| std::env::var(k).ok();
        let options = Options {
            port: Arg::Num(f64::from(config.port)),
            demo: Arg::Bool(config.demo),
            bin: bin(config),
            home: config.home.clone(),
            claude_config_dir: env("CLAUDE_CONFIG_DIR"),
            codex_home: env("CODEX_HOME"),
            grok_home: env("GROK_HOME"),
            // Installed XenoNet's console operator, which Desktop registers.
            console_mcp: env("XENONET_LAUNCHER_BIN").is_some_and(|b| !b.is_empty()) || config.launcher_exec.is_some(),
            platform: crate::js::platform().into(),
            instances: INSTANCES.iter().map(|i| Instance { id: i.id.into(), name: i.name.into(), port: Arg::Num(f64::from(i.port)) }).collect(),
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
        // apply(apps = every app connect goes through that's here).
        let apps: Option<Vec<Arg>> = match apps {
            Value::Null => None,
            Value::Array(a) => Some(a.iter().map(crate::js::arg).collect()),
            other => Some(vec![crate::js::arg(&other)]),
        };
        self.run(move |c, k| {
            let apps = match apps {
                Some(a) => a,
                None => c.installed_ids(k)?,
            };
            c.apply(k, &apps)
        })
        .await
    }
    async fn remove(&self) -> R<Value> {
        self.run(|c, k| c.remove(k)).await
    }
    async fn hook_connected(&self, event: &str) -> R<bool> {
        let event = event.to_string();
        Ok(self.run(move |c, k| c.hook_connected(k, &event).map(Value::Bool)).await?.as_bool().unwrap_or(false))
    }
}
