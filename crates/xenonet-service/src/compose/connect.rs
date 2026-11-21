//! Connecting your agents from the service (xenonet-adapter's connect,
//! through the adapters in place): what XenoNet adds to Claude Code's,
//! Codex's and Grok Build's own settings, shown first and taken out again.
//! The files and the agents' CLIs (claude mcp, codex mcp) are here, with
//! XenoNet's instances from the app's table; the hooks and servers run the
//! xenonet command the launcher names. It sits in the composition, since it
//! needs XenoNet's install (xenonet-components) beside the runtime's files
//! and processes.

use crate::compose::runtime::exec::{self, Spec};
use crate::compose::runtime::fs;
use crate::compose::runtime::Setup;
use async_trait::async_trait;
use serde_json::Value;
use xenonet_adapter::connect::{Connect as Core, Instance, Machine, Options, Problem};
use xenonet_api::cores::{Connect, CoreError, R};
use xenonet_api::instances::INSTANCES;
use xenonet_os::fs::FsError;

/// This machine: its files, and the agents' CLIs on PATH.
struct Here;

fn problem(e: FsError) -> Problem {
    Problem { message: e.message, code: e.code }
}

impl Machine for Here {
    fn read_text(&mut self, file: &str) -> Result<String, Problem> {
        fs::read_text(file).map_err(problem)
    }
    fn write_text(&mut self, file: &str, text: &str) -> Result<(), Problem> {
        fs::write_file(file, text.as_bytes(), None).map_err(problem)
    }
    fn exists(&mut self, path: &str) -> bool {
        fs::exists(path)
    }
    fn make_dir(&mut self, dir: &str) -> Result<(), Problem> {
        fs::mkdir_all(dir).map_err(problem)
    }
    fn copy(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs::copy(from, to).map_err(problem)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs::rename(from, to).map_err(problem)
    }
    fn remove(&mut self, file: &str) -> Result<(), Problem> {
        fs::remove_if_there(file).map_err(problem)
    }
    /// A CLI on PATH (XenoNet's PATH is your login shell's).
    fn cli(&mut self, name: &str) -> Option<String> {
        let path = std::env::var("PATH").unwrap_or_default();
        xenonet_os::paths::on_path(name, &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut |f: &str| std::path::Path::new(f).exists())
    }
    /// A CLI's output, or what it said went wrong.
    fn run(&mut self, bin: &str, args: &[String]) -> Result<String, Problem> {
        let mut s = Spec::new(bin, args, 20000);
        s.env = vec![("ELECTRON_RUN_AS_NODE".into(), None)];
        exec::run_full(&s).map(|(out, _)| out).map_err(|f| {
            let said = xenonet_text::text::trim(&f.stderr).to_string();
            Problem::new(if said.is_empty() { f.message } else { said })
        })
    }
}

/// This system as Node's process.platform names it.
fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// The xenonet binary the hooks and servers run: XENONET_BIN, installed
/// XenoNet's launcher, which never moves, else this one.
pub(crate) fn bin(setup: &Setup) -> String {
    let env: xenonet_components::runtime::Env = std::env::vars().map(|(k, v)| (k, Value::String(v))).collect();
    let m = xenonet_components::runtime::Machine { platform: platform().into(), arch: std::env::consts::ARCH.into(), home: setup.home.clone() };
    let this = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    xenonet_components::runtime::xenonet_bin(&env, &m, &this)
}

/// What is connected: the apps' own folders (their default roots), and
/// each folder she named for an app (`xenonet folders`), which has hooks of
/// its own.
pub struct ConnectDriver {
    options: Options,
    core: std::sync::Arc<Core>,
    /// Each named root's app, and the connect for that root.
    roots: Vec<(String, String, std::sync::Arc<Core>)>,
}

/// The options with a root of `app` named: the variable the app's folder is
/// in, as its registry entry says, is that root's.
fn in_root(mut o: Options, app: &str, root: &str) -> Option<Options> {
    match xenonet_adapter::apps::app(app)?.roots.as_ref()?.env.as_str() {
        "CLAUDE_CONFIG_DIR" => o.claude_config_dir = Some(root.to_string()),
        "CODEX_HOME" => o.codex_home = Some(root.to_string()),
        "GROK_HOME" => o.grok_home = Some(root.to_string()),
        _ => return None,
    }
    Some(o)
}

impl ConnectDriver {
    pub fn new(setup: &Setup) -> ConnectDriver {
        let env = |k: &str| std::env::var(k).ok();
        let options = Options {
            port: f64::from(setup.port),
            demo: setup.demo,
            bin: bin(setup),
            home: setup.home.clone(),
            claude_config_dir: env("CLAUDE_CONFIG_DIR"),
            codex_home: env("CODEX_HOME"),
            grok_home: env("GROK_HOME"),
            // Installed XenoNet's console operator, which Desktop registers.
            console_mcp: env("XENONET_LAUNCHER_BIN").is_some_and(|b| !b.is_empty()) || setup.launcher_exec.is_some(),
            platform: platform().into(),
            instances: INSTANCES.iter().map(|i| Instance { id: i.id.into(), name: i.name.into(), port: f64::from(i.port) }).collect(),
        };
        // The folders she named, each connected as the app's own is.
        let named = setup.db.as_ref().and_then(|d| d.setting("apps.roots").ok()).unwrap_or_default();
        let roots = named
            .as_object()
            .into_iter()
            .flatten()
            .flat_map(|(app, roots)| roots.as_array().into_iter().flatten().filter_map(|r| Some((app.clone(), r.as_str()?.to_string()))))
            .filter_map(|(app, root)| Some((app.clone(), root.clone(), std::sync::Arc::new(Core::new(in_root(options.clone(), &app, &root)?)))))
            .collect();
        ConnectDriver { core: std::sync::Arc::new(Core::new(options.clone())), options, roots }
    }

    async fn run(&self, f: impl FnOnce(&Core, &mut Here) -> Result<Value, Problem> + Send + 'static) -> R<Value> {
        let core = self.core.clone();
        run_on(move || f(&core, &mut Here)).await
    }
}

async fn run_on(f: impl FnOnce() -> Result<Value, Problem> + Send + 'static) -> R<Value> {
    tokio::task::spawn_blocking(f).await.map_err(|e| CoreError::new(e.to_string()))?.map_err(|p| CoreError { code: p.code, ..CoreError::new(p.message) })
}

#[async_trait]
impl Connect for ConnectDriver {
    async fn preview(&self) -> R<Value> {
        let mut out = self.run(|c, k| c.preview(k)).await?;
        let named = self.named_previews().await?;
        if !named.is_empty() {
            out["roots"] = Value::Array(named);
        }
        Ok(out)
    }
    async fn apply(&self, apps: Value) -> R<Value> {
        // apply(apps = every app connect goes through that's here).
        let id = |v: &Value| v.as_str().map(str::to_string);
        let apps: Option<Vec<String>> = match apps {
            Value::Null => None,
            Value::Array(a) => Some(a.iter().filter_map(id).collect()),
            other => Some(id(&other).into_iter().collect()),
        };
        let wanted = apps.clone();
        let mut out = self
            .run(move |c, k| {
                let apps = match apps {
                    Some(a) => a,
                    None => c.installed_ids(k)?,
                };
                c.apply(k, &apps)
            })
            .await?;
        // Each folder she named for an app gets the app's hooks too, when
        // the app is one connect goes through.
        let here = self.run(|c, k| c.installed_ids(k).map(|ids| serde_json::json!(ids))).await?;
        let here: Vec<String> = here.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        for (app, _, core) in &self.roots {
            if wanted.as_ref().map_or(here.contains(app), |w| w.contains(app)) {
                let (core, app) = (core.clone(), app.clone());
                run_on(move || core.apply(&mut Here, &[app])).await?;
            }
        }
        let named = self.named_previews().await?;
        if !named.is_empty() {
            out["roots"] = Value::Array(named);
        }
        Ok(out)
    }
    async fn remove(&self) -> R<Value> {
        let mut out = self.run(|c, k| c.remove(k)).await?;
        for (_, _, core) in &self.roots {
            let core = core.clone();
            run_on(move || core.remove(&mut Here)).await?;
        }
        let named = self.named_previews().await?;
        if !named.is_empty() {
            out["roots"] = Value::Array(named);
        }
        Ok(out)
    }
    async fn remove_root_hooks(&self, app: &str, root: &str) -> R<Value> {
        let Some(options) = in_root(self.options.clone(), app, root) else {
            return Err(CoreError::new(format!("{app} keeps its hooks in one folder, so there is none to take them out of")));
        };
        let (core, name) = (Core::new(options), app.to_string());
        run_on(move || {
            core.remove_hooks_of(&mut Here, &name)?;
            Ok(serde_json::json!({ "app": name, "hooks": "removed" }))
        })
        .await
        .map(|mut v| {
            v["root"] = Value::String(root.to_string());
            v
        })
    }
    async fn hook_connected(&self, event: &str) -> R<bool> {
        let event = event.to_string();
        Ok(self.run(move |c, k| c.hook_connected(k, &event).map(Value::Bool)).await?.as_bool().unwrap_or(false))
    }
}

impl ConnectDriver {
    /// What each folder she named has connected, as the app's entry says:
    /// `{ app, root, entry }`.
    async fn named_previews(&self) -> R<Vec<Value>> {
        let mut out = Vec::new();
        for (app, root, core) in &self.roots {
            let core = core.clone();
            let preview = run_on(move || core.preview(&mut Here)).await?;
            let entry = preview.get("apps").and_then(Value::as_array).and_then(|a| a.iter().find(|e| e.get("id").and_then(Value::as_str) == Some(app.as_str()))).cloned();
            out.push(serde_json::json!({ "app": app, "root": root, "entry": entry }));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        Options {
            port: 47840.0,
            demo: false,
            bin: "xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            grok_home: None,
            console_mcp: false,
            platform: "linux".into(),
            instances: Vec::new(),
        }
    }

    #[tokio::test]
    async fn a_folder_taken_away_has_xenonets_hooks_taken_out_and_nothing_else() {
        xenonet_platforms::install();
        let dir = std::env::temp_dir().join(format!("xenonet-connect-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let ours = serde_json::json!({ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook claude" });
        let mine = serde_json::json!({ "type": "command", "command": "my-own-hook" });
        let settings = serde_json::json!({ "theme": "dark", "hooks": { "Stop": [{ "hooks": [ours, mine] }] } });
        std::fs::write(&file, settings.to_string()).unwrap();
        let driver = ConnectDriver { options: options(), core: std::sync::Arc::new(Core::new(options())), roots: Vec::new() };
        let done = driver.remove_root_hooks("claude", &dir.to_string_lossy()).await.unwrap();
        assert_eq!((done["app"].as_str(), done["hooks"].as_str()), (Some("claude"), Some("removed")), "{done}");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(after, serde_json::json!({ "theme": "dark", "hooks": { "Stop": [{ "hooks": [mine] }] } }));
        // An app that keeps one folder has no folder to take them out of.
        assert!(driver.remove_root_hooks("hermes", "/x").await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_named_root_is_the_folder_its_app_keeps_things_in() {
        xenonet_platforms::install();
        let o = in_root(options(), "claude", "/h/.claude-work").unwrap();
        assert_eq!((o.claude_config_dir.as_deref(), o.codex_home.as_deref()), (Some("/h/.claude-work"), None));
        let o = in_root(options(), "codex", "/h/.codex-work").unwrap();
        assert_eq!((o.codex_home.as_deref(), o.claude_config_dir.as_deref()), (Some("/h/.codex-work"), None));
        assert!(in_root(options(), "nothing", "/x").is_none());
    }
}
