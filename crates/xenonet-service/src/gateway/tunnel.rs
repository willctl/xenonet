//! The tunnel that puts XenoNet's actor gateway on the internet
//! (server/gateway-tunnel.js), of the kind its owner picks in XenoNet's
//! window: XenoNet assumes none of them is there, finds the ones that are,
//! and keeps the one picked running across restarts, sleep and reboots.
//!
//!   tailscale         Tailscale Funnel: `tailscale funnel --bg` with the
//!                     command that matches the Tailscale running, which
//!                     keeps it across restarts.
//!   cloudflare        a named Cloudflare Tunnel, run as XenoNet's child.
//!   cloudflare-quick  a Cloudflare quick tunnel: a new address each start.
//!   ngrok             ngrok, signed in, on the owner's domain if named.
//!   own               anything the owner runs: XenoNet only checks it.
//!
//! XenoNet never holds a tunnel's secrets: each tool reads its own sign-in.
//! A child tunnel stops with XenoNet and starts again with it; one left
//! behind by a XenoNet that didn't stop cleanly is stopped first (its pid
//! is kept in the state folder).

use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use xenonet_base::json;
use xenonet_base::paths::tilde;
use xenonet_base::sync::Locked;

pub struct Kind {
    pub id: &'static str,
    pub label: &'static str,
    pub through: &'static str,
    pub short: &'static str,
    pub tool: Option<&'static str>,
    pub stable: bool,
    /// What it asks for: name and host, each "required" or "optional".
    pub asks_name: Option<&'static str>,
    pub asks_host: Option<&'static str>,
    pub needs: &'static str,
    pub note: Option<&'static str>,
    pub docs: Option<&'static str>,
}

pub const KINDS: [Kind; 5] = [
    Kind {
        id: "tailscale",
        label: "Tailscale Funnel",
        through: "Tailscale Funnel",
        short: "Your computer's tailnet name",
        tool: Some("tailscale"),
        stable: true,
        asks_name: None,
        asks_host: None,
        needs: "Needs Tailscale signed in, with Funnel allowed for this computer in your tailnet's policy.",
        note: Some("Funnel makes this computer's tailnet name public. Rename it in Tailscale first if the name says too much."),
        docs: Some("https://tailscale.com/kb/1223/funnel"),
    },
    Kind {
        id: "cloudflare",
        label: "Cloudflare Tunnel",
        through: "a Cloudflare Tunnel",
        short: "A name on your own domain",
        tool: Some("cloudflared"),
        stable: true,
        asks_name: Some("required"),
        asks_host: Some("required"),
        needs: "Needs a tunnel routed to your domain: cloudflared tunnel create xenonet, then cloudflared tunnel route dns xenonet <address>.",
        note: None,
        docs: Some("https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/"),
    },
    Kind {
        id: "cloudflare-quick",
        label: "Cloudflare quick tunnel",
        through: "a Cloudflare quick tunnel",
        short: "No account; a new address each start",
        tool: Some("cloudflared"),
        stable: false,
        asks_name: None,
        asks_host: None,
        needs: "Needs only cloudflared, no account.",
        note: Some("Its address changes every start, and each app then needs the new one. Good for a try."),
        docs: Some("https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/"),
    },
    Kind {
        id: "ngrok",
        label: "ngrok",
        through: "ngrok",
        short: "Your ngrok domain",
        tool: Some("ngrok"),
        stable: true,
        asks_name: None,
        asks_host: Some("optional"),
        needs: "Needs your authtoken added: ngrok config add-authtoken <token>.",
        note: Some("Add your ngrok domain so the address stays the same."),
        docs: Some("https://ngrok.com/docs/agent/"),
    },
    Kind {
        id: "own",
        label: "Your own",
        through: "a tunnel of your own",
        short: "A proxy or tunnel you already run",
        tool: None,
        stable: true,
        asks_name: None,
        asks_host: Some("required"),
        needs: "Forward https://<address>/mcp to the gateway's port on this computer, keeping the Host header.",
        note: Some("XenoNet checks it, but doesn't run it."),
        docs: None,
    },
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

pub fn kind_ids() -> Vec<&'static str> {
    KINDS.iter().map(|k| k.id).collect()
}

/// A Cloudflare tunnel's name or id: ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$
pub fn tunnel_name(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=64).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

// Where each tool may be besides PATH: a service started at login has a
// short PATH, and Homebrew's and the apps' folders aren't on it.
fn places(tool: &str, platform: &str) -> &'static [&'static str] {
    match (tool, platform) {
        ("tailscale", "darwin") => &["/Applications/Tailscale.app/Contents/MacOS/Tailscale", "/opt/homebrew/bin/tailscale", "/usr/local/bin/tailscale"],
        ("tailscale", "linux") => &["/usr/bin/tailscale", "/usr/local/bin/tailscale"],
        ("tailscale", "win32") => &["C:\\Program Files\\Tailscale\\tailscale.exe"],
        ("cloudflared", "darwin") => &["/opt/homebrew/bin/cloudflared", "/usr/local/bin/cloudflared"],
        ("cloudflared", "linux") => &["/usr/bin/cloudflared", "/usr/local/bin/cloudflared"],
        ("cloudflared", "win32") => &["C:\\Program Files (x86)\\cloudflared\\cloudflared.exe", "C:\\Program Files\\cloudflared\\cloudflared.exe"],
        ("ngrok", "darwin") => &["/opt/homebrew/bin/ngrok", "/usr/local/bin/ngrok"],
        ("ngrok", "linux") => &["/usr/local/bin/ngrok", "/usr/bin/ngrok", "/snap/bin/ngrok"],
        _ => &[],
    }
}

const TOOL_MS: f64 = 60e3;
// A child that exits by itself: wait, then start it again, longer each
// time; past as many exits in ten minutes as there are waits, it stays
// failed until the next check or a Repair.
const BACKOFF_MS: [f64; 3] = [5e3, 20e3, 60e3];
const WINDOW_MS: f64 = 10.0 * 60e3;
const LINES: usize = 20;

/// Every copy of a tool on this computer: on PATH, then the usual places.
pub fn find_tools(tool: &str, path_var: &str, pathext: Option<&str>, platform: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let on = xenonet_base::paths::on_path(tool, path_var, pathext, platform == "win32", &mut |f| exists(f));
    for f in on.into_iter().chain(places(tool, platform).iter().map(ToString::to_string)) {
        if exists(&f) && !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// The first x.y.z in a tool's words.
pub fn version_in(text: &str) -> Option<String> {
    let b = text.as_bytes();
    for start in 0..b.len() {
        if !b[start].is_ascii_digit() || (start > 0 && b[start - 1].is_ascii_digit()) {
            continue;
        }
        let mut i = start;
        let mut parts = 0;
        loop {
            let s = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == s {
                break;
            }
            parts += 1;
            if parts == 3 {
                return Some(text[start..i].to_string());
            }
            if i < b.len() && b[i] == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit() {
                i += 1;
            } else {
                break;
            }
        }
    }
    None
}

/// `tailscale status --json` as the gateway needs it.
pub fn tailscale_self(text: &str) -> Value {
    let Ok(s) = json::parse(text) else { return json!({ "up": false, "why": "Tailscale's status isn't JSON" }) };
    let state = s.get("BackendState").and_then(Value::as_str);
    let version = s.get("Version").map(xenonet_base::text::js_string).and_then(|v| version_in(&v));
    if state != Some("Running") {
        let why = match state {
            Some("NeedsLogin") => "Tailscale isn't signed in on this computer".to_string(),
            Some("Stopped") => "Tailscale is turned off on this computer".to_string(),
            Some(st) => format!("Tailscale is {}", st.to_lowercase()),
            None => "Tailscale is not running".to_string(),
        };
        return json!({ "up": false, "version": version, "why": why });
    }
    let me = &s["Self"];
    let mut caps: Vec<String> = me["CapMap"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    caps.extend(me["Capabilities"].as_array().cloned().unwrap_or_default().iter().map(xenonet_base::text::js_string));
    let funnel = caps.iter().any(|c| c == "funnel" || c.starts_with("https://tailscale.com/cap/funnel"));
    json!({ "up": true, "version": version, "host": super::actor_gateway::normal_host(me["DNSName"].as_str()), "funnel": funnel })
}

/// What `tailscale funnel status --json` serves on port 443 of this
/// computer's name.
pub fn funnel_of(conf: &Value, host: &str, port: u16) -> Value {
    let hp = format!("{host}:443");
    let handlers = conf.get("Web").and_then(|w| w.get(&hp)).and_then(|w| w.get("Handlers")).and_then(Value::as_object).cloned().unwrap_or_default();
    if handlers.is_empty() {
        return json!({ "state": "none" });
    }
    let mounts: Vec<String> = handlers.keys().cloned().collect();
    let root = handlers.get("/").and_then(|h| h.get("Proxy")).filter(|p| !p.is_null()).map(xenonet_base::text::js_string);
    let ours = |p: &str| p.replacen("localhost", "127.0.0.1", 1).trim_end_matches('/') == format!("http://127.0.0.1:{port}");
    if mounts.len() > 1 {
        return json!({ "state": "shared", "target": mounts.join(", ") });
    }
    match root {
        Some(r) if ours(&r) => json!({ "state": if conf.get("AllowFunnel").and_then(|a| a.get(&hp)) == Some(&json!(true)) { "ours" } else { "tailnet" } }),
        Some(r) => json!({ "state": "elsewhere", "target": r }),
        None => json!({ "state": "elsewhere", "target": mounts[0] }),
    }
}

/// What XenoNet learns from a line a tunnel tool wrote.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub msg: String,
    pub host: Option<String>,
    pub ready: bool,
    pub error: bool,
}

pub fn read_line(kind: &str, raw: &str) -> Option<Line> {
    let line = xenonet_base::text::js_trim(raw);
    if line.is_empty() {
        return None;
    }
    if kind == "ngrok" {
        let Ok(o) = json::parse(line) else { return Some(Line { msg: line.into(), ..Default::default() }) };
        let err = o.get("err").filter(|e| xenonet_base::text::truthy(Some(e)) && *e != "<nil>").map(xenonet_base::text::js_string);
        let msg = || o.get("msg").filter(|m| !m.is_null()).map_or_else(|| line.to_string(), xenonet_base::text::js_string);
        if o["msg"] == "started tunnel" && xenonet_base::text::truthy(o.get("url")) {
            let url = xenonet_base::text::js_string(&o["url"]);
            return Some(Line { msg: format!("started {url}"), host: super::actor_gateway::normal_host(Some(&url)), ready: true, error: false });
        }
        if err.is_some() || o["lvl"] == "eror" || o["lvl"] == "crit" {
            return Some(Line { msg: err.unwrap_or_else(msg), error: true, ..Default::default() });
        }
        return Some(Line { msg: msg(), ..Default::default() });
    }
    // https://([a-z0-9-]+\.trycloudflare\.com)
    if let Some(i) = line.find("https://") {
        let rest = &line[i + 8..];
        let end = rest.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')).unwrap_or(rest.len());
        let host = &rest[..end];
        if let Some(name) = host.strip_suffix(".trycloudflare.com") {
            if !name.is_empty() && !name.contains('.') {
                return Some(Line { msg: line.into(), host: Some(host.to_string()), ..Default::default() });
            }
        }
    }
    if line.contains("Registered tunnel connection") {
        return Some(Line { msg: line.into(), ready: true, ..Default::default() });
    }
    let word = |w: &str| line.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').any(|x| x == w);
    if word("ERR") || word("FTL") || line.contains("error=") {
        return Some(Line { msg: line.into(), error: true, ..Default::default() });
    }
    Some(Line { msg: line.into(), ..Default::default() })
}

/// The command line XenoNet runs for a kind it keeps as its child.
pub fn child_args(kind: &str, port: u16, name: Option<&str>, host: Option<&str>) -> Option<Vec<String>> {
    let origin = format!("http://127.0.0.1:{port}");
    let s = |v: &[&str]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
    match kind {
        "cloudflare" => Some(s(&["tunnel", "--no-autoupdate", "run", "--url", &origin, name.unwrap_or("")])),
        "cloudflare-quick" => Some(s(&["tunnel", "--no-autoupdate", "--url", &origin])),
        "ngrok" => {
            let mut a = s(&["http", &format!("127.0.0.1:{port}"), "--log", "stdout", "--log-format", "json"]);
            if let Some(h) = host {
                a.extend(s(&["--url", &format!("https://{h}")]));
            }
            Some(a)
        }
        _ => None,
    }
}

// ---------- what the tunnel runs on: the system, or a test's stand-in ----------

#[derive(Clone, Debug, Default)]
pub struct RunOut {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// A child tunnel: its pid, the lines it writes (stdout and stderr), how
/// it ended (`it exited with code 1`, `it couldn't start: ENOENT`), and how
/// to stop it.
pub struct Spawned {
    pub pid: Option<u32>,
    pub lines: mpsc::UnboundedReceiver<String>,
    pub exit: oneshot::Receiver<String>,
    pub kill: Box<dyn Fn() + Send + Sync>,
}

#[async_trait]
pub trait TunnelHost: Send + Sync {
    fn exists(&self, file: &str) -> bool;
    async fn run(&self, file: &str, args: &[String], timeout: Duration) -> RunOut;
    fn spawn(&self, file: &str, args: &[String]) -> Result<Spawned, String>;
    fn now(&self) -> f64;
    fn path_var(&self) -> String;
    fn platform(&self) -> &'static str;
    fn kill_pid(&self, _pid: u32) {}
}

/// This computer.
pub struct SystemHost;

#[async_trait]
impl TunnelHost for SystemHost {
    fn exists(&self, file: &str) -> bool {
        std::fs::metadata(file).is_ok_and(|m| m.is_file())
    }
    async fn run(&self, file: &str, args: &[String], timeout: Duration) -> RunOut {
        let mut cmd = tokio::process::Command::new(file);
        cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        match tokio::time::timeout(timeout, cmd.output()).await {
            Err(_) => RunOut { ok: false, stdout: String::new(), stderr: String::new(), timed_out: true },
            Ok(Err(e)) => RunOut { ok: false, stdout: String::new(), stderr: e.to_string(), timed_out: false },
            Ok(Ok(o)) => RunOut {
                ok: o.status.success(),
                stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
                timed_out: false,
            },
        }
    }
    fn spawn(&self, file: &str, args: &[String]) -> Result<Spawned, String> {
        use tokio::io::AsyncBufReadExt;
        let mut cmd = tokio::process::Command::new(file);
        cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| xenonet_base::fs::code_of(&e).map_or_else(|| e.to_string(), str::to_string))?;
        let pid = child.id();
        let (tx, lines) = mpsc::unbounded_channel();
        for out in [
            child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut r = tokio::io::BufReader::new(out).lines();
                while let Ok(Some(l)) = r.next_line().await {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
        }
        let (done, exit) = oneshot::channel();
        let (kill_tx, mut kill_rx) = mpsc::unbounded_channel::<()>();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s,
                _ = kill_rx.recv() => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            let code = status.ok().and_then(|s| s.code());
            let _ = done.send(match code {
                Some(c) => format!("it exited with code {c}"),
                None => "it exited".into(),
            });
        });
        Ok(Spawned {
            pid,
            lines,
            exit,
            kill: Box::new(move || {
                let _ = kill_tx.send(());
            }),
        })
    }
    fn now(&self) -> f64 {
        crate::buckets::now_ms()
    }
    fn path_var(&self) -> String {
        std::env::var("PATH").unwrap_or_default()
    }
    fn platform(&self) -> &'static str {
        crate::borrowed::platform()
    }
    fn kill_pid(&self, pid: u32) {
        #[cfg(unix)]
        #[expect(unsafe_code, reason = "kill(2), which std doesn't wrap, for the JS's process.kill(pid, \"SIGTERM\")")]
        {
            extern "C" {
                fn kill(pid: i32, sig: i32) -> i32;
            }
            // SAFETY: kill only sends a signal, here SIGTERM to a pid ps
            // has just named as the tool; it touches no memory of ours.
            unsafe {
                kill(pid as i32, 15);
            }
        }
        let _ = pid;
    }
}

// ---------- the tunnel ----------

#[derive(Clone, Debug, PartialEq)]
struct Want {
    kind: String,
    name: Option<String>,
    host: Option<String>,
    // Which pick this is, as the JS compares its objects.
    id: u64,
}

#[derive(Clone)]
struct Tool {
    path: String,
    version: Option<String>,
}

struct ChildHandle {
    gen: u64,
    pid: Option<u32>,
    kill: Box<dyn Fn() + Send + Sync>,
}

struct Inner {
    want: Option<Want>,
    state: (String, Option<String>, f64),
    child: Option<ChildHandle>,
    exits: Vec<f64>,
    lines: Vec<Value>,
    timer: Option<tokio::task::JoinHandle<()>>,
    stopping: bool,
    failed_at: f64,
    tailscale_bin: Option<(f64, Tool)>,
    tools: HashMap<String, (f64, Vec<Tool>)>,
    next: u64,
}

/// What a tunnel tells: the address it got, that it's up, that how it
/// stands changed, and a line for the log.
#[derive(Clone)]
pub struct TunnelHooks {
    pub on_host: Arc<dyn Fn(&str) + Send + Sync>,
    pub on_ready: Arc<dyn Fn() + Send + Sync>,
    pub on_change: Arc<dyn Fn() + Send + Sync>,
    pub log: Arc<dyn Fn(&str, &str) + Send + Sync>,
}

impl Default for TunnelHooks {
    fn default() -> Self {
        TunnelHooks { on_host: Arc::new(|_| {}), on_ready: Arc::new(|| {}), on_change: Arc::new(|| {}), log: Arc::new(|_, _| {}) }
    }
}

#[derive(Clone)]
pub struct Tunnel {
    port: u16,
    pid_file: Option<PathBuf>,
    home: String,
    host: Arc<dyn TunnelHost>,
    hooks: TunnelHooks,
    inner: Arc<Mutex<Inner>>,
}

impl Tunnel {
    pub fn new(port: u16, dir: Option<&str>, home: &str, host: Arc<dyn TunnelHost>, hooks: TunnelHooks) -> Tunnel {
        let now = host.now();
        Tunnel {
            port,
            pid_file: dir.map(|d| PathBuf::from(d).join("gateway-tunnel.pid")),
            home: home.to_string(),
            host,
            hooks,
            inner: Arc::new(Mutex::new(Inner {
                want: None,
                state: ("off".into(), None, now),
                child: None,
                exits: Vec::new(),
                lines: Vec::new(),
                timer: None,
                stopping: false,
                failed_at: 0.0,
                tailscale_bin: None,
                tools: HashMap::new(),
                next: 0,
            })),
        }
    }

    fn set_state(&self, s: &str, why: Option<String>) {
        let changed = {
            let mut i = self.inner.locked();
            if i.state.0 == s && i.state.1 == why {
                false
            } else {
                i.state = (s.to_string(), why, self.host.now());
                true
            }
        };
        if changed {
            (self.hooks.on_change)();
        }
    }

    // The copies of a tool here, each with its version, for a minute.
    async fn tool(&self, name: &str) -> Vec<Tool> {
        let t = self.host.now();
        if let Some((at, found)) = self.inner.locked().tools.get(name) {
            if t - at < TOOL_MS {
                return found.clone();
            }
        }
        let mut found = Vec::new();
        let pathext = std::env::var("PATHEXT").ok();
        for file in find_tools(name, &self.host.path_var(), pathext.as_deref(), self.host.platform(), &|f| self.host.exists(f)) {
            let args: Vec<String> = if name == "ngrok" || name == "cloudflared" { vec!["--version".into()] } else { vec!["version".into()] };
            let out = self.host.run(&file, &args, Duration::from_secs(5)).await;
            let words = if out.stdout.is_empty() { out.stderr } else { out.stdout };
            found.push(Tool { path: file, version: version_in(&words) });
        }
        self.inner.locked().tools.insert(name.to_string(), (self.host.now(), found.clone()));
        found
    }

    // ---------- Tailscale Funnel ----------

    // The Tailscale command to use: the one whose version is the running
    // Tailscale's. An older command beside a newer Tailscale can set Funnel
    // up so that its name never reaches the internet's DNS.
    async fn tailscale(&self) -> (Option<Tool>, Option<Value>, Option<String>) {
        let found = self.tool("tailscale").await;
        if found.is_empty() {
            return (None, None, Some("XenoNet can't find Tailscale on this computer".into()));
        }
        let mut me = None;
        for t in &found {
            let out = self.host.run(&t.path, &["status".into(), "--json".into()], Duration::from_secs(5)).await;
            if xenonet_base::text::js_trim(&out.stdout).is_empty() {
                continue;
            }
            me = Some(tailscale_self(&out.stdout));
            break;
        }
        let Some(me) = me else { return (None, None, Some("Tailscale didn't answer: is it running?".into())) };
        let version = me["version"].as_str();
        let bin = found.iter().find(|t| t.version.is_some() && t.version.as_deref() == version).cloned().unwrap_or_else(|| found[0].clone());
        self.inner.locked().tailscale_bin = Some((self.host.now(), bin.clone()));
        if me["up"] != json!(true) {
            let why = xenonet_base::text::js_string(&me["why"]);
            return (Some(bin), Some(me), Some(why));
        }
        if !me["host"].is_string() {
            return (Some(bin), Some(me), Some("Tailscale didn't say this computer's name: turn on MagicDNS and HTTPS certificates for your tailnet".into()));
        }
        (Some(bin), Some(me), None)
    }

    async fn funnel_now(&self, bin: &Tool, host: &str) -> Value {
        let out = self.host.run(&bin.path, &["funnel".into(), "status".into(), "--json".into()], Duration::from_secs(5)).await;
        let conf = json::parse(if out.stdout.is_empty() { "{}" } else { &out.stdout }).unwrap_or_else(|_| json!({}));
        funnel_of(&conf, host, self.port)
    }

    async fn tailscale_up(&self, take_over: bool) {
        if self.inner.locked().state.0 != "up" {
            self.set_state("starting", Some("setting up Tailscale Funnel".into()));
        }
        // tailscale() names the tool and its status whenever it gives no
        // reason it can't go on.
        let (bin, me) = match self.tailscale().await {
            (Some(bin), Some(me), None) => (bin, me),
            (_, _, why) => return self.set_state("failed", why),
        };
        let host = me["host"].as_str().unwrap_or("").to_string();
        (self.hooks.on_host)(&host);
        if me["funnel"] != json!(true) {
            return self.set_state(
                "failed",
                Some("your tailnet doesn't allow Funnel for this computer yet: allow it in the tailnet's access controls (Tailscale's Funnel guide says how), then Repair".into()),
            );
        }
        let mut f = self.funnel_now(&bin, &host).await;
        if f["state"] == "shared" {
            return self.set_state("failed", Some(format!("Tailscale already serves other paths on this computer's port 443 ({}), and Funnel would put them on the internet with the gateway: pick another tunnel, or move them", xenonet_base::text::js_string(&f["target"]))));
        }
        if f["state"] == "elsewhere" && !take_over {
            return self.set_state(
                "failed",
                Some(format!(
                    "Tailscale Funnel on this computer already forwards to {}, not to this XenoNet's gateway: Repair points it here instead",
                    xenonet_base::text::js_string(&f["target"])
                )),
            );
        }
        if f["state"] != "ours" {
            let out = self.host.run(&bin.path, &["funnel".into(), "--bg".into(), "--yes".into(), self.port.to_string()], Duration::from_secs(20)).await;
            let said = format!("{}\n{}", out.stdout, out.stderr);
            if let Some(i) = said.find("https://login.tailscale.com/") {
                let link: String = said[i..].chars().take_while(|c| !c.is_whitespace()).collect();
                return self.set_state("failed", Some(format!("Tailscale asks you to allow Funnel for this computer at {link}; then Repair")));
            }
            f = self.funnel_now(&bin, &host).await;
            if f["state"] != "ours" {
                let last = xenonet_base::text::js_trim(&said).split('\n').next_back().filter(|l| !l.is_empty()).unwrap_or("it said nothing").to_string();
                return self.set_state("failed", Some(format!("tailscale funnel didn't take: {last}")));
            }
            (self.hooks.log)("gateway.tunnel", &format!("Tailscale Funnel forwards {host} to port {}", self.port));
        }
        self.set_state("up", None);
    }

    // Turns Funnel off, if what it serves is this gateway alone.
    async fn tailscale_down(&self) {
        let (bin, me, _) = self.tailscale().await;
        let (Some(bin), Some(host)) = (bin, me.as_ref().and_then(|m| m["host"].as_str().map(str::to_string))) else { return };
        let f = self.funnel_now(&bin, &host).await;
        if f["state"] != "ours" && f["state"] != "tailnet" {
            return;
        }
        let out = self.host.run(&bin.path, &["funnel".into(), "--yes".into(), "--https=443".into(), "off".into()], Duration::from_secs(10)).await;
        if !out.ok {
            let why = if out.stderr.trim().is_empty() { out.stdout.trim().to_string() } else { out.stderr.trim().to_string() };
            (self.hooks.log)("gateway.tunnel", &format!("tailscale funnel off failed: {why}"));
        }
    }

    // ---------- a child XenoNet runs ----------

    // A tunnel left running by a XenoNet that didn't stop cleanly, stopped
    // if it's still that tool.
    async fn reap(&self, tool: &str) {
        let Some(file) = &self.pid_file else { return };
        if self.host.platform() == "win32" {
            return;
        }
        let Ok(text) = std::fs::read_to_string(file) else { return };
        let _ = std::fs::remove_file(file);
        let Ok(pid) = text.trim().parse::<u32>() else { return };
        if pid <= 1 {
            return;
        }
        let out = self.host.run("ps", &["-o".into(), "comm=".into(), "-p".into(), pid.to_string()], Duration::from_secs(3)).await;
        let comm = xenonet_base::paths::base_name(out.stdout.trim()).to_lowercase();
        if !out.ok || !comm.starts_with(tool) {
            return;
        }
        self.host.kill_pid(pid);
        (self.hooks.log)("gateway.tunnel", &format!("stopped a {tool} left running (pid {pid})"));
    }

    fn child_up(&self, w: Want) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let k = kind(&w.kind).expect("up() makes a Want only of a kind that exists");
            let tool = k.tool.unwrap_or("");
            let found = self.tool(tool).await;
            if found.is_empty() {
                return self.set_state("failed", Some(format!("XenoNet can't find {tool} on this computer: install it, then Repair")));
            }
            self.reap(tool).await;
            {
                let mut i = self.inner.locked();
                if i.want.as_ref().map(|x| x.id) != Some(w.id) {
                    return;
                }
                i.stopping = false;
                i.lines.clear();
            }
            self.set_state("starting", Some(format!("{tool} is starting")));
            let args = child_args(&w.kind, self.port, w.name.as_deref(), w.host.as_deref()).unwrap_or_default();
            let spawned = match self.host.spawn(&found[0].path, &args) {
                Ok(s) => s,
                Err(e) => return self.exited(None, format!("it couldn't start: {e}")),
            };
            let gen = {
                let mut i = self.inner.locked();
                i.next += 1;
                let gen = i.next;
                i.child = Some(ChildHandle { gen, pid: spawned.pid, kill: spawned.kill });
                gen
            };
            if let (Some(file), Some(pid)) = (&self.pid_file, spawned.pid) {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(file, pid.to_string());
            }
            let me = self.clone();
            let kind_id = w.kind.clone();
            let mut lines = spawned.lines;
            let exit = spawned.exit;
            tokio::spawn(async move {
                let mine = |me: &Tunnel| me.inner.locked().child.as_ref().map(|c| c.gen) == Some(gen);
                let mut exit = exit;
                loop {
                    tokio::select! {
                        biased;
                        line = lines.recv() => match line {
                            Some(raw) => {
                                let Some(got) = read_line(&kind_id, &raw) else { continue };
                                {
                                    let mut i = me.inner.locked();
                                    let msg = xenonet_base::log::js_slice(&tilde(&got.msg, &me.home), 300).to_string();
                                    i.lines.push(json!({ "at": json::number(me.host.now()), "error": got.error, "msg": msg }));
                                    let n = i.lines.len();
                                    if n > LINES {
                                        i.lines.drain(0..n - LINES);
                                    }
                                }
                                if let Some(h) = got.host.as_deref().filter(|_| mine(&me)) {
                                    (me.hooks.on_host)(h);
                                }
                                if got.ready && mine(&me) {
                                    me.set_state("up", None);
                                    (me.hooks.on_ready)();
                                }
                            }
                            None => {
                                let why = (&mut exit).await.unwrap_or_else(|_| "it exited".into());
                                me.exited(Some(gen), why);
                                return;
                            }
                        },
                        why = &mut exit => {
                            // What it wrote before it went, then why.
                            while let Ok(raw) = lines.try_recv() {
                                if let Some(got) = read_line(&kind_id, &raw) {
                                    let mut i = me.inner.locked();
                                    i.lines.push(json!({ "at": json::number(me.host.now()), "error": got.error, "msg": xenonet_base::log::js_slice(&tilde(&got.msg, &me.home), 300) }));
                                }
                            }
                            me.exited(Some(gen), why.unwrap_or_else(|_| "it exited".into()));
                            return;
                        }
                    }
                }
            });
        })
    }

    fn exited(&self, gen: Option<u64>, why: String) {
        let w = {
            let mut i = self.inner.locked();
            if gen.is_some() && i.child.as_ref().map(|c| c.gen) != gen {
                return;
            }
            i.child = None;
            if let Some(f) = &self.pid_file {
                let _ = std::fs::remove_file(f);
            }
            match i.want.clone() {
                Some(w) if !i.stopping => w,
                _ => {
                    drop(i);
                    return self.set_state("off", None);
                }
            }
        };
        let t = self.host.now();
        let (count, said) = {
            let mut i = self.inner.locked();
            i.exits.retain(|at| t - at < WINDOW_MS);
            i.exits.push(t);
            let last = i.lines.iter().rev().find(|l| l["error"] == json!(true)).and_then(|l| l["msg"].as_str()).map(str::to_string);
            (i.exits.len(), last.map_or(why.clone(), |l| format!("{why}: {l}")))
        };
        (self.hooks.log)("gateway.tunnel", &said);
        if count > BACKOFF_MS.len() {
            self.inner.locked().failed_at = t;
            return self.set_state("failed", Some(said));
        }
        let wait = BACKOFF_MS[count - 1];
        self.set_state("failed", Some(format!("{said}; trying again in {} s", (wait / 1000.0).round())));
        let me = self.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
            let again = {
                let mut i = me.inner.locked();
                i.timer = None;
                i.want.as_ref().map(|x| x.id) == Some(w.id) && i.child.is_none()
            };
            if again {
                me.child_up(w).await;
            }
        });
        self.inner.locked().timer = Some(handle);
    }

    fn child_down(&self) {
        let mut i = self.inner.locked();
        i.stopping = true;
        if let Some(t) = i.timer.take() {
            t.abort();
        }
        i.exits.clear();
        let c = i.child.take();
        if let Some(f) = &self.pid_file {
            let _ = std::fs::remove_file(f);
        }
        drop(i);
        if let Some(c) = c {
            (c.kill)();
        }
    }

    pub async fn down(&self) {
        let was = self.inner.locked().want.take();
        self.child_down();
        if was.is_some_and(|w| w.kind == "tailscale") {
            self.tailscale_down().await;
        }
        self.set_state("off", None);
    }

    /// Brings the tunnel picked up, stopping another kind first. `take_over`
    /// (Repair) lets Tailscale Funnel replace what it forwards to.
    pub async fn up(&self, kind_id: &str, name: Option<&str>, host: Option<&str>, take_over: bool) -> Result<Value, String> {
        if kind(kind_id).is_none() {
            return Err(format!("no such tunnel: {kind_id}"));
        }
        let (same, had) = {
            let i = self.inner.locked();
            let same = i.want.as_ref().is_some_and(|w| w.kind == kind_id && w.name.as_deref() == name && w.host.as_deref() == host);
            (same, i.want.is_some())
        };
        if had && !same {
            self.down().await;
        }
        {
            let i = self.inner.locked();
            if same && (i.child.is_some() || i.state.0 == "up") && !take_over {
                drop(i);
                return Ok(self.view());
            }
        }
        let w = {
            let mut i = self.inner.locked();
            // The lock was let go since `same` was read, so a down() in
            // between may have taken the want: then it's picked anew.
            let w = match i.want.clone() {
                Some(w) if same => w,
                _ => {
                    i.next += 1;
                    Want { kind: kind_id.to_string(), name: name.map(str::to_string), host: host.map(str::to_string), id: i.next }
                }
            };
            i.want = Some(w.clone());
            i.exits.clear();
            w
        };
        if kind_id == "own" {
            self.set_state("yours", None);
        } else if kind_id == "tailscale" {
            self.tailscale_up(take_over).await;
        } else if self.inner.locked().child.is_none() {
            self.child_up(w).await;
        }
        Ok(self.view())
    }

    /// From the clock and after a failed check: Funnel set up again if it
    /// went, and a child that gave up started again after ten minutes.
    pub async fn ensure(&self) {
        let (w, idle, failed_at) = {
            let i = self.inner.locked();
            (i.want.clone(), i.child.is_none() && i.timer.is_none(), i.failed_at)
        };
        let Some(w) = w else { return };
        if w.kind == "tailscale" {
            return self.tailscale_up(false).await;
        }
        if w.kind != "own" && idle && self.host.now() - failed_at >= WINDOW_MS {
            self.inner.locked().exits.clear();
            self.child_up(w).await;
        }
    }

    /// On XenoNet's way out: its child stops; Funnel stays set up, so the
    /// gateway is back as soon as XenoNet is.
    pub fn stop(&self) {
        self.inner.locked().want = None;
        self.child_down();
        self.set_state("off", None);
    }

    pub fn view(&self) -> Value {
        let i = self.inner.locked();
        let k = i.want.as_ref().and_then(|w| kind(&w.kind));
        let log: Vec<Value> = i.lines.iter().rev().take(6).rev().cloned().collect();
        json!({
            "kind": i.want.as_ref().map(|w| w.kind.clone()),
            "label": k.map(|k| k.label),
            "through": k.map(|k| k.through),
            "state": i.state.0,
            "why": i.state.1,
            "since": json::number(i.state.2),
            "pid": i.child.as_ref().and_then(|c| c.pid),
            "log": log,
        })
    }

    /// Each kind, and whether its tool is on this computer.
    pub async fn kinds(&self) -> Value {
        let mut out = Vec::new();
        for k in &KINDS {
            let found = match k.tool {
                Some(t) => self.tool(t).await,
                None => Vec::new(),
            };
            // Tailscale's: the command that matches the Tailscale running.
            let fresh = self.inner.locked().tailscale_bin.as_ref().is_some_and(|(at, _)| self.host.now() - at < TOOL_MS);
            if k.id == "tailscale" && found.len() > 1 && !fresh {
                self.tailscale().await;
            }
            let bin = self.inner.locked().tailscale_bin.as_ref().map(|(_, b)| b.path.clone());
            let used = (if k.id == "tailscale" { bin.and_then(|b| found.iter().find(|t| t.path == b).cloned()) } else { None }).or_else(|| found.first().cloned());
            let asks = {
                let mut a = serde_json::Map::new();
                if let Some(n) = k.asks_name {
                    a.insert("name".into(), json!(n));
                }
                if let Some(h) = k.asks_host {
                    a.insert("host".into(), json!(h));
                }
                Value::Object(a)
            };
            out.push(json!({
                "kind": k.id, "label": k.label, "through": k.through, "short": k.short, "tool": k.tool,
                "found": k.tool.is_none() || !found.is_empty(),
                "path": used.as_ref().map(|u| tilde(&u.path, &self.home)),
                "version": used.as_ref().and_then(|u| u.version.clone()),
                "stable": k.stable, "asks": asks, "needs": k.needs, "note": k.note, "docs": k.docs,
            }));
        }
        Value::Array(out)
    }
}
