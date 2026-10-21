//! Starting the service at login (docs/design/always-on.md), under the
//! operating system's own supervisor: a per-user launchd agent on macOS,
//! a systemd user unit on Linux, a scheduled task on Windows. Each one runs
//! `xenonet service run` from the launcher's copy of the binary, which
//! never moves, so an update leaves the registration as it is. The
//! supervisor starts it at login and again after a crash, and leaves it
//! stopped after a clean stop.
//!
//! Task Scheduler restarts a task only when it fails to start, and doesn't
//! say which process a task runs. So on Windows `service run` stays as the
//! service's parent: it starts the service again after a crash, as systemd
//! would, and keeps the processes it runs in a file beside the task's
//! definition.
//!
//! Everything this changes goes through `Files` and `Runner`, so the tests
//! use a made-up folder and a made-up launchctl, systemctl or schtasks,
//! never the machine's own.

use crate::runtime::{self, Env, Machine};
use serde_json::{json, Map, Value};
use xenonet_base::nodepath::{Style, NATIVE};

/// The launch agent's label, and the plist's name with `.plist`.
pub const LABEL: &str = "dev.xenonet.service";
/// The systemd user unit's name.
pub const UNIT: &str = "xenonet.service";
/// The scheduled task's name, in Task Scheduler's own folder.
pub const TASK: &str = "XenoNet Service";
/// The task's definition XenoNet keeps (Task Scheduler keeps its own copy),
/// and the file `service run` keeps its processes in, both in XenoNet's
/// folder in the local app data.
pub const TASK_FILE: &str = "xenonet-service.xml";
pub const PIDS_FILE: &str = "xenonet-service.pids.json";
/// launchd starts a job again at most this often, in seconds, so a
/// service that keeps crashing doesn't spin.
pub const THROTTLE_S: u32 = 10;
/// systemd waits this long before starting the service again, and gives up
/// after this many starts in this many seconds. `service run` does the same
/// on Windows.
pub const RESTART_S: u32 = 5;
pub const BURST: u32 = 10;
pub const BURST_S: u32 = 300;
/// Task Scheduler starts a task that failed to start again this often, at
/// most this many times.
pub const TASK_RETRY: &str = "PT1M";
pub const TASK_RETRIES: u32 = 3;

/// What's said where there's no supervisor to register with.
pub const UNSUPPORTED: &str = "Starting the service at login isn't available on this system. Desktop starts the service when it opens.";

/// The supervisor a platform has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Launchd,
    Systemd,
    TaskScheduler,
}

impl Kind {
    pub fn of(platform: &str) -> Option<Kind> {
        match platform {
            "darwin" => Some(Kind::Launchd),
            "linux" => Some(Kind::Systemd),
            "win32" => Some(Kind::TaskScheduler),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Launchd => "launchd",
            Kind::Systemd => "systemd",
            Kind::TaskScheduler => "Task Scheduler",
        }
    }

    /// How its paths are written.
    pub fn style(self) -> Style {
        Style::of(if self == Kind::TaskScheduler { "win32" } else { "linux" })
    }
}

/// Where the registration goes and what's asked to load it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub kind: Kind,
    /// The plist, the unit file or the task's definition.
    pub file: String,
    /// launchctl, systemctl or schtasks.
    pub supervisor: String,
    /// The account: its uid, for launchd's `gui/<uid>` domain, or its SID,
    /// which the scheduled task runs as.
    pub uid: String,
    /// The account's name, for loginctl's word on lingering.
    pub user: String,
    /// Where the system's own programs are: System32 on Windows, for
    /// conhost and tasklist; empty elsewhere.
    pub system: String,
}

/// What a place is worked out from: the machine, and the two settings
/// tests point elsewhere (XENONET_SUPERVISOR_DIR, the folder the plist,
/// unit or task definition goes in; XENONET_SUPERVISOR_BIN, the launchctl,
/// systemctl or schtasks run).
#[derive(Clone, Debug, Default)]
pub struct Where {
    pub platform: String,
    pub home: String,
    pub xdg_config: Option<String>,
    pub uid: String,
    pub user: String,
    pub dir: Option<String>,
    pub supervisor: Option<String>,
    /// LOCALAPPDATA and SystemRoot, on Windows.
    pub local_app_data: Option<String>,
    pub system_root: Option<String>,
}

impl Place {
    /// None where the platform has no supervisor XenoNet registers with.
    pub fn of(w: &Where) -> Option<Place> {
        let kind = Kind::of(&w.platform)?;
        let posix = Style::of("linux");
        let win = Style::of("win32");
        let given = |v: &Option<String>| v.as_deref().filter(|d| !d.is_empty()).map(str::to_string);
        let system = match kind {
            Kind::TaskScheduler => win.join(&[&given(&w.system_root).unwrap_or_else(|| "C:\\Windows".into()), "System32"]),
            _ => String::new(),
        };
        let (dir, name, supervisor) = match kind {
            Kind::Launchd => (posix.join(&[&w.home, "Library", "LaunchAgents"]), format!("{LABEL}.plist"), "/bin/launchctl".to_string()),
            Kind::Systemd => {
                let config = given(&w.xdg_config).unwrap_or_else(|| posix.join(&[&w.home, ".config"]));
                (posix.join(&[&config, "systemd", "user"]), UNIT.to_string(), "systemctl".to_string())
            }
            Kind::TaskScheduler => {
                let local = given(&w.local_app_data).unwrap_or_else(|| win.join(&[&w.home, "AppData", "Local"]));
                (win.join(&[&local, "XenoNet"]), TASK_FILE.to_string(), win.join(&[&system, "schtasks.exe"]))
            }
        };
        let dir = given(&w.dir).unwrap_or(dir);
        Some(Place { kind, file: kind.style().join(&[&dir, &name]), supervisor: given(&w.supervisor).unwrap_or(supervisor), uid: w.uid.clone(), user: w.user.clone(), system })
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn target(&self) -> String {
        format!("gui/{}/{LABEL}", self.uid)
    }

    /// The file `service run` keeps its processes in, on Windows: beside
    /// the task's definition.
    pub fn pids(&self) -> String {
        let style = self.kind.style();
        style.join(&[&style.dirname(&self.file), PIDS_FILE])
    }

    fn system_program(&self, name: &str) -> String {
        Style::of("win32").join(&[&self.system, name])
    }
}

/// The account `service run` registers for: its uid, or on Windows its
/// SID (whoami /user), which a scheduled task names.
#[cfg_attr(unix, expect(unsafe_code, reason = "getuid(2), which std doesn't wrap"))]
pub fn account() -> String {
    #[cfg(unix)]
    {
        // SAFETY: getuid(2) takes nothing and can't fail.
        unsafe { libc::getuid() }.to_string()
    }
    #[cfg(not(unix))]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let whoami = Style::of("win32").join(&[&root, "System32", "whoami.exe"]);
        sid_of(&run_program(&whoami, &["/user", "/fo", "csv", "/nh"], &[], 5000).out).unwrap_or_default()
    }
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

// process.platform, and os.homedir().
fn platform_here() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

fn home_here() -> String {
    var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default()
}

/// Where this machine keeps the registration, or None where there's no
/// supervisor XenoNet registers with. XENONET_SUPERVISOR_DIR and
/// XENONET_SUPERVISOR_BIN put it in another folder and ask another
/// launchctl, systemctl or schtasks, for tests.
pub fn here() -> Option<Place> {
    Place::of(&Where {
        platform: platform_here().into(),
        home: home_here(),
        xdg_config: var("XDG_CONFIG_HOME"),
        uid: account(),
        user: var("USER").or_else(|| var("LOGNAME")).or_else(|| var("USERNAME")).unwrap_or_default(),
        dir: var("XENONET_SUPERVISOR_DIR"),
        supervisor: var("XENONET_SUPERVISOR_BIN"),
        local_app_data: var("LOCALAPPDATA"),
        system_root: var("SystemRoot"),
    })
}

/// The SID in whoami /user /fo csv's line: "desktop\will","S-1-5-21-…".
pub fn sid_of(out: &str) -> Option<String> {
    out.split('"').map(str::trim).find(|t| is_sid(t)).map(str::to_string)
}

fn is_sid(s: &str) -> bool {
    s.strip_prefix("S-1-").is_some_and(|r| !r.is_empty() && r.split('-').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
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

/// What the supervisor runs for a service with the settings in `env`:
/// `service run` from the launcher's copy of the binary, which never
/// moves, or `own` where there's no launcher (a build of your own), with
/// the settings that say where XenoNet keeps things, less those that only
/// say the default; in the home folder, its output to the service's log.
pub fn job(env: &Env, m: &Machine, own: &str, is_file: impl Fn(&str) -> bool) -> Job {
    let launcher = runtime::launcher_bin(env, m);
    let program = if is_file(&launcher) { launcher } else { own.to_string() };
    let port = runtime::port_of(env);
    let log = NATIVE.join(&[&runtime::runtime_dir(env, m), &format!("service-{port}.log")]);
    let mut bare = env.clone();
    bare.shift_remove("XENONET_RUNTIME_DIR");
    let default_runtime = runtime::runtime_dir(&bare, m);
    let kept = kept_env(|k| runtime::var(env, k).map(str::to_string))
        .into_iter()
        .filter(|(k, v)| match k.as_str() {
            "XENONET_PORT" => v != "47840",
            "XENONET_RUNTIME_DIR" => *v != default_runtime,
            _ => true,
        })
        .collect();
    Job { program, args: vec!["service".into(), "run".into()], env: kept, dir: m.home.clone(), log }
}

/// The port a registration's service serves (its XENONET_PORT, else
/// 47840), so a start for another port isn't handed to it.
pub fn port_of(text: &str) -> String {
    let after = |key: &str| text.find(key).map(|i| &text[i + key.len()..]);
    let digits = |t: &str| t.chars().take_while(char::is_ascii_digit).collect::<String>();
    let plist = after("<key>XENONET_PORT</key>").and_then(|t| t.trim_start().strip_prefix("<string>")).map(digits);
    // The unit's Environment="XENONET_PORT=…", the task's XENONET_PORT=… argument.
    let set = after("XENONET_PORT=").map(digits);
    plist.or(set).filter(|p| !p.is_empty()).unwrap_or_else(|| "47840".into())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

fn unxml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// The text between `from` and the next `to` after it.
fn between<'a>(text: &'a str, from: &str, to: &str) -> Option<&'a str> {
    let start = text.find(from)? + from.len();
    let len = text[start..].find(to)?;
    Some(&text[start..start + len])
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

// A quoted word of the unit's, as unit_word and Environment= write one:
// its text, and what follows it.
fn unit_unquote(s: &str) -> (String, &str) {
    let Some(body) = s.strip_prefix('"') else {
        let end = s.find([' ', '\t']).unwrap_or(s.len());
        return (s[..end].to_string(), &s[end..]);
    };
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            }
            '"' => return (out, &body[i + 1..]),
            _ => out.push(c),
        }
    }
    (out, "")
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

/// One argument on a Windows command line, quoted as programs split it
/// again (CommandLineToArgvW): only when it needs it, with the backslashes
/// before a quote doubled.
pub fn win_arg(s: &str) -> String {
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    let mut out = String::from('"');
    let mut slashes = 0;
    for c in s.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

/// A Windows command line split into its arguments, as win_arg quotes
/// them.
pub fn win_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut started, mut quoted) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut slashes = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    slashes += 1;
                }
                if chars.peek() == Some(&'"') {
                    word.push_str(&"\\".repeat(slashes / 2));
                    if slashes % 2 == 1 {
                        chars.next();
                        word.push('"');
                    }
                } else {
                    word.push_str(&"\\".repeat(slashes));
                }
                started = true;
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(word);
    }
    out
}

/// The scheduled task: started when you sign in, as you, with your own
/// rights, no time limit and no battery conditions, one copy at a time,
/// and started again a minute after it fails to start. Its action is
/// `service run` (with the kept settings as arguments, since a task has no
/// environment of its own) under a console host with no window, so
/// nothing shows on the desktop. `user` is the account's SID; `system` is
/// System32.
pub fn task(job: &Job, user: &str, system: &str) -> Result<String, String> {
    if !is_sid(user) {
        return Err("XenoNet couldn't tell which Windows account to start the service for: whoami /user didn't say.".into());
    }
    let all = std::iter::once(&job.program).chain(&job.args).chain([&job.dir]).chain(job.env.iter().flat_map(|(k, v)| [k, v]));
    if all.into_iter().any(|s| s.contains(['\n', '\r', '\0'])) {
        return Err("A path or setting with a line break can't go in a scheduled task.".into());
    }
    let mut words = vec!["--headless".to_string(), win_arg(&job.program)];
    words.extend(job.args.iter().map(|a| win_arg(a)));
    words.extend(job.env.iter().map(|(k, v)| win_arg(&format!("{k}={v}"))));
    let conhost = Style::of("win32").join(&[system, "conhost.exe"]);
    let l: Vec<String> = vec![
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>".into(),
        "<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">".into(),
        "  <RegistrationInfo>".into(),
        "    <Author>XenoNet</Author>".into(),
        "    <Description>XenoNet's service, started when you sign in and again if it stops unexpectedly. Written by xenonet service install; xenonet service uninstall removes it.</Description>".into(),
        "  </RegistrationInfo>".into(),
        "  <Triggers>".into(),
        "    <LogonTrigger>".into(),
        "      <Enabled>true</Enabled>".into(),
        format!("      <UserId>{}</UserId>", xml(user)),
        "    </LogonTrigger>".into(),
        "  </Triggers>".into(),
        "  <Principals>".into(),
        "    <Principal id=\"Author\">".into(),
        format!("      <UserId>{}</UserId>", xml(user)),
        "      <LogonType>InteractiveToken</LogonType>".into(),
        "      <RunLevel>LeastPrivilege</RunLevel>".into(),
        "    </Principal>".into(),
        "  </Principals>".into(),
        "  <Settings>".into(),
        "    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>".into(),
        "    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>".into(),
        "    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>".into(),
        "    <AllowHardTerminate>true</AllowHardTerminate>".into(),
        "    <StartWhenAvailable>false</StartWhenAvailable>".into(),
        "    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>".into(),
        "    <IdleSettings>".into(),
        "      <StopOnIdleEnd>false</StopOnIdleEnd>".into(),
        "      <RestartOnIdle>false</RestartOnIdle>".into(),
        "    </IdleSettings>".into(),
        "    <AllowStartOnDemand>true</AllowStartOnDemand>".into(),
        "    <Enabled>true</Enabled>".into(),
        "    <Hidden>false</Hidden>".into(),
        "    <RunOnlyIfIdle>false</RunOnlyIfIdle>".into(),
        "    <WakeToRun>false</WakeToRun>".into(),
        "    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>".into(),
        "    <Priority>4</Priority>".into(),
        "    <RestartOnFailure>".into(),
        format!("      <Interval>{TASK_RETRY}</Interval>"),
        format!("      <Count>{TASK_RETRIES}</Count>"),
        "    </RestartOnFailure>".into(),
        "  </Settings>".into(),
        "  <Actions Context=\"Author\">".into(),
        "    <Exec>".into(),
        format!("      <Command>{}</Command>", xml(&conhost)),
        format!("      <Arguments>{}</Arguments>", xml(&words.join(" "))),
        format!("      <WorkingDirectory>{}</WorkingDirectory>", xml(&job.dir)),
        "    </Exec>".into(),
        "  </Actions>".into(),
        "</Task>".into(),
        String::new(),
    ];
    Ok(l.join("\r\n"))
}

/// The plist, unit or task for `job`.
pub fn contents(place: &Place, job: &Job) -> Result<String, String> {
    match place.kind {
        Kind::Launchd => Ok(plist(job)),
        Kind::Systemd => unit(job),
        Kind::TaskScheduler => task(job, &place.uid, &place.system),
    }
}

// The task's arguments: --headless, the program, then its own.
fn task_args(text: &str) -> Vec<String> {
    between(text, "<Arguments>", "</Arguments>").map(|a| win_split(&unxml(a))).unwrap_or_default()
}

/// The program a registration runs.
pub fn program_of(kind: Kind, text: &str) -> Option<String> {
    match kind {
        Kind::Launchd => between(text, "<key>ProgramArguments</key>", "</array>").and_then(|a| between(a, "<string>", "</string>")).map(unxml),
        Kind::Systemd => {
            let line = text.lines().find_map(|l| l.strip_prefix("ExecStart="))?;
            let (word, _) = unit_unquote(line);
            Some(word.replace("$$", "$").replace("%%", "%"))
        }
        Kind::TaskScheduler => task_args(text).into_iter().nth(1),
    }
}

/// The settings a registration keeps, as it has them: those of KEPT.
pub fn settings_of(kind: Kind, text: &str) -> Vec<(String, String)> {
    let pairs: Vec<(String, String)> = match kind {
        Kind::Launchd => {
            let Some(dict) = between(text, "<key>EnvironmentVariables</key>", "</dict>") else { return Vec::new() };
            let mut out = Vec::new();
            let mut rest = dict;
            while let Some(k) = between(rest, "<key>", "</key>") {
                let after = &rest[rest.find("</key>").map_or(rest.len(), |i| i + "</key>".len())..];
                let Some(v) = between(after, "<string>", "</string>") else { break };
                out.push((unxml(k), unxml(v)));
                rest = &after[after.find("</string>").map_or(after.len(), |i| i + "</string>".len())..];
            }
            out
        }
        Kind::Systemd => text
            .lines()
            .filter_map(|l| l.strip_prefix("Environment="))
            .filter_map(|l| {
                let (pair, _) = unit_unquote(l);
                let (k, v) = pair.split_once('=')?;
                Some((k.replace("%%", "%"), v.replace("%%", "%")))
            })
            .collect(),
        Kind::TaskScheduler => {
            task_args(text).into_iter().skip_while(|a| a != "run").skip(1).filter_map(|a| a.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect()
        }
    };
    pairs.into_iter().filter(|(k, _)| KEPT.contains(&k.as_str())).collect()
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
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).envs(env.iter().copied()).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: schtasks, tasklist and whoami flash no console.
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = cmd.spawn() else { return Ran { code: None, out: String::new(), err: format!("{program} couldn't start") } };
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

/// How a task definition starts: schtasks reads one only as UTF-16.
const UTF16_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-16\"?>";

/// Text as UTF-16, little-endian, after its byte order mark.
pub fn utf16(text: &str) -> Vec<u8> {
    let mut b = vec![0xff, 0xfe];
    for u in text.encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b
}

/// A file's text, UTF-8, or UTF-16 after its byte order mark.
pub fn text_of(bytes: &[u8]) -> Option<String> {
    match bytes {
        [0xff, 0xfe, rest @ ..] => String::from_utf16(&rest.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect::<Vec<_>>()).ok(),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

impl Files for Disk {
    fn read(&self, file: &str) -> Option<String> {
        text_of(&std::fs::read(file).ok()?)
    }

    // Through a temporary file and a rename, so the supervisor never reads
    // half of one. Readable by all, writable by you: launchd refuses an
    // agent others can write. A task definition goes as UTF-16.
    fn write(&mut self, file: &str, text: &str) -> Result<(), String> {
        let style = Style::of(if cfg!(windows) { "win32" } else { "linux" });
        let dir = style.dirname(file);
        std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't make {dir}: {e}"))?;
        let tmp = format!("{file}.{}.tmp", std::process::id());
        let bytes = if text.starts_with(UTF16_XML) { utf16(text) } else { text.as_bytes().to_vec() };
        let wrote = std::fs::write(&tmp, bytes).and_then(|()| {
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

/// schtasks /query /xml's word on the task: a failure when there's no such
/// task (its message is in the system's language, so any failure reads as
/// none), and its own Enabled in Settings, false when you turned it off in
/// Task Scheduler. Loaded means it starts at sign-in.
pub fn parse_query(ran: &Ran) -> Seen {
    if ran.code.is_none() {
        return Seen { error: Some(ran.said()), ..Seen::default() };
    }
    if !ran.ok() {
        return Seen::default();
    }
    // UTF-16 read as bytes: its ASCII is all the XML here needs.
    let out = ran.out.replace('\0', "");
    let off = between(&out, "<Settings>", "</Settings>").and_then(|s| s.split("<Enabled>").nth(1)).is_some_and(|e| e.trim_start().starts_with("false"));
    Seen { loaded: !off, state: off.then(|| "disabled".into()), ..Seen::default() }
}

/// What `service run` keeps in the pids file on Windows: its own process,
/// the service's while one runs, and how the last one ended.
pub fn pids_text(runner: u32, service: Option<u32>, last_exit: Option<i32>) -> String {
    xenonet_base::json::stringify(&json!({ "runner": runner, "service": service, "lastExit": last_exit.map(|c| c.to_string()) }), 0)
}

fn read_pids(text: Option<String>) -> (Option<u32>, Option<u32>, Option<String>) {
    let v = text.and_then(|t| xenonet_base::json::parse(&t).ok()).unwrap_or(Value::Null);
    let pid = |k: &str| v.get(k).and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 0);
    (pid("runner"), pid("service"), v.get("lastExit").and_then(Value::as_str).map(str::to_string))
}

/// Whether tasklist has a process with this id (its CSV's second field).
pub fn listed(ran: &Ran, pid: u32) -> bool {
    let want = pid.to_string();
    ran.ok() && ran.out.lines().any(|l| l.split("\",\"").nth(1) == Some(want.as_str()))
}

/// What the supervisor says of the service now.
pub fn seen(place: &Place, files: &dyn Files, run: &mut dyn Runner) -> Seen {
    match place.kind {
        Kind::Launchd => parse_print(&run.run(&place.supervisor, &["print", &place.target()])),
        Kind::Systemd => parse_show(&run.run(&place.supervisor, &["--user", "show", UNIT, "--property=LoadState,ActiveState,SubState,MainPID,UnitFileState,ExecMainStatus"])),
        Kind::TaskScheduler => {
            let mut s = parse_query(&run.run(&place.supervisor, &["/query", "/tn", TASK, "/xml", "ONE"]));
            if !s.loaded {
                return s;
            }
            // The processes `service run` keeps, while it's there to keep
            // them: a file left by one that was ended says nothing.
            let (runner, service, last) = read_pids(files.read(&place.pids()));
            let tasklist = place.system_program("tasklist.exe");
            let mut alive = |pid: u32| listed(&run.run(&tasklist, &["/fi", &format!("PID eq {pid}"), "/fo", "csv", "/nh"]), pid);
            if runner.is_some_and(&mut alive) {
                let service = service.filter(|p| alive(*p));
                s.pid = service.or(runner);
                s.state = Some(if service.is_some() { "running" } else { "waiting" }.into());
            } else {
                s.state = Some("not running".into());
            }
            s.last_exit = last;
            s
        }
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

/// Whether the account has a graphical session now (loginctl's Display),
/// on Linux. One without, reached over SSH as a host is, needs lingering
/// for its service to start at boot and keep running between sessions;
/// None where that can't be told.
pub fn graphical(place: &Place, run: &mut dyn Runner) -> Option<bool> {
    if place.kind != Kind::Systemd || place.user.is_empty() {
        return None;
    }
    let r = run.run("loginctl", &["show-user", &place.user, "--property=Display"]);
    if !r.ok() {
        return None;
    }
    Some(field(&r.out, "Display").is_some_and(|v| !v.is_empty()))
}

/// `xenonet service status`'s `login`: whether the service starts at
/// login, and how the supervisor has it.
pub fn status(place: Option<&Place>, files: &dyn Files, run: &mut dyn Runner) -> Value {
    let Some(place) = place else {
        return json!({ "supported": false, "supervisor": null, "registered": false, "reason": UNSUPPORTED });
    };
    let s = seen(place, files, run);
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

/// The doctor's look at the registration: `status`, and whether its file
/// is what `xenonet service install` writes now (`current`; null when it
/// can't be written here, with `expected` saying why), the program it runs
/// and whether that's there, and on Linux whether the account has a
/// graphical session. What install writes is worked out from the
/// registration's own settings over `base` (the variables that aren't
/// kept, such as LOCALAPPDATA), and its own program where there's no
/// launcher, so only what install itself would change shows.
pub fn review(place: Option<&Place>, m: &Machine, base: &Env, files: &dyn Files, run: &mut dyn Runner, is_file: impl Fn(&str) -> bool) -> Value {
    let mut out = status(place, files, run);
    let Some(place) = place else { return out };
    out["user"] = json!(place.user);
    if place.kind == Kind::Systemd {
        out["graphical"] = json!(graphical(place, run));
    }
    let Some(text) = files.read(&place.file) else { return out };
    let mut env = base.clone();
    for (k, v) in settings_of(place.kind, &text) {
        env.insert(k, json!(v));
    }
    let program = program_of(place.kind, &text);
    let job = job(&env, m, program.as_deref().unwrap_or(""), &is_file);
    match contents(place, &job) {
        Ok(want) => out["current"] = json!(want.replace("\r\n", "\n") == text.replace("\r\n", "\n")),
        Err(e) => {
            out["current"] = Value::Null;
            out["expected"] = json!(e);
        }
    }
    out["program"] = json!(program);
    out["programFound"] = json!(program.as_deref().map(&is_file));
    out
}

/// `review` of this machine's registration, as the doctor's `ctx.login()`
/// answers: its place, files and programs, with LOCALAPPDATA, which
/// install takes from the environment but doesn't keep.
pub fn review_here() -> Value {
    let m = Machine { platform: platform_here().into(), arch: String::new(), home: home_here() };
    let mut base = Env::new();
    if let Some(v) = var("LOCALAPPDATA") {
        base.insert("LOCALAPPDATA".into(), json!(v));
    }
    review(here().as_ref(), &m, &base, &Disk, &mut System, |p| std::path::Path::new(p).is_file())
}

fn failed(what: &str, ran: &Ran) -> String {
    format!("{what} failed: {}", ran.said())
}

/// Whether a task is there in Task Scheduler, turned on or off.
fn task_exists(s: &Seen) -> bool {
    s.loaded || s.state.as_deref() == Some("disabled")
}

/// Registers the service to start at login, and starts it: "installed",
/// "updated" (its file changed, and the supervisor reloaded it) or
/// "already" (as it was, and loaded).
pub fn install(place: &Place, job: &Job, files: &mut dyn Files, run: &mut dyn Runner) -> Result<Value, String> {
    let text = contents(place, job)?;
    let had = files.read(&place.file);
    let before = seen(place, files, run);
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
        Kind::TaskScheduler => {
            // /f replaces a task that's there, turned on again.
            let r = run.run(&place.supervisor, &["/create", "/tn", TASK, "/xml", &place.file, "/f"]);
            if !r.ok() {
                return Err(failed("schtasks /create", &r));
            }
            // One running from the old definition is ended, and waited
            // for, so the new one starts now: a task runs one copy at a
            // time.
            if before.pid.is_some() {
                run.run(&place.supervisor, &["/end", "/tn", TASK]);
                for _ in 0..40 {
                    if seen(place, files, run).pid.is_none() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
            let r = run.run(&place.supervisor, &["/run", "/tn", TASK]);
            if !r.ok() {
                return Err(failed("schtasks /run", &r));
            }
        }
    }
    Ok(json!({ "status": if had.is_some() { "updated" } else { "installed" }, "supervisor": place.kind.name(), "file": place.file }))
}

/// Stops the service starting at login, and stops the copy the supervisor
/// runs: "removed", or "absent" when there was nothing to remove.
pub fn uninstall(place: &Place, files: &mut dyn Files, run: &mut dyn Runner) -> Result<Value, String> {
    let had = files.read(&place.file).is_some();
    let before = seen(place, files, run);
    let mut present = had || before.loaded;
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
        Kind::TaskScheduler => {
            if task_exists(&before) {
                present = true;
                run.run(&place.supervisor, &["/end", "/tn", TASK]);
                let r = run.run(&place.supervisor, &["/delete", "/tn", TASK, "/f"]);
                if !r.ok() {
                    return Err(failed("schtasks /delete", &r));
                }
            }
            files.remove(&place.file)?;
            files.remove(&place.pids())?;
        }
    }
    Ok(json!({ "status": if present { "removed" } else { "absent" }, "supervisor": place.kind.name(), "file": place.file }))
}

/// Asks the supervisor to start the service it has (and to load it first
/// when its file is there but the supervisor has let it go). It doesn't
/// stop one that's running.
pub fn start(place: &Place, files: &dyn Files, run: &mut dyn Runner) -> Result<(), String> {
    match place.kind {
        Kind::Launchd => {
            if !seen(place, files, run).loaded {
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
        Kind::TaskScheduler => {
            let s = seen(place, files, run);
            if s.state.as_deref() == Some("disabled") {
                return Err(format!("{TASK} is turned off in Task Scheduler. Turn it on there, or run xenonet service install."));
            }
            if !s.loaded {
                if files.read(&place.file).is_none() {
                    return Err("XenoNet's service isn't set to start at login.".into());
                }
                let r = run.run(&place.supervisor, &["/create", "/tn", TASK, "/xml", &place.file, "/f"]);
                if !r.ok() {
                    return Err(failed("schtasks /create", &r));
                }
            }
            // A task runs one copy at a time, so this leaves a running one
            // alone.
            let r = run.run(&place.supervisor, &["/run", "/tn", TASK]);
            if r.ok() {
                Ok(())
            } else {
                Err(failed("schtasks /run", &r))
            }
        }
    }
}

/// `service run`'s restarts on Windows, as systemd's unit has them: again
/// after a failure, unless BURST starts came within BURST_S seconds.
#[derive(Debug, Default)]
pub struct Restarts {
    at: Vec<std::time::Instant>,
}

impl Restarts {
    /// Whether to start the service again after a failure at `now`.
    pub fn again(&mut self, now: std::time::Instant) -> bool {
        let window = std::time::Duration::from_secs(u64::from(BURST_S));
        self.at.retain(|t| now.saturating_duration_since(*t) < window);
        if self.at.len() >= BURST as usize {
            return false;
        }
        self.at.push(now);
        true
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

    /// A made-up launchctl, systemctl or schtasks: each call kept, and the
    /// agent loaded or not, as the real ones would have it. `alive` are the
    /// processes tasklist lists.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        loaded: bool,
        pid: Option<u32>,
        fail: Option<(&'static str, Ran)>,
        alive: Vec<u32>,
        disabled: bool,
        display: bool,
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
                ["show-user", _, "--property=Linger"] => ok("Linger=no\n"),
                ["show-user", _, "--property=Display"] => ok(if self.display { "Display=2\n" } else { "Display=\n" }),
                ["/query", "/tn", _, "/xml", "ONE"] if self.loaded || self.disabled => ok(&format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n<Task><Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers><Settings><IdleSettings /><Enabled>{}</Enabled></Settings></Task>\r\n",
                    !self.disabled
                )),
                ["/query", ..] => Ran { code: Some(1), out: String::new(), err: "ERROR: The system cannot find the file specified.".into() },
                ["/create", ..] => {
                    self.loaded = true;
                    self.disabled = false;
                    ok("SUCCESS: The scheduled task \"XenoNet Service\" has successfully been created.")
                }
                ["/delete", ..] => {
                    self.loaded = false;
                    self.disabled = false;
                    ok("SUCCESS")
                }
                ["/end", ..] => {
                    self.alive.clear();
                    ok("SUCCESS")
                }
                ["/fi", pid, ..] => {
                    let pid = pid.trim_start_matches("PID eq ");
                    if self.alive.iter().any(|a| a.to_string() == pid) {
                        ok(&format!("\"xenonet.exe\",\"{pid}\",\"Console\",\"1\",\"12,345 K\"\r\n"))
                    } else {
                        ok("INFO: No tasks are running which match the specified criteria.\r\n")
                    }
                }
                _ => ok(""),
            }
        }
    }

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    fn mac() -> Place {
        Place::of(&Where { platform: "darwin".into(), home: "/Users/will".into(), uid: "501".into(), user: "will".into(), ..Where::default() }).unwrap()
    }

    fn linux() -> Place {
        Place::of(&Where { platform: "linux".into(), home: "/home/will".into(), uid: "1000".into(), user: "will".into(), ..Where::default() }).unwrap()
    }

    fn windows() -> Place {
        Place::of(&Where {
            platform: "win32".into(),
            home: "C:\\Users\\will".into(),
            uid: SID.into(),
            user: "will".into(),
            local_app_data: Some("C:\\Users\\will\\AppData\\Local".into()),
            system_root: Some("C:\\Windows".into()),
            ..Where::default()
        })
        .unwrap()
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

    fn win_job() -> Job {
        Job {
            program: "C:\\Users\\will\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe".into(),
            args: vec!["service".into(), "run".into()],
            env: vec![("XENONET_PORT".into(), "47850".into()), ("XENONET_STATE_DIR".into(), "D:\\Xeno Net\\state".into())],
            dir: "C:\\Users\\will".into(),
            log: "C:\\Users\\will\\AppData\\Local\\XenoNet\\runtime\\service-47850.log".into(),
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
        let w = windows();
        assert_eq!(
            (w.kind, w.file.as_str(), w.supervisor.as_str(), w.system.as_str()),
            (Kind::TaskScheduler, "C:\\Users\\will\\AppData\\Local\\XenoNet\\xenonet-service.xml", "C:\\Windows\\System32\\schtasks.exe", "C:\\Windows\\System32")
        );
        assert_eq!(w.pids(), "C:\\Users\\will\\AppData\\Local\\XenoNet\\xenonet-service.pids.json");
        // Without LOCALAPPDATA, it's under the home folder.
        let bare = Place::of(&Where { platform: "win32".into(), home: "C:\\Users\\will".into(), ..Where::default() }).unwrap();
        assert_eq!(bare.file, "C:\\Users\\will\\AppData\\Local\\XenoNet\\xenonet-service.xml");
        // A test's folder and supervisor.
        let t = Place::of(&Where { platform: "darwin".into(), home: "/h".into(), dir: Some("/tmp/x".into()), supervisor: Some("/tmp/fake".into()), ..Where::default() }).unwrap();
        assert_eq!((t.file.as_str(), t.supervisor.as_str()), ("/tmp/x/dev.xenonet.service.plist", "/tmp/fake"));
        assert_eq!(Place::of(&Where { platform: "freebsd".into(), ..Where::default() }), None);
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
    fn the_task_starts_at_sign_in_as_you_without_a_window() {
        let t = task(&win_job(), SID, "C:\\Windows\\System32").unwrap();
        assert!(t.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n<Task version=\"1.2\""), "{t}");
        for part in [
            &format!("    <LogonTrigger>\r\n      <Enabled>true</Enabled>\r\n      <UserId>{SID}</UserId>\r\n    </LogonTrigger>\r\n"),
            &format!("      <UserId>{SID}</UserId>\r\n      <LogonType>InteractiveToken</LogonType>\r\n      <RunLevel>LeastPrivilege</RunLevel>\r\n"),
            "    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\r\n",
            "    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\r\n    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\r\n",
            "    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\r\n",
            "    <RestartOnFailure>\r\n      <Interval>PT1M</Interval>\r\n      <Count>3</Count>\r\n    </RestartOnFailure>\r\n",
            "      <Command>C:\\Windows\\System32\\conhost.exe</Command>\r\n",
            "      <Arguments>--headless C:\\Users\\will\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe service run XENONET_PORT=47850 &quot;XENONET_STATE_DIR=D:\\Xeno Net\\state&quot;</Arguments>\r\n",
            "      <WorkingDirectory>C:\\Users\\will</WorkingDirectory>\r\n",
        ] {
            assert!(t.contains(part), "missing {part:?} in\n{t}");
        }
        assert!(t.ends_with("</Task>\r\n"));
        // Markup is escaped; a line break or an account XenoNet couldn't
        // tell is refused.
        let t = task(&Job { program: "C:\\A&B\\xenonet.exe".into(), env: vec![], ..win_job() }, SID, "C:\\Windows\\System32").unwrap();
        assert!(t.contains("<Arguments>--headless C:\\A&amp;B\\xenonet.exe service run</Arguments>"), "{t}");
        assert!(task(&Job { dir: "C:\\a\r\nb".into(), ..win_job() }, SID, "C:\\Windows\\System32").is_err());
        assert!(task(&win_job(), "", "C:\\Windows\\System32").unwrap_err().contains("whoami"));
        assert!(task(&win_job(), "S-1-5-x", "C:\\Windows\\System32").is_err());
    }

    #[test]
    fn windows_arguments_split_back_as_they_were() {
        for s in ["plain", "with space", "", "quote\"inside", "C:\\Program Files\\x\\", "a\\\\\"b", "trailing\\", "tab\there"] {
            assert_eq!(win_split(&format!("first {} last", win_arg(s))), vec!["first".to_string(), s.to_string(), "last".to_string()], "{s:?} as {}", win_arg(s));
        }
        assert_eq!(win_arg("C:\\Program Files\\x\\"), "\"C:\\Program Files\\x\\\\\"");
        assert_eq!(sid_of("\"desktop-1\\will\",\"S-1-5-21-1-2-3-1001\"\r\n").as_deref(), Some("S-1-5-21-1-2-3-1001"));
        assert_eq!(sid_of("ERROR: something"), None);
    }

    #[test]
    fn a_registration_says_its_port_program_and_settings() {
        assert_eq!(port_of(&plist(&job())), "47860");
        assert_eq!(port_of(&unit(&job()).unwrap()), "47860");
        assert_eq!(port_of(&task(&win_job(), SID, "C:\\W").unwrap()), "47850");
        assert_eq!(port_of(&plist(&Job { env: vec![], ..job() })), "47840");
        assert_eq!(port_of(&unit(&Job { env: vec![("XENONET_STATE_DIR".into(), "/s".into())], ..job() }).unwrap()), "47840");
        assert_eq!(port_of(&task(&Job { env: vec![], ..win_job() }, SID, "C:\\W").unwrap()), "47840");
        // What each was written with comes back out of it.
        let tricky = Job {
            program: "/opt/My Apps/x\"n%$&<".into(),
            env: vec![("XENONET_PORT".into(), "47850".into()), ("XDG_STATE_HOME".into(), "/s p/100% \"q\" \\x &<".into())],
            ..job()
        };
        for (kind, text) in [(Kind::Launchd, plist(&tricky)), (Kind::Systemd, unit(&tricky).unwrap()), (Kind::TaskScheduler, task(&tricky, SID, "C:\\W").unwrap())] {
            assert_eq!(program_of(kind, &text).as_deref(), Some(tricky.program.as_str()), "{kind:?}");
            assert_eq!(settings_of(kind, &text), tricky.env, "{kind:?}");
        }
        assert_eq!(settings_of(Kind::Launchd, &plist(&Job { env: vec![], ..job() })), vec![]);
        // Only XenoNet's own settings.
        assert_eq!(settings_of(Kind::Systemd, "Environment=\"PATH=/bin\"\nEnvironment=\"XENONET_HOSTS=1\"\n"), vec![("XENONET_HOSTS".to_string(), "1".to_string())]);
    }

    #[test]
    fn kept_settings_are_xenonets_own() {
        let env: HashMap<&str, &str> = [("XENONET_PORT", "47860"), ("XENONET_DEMO", "1"), ("XDG_STATE_HOME", "/s"), ("XENONET_STATE_DIR", ""), ("PATH", "/bin")].into();
        let kept = kept_env(|k| env.get(k).map(|v| (*v).to_string()));
        assert_eq!(kept, vec![("XENONET_PORT".to_string(), "47860".to_string()), ("XDG_STATE_HOME".to_string(), "/s".to_string())]);
    }

    fn env_of(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| ((*k).to_string(), json!(v))).collect()
    }

    #[test]
    fn the_job_runs_the_launcher_with_the_settings_that_arent_defaults() {
        let m = Machine { platform: "linux".into(), arch: "x64".into(), home: "/home/will".into() };
        let launcher = "/home/will/.local/state/xenonet/runtime/launcher/xenonet";
        let j = super::job(
            &env_of(&[("XENONET_PORT", "47840"), ("XENONET_RUNTIME_DIR", "/home/will/.local/state/xenonet/runtime"), ("XENONET_HOSTS", "1"), ("PATH", "/bin")]),
            &m,
            "/own/xenonet",
            |p| p == launcher,
        );
        assert_eq!(
            j,
            Job {
                program: launcher.into(),
                args: vec!["service".into(), "run".into()],
                env: vec![("XENONET_HOSTS".into(), "1".into())],
                dir: "/home/will".into(),
                log: "/home/will/.local/state/xenonet/runtime/service-47840.log".into()
            }
        );
        // No launcher: the binary itself, and another port's log.
        let j = super::job(&env_of(&[("XENONET_PORT", "47850")]), &m, "/own/xenonet", |_| false);
        assert_eq!(
            (j.program.as_str(), j.env.clone(), j.log.as_str()),
            ("/own/xenonet", vec![("XENONET_PORT".to_string(), "47850".to_string())], "/home/will/.local/state/xenonet/runtime/service-47850.log")
        );
    }

    #[test]
    fn the_doctor_sees_whether_a_registration_is_current() {
        let m = Machine { platform: "darwin".into(), arch: "arm64".into(), home: "/Users/will".into() };
        let launcher = "/Users/will/Library/Application Support/XenoNet/runtime/launcher/xenonet";
        let there = |p: &str| p == launcher;
        let (place, mut files, mut run) = (mac(), Mem::default(), Fake::default());
        let r = review(Some(&place), &m, &Env::new(), &files, &mut run, there);
        assert_eq!((r["registered"].clone(), r.get("current")), (json!(false), None));
        // As install writes it now.
        let written = super::job(&env_of(&[("XENONET_PORT", "47850")]), &m, "/x", there);
        install(&place, &written, &mut files, &mut run).unwrap();
        let r = review(Some(&place), &m, &Env::new(), &files, &mut run, there);
        assert_eq!((r["current"].clone(), r["program"].clone(), r["programFound"].clone(), r["loaded"].clone()), (json!(true), json!(launcher), json!(true), json!(true)));
        // From an older XenoNet, or pointing at another binary while the
        // launcher is there: not current.
        files.write(&place.file, &plist(&written).replace("<integer>10</integer>", "<integer>30</integer>")).unwrap();
        assert_eq!(review(Some(&place), &m, &Env::new(), &files, &mut run, there)["current"], json!(false));
        files.write(&place.file, &plist(&Job { program: "/gone/xenonet".into(), ..written.clone() })).unwrap();
        let r = review(Some(&place), &m, &Env::new(), &files, &mut run, there);
        assert_eq!((r["current"].clone(), r["programFound"].clone()), (json!(false), json!(false)));
        // With no launcher (a build of your own), its own binary is current.
        let own = super::job(&env_of(&[]), &m, "/build/xenonet", |_| false);
        files.write(&place.file, &plist(&own)).unwrap();
        assert_eq!(review(Some(&place), &m, &Env::new(), &files, &mut run, |p| p == "/build/xenonet")["current"], json!(true));
        // Linux says whether there's a graphical session.
        let (place, mut run) = (linux(), Fake { display: true, ..Fake::default() });
        let r = review(Some(&place), &m, &Env::new(), &Mem::default(), &mut run, |_| false);
        assert_eq!((r["graphical"].clone(), r["linger"].clone(), r["user"].clone()), (json!(true), json!(false), json!("will")));
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
    fn task_scheduler_creates_runs_and_deletes_the_task() {
        let (place, mut files, mut run) = (windows(), Mem::default(), Fake::default());
        let query = "C:\\Windows\\System32\\schtasks.exe /query /tn XenoNet Service /xml ONE";
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["supervisor"].clone(), st["registered"].clone(), st["loaded"].clone(), st.get("error")), (json!("Task Scheduler"), json!(false), json!(false), None));

        run.calls.clear();
        let r = install(&place, &win_job(), &mut files, &mut run).unwrap();
        assert_eq!(r, json!({ "status": "installed", "supervisor": "Task Scheduler", "file": place.file }));
        assert_eq!(files.read(&place.file).unwrap(), task(&win_job(), SID, "C:\\Windows\\System32").unwrap());
        assert_eq!(
            run.calls,
            vec![
                query,
                "C:\\Windows\\System32\\schtasks.exe /create /tn XenoNet Service /xml C:\\Users\\will\\AppData\\Local\\XenoNet\\xenonet-service.xml /f",
                "C:\\Windows\\System32\\schtasks.exe /run /tn XenoNet Service"
            ]
        );
        // Loaded, with service run's processes as it keeps them: its own,
        // waiting for the port, then the service's.
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["loaded"].clone(), st["state"].clone(), st["pid"].clone()), (json!(true), json!("not running"), json!(null)));
        files.write(&place.pids(), &pids_text(300, None, None)).unwrap();
        run.alive = vec![300];
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["state"].clone(), st["pid"].clone()), (json!("waiting"), json!(300)));
        files.write(&place.pids(), &pids_text(300, Some(301), Some(1))).unwrap();
        run.alive = vec![300, 301];
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["state"].clone(), st["pid"].clone(), st["lastExit"].clone()), (json!("running"), json!(301), json!("1")));
        assert_eq!(port_of(&files.read(&place.file).unwrap()), "47850");
        // A file left by a runner that was ended says nothing.
        run.alive = vec![];
        assert_eq!(status(Some(&place), &files, &mut run)["pid"], json!(null));

        // Again, as it is: nothing changes.
        run.calls.clear();
        assert_eq!(install(&place, &win_job(), &mut files, &mut run).unwrap()["status"], json!("already"));
        assert_eq!(run.calls.len(), 2, "asked only: {:?}", run.calls);
        // Changed while running: the old copy ends, and the new one runs.
        run.alive = vec![300, 301];
        run.calls.clear();
        assert_eq!(install(&place, &Job { env: vec![], ..win_job() }, &mut files, &mut run).unwrap()["status"], json!("updated"));
        let named: Vec<&str> = run.calls.iter().filter(|c| c.contains("schtasks")).map(|c| c.split(' ').nth(1).unwrap_or("")).collect();
        assert_eq!(named, ["/query", "/create", "/end", "/query", "/run"], "{:?}", run.calls);

        // Started: run, which leaves a running copy alone.
        run.calls.clear();
        start(&place, &files, &mut run).unwrap();
        assert_eq!(run.calls.last().unwrap(), "C:\\Windows\\System32\\schtasks.exe /run /tn XenoNet Service");

        run.calls.clear();
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("removed"));
        assert!(
            run.calls.ends_with(&[
                "C:\\Windows\\System32\\schtasks.exe /end /tn XenoNet Service".to_string(),
                "C:\\Windows\\System32\\schtasks.exe /delete /tn XenoNet Service /f".to_string()
            ]),
            "{:?}",
            run.calls
        );
        assert_eq!((files.read(&place.file), files.read(&place.pids())), (None, None));
        run.calls.clear();
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("absent"));
        assert_eq!(run.calls, vec![query], "only asked");
        assert!(start(&place, &files, &mut run).is_err(), "nothing to start");
    }

    #[test]
    fn task_scheduler_respects_a_task_turned_off_and_recreates_one_taken_away() {
        let (place, mut files) = (windows(), Mem::default());
        files.write(&place.file, &task(&win_job(), SID, "C:\\Windows\\System32").unwrap()).unwrap();
        // Turned off in Task Scheduler: not loaded, and not turned on again
        // by a start.
        let mut run = Fake { disabled: true, ..Fake::default() };
        let st = status(Some(&place), &files, &mut run);
        assert_eq!((st["loaded"].clone(), st["state"].clone()), (json!(false), json!("disabled")));
        assert!(start(&place, &files, &mut run).unwrap_err().contains("turned off in Task Scheduler"));
        assert!(!run.calls.iter().any(|c| c.contains("/run")));
        // Uninstalling takes it away all the same.
        assert_eq!(uninstall(&place, &mut files, &mut run).unwrap()["status"], json!("removed"));
        assert!(run.calls.iter().any(|c| c.contains("/delete")));
        // Deleted in Task Scheduler while the definition stays: a start
        // creates it again from the file.
        files.write(&place.file, &task(&win_job(), SID, "C:\\Windows\\System32").unwrap()).unwrap();
        let mut run = Fake::default();
        start(&place, &files, &mut run).unwrap();
        let named: Vec<&str> = run.calls.iter().map(|c| c.split(' ').nth(1).unwrap_or("")).collect();
        assert_eq!(named, ["/query", "/create", "/run"]);
        // schtasks that can't run says so.
        let mut run = Fake { fail: Some(("/query", Ran { code: None, out: String::new(), err: "schtasks.exe couldn't start".into() })), ..Fake::default() };
        assert_eq!(status(Some(&place), &files, &mut run)["error"], json!("schtasks.exe couldn't start"));
    }

    #[test]
    fn task_definitions_are_kept_as_utf16() {
        let text = task(&win_job(), SID, "C:\\W").unwrap();
        let bytes = utf16(&text);
        assert_eq!(&bytes[..4], &[0xff, 0xfe, b'<', 0]);
        assert_eq!(text_of(&bytes).as_deref(), Some(text.as_str()));
        assert_eq!(text_of(b"plain").as_deref(), Some("plain"));
        // schtasks /query /xml read as UTF-16 bytes still says enough.
        let ran = Ran { code: Some(0), out: "<Settings><Enabled>false</Enabled></Settings>".chars().flat_map(|c| [c, '\0']).collect(), err: String::new() };
        assert_eq!(parse_query(&ran).state.as_deref(), Some("disabled"));
    }

    #[test]
    fn service_run_restarts_a_failure_unless_it_keeps_failing() {
        let mut r = Restarts::default();
        let t0 = std::time::Instant::now();
        for i in 0..BURST {
            assert!(r.again(t0 + std::time::Duration::from_secs(u64::from(i))), "start {i}");
        }
        assert!(!r.again(t0 + std::time::Duration::from_secs(20)), "the 11th in 5 minutes");
        assert!(r.again(t0 + std::time::Duration::from_secs(u64::from(BURST_S) + 5)), "the window moved on");
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
        assert!(listed(&r(0, "\"xenonet.exe\",\"42\",\"Console\",\"1\",\"9 K\"\r\n"), 42));
        assert!(!listed(&r(0, "\"xenonet.exe\",\"420\",\"Console\",\"1\",\"9 K\"\r\n"), 42));
    }
}
