//! Real shells for the terminal drawer (server/terminals.js): the service
//! owns them, and a Desktop connection only reads their output, so they
//! keep running while Desktop restarts. A pty on macOS and Linux, ConPTY on
//! Windows (portable-pty, as node-pty is for the JS service). Each keeps
//! the last 1,048,576 characters it wrote, counted as JS counts a string's
//! length, so a reader's cursor means what it meant to the JS.

use crate::cores::{CoreError, Terminals, R};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use xenonet_base::sync::Locked;

const KEEP: usize = 1 << 20;
const MAX: usize = 64;

/// A shell a terminal can open.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub file: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn posix_shell(name: &str) -> (String, Vec<String>, Vec<(String, String)>) {
    let l = |a: &[&str]| a.iter().map(ToString::to_string).collect::<Vec<_>>();
    match name {
        "zsh" => ("zsh".into(), l(&["-l"]), vec![]),
        "bash" => ("bash".into(), l(&["-l"]), vec![("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into())]),
        "fish" => ("fish".into(), l(&["-l"]), vec![]),
        "pwsh" => ("PowerShell 7".into(), l(&["-NoLogo", "-Login"]), vec![]),
        "nu" => ("Nushell".into(), l(&["-l"]), vec![]),
        other => (other.into(), l(&["-l"]), vec![]),
    }
}

const POSIX_SHELLS: [&str; 5] = ["zsh", "bash", "fish", "pwsh", "nu"];
const POSIX_DIRS: [&str; 4] = ["/bin", "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

/// macOS and Linux: the login shell first (what a new Terminal window
/// opens), then the other common shells that are installed, all as login
/// shells.
pub fn posix_profiles(login_shell: Option<&str>, exists: &dyn Fn(&str) -> bool) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    let mut add = |file: &str| {
        let name = xenonet_base::paths::base_name(file);
        if out.iter().any(|p| p.id == name) || !exists(file) {
            return;
        }
        let (label, args, env) = posix_shell(&name);
        out.push(Profile { id: name, label, file: file.to_string(), args, env });
    };
    if let Some(s) = login_shell.filter(|s| !s.is_empty()) {
        add(s);
    }
    for name in POSIX_SHELLS {
        for dir in POSIX_DIRS {
            add(&format!("{dir}/{name}"));
        }
    }
    if out.is_empty() {
        out.push(Profile { id: "sh".into(), label: "sh".into(), file: "/bin/sh".into(), args: vec!["-l".into()], env: vec![] });
    }
    out
}

#[cfg(windows)]
fn windows_profiles() -> Vec<Profile> {
    use xenonet_base::process::Background as _;
    let run = |cmd: &str, args: &[&str]| std::process::Command::new(cmd).no_window().args(args).output().ok().filter(|o| o.status.success());
    let which = |cmd: &str| run("where.exe", &[cmd]).and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find(|l| !l.is_empty()).map(str::to_string));
    let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let exists = |f: &str| Path::new(f).exists();
    let mut out = Vec::new();
    let p = |id: &str, label: &str, file: String, args: &[&str]| Profile {
        id: id.into(),
        label: label.into(),
        file,
        args: args.iter().map(ToString::to_string).collect(),
        env: vec![],
    };
    if let Some(pwsh) = which("pwsh").or_else(|| Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe".to_string()).filter(|f| exists(f))) {
        out.push(p("pwsh", "PowerShell 7", pwsh, &["-NoLogo"]));
    }
    let winps = format!("{sys}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    if exists(&winps) {
        out.push(p("powershell", "Windows PowerShell", winps, &["-NoLogo"]));
    }
    out.push(p("cmd", "Command Prompt", std::env::var("ComSpec").unwrap_or_else(|_| format!("{sys}\\System32\\cmd.exe")), &[]));
    if let Some(git) = ["C:\\Program Files\\Git\\bin\\bash.exe", "C:\\Program Files (x86)\\Git\\bin\\bash.exe"].into_iter().find(|f| exists(f)) {
        out.push(p("gitbash", "Git Bash", git.into(), &["--login", "-i"]));
    }
    let wsl = format!("{sys}\\System32\\wsl.exe");
    if exists(&wsl) {
        if let Some(o) = run(&wsl, &["-l", "-q"]) {
            // wsl -l writes UTF-16; Docker Desktop's own distros aren't shells.
            let units: Vec<u16> = o.stdout.chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let text = String::from_utf16_lossy(&units).replace('\0', "");
            if let Some(d) = text.lines().map(str::trim).find(|d| !d.is_empty() && !d.starts_with("docker-desktop")) {
                out.push(p("wsl", &format!("WSL · {d}"), wsl.clone(), &[]));
            }
        }
    }
    out
}

pub fn detect_profiles() -> Vec<Profile> {
    #[cfg(windows)]
    {
        windows_profiles()
    }
    #[cfg(not(windows))]
    {
        let login = std::env::var("SHELL").ok();
        posix_profiles(login.as_deref(), &|f| Path::new(f).exists())
    }
}

/// What XenoNet sets for its own processes, which a shell it opens mustn't
/// pass on.
pub const OWN_ENV: [&str; 13] = [
    "XENONET_PORT",
    "XENONET_STANDALONE",
    "XENONET_RUNTIME_DIR",
    "XENONET_LAUNCHER_EXEC",
    "XENONET_LAUNCHER_ROOT",
    "XENONET_CONTROL",
    "XENONET_EXEC",
    "XENONET_NAME",
    "XENONET_HOT",
    "XENONET_INSTANCE",
    "XENONET_STATE_DIR",
    "XENONET_CACHE_DIR",
    "XENONET_DEMO",
];

/// A terminal's environment: yours, the profile's, the terminal's own, and
/// none of XenoNet's (unless the profile sets one itself).
pub fn shell_env(extra: &[(String, String)], base: &[(String, String)]) -> Vec<(String, String)> {
    let mut env: BTreeMap<String, String> = base.iter().cloned().collect();
    for (k, v) in extra {
        env.insert(k.clone(), v.clone());
    }
    for (k, v) in [("TERM_PROGRAM", "XenoNet"), ("COLORTERM", "truecolor"), ("TERM", "xterm-256color")] {
        env.insert(k.into(), v.into());
    }
    let given = |k: &str| extra.iter().any(|(e, _)| e == k);
    env.retain(|k, _| !(k.starts_with("ELECTRON_") || k == "NODE_OPTIONS" || (OWN_ENV.contains(&k.as_str()) && !given(k))));
    env.into_iter().collect()
}

fn safe_cwd(cwd: Option<&str>) -> String {
    match cwd {
        Some(c) if Path::new(c).is_dir() => c.to_string(),
        _ => crate::config::home_dir(&|k| std::env::var(k).ok()),
    }
}

/// Output kept as JS keeps a string: its length in UTF-16 units.
#[derive(Default)]
struct Output {
    text: String,
    units: usize,
    end: usize,
}

impl Output {
    fn push(&mut self, s: &str) {
        let n = s.encode_utf16().count();
        self.text.push_str(s);
        self.units += n;
        self.end += n;
        if self.units > KEEP {
            // slice(-KEEP): drop the oldest characters.
            let mut drop = self.units - KEEP;
            let mut cut = 0;
            for (i, c) in self.text.char_indices() {
                if drop == 0 {
                    cut = i;
                    break;
                }
                drop = drop.saturating_sub(c.len_utf16());
                cut = i + c.len_utf8();
            }
            self.text.drain(..cut);
            self.units = self.text.encode_utf16().count();
        }
    }

    /// What's after `cursor`, a count of every unit ever written.
    fn after(&self, cursor: usize) -> String {
        let start = self.end - self.units;
        let mut skip = cursor.saturating_sub(start);
        let mut at = self.text.len();
        for (i, c) in self.text.char_indices() {
            if skip == 0 {
                at = i;
                break;
            }
            skip = skip.saturating_sub(c.len_utf16());
        }
        if skip > 0 {
            return String::new();
        }
        self.text[at..].to_string()
    }
}

struct Term {
    id: u64,
    pid: Option<u32>,
    cwd: String,
    profile: Profile,
    cols: u16,
    rows: u16,
    out: Arc<Mutex<Output>>,
    // None while it runs, then its exit code.
    exited: Arc<Mutex<Option<u32>>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

impl Term {
    // Whether it's gone, and how it ended.
    fn refresh(&mut self) {
        let mut exited = self.exited.locked();
        if exited.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                *exited = Some(status.exit_code());
            }
        }
    }

    fn info(&self) -> Value {
        json!({ "id": self.id, "pid": self.pid, "cwd": self.cwd, "profile": self.profile.id, "label": self.profile.label, "exited": self.exited.locked().is_some(), "cols": self.cols, "rows": self.rows })
    }
}

pub struct Shells {
    profiles: Vec<Profile>,
    terms: Mutex<BTreeMap<u64, Term>>,
    next: Mutex<u64>,
}

impl Shells {
    pub fn new() -> Shells {
        Shells { profiles: detect_profiles(), terms: Mutex::new(BTreeMap::new()), next: Mutex::new(1) }
    }
}

impl Default for Shells {
    fn default() -> Self {
        Shells::new()
    }
}

fn int(v: Option<&Value>, d: f64) -> f64 {
    match v {
        None | Some(Value::Null) => d,
        // JS's `x | 0`.
        Some(v) => {
            let n = xenonet_base::text::js_number(Some(v));
            if n.is_finite() {
                n.trunc()
            } else {
                0.0
            }
        }
    }
}

fn gone() -> CoreError {
    CoreError::new("That terminal has closed.")
}

impl Terminals for Shells {
    fn profiles(&self) -> R<Value> {
        Ok(Value::Array(self.profiles.iter().map(|p| json!({ "id": p.id, "label": p.label, "file": p.file })).collect()))
    }

    fn list(&self) -> Value {
        let mut terms = self.terms.locked();
        Value::Array(
            terms
                .values_mut()
                .map(|t| {
                    t.refresh();
                    t.info()
                })
                .collect(),
        )
    }

    fn create(&self, params: &Value) -> R<Value> {
        if self.terms.locked().len() >= MAX {
            return Err(CoreError::new("Close a terminal before opening another (64 maximum)."));
        }
        let wanted = params.get("profile").and_then(Value::as_str);
        let profile =
            self.profiles.iter().find(|p| Some(p.id.as_str()) == wanted).or(self.profiles.first()).cloned().ok_or_else(|| CoreError::new("XenoNet found no shell to open."))?;
        let cwd = safe_cwd(params.get("cwd").and_then(Value::as_str));
        let cols = int(params.get("cols"), 120.0).max(2.0).min(f64::from(u16::MAX)) as u16;
        let rows = int(params.get("rows"), 30.0).max(2.0).min(f64::from(u16::MAX)) as u16;
        let pair = native_pty_system().openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }).map_err(|e| CoreError::new(e.to_string()))?;
        let mut cmd = CommandBuilder::new(&profile.file);
        cmd.args(&profile.args);
        cmd.cwd(&cwd);
        cmd.env_clear();
        let base: Vec<(String, String)> = std::env::vars().collect();
        for (k, v) in shell_env(&profile.env, &base) {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).map_err(|e| CoreError::new(e.to_string()))?;
        drop(pair.slave);
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader().map_err(|e| CoreError::new(e.to_string()))?;
        let writer = pair.master.take_writer().map_err(|e| CoreError::new(e.to_string()))?;
        let out = Arc::new(Mutex::new(Output::default()));
        let exited: Arc<Mutex<Option<u32>>> = Arc::new(Mutex::new(None));
        let id = {
            let mut n = self.next.locked();
            let id = *n;
            *n += 1;
            id
        };
        // Its output, decoded as UTF-8 across reads, as node-pty's is.
        let o = out.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 16 << 10];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let good = match std::str::from_utf8(&pending) {
                            Ok(_) => pending.len(),
                            Err(e) if e.error_len().is_none() => e.valid_up_to(),
                            Err(_) => pending.len(),
                        };
                        let text = String::from_utf8_lossy(&pending[..good]).into_owned();
                        pending.drain(..good);
                        if !text.is_empty() {
                            o.locked().push(&text);
                        }
                    }
                }
            }
        });
        let term = Term { id, pid, cwd, profile, cols, rows, out, exited, master: pair.master, writer, child };
        let info = term.info();
        self.terms.locked().insert(id, term);
        Ok(info)
    }

    fn read(&self, params: &Value) -> R<Value> {
        let id = int(params.get("id"), -1.0) as u64;
        let mut terms = self.terms.locked();
        let t = terms.get_mut(&id).ok_or_else(gone)?;
        t.refresh();
        let cursor = int(params.get("cursor"), 0.0).max(0.0) as usize;
        let out = t.out.locked();
        let start = out.end - out.units;
        let mut info = t.info();
        info["data"] = json!(out.after(cursor));
        info["cursor"] = json!(out.end);
        info["truncated"] = json!(cursor < start);
        info["exitCode"] = t.exited.locked().map_or(Value::Null, |c| json!(c));
        Ok(info)
    }

    fn write(&self, params: &Value) -> R<()> {
        let Some(data) = params.get("data").and_then(Value::as_str).filter(|d| d.encode_utf16().count() <= 1 << 20) else {
            return Err(CoreError::new("Invalid terminal input."));
        };
        let id = int(params.get("id"), -1.0) as u64;
        let mut terms = self.terms.locked();
        let t = terms.get_mut(&id).ok_or_else(gone)?;
        t.writer.write_all(data.as_bytes()).map_err(|e| CoreError::new(e.to_string()))?;
        let _ = t.writer.flush();
        Ok(())
    }

    fn resize(&self, params: &Value) -> R<()> {
        let id = int(params.get("id"), -1.0) as u64;
        let mut terms = self.terms.locked();
        let t = terms.get_mut(&id).ok_or_else(gone)?;
        t.cols = int(params.get("cols"), 0.0).clamp(2.0, 1000.0) as u16;
        t.rows = int(params.get("rows"), 0.0).clamp(2.0, 1000.0) as u16;
        if t.exited.locked().is_none() {
            let _ = t.master.resize(PtySize { rows: t.rows, cols: t.cols, pixel_width: 0, pixel_height: 0 });
        }
        Ok(())
    }

    fn kill(&self, id: &Value) -> R<()> {
        let id = int(Some(id), -1.0) as u64;
        if let Some(mut t) = self.terms.locked().remove(&id) {
            let _ = t.child.kill();
        }
        Ok(())
    }

    fn kill_all(&self) {
        for (_, mut t) in std::mem::take(&mut *self.terms.locked()) {
            let _ = t.child.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // test/terminal-env.test.js's case.
    #[test]
    fn a_terminals_shell_gets_your_environment_without_xenonets_own() {
        let mut base: Vec<(String, String)> =
            vec![("PATH", "/usr/bin"), ("HOME", "/Users/nova"), ("XENONET_PROJECTS_DIR", "/Users/nova/Code"), ("ELECTRON_RUN_AS_NODE", "1"), ("NODE_OPTIONS", "--inspect")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        base.extend(OWN_ENV.iter().map(|k| (k.to_string(), "x".to_string())));
        let env = shell_env(&[], &base);
        let get = |e: &[(String, String)], k: &str| e.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert!(OWN_ENV.iter().all(|k| get(&env, k).is_none()), "none of XenoNet's own");
        assert_eq!((get(&env, "ELECTRON_RUN_AS_NODE"), get(&env, "NODE_OPTIONS")), (None, None));
        assert_eq!(
            [get(&env, "PATH"), get(&env, "HOME"), get(&env, "XENONET_PROJECTS_DIR"), get(&env, "TERM_PROGRAM")],
            [Some("/usr/bin".into()), Some("/Users/nova".into()), Some("/Users/nova/Code".into()), Some("XenoNet".into())]
        );
        assert_eq!(get(&shell_env(&[("XENONET_PORT".into(), "47842".into())], &base), "XENONET_PORT").as_deref(), Some("47842"), "unless the profile sets one itself");
    }

    #[test]
    fn the_login_shell_first_then_the_others_installed() {
        let have = ["/bin/zsh", "/opt/homebrew/bin/fish", "/bin/bash", "/usr/bin/bash"];
        let p = posix_profiles(Some("/opt/homebrew/bin/fish"), &|f| have.contains(&f));
        let ids: Vec<&str> = p.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, vec!["fish", "zsh", "bash"]);
        assert_eq!(p[2].env, vec![("BASH_SILENCE_DEPRECATION_WARNING".to_string(), "1".to_string())]);
        assert_eq!(posix_profiles(None, &|_| false)[0].file, "/bin/sh");
    }

    #[test]
    fn output_is_kept_and_read_as_js_counts_it() {
        let mut o = Output::default();
        o.push("héllo ");
        o.push("😀!");
        assert_eq!((o.end, o.after(0).as_str(), o.after(6).as_str()), (9, "héllo 😀!", "😀!"));
        let big = "x".repeat(KEEP);
        o.push(&big);
        assert_eq!(o.units, KEEP);
        assert_eq!(o.after(o.end - 3), "xxx");
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_runs_writes_and_closes() {
        let s = Shells::new();
        let t = s.create(&json!({ "cwd": "/", "cols": 80, "rows": 24 })).unwrap();
        let id = t["id"].clone();
        assert_eq!((t["cols"].clone(), t["rows"].clone(), t["cwd"].clone(), t["exited"].clone()), (json!(80), json!(24), json!("/"), json!(false)));
        s.write(&json!({ "id": id, "data": "echo xenonet-$((40+2))\nexit\n" })).unwrap();
        let mut seen = String::new();
        for _ in 0..200 {
            let r = s.read(&json!({ "id": id, "cursor": 0 })).unwrap();
            seen = r["data"].as_str().unwrap().to_string();
            if seen.contains("xenonet-42") && r["exited"] == json!(true) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(seen.contains("xenonet-42"), "{seen}");
        assert!(s.write(&json!({ "id": id, "data": 5 })).is_err());
        s.kill(&id).unwrap();
        assert_eq!(s.read(&json!({ "id": id })).unwrap_err().message, "That terminal has closed.");
        assert_eq!(s.list(), json!([]));
    }
}
