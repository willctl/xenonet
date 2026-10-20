//! core/config.js's settings as the app's parts read them (the doctor's
//! `paths`, the MCP catalog's files), through xenonet-app's config: worked
//! out once, after the demo's world has moved the folders.

use serde_json::{json, Map, Value};
use xenonet_app::config::{config, Field, Finds};
use xenonet_service::config::Config;

struct Disk;

impl xenonet_base::dirs::Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        std::path::Path::new(p).exists()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, xenonet_base::dirs::Unreadable> {
        let names = std::fs::read_dir(p).map_err(|_| xenonet_base::dirs::Unreadable)?;
        let mut out: Vec<String> = names.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
        out.sort();
        Ok(out)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)
    }
}

struct Found<'a> {
    world: Option<&'a Value>,
    env: &'a Map<String, Value>,
    home: &'a str,
}

fn var(env: &Map<String, Value>, k: &str) -> Option<String> {
    env.get(k).and_then(Value::as_str).map(str::to_string)
}

impl Finds for Found<'_> {
    fn world(&mut self) -> Value {
        self.world.cloned().unwrap_or(Value::Null)
    }
    fn context_dir(&mut self) -> Option<String> {
        let env = xenonet_sources::xenolith::env_of(&Value::Object(self.env.clone()));
        xenonet_sources::xenolith::find_context_dir(&env, self.home, crate::fx::platform()).map(|(d, _)| d)
    }
    fn context_server(&mut self, dir: Option<&str>) -> Value {
        dir.map_or(Value::Null, |d| json!(xenonet_sources::xenolith::context_server_of(d)))
    }
    fn claude_desktop_dir(&mut self) -> Option<String> {
        let env = xenonet_base::dirs::Env {
            claude_desktop_dir: var(self.env, "XENONET_CLAUDE_DESKTOP_DIR").filter(|d| !d.is_empty()),
            local_app_data: var(self.env, "LOCALAPPDATA"),
            app_data: var(self.env, "APPDATA"),
            xdg_config_home: var(self.env, "XDG_CONFIG_HOME").filter(|d| !d.is_empty()),
        };
        xenonet_base::dirs::find_claude_desktop_dir(&env, crate::fx::platform(), self.home, &mut Disk)
    }
}

/// config.js's object, as JSON: its Sets as lists.
pub fn js_config(c: &Config) -> Value {
    let env = crate::fx::env_map();
    let argv: Vec<String> = [c.demo.then_some("--demo"), c.dev.then_some("--dev")].into_iter().flatten().map(String::from).collect();
    // Your own home, as os.homedir() reads it; the demo's world moves the
    // folders on top of it.
    let home = xenonet_service::config::home_dir(&|k| std::env::var(k).ok());
    let mut finds = Found { world: c.world.as_ref(), env: &env, home: &home };
    let made = config(&argv, &env, &home, crate::fx::platform(), &mut finds);
    let mut out = Map::new();
    for (k, f) in made.fields {
        let v = match f {
            Field::Num(n) => xenonet_base::json::number(n),
            Field::Json(v) => v,
            Field::Set(items) => json!(items),
            Field::Nums(nums) => Value::Object(nums.into_iter().map(|(k, n)| (k.to_string(), xenonet_base::json::number(n))).collect()),
        };
        out.insert(k.to_string(), v);
    }
    // The service's own folders and port win: they're what it runs with.
    out.insert("stateDir".into(), json!(c.state_dir));
    out.insert("cacheDir".into(), json!(c.cache_dir));
    out.insert("homeDir".into(), json!(c.home));
    out.insert("projectsDir".into(), json!(c.projects_dir));
    out.insert("port".into(), json!(c.port));
    Value::Object(out)
}
