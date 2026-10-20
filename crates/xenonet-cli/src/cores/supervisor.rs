//! The service started at login (xenonet-components' supervisor.rs), for
//! xenonet service install, uninstall, status, start and run: where this
//! machine keeps the registration, the job it runs, whether registering is
//! yours to do, handing over from a service Desktop started to the one the
//! supervisor starts, and running the service in the foreground for the
//! supervisor.

use super::components::{machine, service_request};
use crate::out::Fail;
use crate::sys;
use serde_json::{json, Value};
use xenonet_base::nodepath::NATIVE;
use xenonet_components::runtime::{self, Env};
use xenonet_components::supervisor::{self, Disk, Files, Job, Place, System, Where};

/// The account's uid, for launchd's `gui/<uid>` domain.
#[cfg_attr(unix, expect(unsafe_code, reason = "getuid(2), which std doesn't wrap"))]
fn uid() -> String {
    #[cfg(unix)]
    {
        // SAFETY: getuid(2) takes nothing and can't fail.
        unsafe { libc::getuid() }.to_string()
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
}

/// Where this machine keeps the registration, or None where there's no
/// supervisor XenoNet registers with (Windows, for now).
/// XENONET_SUPERVISOR_DIR and XENONET_SUPERVISOR_BIN put it in another
/// folder and ask another launchctl or systemctl, for tests.
pub fn place() -> Option<Place> {
    Place::of(&Where {
        platform: sys::platform().into(),
        home: sys::home(),
        xdg_config: sys::var("XDG_CONFIG_HOME"),
        uid: uid(),
        user: sys::var("USER").or_else(|| sys::var("LOGNAME")).unwrap_or_default(),
        dir: sys::var("XENONET_SUPERVISOR_DIR"),
        supervisor: sys::var("XENONET_SUPERVISOR_BIN"),
    })
}

/// Set in `service run`, which is the supervisor's copy: a service it
/// starts while finishing an interrupted update is started beside it, not
/// by asking the supervisor, which would only find this one running.
static SUPERVISED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The registration for `port`, when there's one: its place.
pub fn registered_for(port: &str) -> Option<Place> {
    if SUPERVISED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let place = place()?;
    let text = Disk.read(&place.file)?;
    (supervisor::port_of(&text) == port).then_some(place)
}

fn this_binary() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    std::fs::canonicalize(&exe).unwrap_or(exe).to_string_lossy().into_owned()
}

/// What the supervisor runs: `service run`, from the launcher's copy of
/// the binary, which never moves (or this binary, where there's no
/// launcher: a build of your own), with the settings that say where
/// XenoNet keeps things, less those that only say the default.
pub fn job(env: &Env) -> Job {
    let m = machine();
    let launcher = runtime::launcher_bin(env, &m);
    let program = if std::path::Path::new(&launcher).is_file() { launcher } else { this_binary() };
    let port = runtime::port_of(env);
    let log = NATIVE.join(&[&runtime::runtime_dir(env, &m), &format!("service-{port}.log")]);
    let mut bare = env.clone();
    bare.shift_remove("XENONET_RUNTIME_DIR");
    let default_runtime = runtime::runtime_dir(&bare, &m);
    let kept = supervisor::kept_env(|k| runtime::var(env, k).map(str::to_string))
        .into_iter()
        .filter(|(k, v)| match k.as_str() {
            "XENONET_PORT" => v != "47840",
            "XENONET_RUNTIME_DIR" => *v != default_runtime,
            _ => true,
        })
        .collect();
    Job { program, args: vec!["service".into(), "run".into()], env: kept, dir: m.home, log }
}

/// "a Claude Code process", as the service's own refusals name one.
fn an_agent(agent: &str) -> String {
    match agent {
        "claude" => "a Claude Code process".into(),
        "codex" => "a Codex process".into(),
        "opencode" => "an OpenCode process".into(),
        other => format!("an agent's process ({other})"),
    }
}

fn not_yours(why: &str) -> Fail {
    Fail::new(format!("this is yours to do, and {why}; run it in your own terminal or in XenoNet"), 1)
}

/// XenoNet's own Desktop, by its executable: an installed XenoNet, XenoNet
/// Dev.app, or the Electron a checkout runs.
fn is_desktop(exe: &str) -> bool {
    let Some((app, _)) = exe.rsplit_once(".app/Contents/MacOS/") else { return false };
    let name = app.rsplit('/').next().unwrap_or("");
    name.starts_with("XenoNet") || name == "Electron"
}

/// Registering the service is yours to do (docs/design/always-on.md),
/// never an agent's. A running service checks this process as it checks
/// the routes that are yours, so XenoNet's own window passes and an agent
/// doesn't. With none running, this process's parents are looked at the
/// same way: the nearer of an agent, an actor's app and XenoNet's Desktop
/// decides. Where XenoNet can't see processes (Linux, Windows), it passes,
/// as the service's own check does.
pub fn yours(env: &Value) -> Result<(), Fail> {
    if let Ok(verdict) = service_request(env, "yours", &json!({})) {
        if verdict.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let reason = verdict.get("reason").and_then(Value::as_str).unwrap_or("this is yours to do");
        return Err(Fail::new(reason, 1));
    }
    let sys = xenonet_wire_engine::procs::System::new();
    if !sys.procs.supported {
        return Ok(());
    }
    let chain = sys.procs.chain(&mut sys.io(), Some(f64::from(std::process::id())), true, true).map_err(|_| not_yours("XenoNet couldn't see which program asked"))?;
    for link in chain.iter().skip(1) {
        if let Some(agent) = link.agent {
            return Err(not_yours(&format!("it came from under {}", an_agent(agent))));
        }
        if link.product.is_some() {
            return Err(not_yours("it came from under an actor's app"));
        }
        if link.exe.clone().flatten().is_some_and(|f| is_desktop(&f)) {
            return Ok(());
        }
    }
    Ok(())
}

/// How the running service stands: its status, or None when none answers.
pub fn running(env: &Value) -> Option<Value> {
    service_request(env, "status", &json!({})).ok()
}

/// Whether a service's status has a terminal still open.
pub fn has_terminals(status: &Value) -> bool {
    status.get("terminals").and_then(Value::as_array).is_some_and(|l| l.iter().any(|t| !xenonet_base::text::truthy(t.get("exited"))))
}

/// Whether the service that answers is the one the supervisor runs.
pub fn supervised(status: &Value, login: &Value) -> bool {
    let pid = |v: &Value| v.get("pid").and_then(Value::as_u64);
    pid(login).is_some() && pid(login) == pid(status)
}

/// After registering, a service Desktop or `service start` started is
/// still running, and the supervisor's copy waits for its port. It's asked
/// to stop now, so the supervisor's takes over ("now"), unless it has
/// terminals open ("terminals") or the supervisor has no copy waiting
/// ("later"); either way the supervisor's takes over when it stops. None
/// when there's nothing to hand over.
pub fn hand_over(env: &Value, place: &Place) -> Option<&'static str> {
    let status = running(env)?;
    let seen = supervisor::seen(place, &mut System);
    if seen.pid.is_some() && seen.pid.map(u64::from) == status.get("pid").and_then(Value::as_u64) {
        return None;
    }
    if seen.pid.is_none() {
        return Some("later");
    }
    if has_terminals(&status) {
        return Some("terminals");
    }
    match service_request(env, "stop", &json!({})) {
        Ok(_) => Some("now"),
        Err(_) => Some("later"),
    }
}

/// Asks the supervisor to start the registered service and waits for it
/// to answer, as ensureService waits for one it started.
pub fn start(env: &Value, place: &Place) -> Result<(), Fail> {
    supervisor::start(place, &Disk, &mut System).map_err(Fail::error)?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < until {
        if running(env).is_some() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(Fail::error("XenoNet Service did not start. See its service log."))
}

// ---------- xenonet service run ----------

const MARK: &str = "__XENONET_ENV__";

/// The login shell's PATH, as a new terminal window would have it: a
/// supervisor starts the service with its own bare one, without the
/// shell profile's folders where Codex, Homebrew's git and the like are
/// (electron/shellenv.js, which does this for Desktop).
fn shell_path(shell: &str) -> Option<String> {
    let fish = shell.rsplit('/').next() == Some("fish");
    let script = if fish { format!("printf '{MARK}%s{MARK}' (string join : $PATH)") } else { format!("printf '{MARK}%s{MARK}' \"$PATH\"") };
    let ran = supervisor::run_program(shell, &["-ilc", &script], &[("DISABLE_AUTO_UPDATE", "true")], 4000);
    let (_, rest) = ran.out.split_once(MARK)?;
    rest.split_once(MARK).map(|(p, _)| p.to_string())
}

/// PATH lists joined, each folder once, in order.
pub fn merge_path(lists: &[&str]) -> String {
    let mut seen = std::collections::HashSet::new();
    lists.iter().flat_map(|l| l.split(':')).filter(|d| !d.is_empty() && seen.insert(d.to_string())).collect::<Vec<_>>().join(":")
}

/// en_US → en_US.UTF-8 if the system has it, else en_US.UTF-8, so shells
/// get UTF-8 rather than the C locale (shellenv.js's langFor).
pub fn lang_for(locale: &str, exists: impl Fn(&str) -> bool) -> String {
    let mut parts = locale.trim().splitn(2, ['-', '_']);
    let (lang, region) = (parts.next().unwrap_or(""), parts.next().unwrap_or("").chars().take(2).collect::<String>());
    let ok = (2..=3).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_lowercase()) && region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic());
    let name = format!("{lang}_{}.UTF-8", region.to_ascii_uppercase());
    if ok && exists(&name) {
        name
    } else {
        "en_US.UTF-8".into()
    }
}

/// The environment the service gets under a supervisor: the login shell's
/// PATH, and on macOS a UTF-8 LANG, as Desktop gives the one it starts.
fn shell_env(env: &mut Env) {
    if cfg!(windows) || runtime::var(env, "TERM_PROGRAM").is_some() {
        return;
    }
    let shell = runtime::var(env, "SHELL").map_or_else(|| if sys::platform() == "darwin" { "/bin/zsh".into() } else { "/bin/sh".into() }, str::to_string);
    let home = sys::home();
    let common = format!("/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:{home}/.local/bin");
    let path = merge_path(&[&shell_path(&shell).unwrap_or_default(), runtime::var(env, "PATH").unwrap_or(""), &common]);
    env.insert("PATH".into(), json!(path));
    if sys::platform() == "darwin" && runtime::var(env, "LANG").is_none() && runtime::var(env, "LC_ALL").is_none() {
        let locale = supervisor::run_program("/usr/bin/defaults", &["read", "-g", "AppleLocale"], &[], 3000).out;
        env.insert("LANG".into(), json!(lang_for(&locale, |n| std::path::Path::new("/usr/share/locale").join(n).exists())));
    }
}

fn port_taken(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err_and(|e| e.kind() == std::io::ErrorKind::AddrInUse)
}

/// `xenonet service run`: the service in the foreground, for launchd or
/// systemd. It finishes an interrupted update first, then runs the
/// installed Service's binary (or this one, with no Service installed)
/// with the environment Desktop gives it. While another XenoNet service
/// has the port, such as one Desktop started before this was registered,
/// it waits for that one to stop rather than failing and being started
/// again and again. On Unix it becomes the service, so the supervisor's
/// signals reach the service itself.
pub fn run(given: &Value) -> Result<i32, Fail> {
    SUPERVISED.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = super::recover_service_update(given);
    let (bin, mut env) = match super::installed_runtime(given) {
        Ok(rt) => {
            let root = rt.get("root").map(xenonet_base::text::js_string).unwrap_or_default();
            let bin = NATIVE.join(&[&root, "bin", runtime::bin_name(sys::platform())]);
            let env: Env = rt.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
            (if std::path::Path::new(&bin).is_file() { bin } else { this_binary() }, env)
        }
        Err(_) => {
            let mut env = super::env_map();
            for (k, v) in given.as_object().into_iter().flatten() {
                env.insert(k.clone(), v.clone());
            }
            (this_binary(), env)
        }
    };
    env.insert("XENONET_STANDALONE".into(), json!("1"));
    env.shift_remove("ELECTRON_RUN_AS_NODE");
    env.shift_remove("XENONET_EXEC");
    shell_env(&mut env);
    let port: u16 = runtime::port_of(&env).parse().unwrap_or(47840);
    let mut said = false;
    while port_taken(port) {
        if !said {
            eprintln!("[xenonet] another XenoNet service has port {port}; this one starts when it stops");
            said = true;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    let mut cmd = std::process::Command::new(&bin);
    cmd.arg("serve").env_clear();
    for (k, v) in &env {
        cmd.env(k, xenonet_base::text::js_string(v));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(Fail::error(format!("couldn't run {bin}: {err}")))
    }
    #[cfg(not(unix))]
    {
        let status = cmd.status().map_err(|e| Fail::error(format!("couldn't run {bin}: {e}")))?;
        Ok(status.code().unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktops_are_xenonets_own_apps() {
        assert!(is_desktop("/Applications/XenoNet.app/Contents/MacOS/XenoNet"));
        assert!(is_desktop("/Users/w/Applications/XenoNet Dev.app/Contents/MacOS/XenoNet Dev"));
        assert!(is_desktop("/w/xenonet/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"));
        assert!(!is_desktop("/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"));
        assert!(!is_desktop("/Applications/Claude.app/Contents/MacOS/Claude"));
        assert!(!is_desktop("/usr/local/bin/xenonet"));
    }

    #[test]
    fn the_shell_path_comes_first_and_each_folder_once() {
        assert_eq!(merge_path(&["/opt/homebrew/bin:/usr/bin", "/usr/bin:/bin", "", "/opt/homebrew/bin:/x"]), "/opt/homebrew/bin:/usr/bin:/bin:/x");
    }

    #[test]
    fn a_locale_becomes_a_utf8_lang() {
        let has = |n: &str| n == "de_DE.UTF-8";
        assert_eq!(lang_for("de_DE", has), "de_DE.UTF-8");
        assert_eq!(lang_for("de-de\n", has), "de_DE.UTF-8");
        assert_eq!(lang_for("fr_FR", has), "en_US.UTF-8");
        assert_eq!(lang_for("", has), "en_US.UTF-8");
    }

    #[test]
    fn handing_over_needs_a_supervised_copy_and_no_terminals() {
        let open = json!({ "pid": 10, "terminals": [{ "pid": 1, "exited": false }] });
        let closed = json!({ "pid": 10, "terminals": [{ "pid": 1, "exited": true }] });
        assert!(has_terminals(&open));
        assert!(!has_terminals(&closed));
        assert!(!has_terminals(&json!({ "pid": 10 })));
        assert!(supervised(&json!({ "pid": 10 }), &json!({ "pid": 10 })));
        assert!(!supervised(&json!({ "pid": 10 }), &json!({ "pid": null })));
        assert!(!supervised(&json!({ "pid": 10 }), &json!({ "pid": 11 })));
    }
}
