//! Where Xenolith keeps its own files (not the wiki): the platform's standard locations.
//! XENOLITH_{CONFIG,DATA,STATE,LOG,CACHE}_DIR set one kind; XENOLITH_HOME puts
//! everything in one folder (portable mode, the old ~/.xenolith layout).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub portable: bool,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub legacy_home: PathBuf,
}

/// The environment as a map, so tests (and the installer) can pass another one.
pub type Env = HashMap<String, String>;

pub fn process_env() -> Env {
    std::env::vars().collect()
}

/// The user's home folder as Node's os.homedir() finds it.
pub fn home_dir(env: &Env) -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(h) = env.get(var).filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    if cfg!(windows) {
        if let (Some(d), Some(p)) = (env.get("HOMEDRIVE"), env.get("HOMEPATH")) {
            return PathBuf::from(format!("{d}{p}"));
        }
    }
    PathBuf::from(".")
}

const OVERRIDES: [(&str, &str); 5] = [("config", "XENOLITH_CONFIG_DIR"), ("data", "XENOLITH_DATA_DIR"), ("state", "XENOLITH_STATE_DIR"), ("log", "XENOLITH_LOG_DIR"), ("cache", "XENOLITH_CACHE_DIR")];

fn abs_env(env: &Env, name: &str) -> Option<PathBuf> {
    env.get(name).filter(|v| Path::new(v).is_absolute()).map(PathBuf::from)
}

fn resolve(p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() {
        return path;
    }
    std::env::current_dir().map(|d| d.join(&path)).unwrap_or(path)
}

impl AppPaths {
    pub fn from_env(env: &Env) -> Self {
        Self::for_platform(env, std::env::consts::OS, &home_dir(env))
    }

    pub fn current() -> Self {
        Self::from_env(&process_env())
    }

    pub fn for_platform(env: &Env, os: &str, home: &Path) -> Self {
        let portable = env.get("XENOLITH_HOME").filter(|v| !v.is_empty()).map(|v| resolve(v));
        let (mut config, mut data, mut state, mut log, mut cache) = if let Some(h) = &portable {
            (h.clone(), h.clone(), h.clone(), h.join("logs"), h.clone())
        } else if os == "windows" {
            let roaming = env.get("APPDATA").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"));
            let local = env.get("LOCALAPPDATA").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
            let data = local.join("Xenolith");
            (roaming.join("Xenolith"), data.clone(), data.join("state"), data.join("logs"), data)
        } else if os == "macos" {
            let support = home.join("Library").join("Application Support").join("Xenolith");
            let state = abs_env(env, "XDG_STATE_HOME").map(|p| p.join("xenolith"));
            (
                abs_env(env, "XDG_CONFIG_HOME").map(|p| p.join("xenolith")).unwrap_or_else(|| support.clone()),
                abs_env(env, "XDG_DATA_HOME").map(|p| p.join("xenolith")).unwrap_or_else(|| support.clone()),
                state.clone().unwrap_or_else(|| support.join("state")),
                state.map(|s| s.join("logs")).unwrap_or_else(|| home.join("Library").join("Logs").join("Xenolith")),
                abs_env(env, "XDG_CACHE_HOME").map(|p| p.join("xenolith")).unwrap_or_else(|| home.join("Library").join("Caches").join("Xenolith")),
            )
        } else {
            let state = abs_env(env, "XDG_STATE_HOME").unwrap_or_else(|| home.join(".local").join("state")).join("xenolith");
            (
                abs_env(env, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")).join("xenolith"),
                abs_env(env, "XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share")).join("xenolith"),
                state.clone(),
                state.join("logs"),
                abs_env(env, "XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache")).join("xenolith"),
            )
        };
        for (kind, name) in OVERRIDES {
            if let Some(v) = env.get(name).filter(|v| !v.is_empty()) {
                let p = resolve(v);
                match kind {
                    "config" => config = p,
                    "data" => data = p,
                    "state" => state = p,
                    "log" => log = p,
                    _ => cache = p,
                }
            }
        }
        AppPaths { portable: portable.is_some(), config_dir: config, data_dir: data, state_dir: state, log_dir: log, cache_dir: cache, legacy_home: home.join(".xenolith") }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.json")
    }
    pub fn install_state(&self) -> PathBuf {
        self.state_dir.join("install-state.json")
    }
    pub fn http_sessions(&self) -> PathBuf {
        self.log_dir.join("http-sessions.json")
    }
    pub fn runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime")
    }
    pub fn curator_codex_home(&self) -> PathBuf {
        self.data_dir.join("curator").join("codex-home")
    }

    /// Environment that makes a child process use the same locations.
    pub fn env_vars(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("XENOLITH_CONFIG_DIR", self.config_dir.clone()),
            ("XENOLITH_DATA_DIR", self.data_dir.clone()),
            ("XENOLITH_STATE_DIR", self.state_dir.clone()),
            ("XENOLITH_LOG_DIR", self.log_dir.clone()),
            ("XENOLITH_CACHE_DIR", self.cache_dir.clone()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn windows_linux_macos_portable() {
        let w = AppPaths::for_platform(&env(&[("APPDATA", r"C:\U\R"), ("LOCALAPPDATA", r"C:\U\L")]), "windows", Path::new(r"C:\U"));
        assert_eq!(w.config_file(), PathBuf::from(r"C:\U\R").join("Xenolith").join("config.json"));
        assert_eq!(w.log_dir, PathBuf::from(r"C:\U\L").join("Xenolith").join("logs"));
        if cfg!(windows) {
            return; // the POSIX layouts are only ever computed on POSIX hosts, where "/cfg" is absolute
        }
        let l = AppPaths::for_platform(&env(&[("XDG_CONFIG_HOME", "/cfg"), ("XDG_DATA_HOME", "rel")]), "linux", Path::new("/home/w"));
        assert_eq!(l.config_dir, PathBuf::from("/cfg/xenolith"));
        assert_eq!(l.data_dir, PathBuf::from("/home/w/.local/share/xenolith"));
        let m = AppPaths::for_platform(&env(&[]), "macos", Path::new("/Users/w"));
        assert_eq!(m.log_dir, PathBuf::from("/Users/w/Library/Logs/Xenolith"));
        let p = AppPaths::for_platform(&env(&[("XENOLITH_HOME", "/tmp/h"), ("XENOLITH_LOG_DIR", "/var/log/xenolith")]), "linux", Path::new("/home/w"));
        assert!(p.portable);
        assert_eq!(p.config_file(), PathBuf::from("/tmp/h/config.json"));
        assert_eq!(p.log_dir, PathBuf::from("/var/log/xenolith"));
    }
}
