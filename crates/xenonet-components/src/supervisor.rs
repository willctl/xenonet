//! Starting the service at login (docs/design/always-on.md), under the
//! operating system's own supervisor: a per-user launchd agent on macOS,
//! a systemd user unit on Linux. Either one runs `xenonet service run`
//! from the launcher's copy of the binary, which never moves, so an update
//! leaves the registration as it is. The supervisor starts it at login and
//! again after a crash, and leaves it stopped after a clean stop. Windows
//! has no supervisor here yet.
//!
//! Everything this changes goes through `Files` and `Runner`, so the tests
//! use a made-up folder and a made-up launchctl or systemctl, never the
//! machine's own.

use serde_json::{json, Map, Value};
use xenonet_base::nodepath::Style;

/// The launch agent's label, and the plist's name with `.plist`.
pub const LABEL: &str = "dev.xenonet.service";
/// The systemd user unit's name.
pub const UNIT: &str = "xenonet.service";
/// launchd starts a job again at most this often, in seconds, so a
/// service that keeps crashing doesn't spin.
pub const THROTTLE_S: u32 = 10;
/// systemd waits this long before starting the service again, and gives up
/// after this many starts in this many seconds.
pub const RESTART_S: u32 = 5;
pub const BURST: u32 = 10;
pub const BURST_S: u32 = 300;

/// What's said where there's no supervisor to register with.
pub const UNSUPPORTED: &str = "Starting the service at login isn't available on Windows yet. Desktop starts the service when it opens.";

/// The supervisor a platform has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Launchd,
    Systemd,
}

impl Kind {
    pub fn of(platform: &str) -> Option<Kind> {
        match platform {
            "darwin" => Some(Kind::Launchd),
            "linux" => Some(Kind::Systemd),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Launchd => "launchd",
            Kind::Systemd => "systemd",
        }
    }
}

/// Where the registration goes and what's asked to load it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub kind: Kind,
    /// The plist or the unit file.
    pub file: String,
    /// launchctl or systemctl.
    pub supervisor: String,
    /// The account's uid, for launchd's `gui/<uid>` domain.
    pub uid: String,
    /// The account's name, for loginctl's word on lingering.
    pub user: String,
}

/// What a place is worked out from: the machine, and the two settings
/// tests point elsewhere (XENONET_SUPERVISOR_DIR, the folder the plist or
/// unit goes in; XENONET_SUPERVISOR_BIN, the launchctl or systemctl run).
#[derive(Clone, Debug, Default)]
pub struct Where {
    pub platform: String,
    pub home: String,
    pub xdg_config: Option<String>,
    pub uid: String,
    pub user: String,
    pub dir: Option<String>,
    pub supervisor: Option<String>,
}

impl Place {
    /// None where the platform has no supervisor XenoNet registers with.
    pub fn of(w: &Where) -> Option<Place> {
        let kind = Kind::of(&w.platform)?;
        let posix = Style::of("linux");
        let given = |v: &Option<String>| v.as_deref().filter(|d| !d.is_empty()).map(str::to_string);
        let (dir, name, supervisor) = match kind {
            Kind::Launchd => (posix.join(&[&w.home, "Library", "LaunchAgents"]), format!("{LABEL}.plist"), "/bin/launchctl"),
            Kind::Systemd => {
                let config = given(&w.xdg_config).unwrap_or_else(|| posix.join(&[&w.home, ".config"]));
                (posix.join(&[&config, "systemd", "user"]), UNIT.to_string(), "systemctl")
            }
        };
        let dir = given(&w.dir).unwrap_or(dir);
        Some(Place { kind, file: posix.join(&[&dir, &name]), supervisor: given(&w.supervisor).unwrap_or_else(|| supervisor.to_string()), uid: w.uid.clone(), user: w.user.clone() })
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn target(&self) -> String {
        format!("gui/{}/{LABEL}", self.uid)
    }
}

/// What the supervisor runs: the program and its arguments, the
/// environment XenoNet's own settings came with, where it works, and the
/// log its output goes to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Job {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub dir: String,
    pub log: String,
}

/// The settings a service started at login keeps from where it was
/// registered: those that say where XenoNet keeps things and which of its
/// parts are on. The rest of the environment is the supervisor's, and
/// `service run` adds the login shell's PATH.
pub const KEPT: &[&str] = &[
    "XENONET_PORT",
    "XENONET_RUNTIME_DIR",
    "XENONET_STATE_DIR",
    "XENONET_CACHE_DIR",
    "XENONET_PROJECTS_DIR",
    "XENONET_GATEWAY_PORT",
    "XENONET_ACTORS",
    "XENONET_ASSISTANT",
    "XENONET_HOSTS",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
];

/// The kept settings `get` has, in KEPT's order.
pub fn kept_env(get: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    KEPT.iter().filter_map(|k| get(k).filter(|v| !v.is_empty()).map(|v| ((*k).to_string(), v))).collect()
}

/// The port a registration's service serves (its XENONET_PORT, else
/// 47840), so a start for another port isn't handed to it.
pub fn port_of(text: &str) -> String {
    let after = |key: &str| text.find(key).map(|i| &text[i + key.len()..]);
    let digits = |t: &str| t.chars().take_while(char::is_ascii_digit).collect::<String>();
    let plist = after("<key>XENONET_PORT</key>").and_then(|t| t.trim_start().strip_prefix("<string>")).map(digits);
    let unit = after("Environment=\"XENONET_PORT=").map(digits);
    plist.or(unit).filter(|p| !p.is_empty()).unwrap_or_else(|| "47840".into())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The launch agent: started at load (login), started again when it exits
/// with an error or a signal but not after a clean stop, at most every
/// THROTTLE_S seconds, with its output appended to the service's log.
/// Interactive, as the service Desktop starts runs: it hosts the terminal
/// drawer's shells, which launchd would otherwise throttle.
pub fn plist(job: &Job) -> String {
    let mut l: Vec<String> = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>".into(),
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">".into(),
        "<!-- XenoNet's service, written by xenonet service install. xenonet service uninstall removes it. -->".into(),
        "<plist version=\"1.0\">".into(),
        "<dict>".into(),
    ];
    let key = |l: &mut Vec<String>, indent: &str, k: &str| l.push(format!("{indent}<key>{}</key>", xml(k)));
    let string = |l: &mut Vec<String>, indent: &str, v: &str| l.push(format!("{indent}<string>{}</string>", xml(v)));
    key(&mut l, "\t", "Label");
    string(&mut l, "\t", LABEL);
    key(&mut l, "\t", "ProgramArguments");
    l.push("\t<array>".into());
    for a in std::iter::once(&job.program).chain(&job.args) {
        string(&mut l, "\t\t", a);
    }
    l.push("\t</array>".into());
    if !job.env.is_empty() {
        key(&mut l, "\t", "EnvironmentVariables");
        l.push("\t<dict>".into());
        for (k, v) in &job.env {
            key(&mut l, "\t\t", k);
            string(&mut l, "\t\t", v);
        }
        l.push("\t</dict>".into());
    }
    key(&mut l, "\t", "WorkingDirectory");
    string(&mut l, "\t", &job.dir);
    key(&mut l, "\t", "RunAtLoad");
    l.push("\t<true/>".into());
    key(&mut l, "\t", "KeepAlive");
    l.extend(["\t<dict>", "\t\t<key>SuccessfulExit</key>", "\t\t<false/>", "\t</dict>"].map(String::from));
    key(&mut l, "\t", "ThrottleInterval");
    l.push(format!("\t<integer>{THROTTLE_S}</integer>"));
    key(&mut l, "\t", "ProcessType");
    string(&mut l, "\t", "Interactive");
    for k in ["StandardOutPath", "StandardErrorPath"] {
        key(&mut l, "\t", k);
        string(&mut l, "\t", &job.log);
    }
    l.extend(["</dict>", "</plist>", ""].map(String::from));
    l.join("\n")
}

// systemd reads % as a specifier everywhere, and $ in ExecStart as a
// variable; a value with a space or a quote is quoted.
fn unit_text(s: &str) -> String {
    s.replace('%', "%%")
}

fn unit_word(s: &str) -> String {
    let s = unit_text(s).replace('$', "$$");
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+:@=,".contains(c)) {
        return s;
    }
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The systemd user unit: started at login (default.target), started again
/// after a failure but not after a clean stop, RESTART_S seconds apart and
/// at most BURST times in BURST_S seconds, with its output appended to the
/// service's log. Lingering (loginctl enable-linger) starts it at boot.
pub fn unit(job: &Job) -> Result<String, String> {
    let all = std::iter::once(&job.program).chain(&job.args).chain([&job.dir, &job.log]).chain(job.env.iter().flat_map(|(k, v)| [k, v]));
    if all.into_iter().any(|s| s.contains(['\n', '\r'])) {
        return Err("A path or setting with a line break can't go in a systemd unit.".into());
    }
    let exec: Vec<String> = std::iter::once(&job.program).chain(&job.args).map(|s| unit_word(s)).collect();
    let mut l: Vec<String> = vec![
        "# XenoNet's service, written by xenonet service install. xenonet service".into(),
        "# uninstall removes it.".into(),
        "[Unit]".into(),
        "Description=XenoNet service".into(),
        format!("StartLimitIntervalSec={BURST_S}"),
        format!("StartLimitBurst={BURST}"),
        String::new(),
        "[Service]".into(),
        "Type=simple".into(),
        format!("ExecStart={}", exec.join(" ")),
        format!("WorkingDirectory={}", unit_text(&job.dir)),
    ];
    for (k, v) in &job.env {
        l.push(format!("Environment=\"{}={}\"", unit_text(k), unit_text(v).replace('\\', "\\\\").replace('"', "\\\"")));
    }
    l.extend(["Restart=on-failure".into(), format!("RestartSec={RESTART_S}")]);
    l.extend(["StandardOutput", "StandardError"].map(|k| format!("{k}=append:{}", unit_text(&job.log))));
    l.extend(["", "[Install]", "WantedBy=default.target", ""].map(String::from));
    Ok(l.join("\n"))
}

/// The plist or unit for `job`.
pub fn contents(kind: Kind, job: &Job) -> Result<String, String> {
    match kind {
        Kind::Launchd => Ok(plist(job)),
        Kind::Systemd => unit(job),
    }
}

/// A program's run: its exit code (None when it couldn't start, or was
/// ended at its timeout), and what it printed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ran {
    pub code: Option<i32>,
    pub out: String,
    pub err: String,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    // What a failure says: its error output, else its output, else its code.
    fn said(&self) -> String {
        let t = if self.err.trim().is_empty() { self.out.trim() } else { self.err.trim() };
        if !t.is_empty() {
            return t.lines().next().unwrap_or(t).to_string();
        }
        self.code.map_or_else(|| "it didn't run".into(), |c| format!("exit code {c}"))
    }
}

/// Runs the supervisor's commands.
pub trait Runner {
    fn run(&mut self, program: &str, args: &[&str]) -> Ran;
}

/// Reads and writes the registration's file.
pub trait Files {
    fn read(&self, file: &str) -> Option<String>;
    fn write(&mut self, file: &str, text: &str) -> Result<(), String>;
    fn remove(&mut self, file: &str) -> Result<(), String>;
}

/// The machine's own programs, each given SYSTEM_MS at most.
pub struct System;

const SYSTEM_MS: u64 = 20000;

impl Runner for System {
    fn run(&mut self, program: &str, args: &[&str]) -> Ran {
        run_program(program, args, &[], SYSTEM_MS)
    }
}

/// Runs a program with `env` over this process's environment, its input
/// empty, for `ms` at most.
pub fn run_program(program: &str, args: &[&str], env: &[(&str, &str)], ms: u64) -> Ran {
    use std::io::Read;
    use std::time::{Duration, Instant};
    let child = std::process::Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let Ok(mut child) = child else { return Ran { code: None, out: String::new(), err: format!("{program} couldn't start") } };
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut s);
            }
            String::from_utf8_lossy(&s).into_owned()
        })
    };
    let out = reader(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = reader(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if started.elapsed() >= Duration::from_millis(ms) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    Ran { code, out: out.join().unwrap_or_default(), err: err.join().unwrap_or_default() }
}

/// The file system itself.
pub struct Disk;

impl Files for Disk {
    fn read(&self, file: &str) -> Option<String> {
        std::fs::read_to_string(file).ok()
    }

    // Through a temporary file and a rename, so the supervisor never reads
    // half of one. Readable by all, writable by you: launchd refuses an
    // agent others can write.
    fn write(&mut self, file: &str, text: &str) -> Result<(), String> {
        let posix = Style::of("linux");
        let dir = posix.dirname(file);
        std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't make {dir}: {e}"))?;
        let tmp = format!("{file}.{}.tmp", std::process::id());
        let wrote = std::fs::write(&tmp, text).and_then(|()| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
            }
            std::fs::rename(&tmp, file)
        });
        wrote.map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("couldn't write {file}: {e}")
        })
    }

    fn remove(&mut self, file: &str) -> Result<(), String> {
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("couldn't remove {file}: {e}")),
            _ => Ok(()),
        }
    }
}

/// How the supervisor has the service: loaded or not, its state, the
/// process it runs (None while none runs) and how that last ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub loaded: bool,
    pub state: Option<String>,
    pub pid: Option<u32>,
    pub last_exit: Option<String>,
    /// The supervisor couldn't be asked: why.
    pub error: Option<String>,
}

fn field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.trim().strip_prefix(name).and_then(|r| r.trim_start().strip_prefix('=')).map(str::trim))
}

/// launchctl print's word on the agent: exit 113 (or "Could not find
/// service") when it isn't loaded. The first of each field is the agent's
/// own; nested ones follow it.
pub fn parse_print(ran: &Ran) -> Seen {
    if !ran.ok() {
        let gone = ran.code == Some(113) || ran.err.contains("Could not find service") || ran.out.contains("Could not find service");
        return Seen { error: (!gone).then(|| ran.said()), ..Seen::default() };
    }
    let pid = field(&ran.out, "pid").and_then(|p| p.parse::<u32>().ok()).filter(|p| *p > 0);
    Seen { loaded: true, state: field(&ran.out, "state").map(str::to_string), pid, last_exit: field(&ran.out, "last exit code").map(str::to_string), error: None }
}

/// systemctl show's properties: LoadState not-found (or an unknown unit
/// file state) when there's no unit, MainPID 0 while none runs.
pub fn parse_show(ran: &Ran) -> Seen {
    if !ran.ok() {
        return Seen { error: Some(ran.said()), ..Seen::default() };
    }
    let get = |k: &str| ran.out.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).map(str::trim).filter(|v| !v.is_empty());
    let loaded = get("LoadState") == Some("loaded") && matches!(get("UnitFileState"), Some("enabled" | "enabled-runtime" | "linked" | "static"));
    let pid = get("MainPID").and_then(|p| p.parse::<u32>().ok()).filter(|p| *p > 0);
    Seen { loaded, state: get("ActiveState").map(str::to_string), pid, last_exit: get("ExecMainStatus").map(str::to_string), error: None }
}

/// What the supervisor says of the service now.
pub fn seen(place: &Place, run: &mut dyn Runner) -> Seen {
    match place.kind {
        Kind::Launchd => parse_print(&run.run(&place.supervisor, &["print", &place.target()])),
        Kind::Systemd => parse_show(&run.run(&place.supervisor, &["--user", "show", UNIT, "--property=LoadState,ActiveState,SubState,MainPID,UnitFileState,ExecMainStatus"])),
    }
}

/// Whether the account's services start at boot without anyone logged in
/// (loginctl enable-linger), on Linux; None where that can't be told.
pub fn lingering(place: &Place, run: &mut dyn Runner) -> Option<bool> {
    if place.kind != Kind::Systemd || place.user.is_empty() {
        return None;
    }
    let r = run.run("loginctl", &["show-user", &place.user, "--property=Linger"]);
    if !r.ok() {
        return None;
    }
    field(&r.out, "Linger").map(|v| v == "yes")
}

/// `xenonet service status`'s `login`: whether the service starts at
/// login, and how the supervisor has it.
pub fn status(place: Option<&Place>, files: &dyn Files, run: &mut dyn Runner) -> Value {
    let Some(place) = place else {
        return json!({ "supported": false, "supervisor": null, "registered": false, "reason": UNSUPPORTED });
    };
    let s = seen(place, run);
    let mut out = Map::new();
    out.insert("supported".into(), json!(true));
    out.insert("supervisor".into(), json!(place.kind.name()));
    out.insert("file".into(), json!(place.file));
    let text = files.read(&place.file);
    out.insert("registered".into(), json!(text.is_some()));
    out.insert("port".into(), json!(text.as_deref().map(port_of)));
    out.insert("loaded".into(), json!(s.loaded));
    out.insert("state".into(), json!(s.state));
    out.insert("pid".into(), json!(s.pid));
    out.insert("lastExit".into(), json!(s.last_exit));
    if place.kind == Kind::Systemd {
        out.insert("linger".into(), json!(lingering(place, run)));
    }
    if let Some(e) = s.error {
        out.insert("error".into(), json!(e));
    }
    Value::Object(out)
}

fn failed(what: &str, ran: &Ran) -> String {
    format!("{what} failed: {}", ran.said())
}

/// Registers the service to start at login, and starts it: "installed",
/// "updated" (its file changed, and the supervisor reloaded it) or
/// "already" (as it was, and loaded).
pub fn install(place: &Place, job: &Job, files: &mut dyn Files, run: &mut dyn Runner) -> Result<Value, String> {
    let text = contents(place.kind, job)?;
    let had = files.read(&place.file);
    let before = seen(place, run);
    if let Some(e) = &before.error {
        if place.kind == Kind::Systemd {
            return Err(format!("systemd couldn't be asked about XenoNet's service: {e}"));
        }
    }
    if had.as_deref() == Some(text.as_str()) && before.loaded {
        return Ok(json!({ "status": "already", "supervisor": place.kind.name(), "file": place.file }));
    }
    files.write(&place.file, &text)?;
    match place.kind {
        Kind::Launchd => {
            // A label turned off with launchctl disable stays off, even
            // through bootstrap, until it's enabled again.
            run.run(&place.supervisor, &["enable", &place.target()]);
            if before.loaded {
                let r = run.run(&place.supervisor, &["bootout", &place.target()]);
                if !r.ok() && r.code != Some(3) && r.code != Some(113) {
                    return Err(failed("launchctl bootout", &r));
                }
            }
            let r = run.run(&place.supervisor, &["bootstrap", &place.domain(), &place.file]);
            if !r.ok() {
                return Err(failed("launchctl bootstrap", &r));
            }
        }
        Kind::Systemd => {
            let r = run.run(&place.supervisor, &["--user", "daemon-reload"]);
            if !r.ok() {
                return Err(failed("systemctl --user daemon-reload", &r));
            }
            let r = run.run(&place.supervisor, &["--user", "enable", "--now", UNIT]);
            if !r.ok() {
                return Err(failed("systemctl --user enable", &r));
            }
        }
    }
    Ok(json!({ "status": if had.is_some() { "updated" } else { "installed" }, "supervisor": place.kind.name(), "file": place.file }))
}

/// Stops the service starting at login, and stops the copy the supervisor
/// runs: "removed", or "absent" when there was nothing to remove.
pub fn uninstall(place: &Place, files: &mut dyn Files, run: &mut dyn Runner) -> Result<Value, String> {
    let had = files.read(&place.file).is_some();
    let before = seen(place, run);
    match place.kind {
        Kind::Launchd => {
            if before.loaded {
                let r = run.run(&place.supervisor, &["bootout", &place.target()]);
                if !r.ok() && r.code != Some(3) && r.code != Some(113) {
                    return Err(failed("launchctl bootout", &r));
                }
            }
            files.remove(&place.file)?;
        }
        Kind::Systemd => {
            if had || before.loaded {
                let r = run.run(&place.supervisor, &["--user", "disable", "--now", UNIT]);
                if !r.ok() && had {
                    return Err(failed("systemctl --user disable", &r));
                }
            }
            files.remove(&place.file)?;
            if had {
                run.run(&place.supervisor, &["--user", "daemon-reload"]);
            }
        }
    }
    Ok(json!({ "status": if had || before.loaded { "removed" } else { "absent" }, "supervisor": place.kind.name(), "file": place.file }))
}

/// Asks the supervisor to start the service it has (and to load it first
/// when its file is there but launchd has let it go). It doesn't stop one
/// that's running.
pub fn start(place: &Place, files: &dyn Files, run: &mut dyn Runner) -> Result<(), String> {
    match place.kind {
        Kind::Launchd => {
            if !seen(place, run).loaded {
                if files.read(&place.file).is_none() {
                    return Err("XenoNet's service isn't set to start at login.".into());
                }
                let r = run.run(&place.supervisor, &["bootstrap", &place.domain(), &place.file]);
                if !r.ok() {
                    return Err(failed("launchctl bootstrap", &r));
                }
            }
            let r = run.run(&place.supervisor, &["kickstart", &place.target()]);
            if r.ok() {
                Ok(())
            } else {
                Err(failed("launchctl kickstart", &r))
            }
        }
        Kind::Systemd => {
            let r = run.run(&place.supervisor, &["--user", "start", UNIT]);
            if r.ok() {
                Ok(())
            } else {
                Err(failed("systemctl --user start", &r))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A folder in memory.
    #[derive(Default)]
    struct Mem(HashMap<String, String>);

    impl Files for Mem {
        fn read(&self, file: &str) -> Option<String> {
            self.0.get(file).cloned()
        }
        fn write(&mut self, file: &str, text: &str) -> Result<(), String> {
            self.0.insert(file.into(), text.into());
            Ok(())
        }
        fn remove(&mut self, file: &str) -> Result<(), String> {
            self.0.remove(file);
            Ok(())
        }
    }

    /// A made-up launchctl or systemctl: each call kept, and the agent
    /// loaded or not, as the real ones would have it.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        loaded: bool,
        pid: Option<u32>,
        fail: Option<(&'static str, Ran)>,
    }

    impl Runner for Fake {
        fn run(&mut self, program: &str, args: &[&str]) -> Ran {
            let line = format!("{program} {}", args.join(" "));
            self.calls.push(line.clone());
            if let Some((word, ran)) = &self.fail {
                if line.contains(word) {
                    return ran.clone();
                }
            }
            let ok = |out: &str| Ran { code: Some(0), out: out.into(), err: String::new() };
            match args {
                ["print", _] if self.loaded => ok(&format!("gui/501/dev.xenonet.service = {{\n\tactive count = 1\n\tstate = {}\n\tpid = {}\n\tlast exit code = (never exited)\n\tendpoints = {{\n\t\tstate = active\n\t}}\n}}\n", if self.pid.is_some() { "running" } else { "waiting" }, self.pid.map_or(String::new(), |p| p.to_string()))),
                ["print", _] => Ran { code: Some(113), out: String::new(), err: "Could not find service \"dev.xenonet.service\" in domain for user gui: 501".into() },
                ["bootstrap", ..] => {
                    self.loaded = true;
                    ok("")
                }
                ["bootout", _] => {
                    let was = self.loaded;
                    self.loaded = false;
                    if was {
                        ok("")
                    } else {
                        Ran { code: Some(3), out: String::new(), err: "Boot-out failed: 3: No such process".into() }
                    }
                }
                ["--user", "show", ..] if self.loaded => ok(&format!("LoadState=loaded\nActiveState=active\nSubState=running\nMainPID={}\nUnitFileState=enabled\nExecMainStatus=0\n", self.pid.unwrap_or(0))),
                ["--user", "show", ..] => ok("LoadState=not-found\nActiveState=inactive\nSubState=dead\nMainPID=0\nUnitFileState=\nExecMainStatus=0\n"),
                ["--user", "enable", "--now", _] => {
                    self.loaded = true;
                    ok("")
                }
                ["--user", "disable", "--now", _] => {
                    self.loaded = false;
                    ok("")
                }
                ["show-user", _, _] => ok("Linger=no\n"),
                _ => ok(""),
            }
        }
    }

    fn mac() -> Place {
        Place::of(&Where { platform: "darwin".into(), home: "/Users/will".into(), uid: "501".into(), user: "will".into(), ..Where::default() }).unwrap()
    }

    fn linux() -> Place {
        Place::of(&Where { platform: "linux".into(), home: "/home/will".into(), uid: "1000".into(), user: "will".into(), ..Where::default() }).unwrap()
    }

    fn job() -> Job {
        Job {
            program: "/Users/will/Library/Application Support/XenoNet/runtime/launcher/xenonet".into(),
            args: vec!["service".into(), "run".into()],
            env: vec![("XENONET_PORT".into(), "47860".into())],
            dir: "/Users/will".into(),
            log: "/Users/will/Library/Application Support/XenoNet/runtime/service-47860.log".into(),
        }
    }

    #[test]
    fn each_platform_has_its_place() {
        let m = mac();
        assert_eq!((m.kind, m.file.as_str(), m.supervisor.as_str()), (Kind::Launchd, "/Users/will/Library/LaunchAgents/dev.xenonet.service.plist", "/bin/launchctl"));
        let l = linux();
        assert_eq!((l.kind, l.file.as_str(), l.supervisor.as_str()), (Kind::Systemd, "/home/will/.config/systemd/user/xenonet.service", "systemctl"));
        let xdg = Place::of(&Where { platform: "linux".into(), home: "/home/will".into(), xdg_config: Some("/cfg".into()), ..Where::default() }).unwrap();
        assert_eq!(xdg.file, "/cfg/systemd/user/xenonet.service");
        // A test's folder and supervisor.
        let t = Place::of(&Where { platform: "darwin".into(), home: "/h".into(), dir: Some("/tmp/x".into()), supervisor: Some("/tmp/fake".into()), ..Where::default() }).unwrap();
        assert_eq!((t.file.as_str(), t.supervisor.as_str()), ("/tmp/x/dev.xenonet.service.plist", "/tmp/fake"));
        assert_eq!(Place::of(&Where { platform: "win32".into(), ..Where::default() }), None);
        let v = status(None, &Mem::default(), &mut Fake::default());
        assert_eq!(v["supported"], json!(false));
        assert_eq!(v["reason"], json!(UNSUPPORTED));
    }

    #[test]
    fn the_plist_restarts_a_crash_and_not_a_clean_stop() {
        let p = plist(&job());
        assert!(p.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist"), "{p}");
        for part in [
            "\t<key>Label</key>\n\t<string>dev.xenonet.service</string>\n",
            "\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>/Users/will/Library/Application Support/XenoNet/runtime/launcher/xenonet</string>\n\t\t<string>service</string>\n\t\t<string>run</string>\n\t</array>\n",
            "\t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>XENONET_PORT</key>\n\t\t<string>47860</string>\n\t</dict>\n",
            "\t<key>WorkingDirectory</key>\n\t<string>/Users/will</string>\n",
            "\t<key>RunAtLoad</key>\n\t<true/>\n",
            "\t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n",
            "\t<key>ThrottleInterval</key>\n\t<integer>10</integer>\n",
            "\t<key>ProcessType</key>\n\t<string>Interactive</string>\n",
            "\t<key>StandardOutPath</key>\n\t<string>/Users/will/Library/Application Support/XenoNet/runtime/service-47860.log</string>\n",
            "\t<key>StandardErrorPath</key>\n\t<string>/Users/will/Library/Application Support/XenoNet/runtime/service-47860.log</string>\n",
        ] {
            assert!(p.contains(part), "missing {part:?} in\n{p}");
        }
        assert!(p.ends_with("</dict>\n</plist>\n"));
        // No settings, no EnvironmentVariables; markup in a path is escaped.
        let p = plist(&Job { env: vec![], program: "/a&b/<x>/\"q\"".into(), ..job() });
        assert!(!p.contains("EnvironmentVariables"));
        assert!(p.contains("<string>/a&amp;b/&lt;x&gt;/&quot;q&quot;</string>"));
    }

    #[test]
    fn the_unit_restarts_a_failure_and_starts_at_login() {
        let u = unit(&Job {
            program: "/home/will/.local/state/xenonet/runtime/launcher/xenonet".into(),
            dir: "/home/will".into(),
            log: "/home/will/.local/state/xenonet/runtime/service-47860.log".into(),
            ..job()
        })
        .unwrap();
        assert_eq!(
            u,
            "# XenoNet's service, written by xenonet service install. xenonet service\n# uninstall removes it.\n[Unit]\nDescription=XenoNet service\nStartLimitIntervalSec=300\nStartLimitBurst=10\n\n[Service]\nType=simple\nExecStart=/home/will/.local/state/xenonet/runtime/launcher/xenonet service run\nWorkingDirectory=/home/will\nEnvironment=\"XENONET_PORT=47860\"\nRestart=on-failure\nRestartSec=5\nStandardOutput=append:/home/will/.local/state/xenonet/runtime/service-47860.log\nStandardError=append:/home/will/.local/state/xenonet/runtime/service-47860.log\n\n[Install]\nWantedBy=default.target\n"
        );
        // Spaces, quotes, % and $ reach the program as they are.
        let u = unit(&Job { program: "/opt/My Apps/x\"n%$".into(), env: vec![("XDG_STATE_HOME".into(), "/s p/100%".into())], ..job() }).unwrap();
        assert!(u.contains("ExecStart=\"/opt/My Apps/x\\\"n%%$$\" service run\n"), "{u}");
        assert!(u.contains("Environment=\"XDG_STATE_HOME=/s p/100%%\"\n"), "{u}");
        assert!(unit(&Job { dir: "/a\nb".into(), ..job() }).is_err());
    }

    #[test]
    fn a_registration_says_its_port() {
        assert_eq!(port_of(&plist(&job())), "47860");
        assert_eq!(port_of(&unit(&job()).unwrap()), "47860");
        assert_eq!(port_of(&plist(&Job { env: vec![], ..job() })), "47840");
        assert_eq!(port_of(&unit(&Job { env: vec![("XENONET_STATE_DIR".into(), "/s".into())], ..job() }).unwrap()), "47840");
    }

    #[test]
    fn kept_settings_are_xenonets_own() {
        let env: HashMap<&str, &str> = [("XENONET_PORT", "47860"), ("XENONET_DEMO", "1"), ("XDG_STATE_HOME", "/s"), ("XENONET_STATE_DIR", ""), ("PATH", "/bin")].into();
        let kept = kept_env(|k| env.get(k).map(|v| (*v).to_string()));
        assert_eq!(kept, vec![("XENONET_PORT".to_string(), "47860".to_string()), ("XDG_STATE_HOME".to_string(), "/s".to_string())]);
    }

    #[test]
    fn launchd_registers_starts_and_lets_go() {
        let (place, mut files, mut run) = (mac(), Mem::default(), Fake::default());
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["registered"].clone(), st["loaded"].clone(), st["pid"].clone()), (json!(false), json!(false), json!(null)));
        assert_eq!(st.get("error"), None, "not loaded isn't an error");

        run.calls.clear();
        let r = install(&place, &job(), &mut files, &mut run).unwrap();
        assert_eq!(r, json!({ "status": "installed", "supervisor": "launchd", "file": place.file }));
        assert_eq!(files.read(&place.file).unwrap(), plist(&job()));
        assert_eq!(
            run.calls,
            vec![
                "/bin/launchctl print gui/501/dev.xenonet.service",
                "/bin/launchctl enable gui/501/dev.xenonet.service",
                "/bin/launchctl bootstrap gui/501 /Users/will/Library/LaunchAgents/dev.xenonet.service.plist"
            ]
        );
        // Again, as it is: nothing changes.
        run.calls.clear();
        assert_eq!(install(&place, &job(), &mut files, &mut run).unwrap()["status"], json!("already"));
        assert_eq!(run.calls, vec!["/bin/launchctl print gui/501/dev.xenonet.service"]);
        // Changed: reloaded.
        run.calls.clear();
        let other = Job { env: vec![], ..job() };
        assert_eq!(install(&place, &other, &mut files, &mut run).unwrap()["status"], json!("updated"));
        assert!(run.calls.iter().any(|c| c.ends_with("bootout gui/501/dev.xenonet.service")), "{:?}", run.calls);
        assert!(run.calls.last().unwrap().contains(" bootstrap gui/501 "));

        run.pid = Some(4242);
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["registered"].clone(), st["loaded"].clone(), st["state"].clone(), st["pid"].clone()), (json!(true), json!(true), json!("running"), json!(4242)));
        assert_eq!(st["lastExit"], json!("(never exited)"));
        assert_eq!(st.get("linger"), None);

        // Started: kickstart, which leaves a running one alone.
        run.calls.clear();
        start(&place, &files, &mut run).unwrap();
        assert_eq!(run.calls.last().unwrap(), "/bin/launchctl kickstart gui/501/dev.xenonet.service");

        run.calls.clear();
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("removed"));
        assert_eq!(run.calls, vec!["/bin/launchctl print gui/501/dev.xenonet.service", "/bin/launchctl bootout gui/501/dev.xenonet.service"]);
        assert_eq!(files.read(&place.file), None);
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("absent"));
        assert!(start(&place, &files, &mut run).is_err(), "nothing to start");
    }

    #[test]
    fn launchd_loads_a_registration_it_let_go_before_starting_it() {
        let (place, mut files, mut run) = (mac(), Mem::default(), Fake::default());
        files.write(&place.file, &plist(&job())).unwrap();
        start(&place, &files, &mut run).unwrap();
        assert_eq!(
            run.calls[1..],
            ["/bin/launchctl bootstrap gui/501 /Users/will/Library/LaunchAgents/dev.xenonet.service.plist", "/bin/launchctl kickstart gui/501/dev.xenonet.service"]
        );
    }

    #[test]
    fn a_supervisor_that_fails_says_why() {
        let (place, mut files, mut run) = (mac(), Mem::default(), Fake::default());
        run.fail = Some(("bootstrap", Ran { code: Some(5), out: String::new(), err: "Bootstrap failed: 5: Input/output error\nTry re-running the command as root".into() }));
        assert_eq!(install(&place, &job(), &mut files, &mut run).unwrap_err(), "launchctl bootstrap failed: Bootstrap failed: 5: Input/output error");
        // launchctl itself missing or broken: status says so.
        let mut run = Fake { fail: Some(("print", Ran { code: None, out: String::new(), err: "/bin/launchctl couldn't start".into() })), ..Fake::default() };
        assert_eq!(status(Some(&place), &files, &mut run)["error"], json!("/bin/launchctl couldn't start"));
    }

    #[test]
    fn systemd_enables_starts_and_disables() {
        let (place, mut files, mut run) = (linux(), Mem::default(), Fake::default());
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["supervisor"].clone(), st["loaded"].clone(), st["linger"].clone()), (json!("systemd"), json!(false), json!(false)));

        run.calls.clear();
        let r = install(&place, &job(), &mut files, &mut run).unwrap();
        assert_eq!(r["status"], json!("installed"));
        assert_eq!(files.read(&place.file).unwrap(), unit(&job()).unwrap());
        assert_eq!(run.calls[1..], ["systemctl --user daemon-reload", "systemctl --user enable --now xenonet.service"], "{:?}", run.calls);
        assert_eq!(install(&place, &job(), &mut files, &mut run).unwrap()["status"], json!("already"));

        run.pid = Some(777);
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["loaded"].clone(), st["state"].clone(), st["pid"].clone()), (json!(true), json!("active"), json!(777)));

        run.calls.clear();
        start(&place, &files, &mut run).unwrap();
        assert_eq!(run.calls, vec!["systemctl --user start xenonet.service"]);

        run.calls.clear();
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("removed"));
        assert_eq!(run.calls[1..], ["systemctl --user disable --now xenonet.service", "systemctl --user daemon-reload"]);
        assert_eq!(files.read(&place.file), None);
        run.calls.clear();
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("absent"));
        assert_eq!(run.calls.len(), 1, "only asked: {:?}", run.calls);
    }

    #[test]
    fn systemd_without_a_user_bus_says_so_and_writes_nothing() {
        let (place, mut files) = (linux(), Mem::default());
        let mut run = Fake { fail: Some(("show", Ran { code: Some(1), out: String::new(), err: "Failed to connect to bus: No medium found".into() })), ..Fake::default() };
        let e = install(&place, &job(), &mut files, &mut run).unwrap_err();
        assert_eq!(e, "systemd couldn't be asked about XenoNet's service: Failed to connect to bus: No medium found");
        assert_eq!(files.read(&place.file), None);
    }

    #[test]
    fn print_and_show_are_read_leniently() {
        let r = |code, out: &str| Ran { code: Some(code), out: out.into(), err: String::new() };
        assert_eq!(
            parse_print(&r(0, "x = {\n\tstate = not running\n\tlast exit code = 1: Operation not permitted\n}\n")),
            Seen { loaded: true, state: Some("not running".into()), pid: None, last_exit: Some("1: Operation not permitted".into()), error: None }
        );
        assert_eq!(parse_print(&Ran { code: Some(113), out: String::new(), err: String::new() }), Seen::default());
        let s = parse_show(&r(0, "LoadState=loaded\nActiveState=failed\nMainPID=0\nUnitFileState=disabled\n"));
        assert_eq!((s.loaded, s.state.as_deref(), s.pid), (false, Some("failed"), None));
    }
}
