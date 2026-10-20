//! core/hosts/link.js: one host's link, the console's end of `ssh <host>
//! xenonet relay` (or of another XenoNet's relay on this machine). It
//! starts the transport, reads the frames, keeps the clock offset, sends
//! gets with time limits, and keeps the host's state:
//!
//!   connecting  starting
//!   live        frames are arriving
//!   stale       no frame for 45 seconds: three missed pings
//!   offline     the connection ended or never came up; tried again with
//!               backoff, from 2 to 60 seconds, with jitter
//!   error       it needs you (the host key, a refused key, no relay, a
//!               protocol this XenoNet doesn't speak, another XenoNet
//!               answering in the host's place); tried again every minute
//!
//! Everything a host sends is untrusted: frames are capped in size and
//! checked. Nothing a link reads can make it write anywhere but the
//! relay's standard input.
//!
//! The link here is its state and decisions. Each call says what the
//! twin is to do, in order (`Effect`): start the program, write a frame,
//! end the program, emit an event, set or clear a timer, settle a get.

use crate::frames::{encode, parse_frame, shallow, FROM_RELAY, MAX_DEPTH, PROTOCOL};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use xenonet_app::jstext::{slice16, utf16_len};
use xenonet_app::jsv::truthy;
use xenonet_base::ids::is_host_id;
use xenonet_base::text::js_string;

pub const HELLO_MS: f64 = 20e3;
pub const STALE_MS: f64 = 45e3;
pub const BACKOFF_MIN: f64 = 2e3;
pub const BACKOFF_MAX: f64 = 60e3;
pub const GET_MS: f64 = 20e3;
pub const TIME_EVERY_MS: f64 = 5.0 * 60e3;

/// ssh as hosts.md sets it: no terminal, no agent or X11 forwarding, never
/// a prompt, a host key that's known and unchanged even if your own config
/// would take a new one, none of your forwards, compression, and a
/// connection that gives up on a silent host in about 45 seconds.
pub const SSH_OPTIONS: [&str; 16] = [
    "-T",
    "-a",
    "-x",
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=yes",
    "-o",
    "ClearAllForwardings=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=3",
    "-C",
];

/// How a link reaches a host, and this machine's part in it.
#[derive(Clone, Debug)]
pub struct Options {
    pub ssh_config: Option<String>,
    /// ssh, or ssh and words before its own (a stand-in for tests).
    pub ssh_bin: Vec<String>,
    /// The xenonet binary a local link runs `relay` with.
    pub bin: String,
    /// A script `bin` runs first: the JS service's, in a checkout with no
    /// binary built, is its cli/xenonet.js on Node. The binary sets none.
    pub entry: Option<String>,
    pub env: Map<String, Value>,
}

/// A program to run: (file, args, env).
pub type Command = (String, Vec<String>, Map<String, Value>);

/// What a link runs: (file, args, env). Fixed words, and values hostEntry
/// checked, checked again here, since the host's shell parses the remote
/// command. The destination is the host's id, an alias in XenoNet's own
/// ssh config, after --, so it can't be an option. The local transport runs
/// only XenoNet's own relay, with none of this XenoNet's XENONET_ settings.
pub fn transport(entry: &Value, o: &Options) -> Result<Command, String> {
    let id = entry.get("id").and_then(Value::as_str).filter(|i| is_host_id(i)).ok_or_else(|| format!("not a host id: {}", show(entry.get("id"))))?;
    let port =
        entry.get("port").and_then(Value::as_f64).filter(|p| p.fract() == 0.0 && (1.0..=65535.0).contains(p)).ok_or_else(|| format!("not a port: {}", show(entry.get("port"))))?;
    let since = entry.get("since").map_or_else(|| "undefined".into(), js_string);
    if !crate::config::since_ok(&since) {
        return Err(format!("not a time: {}", show(entry.get("since"))));
    }
    let mut relay = vec!["relay".to_string(), "--port".into(), xenonet_base::json::number_string(port), "--since".into(), since];
    if entry.get("detail") == Some(&json!(true)) {
        relay.push("--detail".into());
    }
    if entry.get("start") == Some(&json!(true)) {
        relay.push("--start".into());
    }
    match entry.get("transport").and_then(Value::as_str) {
        Some("local") => {
            let env: Map<String, Value> = o.env.iter().filter(|(k, _)| !k.starts_with("XENONET_")).map(|(k, v)| (k.clone(), v.clone())).collect();
            Ok((o.bin.clone(), o.entry.iter().cloned().chain(relay).collect(), env))
        }
        Some("ssh") => {
            let command = entry.get("command").map_or_else(|| "undefined".into(), js_string);
            if !crate::config::command_ok(&command) {
                return Err(format!("not a xenonet command: {}", show(entry.get("command"))));
            }
            let Some(config) = &o.ssh_config else { return Err("ssh needs XenoNet's own ssh config".into()) };
            let mut bin = o.ssh_bin.clone();
            if bin.is_empty() {
                bin.push("ssh".into());
            }
            let file = bin.remove(0);
            let mut args = bin;
            args.extend(["-F".to_string(), config.clone()]);
            args.extend(SSH_OPTIONS.iter().map(ToString::to_string));
            args.extend(["--".to_string(), id.to_string(), command]);
            args.extend(relay);
            Ok((file, args, o.env.clone()))
        }
        _ => Err(format!("not a transport: {}", show(entry.get("transport")))),
    }
}

// JSON.stringify(v), with undefined as JS writes it in a template.
fn show(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), |v| xenonet_base::json::stringify(v, 0))
}

fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// A path as a word a shell takes as it is, for a command a person pastes
/// into a terminal: under the home folder as ~/ and the rest in quotes.
pub fn shell_path(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(p),
    }
}

/// The command that checks a host's key, and accepts it once, in a
/// terminal: ssh with XenoNet's own ssh config.
pub fn trust_command(id: &str, ssh_config: Option<&str>, tilde: &dyn Fn(&str) -> String) -> String {
    match ssh_config {
        Some(c) if !c.is_empty() => format!("ssh -F {} {id}", shell_path(&tilde(c))),
        _ => format!("ssh {id}"),
    }
}

/// Why a connection ended: its state, code, message and fix.
#[derive(Clone, Debug, PartialEq)]
pub struct Why {
    pub state: &'static str,
    pub code: &'static str,
    pub message: String,
    pub fix: Option<String>,
}

fn why(state: &'static str, code: &'static str, message: String, fix: Option<String>) -> Why {
    Why { state, code, message, fix }
}

fn res() -> &'static [Regex; 3] {
    static R: OnceLock<[Regex; 3]> = OnceLock::new();
    R.get_or_init(|| {
        let any = r"[^\n\r\x{2028}\x{2029}]*";
        [
            Regex::new(&format!(r"(?i)host key verification failed|remote host identification has changed|no \S+ host key is known|host key for {any} has changed"))
                .expect("a fixed pattern"),
            Regex::new(r"(?i)permission denied|too many authentication failures|no supported authentication methods").expect("a fixed pattern"),
            Regex::new(r"(?i)(^|\s)xenonet: (command )?not found|command not found: xenonet|xenonet: No such file or directory").expect("a fixed pattern"),
        ]
    })
}

/// How a connection ended, from what ssh and the relay said. ssh names
/// the machine and the user it logs in as, which often hold a person's
/// name: what it says is shown with the host's id in their place.
pub struct Ended<'a> {
    pub exit_code: Option<f64>,
    pub signal: Option<String>,
    pub stderr: &'a str,
    pub bye: Option<&'a Value>,
    pub spawn_error: Option<&'a str>,
}

pub fn why_ended(entry: &Value, e: &Ended<'_>, ssh_config: Option<&str>, tilde: &dyn Fn(&str) -> String) -> Why {
    let id = entry.get("id").map(js_string).unwrap_or_default();
    let ssh = entry.get("transport") == Some(&json!("ssh"));
    let named = |text: &str| -> String {
        let mut out = text.to_string();
        let user = entry.get("ssh").and_then(|s| s.get("user")).filter(|u| truthy(Some(u))).map(|u| format!("{}@", js_string(u)));
        let host = entry.get("ssh").and_then(|s| s.get("hostname")).filter(|h| truthy(Some(h))).map(js_string);
        for (name, to) in [(user, String::new()), (host, id.clone())] {
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                out = out.split(n.as_str()).collect::<Vec<_>>().join(&to);
            }
        }
        out
    };
    let lines: Vec<&str> = xenonet_base::text::js_trim(e.stderr).split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty()).collect();
    let said = named(&lines.last().map_or_else(String::new, |l| slice16(l, 200)));
    let try_it = trust_command(&id, ssh_config, tilde);
    if let Some(err) = e.spawn_error {
        let what = if ssh { "ssh" } else { "the runtime" };
        return why(
            "error",
            "no-ssh",
            format!("{what} couldn't start here ({err})"),
            Some(if ssh { "Install OpenSSH's client, or put ssh on XenoNet's PATH." } else { "Restart XenoNet." }.into()),
        );
    }
    let bye_code = e.bye.and_then(|b| b.get("code")).map(js_string).filter(|c| !c.is_empty());
    match bye_code.as_deref() {
        Some("no-xenonet") => {
            return why(
                "offline",
                "no-xenonet",
                format!("no XenoNet is running on {id}"),
                Some(format!("Start XenoNet on {id}, or add it again with the port its XenoNet uses (xenonet hosts add {id} … --port).")),
            )
        }
        Some("xenonet-gone") => return why("offline", "xenonet-gone", format!("the XenoNet on {id} stopped"), None),
        Some("old-xenonet") => return why("error", "old-xenonet", format!("the XenoNet on {id} is too old to be a host"), Some(format!("Update XenoNet on {id}."))),
        Some(code) => {
            let m = e.bye.and_then(|b| b.get("message")).filter(|m| !m.is_null()).map_or_else(|| code.to_string(), js_string);
            return why("offline", "relay", format!("the relay on {id} stopped: {}", named(&slice16(&m, 200))), None);
        }
        None => {}
    }
    if ssh && e.exit_code == Some(255.0) {
        if res()[0].is_match(e.stderr) {
            return why("error", "host-key", format!("ssh doesn't know {id}'s host key, or it changed"), Some(format!("Check it and accept it once in a terminal: {try_it}")));
        }
        if res()[1].is_match(e.stderr) {
            return why("error", "refused", format!("{id} refused the key"), Some(format!("Install the key in {id}'s ~/.ssh/authorized_keys, then try {try_it}")));
        }
        let at = if said.is_empty() { String::new() } else { format!(": {}", said.strip_prefix("ssh: ").unwrap_or(&said)) };
        return why("offline", "unreachable", format!("ssh couldn't reach {id}{at}"), Some(format!("Check that {id} is on and reachable: {try_it}")));
    }
    if e.exit_code == Some(127.0) || res()[2].is_match(e.stderr) {
        return why(
            "error",
            "no-relay",
            format!("{id} has no xenonet command"),
            Some(format!("Install XenoNet on {id} and its command (xenonet install), or add it again with the command's full path (--command).")),
        );
    }
    let how = match (&e.exit_code, &e.signal) {
        (Some(c), _) => format!(" (exit {})", xenonet_base::json::number_string(*c)),
        (None, Some(s)) if !s.is_empty() => format!(" ({s})"),
        _ => String::new(),
    };
    let at = if said.is_empty() { String::new() } else { format!(": {said}") };
    why("offline", "ended", format!("the relay on {id} stopped{how}{at}"), None)
}

/// What the twin is to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Spawn {
        run: u64,
        file: String,
        args: Vec<String>,
        env: Map<String, Value>,
    },
    Write {
        run: u64,
        text: String,
    },
    /// End the run's program (SIGTERM) if it's still there, or else tell
    /// the link it ended (`end`).
    Fail {
        run: u64,
    },
    /// End the run's program if it's still there.
    Kill {
        run: u64,
    },
    Emit {
        name: &'static str,
        data: Value,
    },
    Timer {
        name: String,
        ms: f64,
    },
    Interval {
        name: String,
        ms: f64,
    },
    Clear {
        name: String,
    },
    Resolve {
        id: u64,
        value: Value,
    },
    Reject {
        id: u64,
        message: String,
        status: u16,
    },
}

/// What a link asks of the JS around it: a random number for the jitter,
/// and the schema's check of a hello (zod's), with what's wrong.
pub trait Asks {
    fn random(&mut self) -> f64;
    fn check_hello(&mut self, frame: &Value) -> Option<String>;
    fn tilde(&self, p: &str) -> String;
}

struct Run {
    id: u64,
    hello: bool,
    snapshot: bool,
    bye: Option<Value>,
    stderr: String,
    problem: Option<Why>,
    ended: bool,
    pending: Vec<u64>,
    times: HashMap<u64, f64>,
}

/// One host's link.
pub struct Link {
    pub entry: Value,
    pub options: Options,
    pub demo_only: bool,
    pub one_shot: bool,
    pub hello_ms: f64,
    pub stale_ms: f64,
    pub backoff: (f64, f64),
    pub get_ms: f64,
    pub state: &'static str,
    pub since: f64,
    pub last_heard: Option<f64>,
    pub offset_ms: f64,
    pub rtt_ms: Option<f64>,
    pub hello: Value,
    pub error: Value,
    pub next_try_at: Option<f64>,
    pub attempt: u32,
    connects: f64,
    frames: f64,
    dropped: f64,
    last_drop: Value,
    seq: u64,
    run: Option<Run>,
    runs: u64,
    pub stopped: bool,
    fx: Vec<Effect>,
}

impl Link {
    #[expect(clippy::too_many_arguments, reason = "the JS's Link and its options, which xenonet-node passes one by one")]
    pub fn new(entry: Value, options: Options, now: f64, demo_only: bool, one_shot: bool, hello_ms: f64, stale_ms: f64, backoff: (f64, f64), get_ms: f64) -> Self {
        Link {
            entry,
            options,
            demo_only,
            one_shot,
            hello_ms,
            stale_ms,
            backoff,
            get_ms,
            state: "offline",
            since: now,
            last_heard: None,
            offset_ms: 0.0,
            rtt_ms: None,
            hello: Value::Null,
            error: Value::Null,
            next_try_at: None,
            attempt: 0,
            connects: 0.0,
            frames: 0.0,
            dropped: 0.0,
            last_drop: Value::Null,
            seq: 0,
            run: None,
            runs: 0,
            stopped: true,
            fx: Vec::new(),
        }
    }

    fn id(&self) -> String {
        self.entry.get("id").map(js_string).unwrap_or_default()
    }

    fn take(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.fx)
    }

    fn emit(&mut self, name: &'static str, data: Value) {
        self.fx.push(Effect::Emit { name, data });
    }

    pub fn start(&mut self, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        if self.stopped {
            self.stopped = false;
            self.fx.push(Effect::Interval { name: "stale".into(), ms: 5e3 });
            self.connect(now, a);
        }
        self.take()
    }

    pub fn stop(&mut self, now: f64) -> Vec<Effect> {
        self.stop_in(now);
        self.take()
    }

    fn stop_in(&mut self, _now: f64) {
        self.stopped = true;
        self.fx.push(Effect::Clear { name: "stale".into() });
        self.fx.push(Effect::Clear { name: "retry".into() });
        if let Some(run) = self.run.take() {
            self.close(run, "the link stopped");
        }
        self.next_try_at = None;
    }

    /// Down until resume(), as if the host went away.
    pub fn pause(&mut self, message: &str, now: f64) -> Vec<Effect> {
        self.stop_in(now);
        self.error = json!({ "code": "paused", "message": message });
        self.set("offline", now);
        self.take()
    }

    /// Now, rather than when the backoff says.
    pub fn reconnect(&mut self, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        if self.stopped {
            return self.start(now, a);
        }
        self.attempt = 0;
        self.fx.push(Effect::Clear { name: "retry".into() });
        if let Some(run) = self.run.take() {
            self.close(run, "connecting again");
        }
        self.connect(now, a);
        self.take()
    }

    /// A read through the relay: Err with its status when there's no
    /// connection to read through; else its id, with its time limit and
    /// the frame to send among the effects.
    pub fn get(&mut self, route: &Value, timeout_ms: f64) -> Result<(u64, Vec<Effect>), (String, u16)> {
        let id_s = self.id();
        let state = self.state;
        let Some(run) = self.run.as_mut().filter(|r| r.snapshot) else { return Err((format!("{id_s} isn't connected ({state})"), 503)) };
        self.seq += 1;
        let id = self.seq;
        run.pending.push(id);
        let run_id = run.id;
        self.fx.push(Effect::Timer { name: format!("get:{id}"), ms: timeout_ms });
        self.fx.push(Effect::Write { run: run_id, text: encode(&json!({ "t": "get", "id": id, "path": route })) });
        Ok((id, self.take()))
    }

    /// A get's time ran out.
    pub fn get_timeout(&mut self, id: u64, timeout_ms: f64) -> Vec<Effect> {
        let mut found = false;
        if let Some(r) = &mut self.run {
            if let Some(i) = r.pending.iter().position(|p| *p == id) {
                r.pending.remove(i);
                found = true;
            }
        }
        let _ = found;
        let message = format!("{} didn't answer in {} s", self.id(), xenonet_base::json::number_string(xenonet_base::units::js_round(timeout_ms / 1000.0)));
        self.fx.push(Effect::Reject { id, message, status: 504 });
        self.take()
    }

    /// Where the host stands, for /api/hosts and the doctor.
    pub fn info(&self) -> Value {
        let e = &self.entry;
        let local = e.get("transport") == Some(&json!("local"));
        let target = if local {
            format!("this machine, port {}", e.get("port").map_or_else(|| "undefined".into(), js_string))
        } else {
            let ssh = e.get("ssh");
            let user = ssh.and_then(|s| s.get("user")).filter(|u| truthy(Some(u))).map_or(String::new(), |u| format!("{}@", js_string(u)));
            let host = ssh.and_then(|s| s.get("hostname")).filter(|h| !h.is_null()).map_or_else(|| self.id(), js_string);
            let port = ssh.and_then(|s| s.get("port")).filter(|p| truthy(Some(p))).map_or(String::new(), |p| format!(":{}", js_string(p)));
            format!("{user}{host}{port}")
        };
        let mut m = Map::new();
        m.insert("id".into(), e.get("id").cloned().unwrap_or(Value::Null));
        if let Some(t) = e.get("transport") {
            m.insert("transport".into(), t.clone());
        }
        m.insert("target".into(), json!(target));
        if let Some(p) = e.get("port") {
            m.insert("port".into(), p.clone());
        }
        m.insert("state".into(), json!(self.state));
        m.insert("since".into(), xenonet_base::json::number(self.since));
        m.insert("lastHeard".into(), self.last_heard.map_or(Value::Null, xenonet_base::json::number));
        m.insert("offsetMs".into(), if self.rtt_ms.is_none() { Value::Null } else { xenonet_base::json::number(self.offset_ms) });
        m.insert("rttMs".into(), self.rtt_ms.map_or(Value::Null, xenonet_base::json::number));
        m.insert("hello".into(), self.hello.clone());
        m.insert("error".into(), self.error.clone());
        m.insert("nextTryAt".into(), if matches!(self.state, "offline" | "error") { self.next_try_at.map_or(Value::Null, xenonet_base::json::number) } else { Value::Null });
        m.insert("counts".into(), json!({ "connects": self.connects, "frames": self.frames, "dropped": self.dropped }));
        m.insert("lastDrop".into(), self.last_drop.clone());
        Value::Object(m)
    }

    fn connect(&mut self, now: f64, a: &mut dyn Asks) {
        self.fx.push(Effect::Clear { name: "retry".into() });
        if self.stopped {
            return;
        }
        let cmd = match transport(&self.entry, &self.options) {
            Ok(c) => c,
            Err(err) => {
                let id = self.id();
                let w =
                    why("error", "bad-entry", format!("XenoNet can't reach {id} as it's kept: {err}"), Some(format!("Remove it and add it again (xenonet hosts remove {id}).")));
                return self.ended(w, now, a);
            }
        };
        self.connects += 1.0;
        self.set("connecting", now);
        self.runs += 1;
        let run = Run { id: self.runs, hello: false, snapshot: false, bye: None, stderr: String::new(), problem: None, ended: false, pending: Vec::new(), times: HashMap::new() };
        let run_id = run.id;
        self.run = Some(run);
        self.fx.push(Effect::Spawn { run: run_id, file: cmd.0, args: cmd.1, env: cmd.2 });
        self.fx.push(Effect::Timer { name: format!("hello:{run_id}"), ms: self.hello_ms });
    }

    fn current(&mut self, run: u64) -> Option<&mut Run> {
        self.run.as_mut().filter(|r| r.id == run && !r.ended)
    }

    /// The hello timer of `run` went off.
    pub fn hello_timeout(&mut self, run: u64, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        let hello_ms = self.hello_ms;
        let id = self.id();
        if let Some(r) = self.current(run) {
            if !r.snapshot {
                let w = if r.hello {
                    why("offline", "no-snapshot", format!("{id} said hello but sent no map in {} s", xenonet_base::json::number_string(hello_ms / 1000.0)), None)
                } else {
                    why("offline", "no-hello", format!("{id} didn't answer in {} s", xenonet_base::json::number_string(hello_ms / 1000.0)), None)
                };
                self.fail(run, w, now, a);
            }
        }
        self.take()
    }

    /// What the run's program wrote on its standard error.
    pub fn stderr(&mut self, run: u64, chunk: &str) {
        if let Some(r) = self.run.as_mut().filter(|r| r.id == run) {
            r.stderr.push_str(chunk);
            let n = utf16_len(&r.stderr);
            if n > 4096 {
                let units: Vec<u16> = r.stderr.encode_utf16().collect();
                r.stderr = String::from_utf16_lossy(&units[n - 4096..]);
            }
        }
    }

    /// A frame too big to read went past, on any run's output (counted as
    /// the JS counts it).
    pub fn too_big(&mut self, _run: u64, now: f64) {
        self.drop_frame("a frame over 16 MB", now);
    }

    /// A line the run's program wrote.
    pub fn line(&mut self, run: u64, line: &str, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        if self.current(run).is_none() {
            return self.take();
        }
        self.frames += 1.0;
        let frame = match parse_frame(line, Some(&FROM_RELAY)) {
            Ok(f) => f,
            Err(e) => {
                self.drop_frame(&e, now);
                return self.take();
            }
        };
        if !shallow(&frame, MAX_DEPTH) {
            self.drop_frame("a frame nested too deep", now);
            return self.take();
        }
        let t = frame["t"].as_str().unwrap_or("").to_string();
        let has_hello = self.run.as_ref().is_some_and(|r| r.hello);
        if !has_hello && t != "hello" && t != "bye" {
            self.drop_frame(&format!("a {t} before the hello"), now);
            return self.take();
        }
        self.last_heard = Some(now);
        match t.as_str() {
            "hello" => self.hello_frame(run, &frame, now, a),
            "snapshot" => self.snapshot(run, &frame, now),
            "event" => {
                let name_ok = frame.get("name").and_then(Value::as_str).is_some_and(|n| utf16_len(n) <= 40);
                if !name_ok || !frame.get("data").is_some_and(Value::is_object) {
                    self.drop_frame("an event that isn't one", now);
                } else if self.run.as_ref().is_some_and(|r| r.snapshot) {
                    self.heard(now);
                    self.emit("event", json!({ "name": frame["name"], "data": frame["data"] }));
                }
            }
            "res" => {
                let fid = frame.get("id").and_then(Value::as_f64).filter(|x| x.fract() == 0.0 && *x >= 1.0).map(|x| x as u64);
                let at = fid.and_then(|f| self.run.as_ref().and_then(|r| r.pending.iter().position(|p| *p == f)));
                match (fid, at) {
                    (Some(f), Some(i)) => {
                        // `at` was found in the run's pending gets, so there's a run.
                        if let Some(r) = self.run.as_mut() {
                            r.pending.remove(i);
                        }
                        self.fx.push(Effect::Clear { name: format!("get:{f}") });
                        self.heard(now);
                        let status = frame.get("status").and_then(Value::as_f64).filter(|s| s.fract() == 0.0 && (100.0..600.0).contains(s)).unwrap_or(502.0);
                        let body = frame.get("body").filter(|b| !b.is_null()).cloned().unwrap_or(Value::Null);
                        self.fx.push(Effect::Resolve { id: f, value: json!({ "status": status as i64, "body": body }) });
                    }
                    _ => self.drop_frame("an answer to nothing asked", now),
                }
            }
            "time" => self.timed(run, &frame, now),
            "ping" => self.heard(now),
            "bye" => {
                let code = slice16(&frame.get("code").filter(|v| !v.is_null()).map_or_else(String::new, js_string), 40);
                let message = slice16(&frame.get("message").filter(|v| !v.is_null()).map_or_else(String::new, js_string), 300);
                if let Some(r) = self.run.as_mut() {
                    r.bye = Some(json!({ "code": code, "message": message }));
                }
            }
            _ => {}
        }
        self.take()
    }

    fn hello_frame(&mut self, run: u64, f: &Value, now: f64, a: &mut dyn Asks) {
        if self.run.as_ref().is_some_and(|r| r.hello) {
            return self.drop_frame("a second hello", now);
        }
        let id = self.id();
        if let Some(problem) = a.check_hello(f) {
            let first = slice16(problem.split('\n').next().unwrap_or(""), 160);
            return self.fail(
                run,
                why(
                    "error",
                    "bad-hello",
                    format!("{id} answered with something that isn't a relay's hello ({first})"),
                    Some(format!("Update XenoNet on {id} and here to the same version.")),
                ),
                now,
                a,
            );
        }
        let protocol = f.get("protocol").and_then(Value::as_f64);
        if protocol != Some(PROTOCOL as f64) {
            let p = f.get("protocol").map_or_else(|| "undefined".into(), js_string);
            return self.fail(
                run,
                why(
                    "error",
                    "protocol",
                    format!(
                        "{id} runs XenoNet {}, whose relay speaks protocol {p}; this XenoNet speaks {PROTOCOL}",
                        f.get("version").map_or_else(|| "undefined".into(), js_string)
                    ),
                    Some("Update the older of the two.".into()),
                ),
                now,
                a,
            );
        }
        let demo = truthy(f.get("demo"));
        if self.demo_only && !demo {
            return self.fail(
                run,
                why(
                    "error",
                    "not-demo",
                    format!("{id} shows real sessions, and this XenoNet shows made-up data only"),
                    Some("Watch it from a XenoNet that shows your own sessions, or run a demo XenoNet there.".into()),
                ),
                now,
                a,
            );
        }
        let key = f.get("host").and_then(|h| h.get("key")).cloned().unwrap_or(Value::Null);
        let known = self.entry.get("key").filter(|k| truthy(Some(k))).cloned();
        // A demo world is made afresh at every start, key and all, and its
        // data is made up, so its new key is taken.
        if let Some(k) = &known {
            if *k != key && !demo {
                return self.fail(
                    run,
                    why(
                        "error",
                        "key-changed",
                        format!("another XenoNet answered as {id}: its key isn't the one {id} had"),
                        Some(format!("If {id} was set up again, or its alias now points elsewhere on purpose, add it again (xenonet hosts add {id} … ) to accept the new one.")),
                    ),
                    now,
                    a,
                );
            }
        }
        if let Some(r) = self.run.as_mut() {
            r.hello = true;
        }
        let host_id = f.get("host").and_then(|h| h.get("id")).cloned().unwrap_or(Value::Null);
        let mut hello = Map::new();
        for (k, v) in [("id", host_id.clone()), ("key", key.clone())] {
            hello.insert(k.into(), v);
        }
        for k in ["version", "platform", "demo", "protocol", "sinceMs", "detail"] {
            if let Some(v) = f.get(k) {
                hello.insert(k.into(), v.clone());
            }
        }
        self.hello = Value::Object(hello);
        let name_now = self.entry.get("name").filter(|n| !n.is_null()).cloned().unwrap_or(Value::Null);
        let host_name = if host_id.is_null() { Value::Null } else { host_id.clone() };
        if Some(&key) != self.entry.get("key") || host_name != name_now {
            self.entry["key"] = key.clone();
            self.entry["name"] = if truthy(Some(&host_id)) && host_id != json!(id) { host_id.clone() } else { Value::Null };
            self.emit("learn", json!({ "key": key, "name": host_name }));
        }
        self.time(run, now);
        self.fx.push(Effect::Interval { name: format!("time:{run}"), ms: TIME_EVERY_MS });
    }

    fn snapshot(&mut self, run: u64, f: &Value, now: f64) {
        let graph_ok = f.get("graph").is_some_and(|g| g.is_object() && g.get("nodes").is_some_and(Value::is_array));
        let live_ok = f.get("live").is_some_and(Value::is_object);
        let engine_ok = f.get("engine").is_none_or(|e| e.is_null() || e.is_object());
        if !graph_ok || !live_ok || !engine_ok {
            return self.drop_frame("a snapshot that isn't one", now);
        }
        let first = !self.run.as_ref().is_some_and(|r| r.snapshot);
        if let Some(r) = self.run.as_mut() {
            r.snapshot = true;
        }
        self.fx.push(Effect::Clear { name: format!("hello:{run}") });
        self.attempt = 0;
        self.error = Value::Null;
        self.set("live", now);
        let at = f.get("at").and_then(Value::as_f64).map_or(Value::Null, xenonet_base::json::number);
        self.emit(
            "snapshot",
            json!({ "graph": f["graph"], "live": f["live"], "engine": f.get("engine").filter(|e| !e.is_null()).cloned().unwrap_or(Value::Null), "at": at, "first": first }),
        );
    }

    /// The time interval of `run`: ask the host its time again.
    pub fn time_tick(&mut self, run: u64, now: f64) -> Vec<Effect> {
        if self.run.as_ref().is_some_and(|r| r.id == run) {
            self.time(run, now);
        }
        self.take()
    }

    // The offset: the host's time minus the midpoint of the round trip,
    // from the quickest round trip so far.
    fn time(&mut self, run: u64, now: f64) {
        self.seq += 1;
        let id = self.seq;
        if let Some(r) = self.run.as_mut() {
            r.times.insert(id, now);
        }
        self.fx.push(Effect::Write { run, text: encode(&json!({ "t": "time", "id": id })) });
    }

    fn timed(&mut self, _run: u64, f: &Value, now: f64) {
        let fid = f.get("id").and_then(Value::as_f64).filter(|x| x.fract() == 0.0 && *x >= 0.0).map(|x| x as u64);
        let sent = fid.and_then(|id| self.run.as_ref().and_then(|r| r.times.get(&id).copied()));
        let host_now = f.get("now").and_then(Value::as_f64);
        let (Some(id), Some(sent), Some(host_now)) = (fid, sent, host_now) else { return self.drop_frame("a time frame for no question", now) };
        if let Some(r) = self.run.as_mut() {
            r.times.remove(&id);
        }
        let rtt = now - sent;
        if self.rtt_ms.is_none_or(|r| rtt <= (r * 2.0).max(250.0)) {
            self.offset_ms = xenonet_base::units::js_round(host_now - f64::midpoint(sent, now));
            self.rtt_ms = Some(self.rtt_ms.map_or(rtt, |r| r.min(rtt)));
            let o = self.offset_ms;
            self.emit("offset", xenonet_base::json::number(o));
        }
        self.heard(now);
    }

    fn heard(&mut self, now: f64) {
        if self.state == "stale" && self.run.as_ref().is_some_and(|r| r.snapshot) {
            self.set("live", now);
        }
    }

    /// The stale interval went off.
    pub fn check_stale(&mut self, now: f64) -> Vec<Effect> {
        if self.state == "live" && self.last_heard.is_some_and(|h| now - h > self.stale_ms) {
            self.set("stale", now);
        }
        self.take()
    }

    fn drop_frame(&mut self, why: &str, now: f64) {
        self.dropped += 1.0;
        self.last_drop = json!({ "at": xenonet_base::json::number(now), "why": slice16(why, 120) });
    }

    // Ends this connection for a reason found here (a bad hello, silence).
    fn fail(&mut self, run: u64, w: Why, _now: f64, _a: &mut dyn Asks) {
        if let Some(r) = self.run.as_mut().filter(|r| r.id == run) {
            if r.problem.is_none() {
                r.problem = Some(w);
            }
        }
        self.fx.push(Effect::Fail { run });
    }

    fn close(&mut self, mut run: Run, reason: &str) {
        if run.problem.is_none() {
            run.problem = Some(why("offline", "stopped", reason.into(), None));
        }
        run.ended = true;
        let id = run.id;
        self.settle(&mut run, reason);
        self.fx.push(Effect::Kill { run: id });
    }

    fn settle(&mut self, run: &mut Run, reason: &str) {
        self.fx.push(Effect::Clear { name: format!("hello:{}", run.id) });
        self.fx.push(Effect::Clear { name: format!("time:{}", run.id) });
        let id_s = self.id();
        for p in std::mem::take(&mut run.pending) {
            self.fx.push(Effect::Clear { name: format!("get:{p}") });
            self.fx.push(Effect::Reject { id: p, message: format!("{id_s}: {reason}"), status: 503 });
        }
    }

    /// The run's program ended (or couldn't start).
    pub fn end(&mut self, run: u64, e: &Ended<'_>, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        let Some(mut r) = self.run.take() else { return self.take() };
        if r.id != run || r.ended {
            self.run = Some(r);
            return self.take();
        }
        r.ended = true;
        self.settle(&mut r, "the connection ended");
        let tilde = |p: &str| a.tilde(p);
        let w = match r.problem.take() {
            Some(p) => p,
            None => {
                let bye = r.bye.clone();
                let ended = Ended { exit_code: e.exit_code, signal: e.signal.clone(), stderr: &r.stderr, bye: bye.as_ref(), spawn_error: e.spawn_error };
                why_ended(&self.entry, &ended, self.options.ssh_config.as_deref(), &tilde)
            }
        };
        self.ended(w, now, a);
        self.take()
    }

    fn ended(&mut self, w: Why, now: f64, a: &mut dyn Asks) {
        let mut err = Map::new();
        err.insert("code".into(), json!(w.code));
        err.insert("message".into(), json!(w.message));
        if let Some(f) = &w.fix {
            err.insert("fix".into(), json!(f));
        }
        self.error = Value::Object(err);
        self.next_try_at = None;
        if !self.stopped && !self.one_shot {
            let (min, max) = self.backoff;
            let delay = if w.state == "error" { max } else { (min * 2f64.powi(self.attempt as i32)).min(max) * (0.75 + a.random() * 0.5) };
            self.attempt = (self.attempt + 1).min(16);
            self.next_try_at = Some(now + xenonet_base::units::js_round(delay));
            self.fx.push(Effect::Timer { name: "retry".into(), ms: delay });
        }
        self.set(w.state, now);
    }

    /// The retry timer went off.
    pub fn retry(&mut self, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        self.connect(now, a);
        self.take()
    }

    fn set(&mut self, state: &'static str, now: f64) {
        if state == self.state && state != "offline" && state != "error" {
            return;
        }
        let changed = state != self.state;
        self.state = state;
        if changed {
            self.since = now;
        }
        if state == "live" || state == "connecting" {
            self.next_try_at = None;
        }
        let info = self.info();
        self.emit("state", info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Options {
        Options { ssh_config: Some("/x/ssh_config".into()), ssh_bin: vec!["ssh".into()], bin: "/x/xenonet".into(), entry: None, env: Map::new() }
    }

    #[test]
    fn ssh_as_hosts_md_sets_it() {
        let e = json!({ "id": "vm1", "transport": "ssh", "port": 47840, "since": "7d", "command": "xenonet" });
        let (file, args, _) = transport(&e, &opts()).unwrap();
        assert_eq!(file, "ssh");
        let mut want: Vec<String> = vec!["-F".into(), "/x/ssh_config".into()];
        want.extend(SSH_OPTIONS.iter().map(ToString::to_string));
        want.extend(["--", "vm1", "xenonet", "relay", "--port", "47840", "--since", "7d"].iter().map(ToString::to_string));
        assert_eq!(args, want);
        let bad = json!({ "id": "vm1", "transport": "ssh", "port": 47840, "since": "7d", "command": "xenonet; rm" });
        assert!(transport(&bad, &opts()).unwrap_err().contains("not a xenonet command"));
    }

    #[test]
    fn why_a_connection_ended() {
        let e = json!({ "id": "vm1", "transport": "ssh", "ssh": { "hostname": "nova-mac.local", "user": "nova" } });
        let t = |p: &str| p.to_string();
        let w = why_ended(&e, &Ended { exit_code: Some(255.0), signal: None, stderr: "Host key verification failed.\n", bye: None, spawn_error: None }, Some("/x/c"), &t);
        assert_eq!((w.state, w.code), ("error", "host-key"));
        let w = why_ended(
            &e,
            &Ended { exit_code: Some(255.0), signal: None, stderr: "ssh: connect to host nova-mac.local port 22: Connection refused\n", bye: None, spawn_error: None },
            None,
            &t,
        );
        assert_eq!(w.message, "ssh couldn't reach vm1: connect to host vm1 port 22: Connection refused");
        assert_eq!(shell_path("~/Library/Application Support/x"), "~/'Library/Application Support/x'");
    }
}
