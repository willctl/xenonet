//! Everything the migration does outside files goes through `Sys`: service managers (launchctl,
//! systemctl, sc, schtasks, reg, icacls), the apps' CLIs (claude, codex), agent-wiki's tray, the
//! process list, ports, and `xenolith install`. The real one acts on this machine; the sandbox one
//! (--sandbox) only records what it would run and reads stand-ins from the fake home; the tests
//! have their own. No other code in migrate/ starts a program.

use crate::install::sys::{Ran, get_json, run_in, which};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A process: its id and its command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: i64,
    pub command: String,
}

pub trait Sys {
    /// "macos", "linux" or "windows": which layout and which service manager.
    fn os(&self) -> &str;
    /// Runs a program and waits. `program` is a bare name for the system's own tools and the apps'
    /// CLIs, or a path for agent-wiki's programs.
    fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Ran;
    /// Starts a program without waiting (a tray app).
    fn spawn(&self, program: &Path, args: &[&str]);
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Whether the Claude desktop app runs (it writes its config back over an edit).
    fn desktop_running(&self) -> bool;
    fn processes(&self) -> Vec<Proc>;
    /// Whether something listens on 127.0.0.1:<port>, and its /health when it answers one.
    fn port(&self, port: u16) -> (bool, Option<Value>);
    /// GET http://127.0.0.1:<port><path>.
    fn get_json(&self, port: u16, path: &str) -> Option<Value>;
    /// The installed server over stdio: whether context_start and wiki_start answer.
    fn mcp_start(&self, agent: &Path, env: &[(String, String)]) -> Result<(bool, bool), String>;
    /// This program with these arguments (install, uninstall), its output captured.
    fn xenolith(&self, args: &[String]) -> Ran;
    fn uid(&self) -> u32;
    fn sleep(&self, ms: u64);
    /// Windows: the app package this terminal runs inside, whose AppData writes it captures.
    fn packaged(&self) -> Option<String> {
        None
    }
}

const LONG: Duration = Duration::from_secs(600);

/// A system tool by name: its full path where the name alone could find another program (Git for
/// Windows puts its own tools first on PATH), else the name.
fn tool(name: &Path) -> PathBuf {
    let n = name.to_string_lossy();
    if name.components().count() > 1 {
        return name.to_path_buf();
    }
    if cfg!(windows) && ["sc", "schtasks", "reg", "icacls", "whoami"].contains(&n.as_ref()) {
        return PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join("System32").join(format!("{n}.exe"));
    }
    if cfg!(target_os = "macos") && n == "launchctl" {
        return PathBuf::from("/bin/launchctl");
    }
    which(&n).unwrap_or_else(|| name.to_path_buf())
}

pub struct RealSys;

impl Sys for RealSys {
    fn os(&self) -> &str {
        std::env::consts::OS
    }
    fn run(&self, program: &Path, args: &[&str], env: &[(&str, &str)]) -> Ran {
        run_in(&tool(program), args, None, env, Duration::from_secs(120))
    }
    fn spawn(&self, program: &Path, args: &[&str]) {
        let mut c = std::process::Command::new(program);
        c.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        let _ = c.spawn();
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        which(name)
    }
    fn desktop_running(&self) -> bool {
        crate::install::sys::claude_desktop_running()
    }
    fn processes(&self) -> Vec<Proc> {
        real_processes()
    }
    fn port(&self, port: u16) -> (bool, Option<Value>) {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let busy = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok();
        (busy, if busy { get_json(port, "/health") } else { None })
    }
    fn get_json(&self, port: u16, path: &str) -> Option<Value> {
        get_json(port, path)
    }
    fn mcp_start(&self, agent: &Path, _env: &[(String, String)]) -> Result<(bool, bool), String> {
        mcp_starts(agent, &[])
    }
    fn xenolith(&self, args: &[String]) -> Ran {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("xenolith"));
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        run_in(&exe, &a, None, &[], LONG)
    }
    fn uid(&self) -> u32 {
        #[cfg(unix)]
        {
            // SAFETY: getuid has no failure mode.
            unsafe { libc::getuid() }
        }
        #[cfg(not(unix))]
        {
            0
        }
    }
    fn sleep(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    fn packaged(&self) -> Option<String> {
        #[cfg(windows)]
        {
            crate::install::windows::packaged()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

/// context_start through the installer's own check, then wiki_start (the old name) on its own.
fn mcp_starts(agent: &Path, env: &[(String, String)]) -> Result<(bool, bool), String> {
    let mut c = std::process::Command::new(agent);
    c.arg("serve").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    for (k, _) in std::env::vars() {
        if k.starts_with("XENOLITH_") || k.starts_with("AGENT_WIKI_") {
            c.env_remove(k);
        }
    }
    for (k, v) in env {
        c.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().map_err(|e| format!("cannot start {}: {e}", agent.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut lines = std::io::BufRead::lines(std::io::BufReader::new(child.stdout.take().ok_or("no stdout")?));
    let mut ask = |msg: &str| -> Result<Value, String> {
        writeln!(stdin, "{msg}").map_err(|e| e.to_string())?;
        let line = lines.next().ok_or("the server closed stdout")?.map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    };
    let init = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"xenolith-migrate","version":"{}"}}}}}}"#,
        xenolith_core::VERSION
    );
    ask(&init)?;
    let call = |id: u32, name: &str| format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{{"app":"migrate","topic":"migration"}}}}}}"#);
    let ok = |v: &Value| v["result"].is_object() && v["result"]["isError"] != true;
    let a = ask(&format!("{}\n{}", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, call(2, "context_start")))?;
    let b = ask(&call(3, "wiki_start"))?;
    drop(stdin);
    let _ = child.wait();
    Ok((ok(&a), ok(&b)))
}

/// The process list: `ps` on macOS and Linux, CIM on Windows.
fn real_processes() -> Vec<Proc> {
    if cfg!(windows) {
        let ps = PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId)`t$($_.ExecutablePath) $($_.CommandLine)\" }";
        let r = run_in(&ps, &["-NoProfile", "-NonInteractive", "-Command", script], None, &[], Duration::from_secs(60));
        parse_ps(&r.stdout, '\t')
    } else {
        let r = run_in(Path::new("/bin/ps"), &["-axww", "-o", "pid=,command="], None, &[], Duration::from_secs(30));
        parse_ps(&r.stdout, ' ')
    }
}

pub fn parse_ps(text: &str, sep: char) -> Vec<Proc> {
    let me = std::process::id() as i64;
    text.lines()
        .filter_map(|l| {
            let (pid, cmd) = l.trim_start().split_once(sep)?;
            Some(Proc { pid: pid.trim().parse().ok()?, command: cmd.trim().to_string() })
        })
        .filter(|p| p.pid != me && !p.command.contains("Get-CimInstance Win32_Process"))
        .collect()
}

/// --sandbox: files are real (in the fake home), the rest is stood in for. It never runs a
/// service manager or an app's CLI: it appends what it would run to <home>/.migrate-sys.log and
/// answers from files in the fake home: claude-desktop-running (the installer's own stand-in),
/// .fake/processes (one "pid command" per line), .fake/port-<n> (something listens), .fake/cli-<name>
/// (that CLI is installed), .fake/codex-signed-in. `xenolith install` and `uninstall` run for
/// real with --sandbox, which only ever touches the fake home.
pub struct SandboxSys {
    pub home: PathBuf,
    pub child_env: Vec<(String, String)>,
}

impl SandboxSys {
    fn fake(&self, name: &str) -> PathBuf {
        self.home.join(".fake").join(name)
    }
    /// What stopping a service or the tray would end: those lines leave .fake/processes.
    fn stopped(&self, program: &str, args: &[&str]) {
        let all = args.join(" ");
        let gone: &[&str] = if program.contains("tray") || program.contains("Tray") {
            if args.contains(&"--quit") { &["tray", "Tray", " curator"] } else { &[] }
        } else if (program == "launchctl" && args.first() == Some(&"bootout")) || (program == "systemctl" && (args.contains(&"stop") || args.contains(&"disable"))) {
            if all.contains("curator") {
                &[" curator"]
            } else if all.contains("tray") {
                &["tray"]
            } else {
                &["--http"]
            }
        } else if program == "sc" && args.first() == Some(&"stop") {
            &["AgentWikiService", " service --config"]
        } else {
            &[]
        };
        if gone.is_empty() {
            return;
        }
        let file = self.fake("processes");
        if let Ok(t) = std::fs::read_to_string(&file) {
            let kept: Vec<&str> = t.lines().filter(|l| !gone.iter().any(|g| l.contains(g))).collect();
            let _ = std::fs::write(&file, kept.iter().map(|l| format!("{l}\n")).collect::<String>());
        }
    }

    fn record(&self, line: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.home.join(".migrate-sys.log")) {
            let _ = writeln!(f, "{line}");
        }
    }
}

impl Sys for SandboxSys {
    fn os(&self) -> &str {
        std::env::consts::OS
    }
    fn run(&self, program: &Path, args: &[&str], _env: &[(&str, &str)]) -> Ran {
        self.record(&format!("{} {}", program.to_string_lossy(), args.join(" ")));
        let name = program.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.stopped(&name, args);
        // A query finds nothing registered; `codex login status` answers from the stand-in.
        let query = args.first().is_some_and(|a| ["query", "qc", "/query"].contains(a));
        let login = name.starts_with("codex") && args == ["login", "status"];
        let ok = if login { self.fake("codex-signed-in").exists() } else { !query };
        Ran { ok, code: Some(if ok { 0 } else { 1 }), stdout: String::new(), stderr: String::new() }
    }
    fn spawn(&self, program: &Path, args: &[&str]) {
        self.record(&format!("(start) {} {}", program.to_string_lossy(), args.join(" ")));
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        self.fake(&format!("cli-{name}")).exists().then(|| PathBuf::from(name))
    }
    fn desktop_running(&self) -> bool {
        self.home.join("claude-desktop-running").exists()
    }
    fn processes(&self) -> Vec<Proc> {
        std::fs::read_to_string(self.fake("processes")).map(|t| parse_ps(&t, ' ')).unwrap_or_default()
    }
    fn port(&self, port: u16) -> (bool, Option<Value>) {
        (self.fake(&format!("port-{port}")).exists(), None)
    }
    fn get_json(&self, _port: u16, _path: &str) -> Option<Value> {
        None
    }
    fn mcp_start(&self, agent: &Path, env: &[(String, String)]) -> Result<(bool, bool), String> {
        let mut all = self.child_env.clone();
        all.extend(env.iter().cloned());
        mcp_starts(agent, &all)
    }
    fn xenolith(&self, args: &[String]) -> Ran {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("xenolith"));
        let mut a: Vec<String> = args.to_vec();
        a.extend(["--sandbox".to_string(), self.home.to_string_lossy().into_owned()]);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        run_in(&exe, &a, None, &[], LONG)
    }
    fn uid(&self) -> u32 {
        501
    }
    fn sleep(&self, _ms: u64) {}
}
