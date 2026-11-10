//! OpenCode: every TUI runs its own HTTP server, on a random port unless
//! started with --port: core/delivery/opencode.js. XenoNet finds the
//! running opencode processes and the ports they listen on, asks each which
//! sessions it has, and posts the message to the session with
//! prompt_async, which starts a turn in the live TUI.
//!
//! The commands and files that find the servers are `System`'s; the clock,
//! the servers' processes and ports, and HTTP to them are `Host`'s.

use super::{number_text, template, truthy, Delivered, Result};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::sync::Mutex;
use xenonet_base::sync::Locked;

/// Commands and files, as finding OpenCode's servers reads them.
pub trait System {
    /// A command's output, or "" when it failed.
    fn run(&mut self, cmd: &str, args: &[&str]) -> String;
    /// A file's text, or None when it can't be read.
    fn read_file(&mut self, path: &str) -> Option<String>;
    /// A folder's entries' names, or None when it can't be read.
    fn readdir(&mut self, path: &str) -> Option<Vec<String>>;
    /// Where a link points, or None when it can't be read.
    fn readlink(&mut self, path: &str) -> Option<String>;
}

/// What a POST to prompt_async answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posted {
    pub ok: bool,
    pub status: f64,
}

/// The clock, the opencode processes and their ports, and HTTP to a
/// server.
pub trait Host {
    fn now(&mut self) -> f64;
    /// The opencode processes running (`open_code_processes`).
    fn processes(&mut self) -> Vec<Proc>;
    /// The TCP ports a process listens on (`listening_ports`).
    fn ports(&mut self, pid: f64) -> Vec<f64>;
    /// A GET's JSON, when it answered ok within `ms`; None otherwise.
    fn get(&mut self, url: &str, ms: f64) -> Option<Value>;
    /// A POST of JSON: what it answered, or why it couldn't be made.
    fn post(&mut self, url: &str, body: &str) -> std::result::Result<Posted, String>;
}

/// OpenCode's own id for XenoNet's (xenonet_base::opencode, as its reader
/// wrote it).
pub fn open_code_id(id: &str) -> String {
    xenonet_base::opencode::open_code_id(id)
}

/// XenoNet's id for a session named the way OpenCode shows it (ses_…, or
/// opencode:ses_…). Any other id comes back as it was.
pub fn xenonet_id(id: &str) -> String {
    let s = xenonet_base::text::js_trim(id);
    match crate::re!(r"^(?:opencode:)?(ses(?:_[0-9A-Za-z]+)+)$").captures(s) {
        Some(m) => format!("opencode:{}", xenonet_base::opencode::local_id(&m[1])),
        None => s.to_string(),
    }
}

/// A process: its pid and command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: f64,
    pub args: String,
}

pub fn parse_ps(out: &str) -> Vec<Proc> {
    out.split('\n')
        .filter_map(|l| crate::re!(r"^[\s\x{FEFF}--\x{85}]*([0-9]+)[\s\x{FEFF}--\x{85}]+([^\n\r\x{2028}\x{2029}]*)$").captures(l))
        .map(|m| Proc { pid: number(&m[1]), args: m[2].to_string() })
        .collect()
}

/// opencode processes: the command itself, not a shell or tmux that names
/// it in its arguments.
pub fn open_code_processes(sys: &mut dyn System, windows: bool) -> Vec<Proc> {
    if windows {
        let out = sys.run(
            "powershell.exe",
            &["-NoProfile", "-Command", "Get-CimInstance Win32_Process -Filter \"Name like 'opencode%'\" | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }"],
        );
        return parse_ps(&out);
    }
    let out = sys.run("ps", &["-axo", "pid=,args="]);
    parse_ps(&out)
        .into_iter()
        .filter(|p| {
            crate::re!(r"^([^\s\x{FEFF}--\x{85}]*[/\\])?opencode(\.exe)?([\s\x{FEFF}--\x{85}]|$)").is_match(&p.args)
                || crate::re!(r"^[^\s\x{FEFF}--\x{85}]*node[^\s\x{FEFF}--\x{85}]*[\s\x{FEFF}--\x{85}]+[^\s\x{FEFF}--\x{85}]*opencode-ai").is_match(&p.args)
        })
        .collect()
}

fn unique(v: Vec<f64>) -> Vec<f64> {
    let mut out: Vec<f64> = Vec::new();
    for x in v {
        if !out.iter().any(|y| *y == x || (x.is_nan() && y.is_nan())) {
            out.push(x);
        }
    }
    out
}

fn words(s: &str) -> Vec<&str> {
    crate::re!(r"[\s\x{FEFF}--\x{85}]+").split(xenonet_base::text::js_trim(s)).collect()
}

fn number(s: &str) -> f64 {
    xenonet_base::text::js_number(Some(&Value::String(s.to_string())))
}

/// parseInt(s, 16).
fn parse_hex(s: &str) -> f64 {
    let t = xenonet_base::text::js_trim(s);
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let digits: String = t.chars().take_while(char::is_ascii_hexdigit).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    digits.chars().fold(0.0, |n, d| n * 16.0 + f64::from(d.to_digit(16).expect("a hex digit, as taken above")))
}

/// The TCP ports a process listens on, on loopback or any address.
/// `platform` is the system as Node names it (darwin, linux, win32).
pub fn listening_ports(sys: &mut dyn System, pid: f64, platform: &str) -> Vec<f64> {
    let pid_text = number_text(pid);
    if platform == "win32" {
        let out = sys.run("netstat", &["-ano", "-p", "tcp"]);
        let mut ports = Vec::new();
        for l in out.split('\n') {
            let cols = words(l);
            if cols.get(3) == Some(&"LISTENING") && cols.get(4).map(|p| number(p)) == Some(pid) {
                ports.push(number(cols[1].rsplit(':').next().unwrap_or("")));
            }
        }
        return unique(ports);
    }
    if platform == "linux" {
        if let Some(fds) = sys.readdir(&format!("/proc/{pid_text}/fd")) {
            let mut inodes: Vec<String> = Vec::new();
            for fd in fds {
                if let Some(link) = sys.readlink(&format!("/proc/{pid_text}/fd/{fd}")) {
                    if let Some(m) = crate::re!(r"^socket:\[([0-9]+)\]$").captures(&link) {
                        inodes.push(m[1].to_string());
                    }
                }
            }
            let mut ports = Vec::new();
            for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
                let text = sys.read_file(f).unwrap_or_default();
                for line in text.split('\n').skip(1) {
                    let cols = words(line);
                    if cols.get(3) == Some(&"0A") && cols.get(9).is_some_and(|i| inodes.iter().any(|n| n == i)) {
                        ports.push(parse_hex(cols[1].split(':').nth(1).unwrap_or("")));
                    }
                }
            }
            if !ports.is_empty() {
                return unique(ports);
            }
        }
    }
    let out = sys.run("lsof", &["-nP", "-a", "-p", &pid_text, "-iTCP", "-sTCP:LISTEN", "-Fn"]);
    unique(regex_lsof().captures_iter(&out).map(|m| number(&m[1])).collect())
}

/// /^n.*:(\d+)$/gm, with \r a line's end as JS's m flag has it.
fn regex_lsof() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(&format!(r"(?mR)^n{}*:([0-9]+)$", xenonet_base::text::JS_DOT)).expect("a fixed pattern"))
}

/// One folder, as OpenCode spells it in two places: the same once a
/// trailing slash goes, and on Windows whatever the case or the slashes.
pub fn same_path(a: &str, b: &str, windows: bool) -> bool {
    let norm = |p: &str| {
        let s = p.trim_end_matches(['\\', '/']).to_string();
        if windows {
            s.replace('\\', "/").to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

/// encodeURIComponent.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || "-_.!~*'()".contains(ch) {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// A server: its process, where it answers, and the folder it was started
/// in, as it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub pid: f64,
    pub base: String,
    pub dir: Option<String>,
}

#[derive(Default)]
struct Inner {
    cache_at: f64,
    servers: Vec<Server>,
    /// ses_… → (when, what OpenCode said), the oldest first.
    infos: Vec<(String, f64, Option<Value>)>,
}

pub struct OpenCode {
    cache_ms: f64,
    inner: Mutex<Inner>,
}

impl OpenCode {
    pub fn new(cache_ms: f64) -> Self {
        OpenCode { cache_ms, inner: Mutex::new(Inner { cache_at: f64::NEG_INFINITY, ..Inner::default() }) }
    }

    /// The OpenCode servers running now.
    pub fn servers(&self, h: &mut dyn Host) -> Vec<Server> {
        let t = h.now();
        {
            let i = self.inner.locked();
            if t - i.cache_at < self.cache_ms {
                return i.servers.clone();
            }
        }
        let mut found: Vec<Server> = Vec::new();
        for p in h.processes() {
            let ports = match crate::re!(r"--port[= ]([0-9]+)").captures(&p.args) {
                Some(m) => vec![number(&m[1])],
                None => h.ports(p.pid),
            };
            for port in ports {
                let base = format!("http://127.0.0.1:{}", number_text(port));
                if found.iter().any(|f| f.base == base) {
                    continue;
                }
                let Some(health) = h.get(&format!("{base}/global/health"), 800.0) else { continue };
                if !truthy(health.get("healthy")) {
                    continue;
                }
                let dir = h.get(&format!("{base}/path"), 800.0).and_then(|x| x.get("directory").and_then(Value::as_str).map(str::to_string));
                found.push(Server { pid: p.pid, base, dir });
            }
        }
        let at = h.now();
        let mut i = self.inner.locked();
        i.cache_at = at;
        i.servers.clone_from(&found);
        found
    }

    /// The server whose TUI has the session in hand: the one that reports
    /// a status for it, else one started in the session's folder, else any
    /// that knows it.
    pub fn server_for(&self, h: &mut dyn Host, id: &str, windows: bool) -> Option<Server> {
        let ses = open_code_id(id);
        let (mut here, mut known) = (None, None);
        for s in self.servers(h) {
            let Some(status) = h.get(&format!("{}/session/status", s.base), 1500.0) else { continue };
            let has = match &status {
                Value::Object(f) => f.contains_key(&ses),
                Value::Array(l) => ses.parse::<usize>().is_ok_and(|i| i < l.len() && i.to_string() == ses),
                _ => false,
            };
            if truthy(Some(&status)) && has {
                return Some(s);
            }
            if here.is_none() {
                let Some(info) = h.get(&format!("{}/session/{}", s.base, encode_uri_component(&ses)), 1500.0) else { continue };
                if known.is_none() {
                    known = Some(s.clone());
                }
                let dir = info.get("directory").filter(|d| truthy(Some(d))).map(template);
                if let (Some(dir), Some(sdir)) = (dir, s.dir.as_deref().filter(|d| !d.is_empty())) {
                    if same_path(&dir, sdir, windows) {
                        here = Some(s);
                    }
                }
            }
        }
        here.or(known)
    }

    /// What OpenCode says of a session: its title, its folder, when it last
    /// changed, and whether a turn is under way; None when no running
    /// OpenCode knows it. Each answer is kept for the cache's while.
    pub fn info(&self, h: &mut dyn Host, id: &str, windows: bool) -> Option<Value> {
        let ses = open_code_id(id);
        let hit = self.inner.locked().infos.iter().find(|(k, _, _)| *k == ses).map(|(_, at, v)| (*at, v.clone()));
        if let Some((at, v)) = hit {
            if h.now() - at < self.cache_ms {
                return v;
            }
        }
        let mut value = None;
        if let Some(s) = self.server_for(h, id, windows) {
            let session = h.get(&format!("{}/session/{}", s.base, encode_uri_component(&ses)), 1500.0);
            let status = h.get(&format!("{}/session/status", s.base), 1500.0);
            if let (Some(session), Some(status)) = (session, status) {
                let st = status.get(&ses).and_then(|s| s.get("type"));
                let updated = session.get("time").and_then(|t| t.get("updated"));
                let updated = match updated {
                    None => Value::Null,
                    Some(u) => {
                        let n = xenonet_base::text::js_number(Some(u));
                        if n == 0.0 || n.is_nan() {
                            Value::Null
                        } else {
                            xenonet_base::json::number(n)
                        }
                    }
                };
                value = Some(json!({
                    "title": session.get("title").filter(|t| t.is_string()).cloned().unwrap_or(Value::Null),
                    "directory": session.get("directory").cloned().unwrap_or(Value::Null),
                    "updated": updated,
                    "busy": truthy(st) && st.and_then(Value::as_str) != Some("idle"),
                }));
            }
        }
        let at = h.now();
        let mut i = self.inner.locked();
        i.infos.retain(|(k, _, _)| *k != ses);
        i.infos.push((ses, at, value.clone()));
        if i.infos.len() > 256 {
            i.infos.remove(0);
        }
        value
    }

    pub fn forget(&self) {
        let mut i = self.inner.locked();
        i.cache_at = f64::NEG_INFINITY;
        i.servers.clear();
        i.infos.clear();
    }

    /// The message posted to the session's server: where, or why not.
    pub fn deliver(&self, h: &mut dyn Host, id: &str, text: &str, windows: bool) -> Result<Delivered> {
        let Some(s) = self.server_for(h, id, windows) else {
            return Ok(Err("no running OpenCode has this session".into()));
        };
        let url = format!("{}/session/{}/prompt_async", s.base, encode_uri_component(&open_code_id(id)));
        let body = xenonet_base::json::stringify(&json!({ "parts": [{ "type": "text", "text": text }] }), 0);
        let r = h.post(&url, &body)?;
        if r.status == 204.0 || r.ok {
            return Ok(Ok(s.base));
        }
        Ok(Err(format!("OpenCode answered {}", number_text(r.status))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_go_both_ways() {
        assert_eq!(open_code_id("opencode:ses-abc123"), "ses_abc123");
        assert_eq!(xenonet_id(" ses_abc123 "), "opencode:ses-abc123");
        assert_eq!(xenonet_id("claude:x"), "claude:x");
        assert_eq!(xenonet_id(""), "");
    }

    #[test]
    fn processes_and_ports_read_as_the_js_read_them() {
        let ps = parse_ps("  10 opencode --port 4096\n 20 /bin/zsh -c opencode\nx\n");
        assert_eq!(ps, vec![Proc { pid: 10.0, args: "opencode --port 4096".into() }, Proc { pid: 20.0, args: "/bin/zsh -c opencode".into() }]);
        let ports: Vec<f64> = regex_lsof().captures_iter("p10\nn127.0.0.1:4096\r\nn*:5000\n").map(|m| number(&m[1])).collect();
        assert_eq!(ports, vec![4096.0, 5000.0]);
        assert_eq!(parse_hex("1F90"), 8080.0);
        assert!(same_path("C:\\X\\", "c:/x", true));
        assert_eq!(encode_uri_component("ses_a b/é"), "ses_a%20b%2F%C3%A9");
    }
}
