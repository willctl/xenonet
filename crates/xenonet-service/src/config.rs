//! What the service runs as, from its arguments and environment: the part of
//! core/config.js the service itself reads (its port, its folders, demo
//! and dev), and server/index.js's own switches (its name, which instance
//! it is, the layers XENONET_ACTORS, XENONET_ASSISTANT and XENONET_HOSTS
//! turn on and off). The readers' own settings (the projects scan's
//! limits, the poll, the identities…) are theirs to read.

use std::path::{Path, PathBuf};
use xenonet_base::dirs::app_dir;
use xenonet_base::json::Value;
use xenonet_base::text::js_number;

#[derive(Clone, Debug)]
pub struct Config {
    pub demo: bool,
    pub dev: bool,
    pub host: String,
    pub port: u16,
    pub home: String,
    pub projects_dir: String,
    pub state_dir: String,
    pub cache_dir: String,
    /// The app's name, which the desktop app passes on.
    pub name: String,
    /// installed, demo, dev, or the instance XenoNet Dev names.
    pub instance: Option<String>,
    pub actors: bool,
    pub assistant: bool,
    pub hosts: bool,
    /// No app started it (always-on): the component service listens.
    pub standalone: bool,
    /// XenoNet Dev's control channel goes through it.
    pub control: bool,
    pub upstream: Option<String>,
    /// The actor gateway's port when XENONET_GATEWAY_PORT names one; else
    /// the gateway's saved setting has one, or gateway_port_of(port).
    pub gateway_port: Option<u16>,
    pub gateway_probe_local: bool,
    /// Where XenoNet's files are: package.json, dist/, cli/.
    pub root: PathBuf,
    pub launcher_root: Option<String>,
    pub launcher_exec: Option<String>,
    pub exec: Option<String>,
    pub runtime_dir: String,
    /// The demo's made-up world, once it's made (core/demo.js createWorld).
    pub world: Option<Value>,
}

/// How far above the service's port the actor gateway's is by default
/// (server/gateway-control.js's GATEWAY_OFFSET).
pub const GATEWAY_OFFSET: u16 = 1010;

/// The actor gateway's port when nothing names one: 1010 above the
/// service's, so 48850 for an installed XenoNet on 47840. That's clear of
/// every instance's port (47840 to 47845), their pages' (1000 above
/// theirs) and Xenolith's (47850 for its HTTP server, 47859 for its
/// preview), and below the ports macOS and Windows hand out for outgoing
/// connections. One that wouldn't fit goes as far below.
pub fn gateway_port_of(port: u16) -> u16 {
    port.checked_add(GATEWAY_OFFSET).or_else(|| port.checked_sub(GATEWAY_OFFSET)).unwrap_or(port)
}

/// The default before that: 10 above the service's (47850, Xenolith's
/// port, for an installed XenoNet). A gateway set up then keeps it.
pub const OLD_GATEWAY_OFFSET: u16 = 10;

/// The old default port for a service on `port`.
pub fn old_gateway_port_of(port: u16) -> u16 {
    port.checked_add(OLD_GATEWAY_OFFSET).unwrap_or_else(|| gateway_port_of(port))
}

/// os.homedir(): HOME (USERPROFILE on Windows), as Node reads it.
pub fn home_dir(env: &dyn Fn(&str) -> Option<String>) -> String {
    let named = if cfg!(windows) { env("USERPROFILE") } else { env("HOME") };
    named.filter(|h| !h.is_empty()).unwrap_or_else(|| if cfg!(windows) { "C:\\".into() } else { "/".into() })
}

fn platform() -> &'static str {
    crate::borrowed::platform()
}

/// path.resolve of a folder that may be relative.
fn resolve(p: &str) -> String {
    let path = Path::new(p);
    if path.is_absolute() {
        p.to_string()
    } else {
        std::env::current_dir().map_or_else(|_| p.to_string(), |d| d.join(path).to_string_lossy().into_owned())
    }
}

/// Where XenoNet's own files are: XENONET_ROOT, or the folder above the
/// binary's that holds package.json (bin/xenonet in a package,
/// `target/<profile>/xenonet` in a checkout), or else here. Never the
/// launcher's folder, which holds only the launcher.
pub fn find_root(env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    if let Some(r) = env("XENONET_ROOT").filter(|r| !r.is_empty()) {
        return PathBuf::from(r);
    }
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(4) {
            if dir.join("package.json").is_file() {
                return dir.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

impl Config {
    pub fn from_env(args: &[String]) -> Config {
        let get = |k: &str| std::env::var(k).ok();
        Config::from(args, &get)
    }

    pub fn from(args: &[String], env: &dyn Fn(&str) -> Option<String>) -> Config {
        let set = |k: &str| env(k).filter(|v| !v.is_empty());
        let demo = args.iter().any(|a| a == "--demo") || env("XENONET_DEMO").as_deref() == Some("1");
        let dev = args.iter().any(|a| a == "--dev");
        // The service alone from a checkout (npm run dev) is XenoNet Dev's:
        // its port and folders, never the installed XenoNet's.
        let server_dev = !demo && dev && set("XENONET_PORT").is_none();
        let home = home_dir(env);
        let dir_of = |kind: &str, name: &str| {
            app_dir(kind, set(if kind == "state" { "XDG_STATE_HOME" } else { "XDG_CACHE_HOME" }).as_deref(), env("LOCALAPPDATA").as_deref(), platform(), &home, name)
        };
        let dev_dir = |kind: &str| server_dev.then(|| dir_of(kind, "XenoNet Dev"));
        // Number(XENONET_PORT ?? default): "" is 0, as in JS.
        let port = match env("XENONET_PORT") {
            Some(p) => js_number(Some(&Value::String(p))),
            None => {
                if demo {
                    47841.0
                } else if server_dev {
                    47842.0
                } else {
                    47840.0
                }
            }
        };
        let port = if port.fract() == 0.0 && (0.0..=65535.0).contains(&port) { port as u16 } else { 0 };
        let standalone = env("XENONET_STANDALONE").as_deref() == Some("1");
        let launcher_exec = set("XENONET_LAUNCHER_EXEC");
        let instance = set("XENONET_INSTANCE").or_else(|| {
            if standalone || launcher_exec.is_some() {
                Some("installed".into())
            } else if demo {
                Some("demo".into())
            } else if dev {
                Some("dev".into())
            } else {
                None
            }
        });
        let state_dir = resolve(&set("XENONET_STATE_DIR").or_else(|| dev_dir("state")).unwrap_or_else(|| dir_of("state", "XenoNet")));
        let cache_dir = resolve(&set("XENONET_CACHE_DIR").or_else(|| dev_dir("cache")).unwrap_or_else(|| dir_of("cache", "XenoNet")));
        let runtime_dir = set("XENONET_RUNTIME_DIR").unwrap_or_else(|| Path::new(&dir_of("state", "XenoNet")).join("runtime").to_string_lossy().into_owned());
        let projects_dir = resolve(&set("XENONET_PROJECTS_DIR").unwrap_or_else(|| Path::new(&home).join("Projects").to_string_lossy().into_owned()));
        let gateway_port =
            set("XENONET_GATEWAY_PORT").map(|p| js_number(Some(&Value::String(p)))).filter(|n| *n != 0.0 && n.fract() == 0.0 && (1.0..=65535.0).contains(n)).map(|n| n as u16);
        Config {
            demo,
            dev,
            host: "127.0.0.1".into(),
            port,
            home,
            projects_dir,
            state_dir,
            cache_dir,
            name: set("XENONET_NAME").unwrap_or_else(|| if demo { "XenoNet (demo)".into() } else { "XenoNet".into() }),
            instance,
            actors: env("XENONET_ACTORS").as_deref() != Some("off"),
            assistant: env("XENONET_ASSISTANT").as_deref() != Some("off"),
            hosts: env("XENONET_HOSTS").as_deref() == Some("1"),
            standalone,
            control: env("XENONET_CONTROL").as_deref() == Some("1"),
            upstream: if dev { set("XENONET_UPSTREAM") } else { None },
            gateway_port,
            gateway_probe_local: env("XENONET_GATEWAY_PROBE").as_deref() == Some("local"),
            root: find_root(env),
            launcher_root: set("XENONET_LAUNCHER_ROOT"),
            launcher_exec,
            exec: set("XENONET_EXEC"),
            runtime_dir,
            world: None,
        }
    }

    /// Takes the demo world's folders (core/demo.js's createWorld).
    pub fn use_world(&mut self, world: &Value) {
        let take = |k: &str, slot: &mut String| {
            if let Some(v) = world.get(k).and_then(Value::as_str) {
                *slot = v.to_string();
            }
        };
        take("home", &mut self.home);
        take("projectsDir", &mut self.projects_dir);
        take("stateDir", &mut self.state_dir);
        take("cacheDir", &mut self.cache_dir);
        self.world = Some(world.clone());
    }

    /// The demo's folders when its world can't be made yet: the folder it
    /// would be in, in the temp folder, so a demo never writes to your own
    /// state.
    pub fn demo_folders(&mut self) {
        let tmp = xenonet_base::paths::real_path(&std::env::temp_dir());
        let root = tmp.join(format!("xenonet-demo-{}", self.port));
        self.home = root.join("home").to_string_lossy().into_owned();
        self.projects_dir = root.join("home").join("Projects").to_string_lossy().into_owned();
        self.state_dir = root.join("state").to_string_lossy().into_owned();
        self.cache_dir = root.join("cache").to_string_lossy().into_owned();
    }

    /// The service's descriptor for the component service's socket
    /// (core/components/runtime.js's descriptorFile).
    pub fn descriptor_file(&self) -> PathBuf {
        Path::new(&self.runtime_dir).join(format!("service-{}.json", std::env::var("XENONET_PORT").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "47840".into())))
    }

    /// The assistant endpoint's (core/assistant/protocol.js's endpointFile).
    pub fn endpoint_file(&self) -> PathBuf {
        Path::new(&self.runtime_dir).join(format!("assistant-{}.json", std::env::var("XENONET_PORT").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "47840".into())))
    }

    /// The xenonet command's entry for the launcher and connect: cli/xenonet.js
    /// under the launcher's root or this one.
    pub fn cli_entry(&self) -> PathBuf {
        Path::new(self.launcher_root.as_deref().map_or(self.root.as_path(), Path::new)).join("cli").join("xenonet.js")
    }

    /// package.json's version, as the service reports it.
    pub fn version(&self, built: &str) -> String {
        std::fs::read_to_string(self.root.join("package.json"))
            .ok()
            .and_then(|t| xenonet_base::json::parse(&t).ok())
            .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| built.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(args: &[&str], vars: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        Config::from(&args, &|k: &str| map.get(k).cloned())
    }

    #[test]
    fn ports_and_names_as_the_js_service_picks_them() {
        let c = config(&[], &[("HOME", "/home/nova")]);
        assert_eq!((c.port, c.name.as_str(), c.instance.as_deref()), (47840, "XenoNet", None));
        let c = config(&["--demo"], &[("HOME", "/home/nova")]);
        assert_eq!((c.port, c.name.as_str(), c.instance.as_deref()), (47841, "XenoNet (demo)", Some("demo")));
        let c = config(&["--dev"], &[("HOME", "/home/nova")]);
        assert_eq!((c.port, c.instance.as_deref()), (47842, Some("dev")));
        let c = config(&["--dev"], &[("HOME", "/home/nova"), ("XENONET_PORT", "47999"), ("XENONET_STANDALONE", "1")]);
        assert_eq!((c.port, c.gateway_port, c.instance.as_deref()), (47999, None, Some("installed")));
        assert_eq!(config(&[], &[("HOME", "/home/nova"), ("XENONET_GATEWAY_PORT", "50123")]).gateway_port, Some(50123));
        assert_eq!(config(&[], &[("HOME", "/home/nova"), ("XENONET_GATEWAY_PORT", "70000")]).gateway_port, None, "not a port");
        // Clear of the instances' ports, their pages' and Xenolith's.
        let taken: Vec<u16> = (47840..=47845).chain(48840..=48845).chain([47850, 47859]).collect();
        for p in 47840..=47845 {
            assert!(!taken.contains(&gateway_port_of(p)), "{p}");
        }
        assert_eq!((gateway_port_of(47840), gateway_port_of(65000)), (48850, 63990));
        let c = config(&[], &[("HOME", "/home/nova"), ("XENONET_PORT", "")]);
        assert_eq!(c.port, 0, "Number(\"\") is 0");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn folders_follow_xdg() {
        let c = config(&[], &[("HOME", "/home/nova"), ("XDG_STATE_HOME", "/x/state")]);
        assert_eq!(c.state_dir, "/x/state/xenonet");
        assert_eq!(c.cache_dir, "/home/nova/.cache/xenonet");
    }
}
