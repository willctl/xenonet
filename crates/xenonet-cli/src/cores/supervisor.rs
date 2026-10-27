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
use xenonet_components::packages::{current_component, TRAY, TRAY_APP};
use xenonet_components::runtime::{self, Env};
use xenonet_components::supervisor::{self, Disk, Files, Job, Place, System};
use xenonet_components::tray::{self, TrayPlace};

/// Where this machine keeps the registration, or None where there's no
/// supervisor XenoNet registers with. XENONET_SUPERVISOR_DIR and
/// XENONET_SUPERVISOR_BIN put it in another folder and ask another
/// launchctl, systemctl or schtasks, for tests.
pub fn place() -> Option<Place> {
    supervisor::here()
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
    supervisor::job(env, &machine(), &this_binary(), |p| std::path::Path::new(p).is_file())
}

/// Where the tray goes on this Mac: the home folder's Applications, and its
/// launch agent beside the service's; None off a Mac.
/// XENONET_APPLICATIONS_DIR puts the app in another folder, for tests, as
/// XENONET_SUPERVISOR_DIR does the agents.
pub fn tray_place(place: &Place) -> Option<TrayPlace> {
    let dir = sys::var("XENONET_APPLICATIONS_DIR").filter(|d| !d.is_empty()).unwrap_or_else(|| NATIVE.join(&[&sys::home(), "Applications"]));
    TrayPlace::of(place, &dir)
}

/// The installed tray component's XenoNet Tray.app, if there's one: none
/// in a build of your own, which has no components.
fn tray_source(env: &Env) -> Option<String> {
    let current = current_component(TRAY, env, &machine()).ok().flatten()?;
    Some(NATIVE.join(&[current.get("root")?.as_str()?, TRAY_APP]))
}

/// Puts the tray in Applications and starts it at login, as the service
/// does (xenonet-components' tray.rs).
pub fn install_tray(place: &TrayPlace, env: &Env, job: &Job) -> Result<Value, String> {
    tray::install(place, tray_source(env).as_deref(), job, &mut Disk, &mut tray::Bundles, &mut System)
}

pub fn uninstall_tray(place: &TrayPlace) -> Result<Value, String> {
    tray::uninstall(place, &mut Disk, &mut tray::Bundles, &mut System)
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
/// Dev.app, or the Electron a checkout runs; on Windows XenoNet.exe (the
/// xenonet.exe of the command and the service is lower case) or a
/// checkout's electron.exe, and on Linux a checkout's electron.
fn is_desktop(exe: &str) -> bool {
    let exe = exe.replace('\\', "/");
    if let Some((app, _)) = exe.rsplit_once(".app/Contents/MacOS/") {
        let name = app.rsplit('/').next().unwrap_or("");
        return name.starts_with("XenoNet") || name == "Electron";
    }
    let name = exe.rsplit('/').next().unwrap_or("");
    let electron = exe.contains("/node_modules/electron/dist/") && (name == "electron" || name.eq_ignore_ascii_case("electron.exe"));
    electron || (name.starts_with("XenoNet") && name.to_ascii_lowercase().ends_with(".exe"))
}

/// Registering the service is yours to do (docs/design/always-on.md),
/// never an agent's. A running service checks this process as it checks
/// the routes that are yours, so XenoNet's own window passes and an agent
/// doesn't. With none running, this process's parents are looked at the
/// same way: the nearer of an agent, an actor's app and XenoNet's Desktop
/// decides, on macOS, Linux and Windows alike; elsewhere it passes, as the
/// service's own check does.
pub fn yours(env: &Value) -> Result<(), Fail> {
    if let Ok(verdict) = service_request(env, "yours", &json!({})) {
        if verdict.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let reason = verdict.get("reason").and_then(Value::as_str).unwrap_or("this is yours to do");
        return Err(Fail::new(reason, 1));
    }
    let sys = xenonet_wire_engine::procs::System::new();
    if !sys.procs.chains {
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
    let seen = supervisor::seen(place, &Disk, &mut System);
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

/// How a wait for the port ended: it's free, or the service that had it
/// was stopped with `xenonet service stop`, and this one stays stopped too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    Free,
    Stopped,
}

/// While `taken`, waits (`pause`), saying so once (`waiting`). Once the
/// port is free after a wait, a stop asked for meanwhile (`stopped`) means
/// stay stopped. A port that was never taken is just free.
pub fn wait_until_free(mut taken: impl FnMut() -> bool, mut stopped: impl FnMut() -> bool, mut pause: impl FnMut(), mut waiting: impl FnMut()) -> Waited {
    let mut waited = false;
    while taken() {
        if !waited {
            waiting();
            waited = true;
        }
        pause();
    }
    if waited && stopped() {
        Waited::Stopped
    } else {
        Waited::Free
    }
}

/// Whether `xenonet service stop` left its stop file since `since`. One
/// from before (a stop while nothing waited) says nothing about this wait.
pub fn stopped_since(file: &str, since: std::time::SystemTime) -> bool {
    std::fs::metadata(file).and_then(|m| m.modified()).is_ok_and(|at| at >= since)
}

// While another XenoNet service has the port, such as one Desktop started
// before this was registered: waits for it to stop. If it was stopped with
// `xenonet service stop`, this one stays stopped too (its stop file is
// taken away), until the next login or `xenonet service start`; otherwise
// it starts now.
fn wait_for_port(port: u16, stop_file: &str, say: impl Fn(&str)) -> Waited {
    let since = std::time::SystemTime::now();
    let waited = wait_until_free(
        || port_taken(port),
        || stopped_since(stop_file, since),
        || std::thread::sleep(std::time::Duration::from_secs(1)),
        || say(&format!("another XenoNet service has port {port}; this one starts when it stops")),
    );
    if waited == Waited::Stopped {
        let _ = std::fs::remove_file(stop_file);
        say("that service was stopped with xenonet service stop, so this one stays stopped until you next log in or run xenonet service start");
    }
    waited
}

/// The settings a scheduled task gives `service run` as its arguments
/// (NAME=VALUE, XenoNet's own only, since a task has no environment of its
/// own), over those `given` has.
pub fn with_settings(given: &Value, settings: &[String]) -> Value {
    let mut out = given.as_object().cloned().unwrap_or_default();
    for (k, v) in settings.iter().filter_map(|s| s.split_once('=')) {
        if supervisor::KEPT.contains(&k) {
            out.insert(k.into(), json!(v));
        }
    }
    Value::Object(out)
}

/// `xenonet service run`: the service in the foreground, for launchd,
/// systemd or Task Scheduler. It finishes an interrupted update first, then
/// runs the installed Service's binary (or this one, with no Service
/// installed) with the environment Desktop gives it. While another XenoNet
/// service has the port, such as one Desktop started before this was
/// registered, it waits for that one to stop rather than failing and being
/// started again and again. On Unix it becomes the service, so the
/// supervisor's signals reach the service itself; on Windows it stays as
/// its parent (`supervise`).
pub fn run(given: &Value, settings: &[String]) -> Result<i32, Fail> {
    SUPERVISED.store(true, std::sync::atomic::Ordering::Relaxed);
    let given = &with_settings(given, settings);
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
    let stop_file = runtime::stop_file(&env, &machine());
    let mut cmd = std::process::Command::new(&bin);
    xenonet_base::process::Background::no_window(&mut cmd);
    cmd.arg("serve").env_clear();
    for (k, v) in &env {
        cmd.env(k, xenonet_base::text::js_string(v));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A clean exit: launchd and systemd start it again only after a
        // failure.
        if wait_for_port(port, &stop_file, |t| eprintln!("[xenonet] {t}")) == Waited::Stopped {
            return Ok(0);
        }
        let err = cmd.exec();
        Err(Fail::error(format!("couldn't run {bin}: {err}")))
    }
    #[cfg(not(unix))]
    {
        supervise(cmd, &bin, port, &stop_file, &env)
    }
}

/// `service run` on Windows, where Task Scheduler restarts a task only when
/// it fails to start and can't say which process it runs: it stays as the
/// service's parent, appends the service's output to its log as launchd
/// and systemd do, starts it again RESTART_S seconds after it fails unless
/// it keeps failing (supervisor::Restarts, systemd's limits), and keeps its
/// own process and the service's in the pids file for `service status`. A
/// clean stop ends it too, until the next sign-in, and so does `xenonet
/// service stop` of the service it was waiting on.
#[cfg(not(unix))]
fn supervise(mut cmd: std::process::Command, bin: &str, port: u16, stop_file: &str, env: &Env) -> Result<i32, Fail> {
    use std::io::Write;
    use xenonet_components::supervisor::{BURST, BURST_S, RESTART_S};
    let pids = place().filter(|p| p.kind == supervisor::Kind::TaskScheduler).map(|p| p.pids());
    let log_file = NATIVE.join(&[&runtime::runtime_dir(env, &machine()), &format!("service-{port}.log")]);
    let keep = |service: Option<u32>, last: Option<i32>| {
        if let Some(f) = &pids {
            let _ = Disk.write(f, &supervisor::pids_text(std::process::id(), service, last));
        }
    };
    let done = || {
        if let Some(f) = &pids {
            let _ = Disk.remove(f);
        }
    };
    let note = |text: &str| {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
            let _ = writeln!(f, "[xenonet] {text}");
        }
    };
    let open = || std::fs::OpenOptions::new().create(true).append(true).open(&log_file).map_err(|e| Fail::error(format!("couldn't open {log_file}: {e}")));
    keep(None, None);
    if wait_for_port(port, stop_file, note) == Waited::Stopped {
        done();
        return Ok(0);
    }
    let mut restarts = supervisor::Restarts::default();
    let mut last = None;
    loop {
        let mut child = match cmd.stdout(open()?).stderr(open()?).spawn() {
            Ok(c) => c,
            Err(e) => {
                done();
                return Err(Fail::error(format!("couldn't run {bin}: {e}")));
            }
        };
        keep(Some(child.id()), last);
        let code = child.wait().map_or(1, |s| s.code().unwrap_or(1));
        if code == 0 {
            done();
            return Ok(0);
        }
        last = Some(code);
        if !restarts.again(std::time::Instant::now()) {
            note(&format!("the service stopped unexpectedly {BURST} times in {} minutes, so it isn't started again until you next sign in", BURST_S / 60));
            done();
            return Ok(code);
        }
        keep(None, last);
        note(&format!("the service stopped unexpectedly (exit code {code}); starting it again in {RESTART_S} seconds"));
        std::thread::sleep(std::time::Duration::from_secs(RESTART_S.into()));
        if wait_for_port(port, stop_file, note) == Waited::Stopped {
            done();
            return Ok(0);
        }
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
        // Windows and Linux.
        assert!(is_desktop("C:\\Users\\w\\AppData\\Local\\Programs\\XenoNet\\XenoNet.exe"));
        assert!(is_desktop("C:\\w\\xenonet\\node_modules\\electron\\dist\\electron.exe"));
        assert!(is_desktop("/home/w/xenonet/node_modules/electron/dist/electron"));
        assert!(!is_desktop("C:\\Users\\w\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe"), "the command and the service");
        assert!(!is_desktop("C:\\Users\\w\\AppData\\Roaming\\Claude\\claude-code\\2.1.0\\claude.exe"));
        assert!(!is_desktop("/home/w/.local/bin/electron"));
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
    fn a_tasks_arguments_are_xenonets_own_settings() {
        let given = json!({ "XENONET_PORT": "47850" });
        let settings: Vec<String> = ["XENONET_STATE_DIR=D:\\Xeno Net\\state", "PATH=C:\\elsewhere", "XENONET_HOSTS=1", "plain"].map(String::from).to_vec();
        assert_eq!(with_settings(&given, &settings), json!({ "XENONET_PORT": "47850", "XENONET_STATE_DIR": "D:\\Xeno Net\\state", "XENONET_HOSTS": "1" }));
        assert_eq!(with_settings(&given, &[]), given);
    }

    #[test]
    fn a_wait_for_the_port_ends_free_or_stopped() {
        // Taken twice, then free; a stop meanwhile or not.
        let run = |stop: bool| {
            let (mut left, mut said, mut pauses) = (2, 0, 0);
            let waited = wait_until_free(
                || {
                    left -= 1;
                    left >= 0
                },
                || stop,
                || pauses += 1,
                || said += 1,
            );
            (waited, said, pauses)
        };
        assert_eq!(run(false), (Waited::Free, 1, 2), "said once, waited twice");
        assert_eq!(run(true), (Waited::Stopped, 1, 2));
        // A port that was free from the start: a stop file says nothing.
        assert_eq!(wait_until_free(|| false, || true, || {}, || {}), Waited::Free);
    }

    #[test]
    fn only_a_stop_since_the_wait_began_counts() {
        let dir = std::env::temp_dir().join(format!("xenonet-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("service-47999.stop").to_string_lossy().into_owned();
        let before = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        assert!(!stopped_since(&file, before), "no file");
        std::fs::write(&file, "{}").unwrap();
        assert!(stopped_since(&file, before));
        assert!(!stopped_since(&file, std::time::SystemTime::now() + std::time::Duration::from_secs(60)), "an earlier stop's");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_supervisor_waiting_on_a_service_that_is_stopped_stays_stopped() {
        let dir = std::env::temp_dir().join(format!("xenonet-held-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("service.stop").to_string_lossy().into_owned();
        // The service Desktop started, holding the port, and `xenonet
        // service stop` a moment into the wait: the stop file first, then
        // the port freed.
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let f = file.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            std::fs::write(&f, "{}").unwrap();
            drop(held);
        });
        let said = std::sync::Mutex::new(Vec::new());
        let waited = wait_for_port(port, &file, |t| said.lock().unwrap().push(t.to_string()));
        stopper.join().unwrap();
        assert_eq!(waited, Waited::Stopped);
        assert!(!std::path::Path::new(&file).exists(), "the stop file is taken away once it's done its work");
        let said = said.into_inner().unwrap();
        assert!(said[0].contains(&format!("port {port}")) && said[1].contains("stays stopped"), "{said:?}");
        // With no stop, the port freed means start.
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        let freer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(held);
        });
        assert_eq!(wait_for_port(port, &file, |_| {}), Waited::Free);
        freer.join().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
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
