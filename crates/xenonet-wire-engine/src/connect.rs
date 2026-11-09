//! Connecting your agents from the service (xenonet-adapter's connect,
//! through the adapters in place): what XenoNet adds to Claude Code's,
//! Codex's and Grok Build's own settings, shown first and taken out again.
//! The files and the agents' CLIs (claude mcp, codex mcp) are here, with
//! XenoNet's instances from the app's table; the hooks and servers run the
//! xenonet command the launcher names.

use crate::exec::Spec;
use async_trait::async_trait;
use serde_json::Value;
use xenonet_adapter::connect::{Connect as Core, Instance, Machine, Options, Problem};
use xenonet_app::instances::INSTANCES;
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::Arg;
use xenonet_service::config::Config;
use xenonet_service::cores::{Connect, CoreError, R};

/// This machine: its files, and the agents' CLIs on PATH.
struct Here;

fn problem(t: Thrown) -> Problem {
    Problem { message: t.message, code: t.code.as_deref().filter(|v| !v.is_null()).map(xenonet_base::text::js_string) }
}

fn fs(method: &str, args: &[Arg]) -> Result<Arg, Problem> {
    crate::fsx::call(method, args).map_err(problem)
}

impl Machine for Here {
    fn read_text(&mut self, file: &str) -> Result<String, Problem> {
        fs("readFileSync", &[Arg::text(file), Arg::text("utf8")]).map(|a| crate::js::text(&a))
    }
    fn write_text(&mut self, file: &str, text: &str) -> Result<(), Problem> {
        fs("writeFileSync", &[Arg::text(file), Arg::text(text)]).map(drop)
    }
    fn exists(&mut self, path: &str) -> bool {
        fs("existsSync", &[Arg::text(path)]).is_ok_and(|a| a.truthy())
    }
    fn make_dir(&mut self, dir: &str) -> Result<(), Problem> {
        fs("mkdirSync", &[Arg::text(dir), Arg::Obj(vec![("recursive".into(), Arg::Bool(true))])]).map(drop)
    }
    fn copy(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs("copyFileSync", &[Arg::text(from), Arg::text(to)]).map(drop)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs("renameSync", &[Arg::text(from), Arg::text(to)]).map(drop)
    }
    fn remove(&mut self, file: &str) -> Result<(), Problem> {
        fs("rmSync", &[Arg::text(file), Arg::Obj(vec![("force".into(), Arg::Bool(true))])]).map(drop)
    }
    /// A CLI on PATH (XenoNet's PATH is your login shell's).
    fn cli(&mut self, name: &str) -> Option<String> {
        let path = std::env::var("PATH").unwrap_or_default();
        xenonet_base::paths::on_path(name, &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut |f: &str| std::path::Path::new(f).exists())
    }
    /// A CLI's output, or what it said went wrong.
    fn run(&mut self, bin: &str, args: &[String]) -> Result<String, Problem> {
        let mut s = Spec::new(bin, args, 20000);
        s.env = vec![("ELECTRON_RUN_AS_NODE".into(), None)];
        crate::exec::run_full(&s).map(|(out, _)| out).map_err(|f| {
            let said = xenonet_base::text::js_trim(&f.stderr).to_string();
            Problem::new(if said.is_empty() { f.message } else { said })
        })
    }
}

/// The xenonet binary the hooks and servers run: XENONET_BIN, installed
/// XenoNet's launcher, which never moves, else this one.
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
            port: f64::from(config.port),
            demo: config.demo,
            bin: bin(config),
            home: config.home.clone(),
            claude_config_dir: env("CLAUDE_CONFIG_DIR"),
            codex_home: env("CODEX_HOME"),
            grok_home: env("GROK_HOME"),
            // Installed XenoNet's console operator, which Desktop registers.
            console_mcp: env("XENONET_LAUNCHER_BIN").is_some_and(|b| !b.is_empty()) || config.launcher_exec.is_some(),
            platform: crate::js::platform().into(),
            instances: INSTANCES.iter().map(|i| Instance { id: i.id.into(), name: i.name.into(), port: f64::from(i.port) }).collect(),
        };
        ConnectDriver(std::sync::Arc::new(Core::new(options)))
    }

    async fn run(&self, f: impl FnOnce(&Core, &mut Here) -> Result<Value, Problem> + Send + 'static) -> R<Value> {
        let core = self.0.clone();
        tokio::task::spawn_blocking(move || f(&core, &mut Here))
            .await
            .map_err(|e| CoreError::new(e.to_string()))?
            .map_err(|p| CoreError { code: p.code, ..CoreError::new(p.message) })
    }
}

#[async_trait]
impl Connect for ConnectDriver {
    async fn preview(&self) -> R<Value> {
        self.run(|c, k| c.preview(k)).await
    }
    async fn apply(&self, apps: Value) -> R<Value> {
        // apply(apps = every app connect goes through that's here).
        let id = |v: &Value| v.as_str().map(str::to_string);
        let apps: Option<Vec<String>> = match apps {
            Value::Null => None,
            Value::Array(a) => Some(a.iter().filter_map(id).collect()),
            other => Some(id(&other).into_iter().collect()),
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
