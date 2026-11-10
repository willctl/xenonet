//! Drives XenoNet Dev from scripts, tests and agents. It starts an
//! instance (see xenonet-components' instances), talks to its control
//! channel (POST /api/dev/control on the instance's own server) and quits
//! it. The console MCP server, xenonet console and the end-to-end tests all
//! go through here.
//!
//! What it waits on is asked of its host (`DevHost`): requests to the
//! instance's server, whether a process is there, signals, the machine's
//! processes, starting XenoNet Dev and whether it has exited, and the
//! environment it starts with. Tests stand in for the machine that way.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use xenonet_components::instances::{instance, INSTANCES};
use xenonet_os::error::Failure;
use xenonet_os::files;
use xenonet_os::nodepath::NATIVE;
use xenonet_text::json::{number_string, stringify};
use xenonet_text::text::{display, truthy};
use xenonet_text::time::now_ms;
use xenonet_text::value::raw;

/// A request to an instance's server: a GET, or a POST of this JSON.
pub struct Request {
    pub url: String,
    pub post: Option<String>,
    pub timeout_ms: Option<f64>,
}

/// What the server answered.
pub struct Response {
    pub status: u16,
    pub status_text: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn json(&self) -> Result<Value, xenonet_text::json::SyntaxError> {
        xenonet_text::json::parse(&String::from_utf8_lossy(&self.body))
    }

    // Its body as JSON, a body that isn't failing as a SyntaxError.
    fn json_or_fail(&self) -> Result<Value, Failure> {
        self.json().map_err(|e| {
            let mut m = Map::new();
            m.insert("name".into(), json!("SyntaxError"));
            Failure::with(e.to_string(), m)
        })
    }
}

/// How XenoNet Dev starts: its program and arguments.
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

/// What driving an instance waits on. A request that couldn't reach the
/// server fails with its cause's code (`causeCode`: ECONNREFUSED when
/// nothing listens) and message (`causeMessage`).
#[async_trait(?Send)]
pub trait DevHost {
    async fn fetch(&self, req: &Request) -> Result<Response, Failure>;
    async fn alive(&self, pid: f64) -> bool;
    async fn kill(&self, pid: f64, signal: &str);
    /// The machine's processes as `parse_processes` reads ps, or null
    /// where it can't list them.
    async fn processes(&self) -> Value;
    /// The environment XenoNet Dev starts from.
    async fn env(&self) -> Map<String, Value>;
    /// XenoNet Dev's program for this checkout.
    async fn command(&self) -> Result<Launch, Failure>;
    /// Starts it detached, in `cwd`, with `env`, its output appended to
    /// `log_file`.
    async fn spawn(&self, launch: &Launch, cwd: &str, env: &Map<String, Value>, log_file: &str) -> Result<(), Failure>;
    /// What the copy it started exited with (its code or signal), or null
    /// while it runs.
    async fn exited(&self) -> Value;
    async fn sleep(&self, ms: f64);
}

/// Where an instance keeps its Electron data, its single-instance lock
/// among it: a folder of its name in the system's app data folder.
pub fn user_data_dir(name: &str, platform: &str, appdata: Option<&str>, xdg_config_home: Option<&str>, home: &str) -> String {
    match platform {
        "darwin" => NATIVE.join(&[home, "Library", "Application Support", name]),
        "win32" => NATIVE.join(&[&appdata.map_or_else(|| NATIVE.join(&[home, "AppData", "Roaming"]), String::from), name]),
        _ => NATIVE.join(&[&xdg_config_home.filter(|x| !x.is_empty()).map_or_else(|| NATIVE.join(&[home, ".config"]), String::from), name]),
    }
}

/// How to start XenoNet Dev from this checkout: XenoNet Dev.app on a Mac
/// when `npm run dev:install` put it in place for this checkout (it
/// carries the name and icon), else the Electron npm installed (None: the
/// caller finds it), with these arguments.
pub fn electron_command(root: &str, home: &str, platform: &str, software_rendering: bool) -> (Option<String>, Vec<String>) {
    let bundle = NATIVE.join(&[home, "Applications", "XenoNet Dev.app"]);
    let pointer = NATIVE.join(&[&bundle, "Contents", "Resources", "app", "main.js"]);
    if platform == "darwin" {
        if let Ok(text) = std::fs::read_to_string(&pointer) {
            if text.contains(&xenonet_text::url::file_url(&NATIVE.join(&[root, "electron", "main.js"]))) {
                return (Some(NATIVE.join(&[&bundle, "Contents", "MacOS", "Electron"])), Vec::new());
            }
        }
    }
    // Headless Linux and WSL test displays have no hardware WebGL context.
    let mut args = vec![root.to_string()];
    if platform == "linux" && software_rendering {
        args.extend(["--use-angle=swiftshader".into(), "--enable-unsafe-swiftshader".into(), "--ignore-gpu-blocklist".into()]);
    }
    (None, args)
}

/// The pid in a SingletonLock link ("mac.local-4321"), if it names one.
fn lock_pid(link: &str) -> Option<f64> {
    let digits: String = link.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    if digits.is_empty() || !link[..link.len() - digits.len()].ends_with('-') {
        return None;
    }
    digits.parse::<f64>().ok().filter(|p| *p > 0.0)
}

fn connects(socket: &str) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(socket).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
        false
    }
}

/// The process holding an instance's single-instance lock, or null.
/// Chromium keeps the lock in the data folder: a link, SingletonLock,
/// naming the host and pid that hold it, and the socket a second launch
/// hands its arguments to (SingletonSocket). A process that died leaves
/// both behind, and its pid may since be another program's, so only a live
/// process listening on the socket counts. Windows has neither file, so
/// there it's always null.
pub async fn lock_holder(host: &dyn DevHost, dir: &str) -> Value {
    let Ok(link) = std::fs::read_link(NATIVE.join(&[dir, "SingletonLock"])) else { return Value::Null };
    let Some(pid) = lock_pid(&link.to_string_lossy()) else { return Value::Null };
    if !host.alive(pid).await {
        return Value::Null;
    }
    // The socket's link points into a temporary folder: a path short
    // enough for a socket, where the data folder's might not be.
    let socket = NATIVE.join(&[dir, "SingletonSocket"]);
    let socket = files::realpath(&socket).unwrap_or(socket);
    if connects(&socket) {
        xenonet_text::json::number(pid)
    } else {
        Value::Null
    }
}

// Whether a request failed because nothing listens on its port.
fn refused(e: &Failure) -> bool {
    e.field("causeCode").and_then(Value::as_str) == Some("ECONNREFUSED")
}

/// A process of the machine's: its pid, its parent's, and when it started,
/// which tells it from a later one given the same pid.
#[derive(Clone, Debug, PartialEq)]
struct Proc {
    pid: f64,
    ppid: f64,
    start: Value,
}

/// The machine's processes from `ps -A -o pid=,ppid=,lstart=`, each as
/// { pid, ppid, start }.
pub fn parse_processes(text: &str) -> Value {
    let number = |s: &str| Some(s).filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).and_then(|s| s.parse::<u64>().ok());
    let row = |line: &str| -> Option<Value> {
        let (pid, rest) = line.trim_start().split_once(char::is_whitespace)?;
        let (ppid, start) = rest.trim_start().split_once(char::is_whitespace)?;
        let start = start.trim();
        (!start.is_empty()).then(|| json!({ "pid": number(pid), "ppid": number(ppid), "start": start })).filter(|r| !r["pid"].is_null() && !r["ppid"].is_null())
    };
    Value::Array(text.split('\n').filter_map(row).collect())
}

fn procs_of(v: &Value) -> Option<Vec<Proc>> {
    let list = v.as_array()?;
    Some(list.iter().filter_map(|p| Some(Proc { pid: p.get("pid")?.as_f64()?, ppid: p.get("ppid")?.as_f64()?, start: p.get("start").cloned().unwrap_or(Value::Null) })).collect())
}

/// `pid`'s descendants in a process table: its children, theirs, and so
/// on, in the table's order a generation at a time.
fn descendants(table: &[Proc], pid: f64) -> Vec<Proc> {
    let mut out: Vec<Proc> = Vec::new();
    let mut next = vec![pid];
    let mut i = 0;
    while i < next.len() {
        let parent = next[i];
        for p in table.iter().filter(|p| p.ppid == parent) {
            if p.pid == pid || out.iter().any(|o| o.pid == p.pid) {
                continue;
            }
            out.push(p.clone());
            next.push(p.pid);
        }
        i += 1;
    }
    out
}

/// Whether a hard stop of an instance takes its whole process tree: a test
/// instance's only, and never on the ports of the installed XenoNet or of
/// XenoNet Dev's demo and dev instances (47840 to 47842), which are yours.
pub fn owns_tree(id: &str, port: &Value) -> bool {
    id == "test" && !(47840.0..=47842.0).contains(&xenonet_text::text::to_number(Some(port)))
}

/// One instance of XenoNet Dev, and how to reach it.
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub port: Value,
    pub base: String,
    pub made_up: bool,
    pub lock_dir: String,
    pub log_dir: String,
    pub root: String,
    pub settle_ms: f64,
    pub term_ms: f64,
}

/// The settings `xenonet(id, opts)` was given, and the machine's.
pub struct Opts<'a> {
    pub port: Option<Value>,
    pub user_data: Option<&'a str>,
    pub log_dir: Option<&'a str>,
    pub settle_ms: f64,
    pub term_ms: f64,
    pub root: &'a str,
    pub tmpdir: &'a str,
    pub platform: &'a str,
    pub appdata: Option<&'a str>,
    pub xdg_config_home: Option<&'a str>,
    pub home: &'a str,
}

impl Instance {
    pub fn new(id: &str, o: &Opts<'_>) -> Result<Instance, String> {
        let Some(i) = instance(id) else {
            let ids: Vec<&str> = INSTANCES.iter().map(|i| i.id).collect();
            return Err(format!("no instance \"{id}\"; there's {}", ids.join(", ")));
        };
        let port = o.port.clone().filter(|p| !p.is_null()).unwrap_or(json!(i.port));
        let base = format!("http://127.0.0.1:{}", display(&port));
        let lock_dir = match o.user_data {
            Some(d) => d.to_string(),
            None if id == "test" => NATIVE.join(&[o.tmpdir, &format!("xenonet-demo-{}", display(&port)), "electron"]),
            None => user_data_dir(i.name, o.platform, o.appdata, o.xdg_config_home, o.home),
        };
        Ok(Instance {
            id: id.into(),
            name: i.name.into(),
            port,
            base,
            made_up: i.demo,
            lock_dir,
            log_dir: o.log_dir.map_or_else(|| NATIVE.join(&[o.root, ".xenonet", "logs"]), String::from),
            root: o.root.into(),
            settle_ms: o.settle_ms,
            term_ms: o.term_ms,
        })
    }

    fn port_text(&self) -> String {
        display(&self.port)
    }

    // What's on the port: "up" (a XenoNet answers), "down" (nothing
    // listens, so the connection is refused) or "busy" (something listens
    // but doesn't answer, such as a server on its way out).
    async fn probe(&self, host: &dyn DevHost) -> &'static str {
        match host.fetch(&Request { url: format!("{}/api/health", self.base), post: None, timeout_ms: Some(2000.0) }).await {
            Ok(r) if r.ok() => "up",
            Ok(_) => "busy",
            Err(e) if refused(&e) => "down",
            Err(_) => "busy",
        }
    }

    pub async fn health(&self, host: &dyn DevHost) -> bool {
        self.probe(host).await == "up"
    }

    // Until `pid` has exited, for `ms` at most; whether it did.
    async fn gone(&self, host: &dyn DevHost, pid: f64, ms: f64) -> bool {
        let until = now_ms() + ms;
        loop {
            if !host.alive(pid).await {
                return true;
            }
            if now_ms() >= until {
                return false;
            }
            host.sleep(100.0).await;
        }
    }

    // What's said of a copy that holds the lock without answering here.
    fn held(&self, pid: &Value) -> String {
        let (pid, name, port) = (display(pid), &self.name, self.port_text());
        format!("process {pid} holds {name}'s single-instance lock but doesn't answer on port {port}: it's stuck (on its way out, or on an error page) or running on another port, and a new copy would quit at once. Launch with restart to end it, or end it yourself: kill {pid}, then kill -9 {pid} if it stays.")
    }

    /// One control request. While the server restarts, the request waits
    /// for it to come back, for up to `wait` ms; it's sent only once, so
    /// nothing runs twice.
    pub async fn call(&self, host: &dyn DevHost, body: &Value, wait: f64) -> Result<Value, Failure> {
        let until = now_ms() + wait;
        let req = Request { url: format!("{}/api/dev/control", self.base), post: Some(stringify(body, 0)), timeout_ms: None };
        loop {
            let res = match host.fetch(&req).await {
                Ok(r) => r,
                Err(e) => {
                    let refused = refused(&e);
                    if refused && now_ms() < until {
                        host.sleep(400.0).await;
                        continue;
                    }
                    // Sent, then cut off: it may have run, so it isn't sent again.
                    return Err(Failure::msg(if refused {
                        format!("{} isn't running (nothing on port {}); launch it first", self.name, self.port_text())
                    } else {
                        let why = e.field("causeMessage").map_or_else(|| e.message.clone(), display);
                        format!("lost the connection to {} mid-request (its server restarted?): {why}", self.name)
                    }));
                }
            };
            let status_line = format!("{} {}", res.status, res.status_text);
            let data = res.json().unwrap_or_else(|_| json!({ "error": status_line }));
            if !res.ok() {
                let why = raw(Some(&data), "error").filter(|e| !e.is_null()).map_or(status_line, display);
                return Err(Failure::msg(why));
            }
            return Ok(data);
        }
    }

    /// The instance and its window, or null if it isn't running.
    pub async fn ping(&self, host: &dyn DevHost) -> Value {
        if !self.health(host).await {
            return Value::Null;
        }
        match self.call(host, &json!({ "op": "ping" }), 0.0).await {
            Ok(v) => v,
            Err(e) => json!({ "instance": self.id, "name": self.name, "port": self.port, "controlled": false, "error": e.message() }),
        }
    }

    pub async fn ready(&self, host: &dyn DevHost, timeout: f64) -> Result<Value, Failure> {
        let until = now_ms() + timeout;
        let mut last = Value::Null;
        while now_ms() < until {
            last = self.ping(host).await;
            if truthy(raw(raw(Some(&last), "page"), "ready")) {
                return Ok(last);
            }
            if !last.is_null() && raw(Some(&last), "controlled") == Some(&json!(false)) {
                return Err(Failure::msg(format!(
                    "port {} has a XenoNet server with no window to control ({})",
                    self.port_text(),
                    raw(Some(&last), "error").map_or_else(|| "undefined".into(), display)
                )));
            }
            host.sleep(500.0).await;
        }
        let page = if last.is_null() { String::new() } else { format!(": {}", raw(Some(&last), "page").map_or_else(|| "undefined".into(), |p| stringify(p, 0))) };
        Err(Failure::msg(format!("{} wasn't ready after {}s{page}", self.name, number_string(timeout / 1000.0))))
    }

    /// Starts the instance unless it's running, and waits until its page is
    /// ready. `show`: "inactive" brings the window up without taking focus,
    /// "front" focuses it, "hidden" never shows it. `restart` quits it
    /// first, and ends a copy stuck holding its lock. What it returns says
    /// what it ended on the way, if anything.
    pub async fn launch(&self, host: &dyn DevHost, show: Option<&Value>, restart: bool, timeout: f64) -> Result<Value, Failure> {
        let show = show.filter(|s| !s.is_null()).map_or_else(|| if self.id == "test" { "hidden".to_string() } else { "inactive".to_string() }, display);
        // An unknown word would reach the window as XENONET_SHOW, which shows
        // it in front: refused here instead.
        if !["inactive", "front", "hidden"].contains(&show.as_str()) {
            return Err(Failure::msg(format!("a window shows inactive, front or hidden, not \"{show}\"")));
        }
        let mut ended: Vec<Value> = Vec::new();
        let done = |p: Value, ended: &Vec<Value>| -> Value {
            match (p, ended.is_empty()) {
                (Value::Object(mut m), false) => {
                    m.insert("ended".into(), json!(ended));
                    Value::Object(m)
                }
                (p, _) => p,
            }
        };
        let mut running = self.ping(host).await;
        if !running.is_null() && restart {
            self.quit(host, 20000.0).await?;
            running = Value::Null;
        }
        if raw(Some(&running), "controlled") == Some(&json!(false)) {
            return Err(Failure::msg(format!("port {} has a XenoNet server with no window (npm run dev or npm run demo?); stop it, or use another instance", self.port_text())));
        }
        if running.is_null() {
            running = self.settle(host, restart, &mut ended).await?;
        }
        if !running.is_null() {
            if show != "hidden" && !truthy(raw(raw(Some(&running), "window"), "visible")) {
                self.call(host, &json!({ "op": "window", "show": show }), 20000.0).await?;
            }
            let r = self.ready(host, timeout).await?;
            return Ok(done(r, &ended));
        }
        let log_file = NATIVE.join(&[&self.log_dir, &format!("{}.log", self.id)]);
        files::mkdir_p(&self.log_dir, None)?;
        let mut env = host.env().await;
        env.insert("XENONET_INSTANCE".into(), json!(self.id));
        env.insert("XENONET_SHOW".into(), json!(show));
        env.shift_remove("XENONET_TEST_PROFILE");
        if self.id == "test" {
            env.insert("XENONET_TEST_PROFILE".into(), json!(self.lock_dir));
        }
        // Set by Electron apps for their child processes; XenoNet Dev would
        // start as plain Node.
        env.shift_remove("ELECTRON_RUN_AS_NODE");
        for k in ["XENONET_DEMO", "XENONET_OFFSCREEN", "XENONET_CONTROL"] {
            env.shift_remove(k);
        }
        env.insert("XENONET_PORT".into(), json!(self.port_text()));
        // Muted, unless XENONET_MUTED=0 says otherwise; the film and
        // showcase instances keep their sound, for recordings.
        if ["dev", "demo", "test"].contains(&self.id.as_str()) && env.get("XENONET_MUTED").is_none_or(Value::is_null) {
            env.insert("XENONET_MUTED".into(), json!("1"));
        }
        let launch = host.command().await?;
        host.spawn(&launch, &self.root, &env, &log_file).await?;
        let relative = NATIVE.relative(&self.root, &log_file);
        let until = now_ms() + timeout;
        while now_ms() < until {
            let p = self.ping(host).await;
            if truthy(raw(raw(Some(&p), "page"), "ready")) {
                return Ok(done(p, &ended));
            }
            let exited = host.exited().await;
            if !exited.is_null() && p.is_null() {
                // It handed off to a copy that took the lock in the meantime.
                let holder = lock_holder(host, &self.lock_dir).await;
                if !holder.is_null() {
                    return Err(Failure::msg(format!("{} quit at once: {}", self.name, self.held(&holder))));
                }
                let text = files::read_text(&log_file)?;
                let lines: Vec<&str> = xenonet_text::text::trim(&text).split('\n').collect();
                let tail = lines[lines.len().saturating_sub(12)..].join("\n");
                return Err(Failure::msg(format!("{} exited ({}) before it was ready. The end of {relative}:\n{tail}", self.name, display(&exited))));
            }
            host.sleep(500.0).await;
        }
        Err(Failure::msg(format!("{} wasn't ready after {}s; see {relative}", self.name, number_string(timeout / 1000.0))))
    }

    // Before a new copy starts, the port has to be free and the instance's
    // lock not held. A copy on its way in or out gets `settle_ms` to get
    // there. One that holds the lock all that time without answering here
    // is stuck: it's ended when its data is made up (demo, test), and
    // reported otherwise, for a launch with restart to end, which doesn't
    // wait. Each copy ended goes on `ended`. Returns the instance if it
    // came up meanwhile.
    async fn settle(&self, host: &dyn DevHost, restart: bool, ended: &mut Vec<Value>) -> Result<Value, Failure> {
        let mut until = now_ms() + self.settle_ms;
        loop {
            let state = self.probe(host).await;
            if state == "up" {
                let p = self.ping(host).await;
                if !p.is_null() && raw(Some(&p), "controlled") != Some(&json!(false)) {
                    return Ok(p);
                }
            }
            let holder = lock_holder(host, &self.lock_dir).await;
            if state == "down" && holder.is_null() {
                return Ok(Value::Null);
            }
            let late = now_ms() >= until;
            if !holder.is_null() && (restart || (late && self.made_up)) {
                self.end(host, holder.as_f64().unwrap_or(f64::NAN)).await?;
                ended.push(json!({ "pid": holder, "reason": format!("it held {}'s single-instance lock without answering on port {}", self.name, self.port_text()) }));
                // Its server gets as long again to let go of the port.
                until = now_ms() + self.settle_ms;
                continue;
            }
            if late {
                if !holder.is_null() {
                    return Err(Failure::msg(self.held(&holder)));
                }
                if state == "up" {
                    return Err(Failure::msg(format!(
                        "port {} has a XenoNet server with no window (npm run dev or npm run demo?); stop it, or use another instance",
                        self.port_text()
                    )));
                }
                return Err(Failure::msg(format!(
                    "something on port {} doesn't answer (a server still on its way out, or another program); stop it, or run {} on another port",
                    self.port_text(),
                    self.name
                )));
            }
            host.sleep(300.0).await;
        }
    }

    // Ends a copy holding the lock: SIGTERM asks it to quit, which one
    // stuck on its way out ignores, so SIGKILL follows.
    async fn end(&self, host: &dyn DevHost, pid: f64) -> Result<(), Failure> {
        host.kill(pid, "SIGTERM").await;
        if self.gone(host, pid, self.term_ms).await {
            return Ok(());
        }
        host.kill(pid, "SIGKILL").await;
        if !self.gone(host, pid, 5000.0).await {
            return Err(Failure::msg(format!("process {} holds {}'s single-instance lock and is still there after kill -9", number_string(pid), self.name)));
        }
        Ok(())
    }

    // The machine's processes, or None where it can't list them.
    async fn processes(&self, host: &dyn DevHost) -> Option<Vec<Proc>> {
        procs_of(&host.processes().await)
    }

    // A test instance's process tree, but the app itself: its helpers (such
    // as Chromium's GPU and network processes) and the service it started,
    // with what they started. None for any other instance, or where the
    // machine can't list its processes.
    async fn tree_of(&self, host: &dyn DevHost, pid: Option<f64>) -> Option<Vec<Proc>> {
        let pid = pid.filter(|_| owns_tree(&self.id, &self.port))?;
        self.processes(host).await.map(|t| descendants(&t, pid))
    }

    // The pids of a tree taken earlier that are still there, each the same
    // process (started when it was), with the app's descendants now while
    // it's there.
    async fn left_of(&self, host: &dyn DevHost, pid: f64, before: &[Proc]) -> Vec<f64> {
        let table = self.processes(host).await.unwrap_or_default();
        let mut all = before.to_vec();
        if host.alive(pid).await {
            all.extend(descendants(&table, pid));
        }
        let mut left: Vec<f64> = Vec::new();
        for p in all {
            if p.pid != pid && !left.contains(&p.pid) && table.iter().any(|q| q.pid == p.pid && q.start == p.start) {
                left.push(p.pid);
            }
        }
        left
    }

    // Kills each of `pids`, and whether all have gone within 5 seconds.
    async fn killed(&self, host: &dyn DevHost, pids: &[f64]) -> bool {
        for p in pids {
            host.kill(*p, "SIGKILL").await;
        }
        let until = now_ms() + 5000.0;
        for p in pids {
            if !self.gone(host, *p, (until - now_ms()).max(0.0)).await {
                return false;
            }
        }
        true
    }

    /// Quits the instance, and returns once its process has exited and its
    /// server has let go of the port. A process still there `timeout` ms
    /// after its server went is stuck on its way out and is killed. For a
    /// test instance, that takes its whole tree, as it stood before it was
    /// told to quit and as it stands then: killed alone, the app would
    /// leave its GPU and network processes and its service running
    /// (`ended`).
    pub async fn quit(&self, host: &dyn DevHost, timeout: f64) -> Result<Value, Failure> {
        let running = self.ping(host).await;
        if running.is_null() {
            return Ok(json!({ "quit": false, "running": false }));
        }
        let pid = raw(Some(&running), "pid").and_then(Value::as_f64).filter(|p| p.fract() == 0.0);
        let tree = self.tree_of(host, pid).await;
        let _ = self.call(host, &json!({ "op": "quit" }), 0.0).await;
        let until = now_ms() + timeout;
        let pids = |l: &[f64]| Value::Array(l.iter().map(|p| xenonet_text::json::number(*p)).collect());
        loop {
            let state = self.probe(host).await;
            let alive = match pid {
                Some(p) => host.alive(p).await,
                None => false,
            };
            if state == "down" && !alive {
                return Ok(json!({ "quit": true }));
            }
            if now_ms() >= until {
                if state == "up" {
                    return Err(Failure::msg(format!("{} didn't quit in {}s", self.name, number_string(timeout / 1000.0))));
                }
                let left = match &tree {
                    Some(t) => self.left_of(host, pid.unwrap_or(f64::NAN), t).await,
                    None => Vec::new(),
                };
                if !alive {
                    // The app has gone, and what still holds the port may be
                    // the service it started, stuck on its way out.
                    if !left.is_empty() && self.killed(host, &left).await && self.probe(host).await == "down" {
                        return Ok(json!({ "quit": true, "ended": pids(&left) }));
                    }
                    return Err(Failure::msg(format!("{} quit, but something on port {} still doesn't answer", self.name, self.port_text())));
                }
                let pid = pid.unwrap_or(f64::NAN);
                let all: Vec<f64> = std::iter::once(pid).chain(left.iter().copied()).collect();
                if self.killed(host, &all).await {
                    let mut out = json!({ "quit": true, "killed": xenonet_text::json::number(pid) });
                    if !left.is_empty() {
                        out["ended"] = pids(&left);
                    }
                    return Ok(out);
                }
                let others = if left.is_empty() { String::new() } else { format!(" or one of {}", left.iter().map(|p| number_string(*p)).collect::<Vec<_>>().join(", ")) };
                return Err(Failure::msg(format!(
                    "{} didn't quit in {}s, and process {}{others} is still there after kill -9",
                    self.name,
                    number_string(timeout / 1000.0),
                    number_string(pid)
                )));
            }
            host.sleep(300.0).await;
        }
    }

    /// A call into the page's driver. If the page is reloading (it does
    /// after a server restart), it waits for it, then tries once more.
    pub async fn driver(&self, host: &dyn DevHost, method: &Value, args: &[Value]) -> Result<Value, Failure> {
        let timeout = args.iter().find(|a| matches!(a, Value::Object(m) if m.contains_key("timeout"))).and_then(|a| a.get("timeout")).cloned();
        let mut body = Map::new();
        body.insert("op".into(), json!("driver"));
        body.insert("method".into(), method.clone());
        body.insert("args".into(), json!(args));
        if let Some(t) = timeout {
            body.insert("timeout".into(), t);
        }
        let body = Value::Object(body);
        match self.call(host, &body, 20000.0).await {
            Err(e) if e.message().contains("has no driver") => {
                self.ready(host, 60000.0).await?;
                self.call(host, &body, 20000.0).await
            }
            r => r,
        }
    }

    pub async fn simulate(&self, host: &dyn DevHost, kind: &Value, a: Option<&Value>, b: Option<&Value>) -> Result<Value, Failure> {
        let mut body = Map::new();
        body.insert("kind".into(), kind.clone());
        if let Some(a) = a {
            body.insert("a".into(), a.clone());
        }
        if let Some(b) = b {
            body.insert("b".into(), b.clone());
        }
        let res = host.fetch(&Request { url: format!("{}/api/dev/simulate", self.base), post: Some(stringify(&Value::Object(body), 0)), timeout_ms: None }).await?;
        let data = res.json_or_fail()?;
        if !res.ok() {
            let why = match &data {
                Value::Null => return Err(Failure::type_error("Cannot read properties of null (reading 'error')")),
                d => raw(Some(d), "error").filter(|e| !e.is_null()).map_or_else(|| res.status_text.clone(), display),
            };
            return Err(Failure::msg(why));
        }
        Ok(data)
    }

    /// Any read-only API route, such as /api/live.
    pub async fn api(&self, host: &dyn DevHost, route: &str) -> Result<Value, Failure> {
        host.fetch(&Request { url: format!("{}{route}", self.base), post: None, timeout_ms: None }).await?.json_or_fail()
    }

    /// A control request with `op` and the options given, as screenshot,
    /// input, window, record, logs and eval send them.
    pub fn body(op: &str, first: Option<(&str, Value)>, opts: Option<&Value>) -> Value {
        let mut b = Map::new();
        b.insert("op".into(), json!(op));
        if let Some((k, v)) = first {
            b.insert(k.into(), v);
        }
        if let Some(Value::Object(o)) = opts {
            for (k, v) in o {
                b.insert(k.clone(), v.clone());
            }
        }
        if op == "record" && raw(opts, "action").and_then(Value::as_str) == Some("stop") {
            b.insert("timeout".into(), json!(120000));
        }
        Value::Object(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // Runs to its end what never waits, as the made-up machines' answers.
    fn ready<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("a made-up machine waited"),
        }
    }

    fn refused() -> Failure {
        let mut m = Map::new();
        m.insert("causeCode".into(), json!("ECONNREFUSED"));
        Failure::with("fetch failed", m)
    }

    /// A made-up machine: its server, its processes (pid, parent) and
    /// what was killed.
    #[derive(Default)]
    struct Fake {
        // Nothing listens, ever.
        down: bool,
        // Its server answers until it's told to quit, then is gone.
        quitting: RefCell<bool>,
        table: RefCell<Vec<(u64, u64)>>,
        killed: RefCell<Vec<(u64, String)>>,
    }

    #[async_trait(?Send)]
    impl DevHost for Fake {
        async fn fetch(&self, req: &Request) -> Result<Response, Failure> {
            if self.down {
                return Err(refused());
            }
            if req.url.ends_with("/api/dev/control") {
                *self.quitting.borrow_mut() |= req.post.as_deref().unwrap_or("").contains("\"quit\"");
                return Ok(Response { status: 200, status_text: String::new(), body: br#"{"instance":"test","pid":501}"#.to_vec() });
            }
            if *self.quitting.borrow() {
                return Err(refused());
            }
            Ok(Response { status: 200, status_text: String::new(), body: Vec::new() })
        }
        async fn alive(&self, pid: f64) -> bool {
            self.table.borrow().iter().any(|(p, _)| *p as f64 == pid)
        }
        async fn kill(&self, pid: f64, signal: &str) {
            self.killed.borrow_mut().push((pid as u64, signal.into()));
            self.table.borrow_mut().retain(|(p, _)| *p as f64 != pid);
        }
        async fn processes(&self) -> Value {
            Value::Array(self.table.borrow().iter().map(|(p, pp)| json!({ "pid": p, "ppid": pp, "start": format!("at {p}") })).collect())
        }
        async fn env(&self) -> Map<String, Value> {
            Map::new()
        }
        async fn command(&self) -> Result<Launch, Failure> {
            Err(Failure::msg("no XenoNet Dev here"))
        }
        async fn spawn(&self, _launch: &Launch, _cwd: &str, _env: &Map<String, Value>, _log_file: &str) -> Result<(), Failure> {
            Err(Failure::msg("no XenoNet Dev here"))
        }
        async fn exited(&self) -> Value {
            Value::Null
        }
        async fn sleep(&self, _ms: f64) {}
    }

    // Spelled as on a unix host; on Windows the JS tests run against the
    // Rust (test:rust) check where data lives.
    #[cfg(unix)]
    #[test]
    fn where_data_lives() {
        assert_eq!(user_data_dir("XenoNet Dev (demo)", "darwin", None, None, "/Users/nova"), "/Users/nova/Library/Application Support/XenoNet Dev (demo)");
        assert_eq!(user_data_dir("XenoNet Dev", "linux", None, Some("/xdg"), "/home/nova"), "/xdg/XenoNet Dev");
        assert_eq!(lock_pid("mac.local-4321"), Some(4321.0));
        assert_eq!(lock_pid("4321"), None);
    }

    fn opts() -> Opts<'static> {
        Opts {
            port: Some(json!(5999)),
            user_data: Some("/nowhere"),
            log_dir: Some("/nowhere/logs"),
            settle_ms: 300.0,
            term_ms: 300.0,
            root: "/src",
            tmpdir: "/tmp",
            platform: "linux",
            appdata: None,
            xdg_config_home: None,
            home: "/home/nova",
        }
    }

    #[test]
    fn a_call_says_when_nothing_runs() {
        let i = Instance::new("demo", &opts()).unwrap();
        assert_eq!(i.base, "http://127.0.0.1:5999");
        let out = ready(i.call(&Fake { down: true, ..Fake::default() }, &json!({ "op": "ping" }), 0.0));
        assert_eq!(out, Err(Failure::msg("XenoNet Dev (demo) isn't running (nothing on port 5999); launch it first")));
        assert_eq!(Instance::new("prod", &opts()).unwrap_err(), "no instance \"prod\"; there's dev, demo, test, film, installed, showcase");
    }

    #[test]
    fn whose_process_trees_a_hard_stop_takes() {
        assert!(owns_tree("test", &json!(48011)) && owns_tree("test", &json!("47843")));
        for port in [47840, 47841, 47842] {
            assert!(!owns_tree("test", &json!(port)), "{port} is yours");
        }
        assert!(!owns_tree("dev", &json!(48011)) && !owns_tree("demo", &json!(48011)) && !owns_tree("film", &json!(48011)));

        let ps = "  501     1 Mon Oct 19 09:00:00 2026\n  600   501 Mon Oct 19 09:00:01 2026\n  601   501 Mon Oct 19 09:00:01 2026\n  700   600 Mon Oct 19 09:00:02 2026\n  800     1 Mon Oct 19 09:00:03 2026\nnot a row\n   9  x Mon\n";
        let table = procs_of(&parse_processes(ps)).unwrap();
        assert_eq!(table.len(), 5);
        assert_eq!(table[1], Proc { pid: 600.0, ppid: 501.0, start: json!("Mon Oct 19 09:00:01 2026") });
        let tree: Vec<f64> = descendants(&table, 501.0).iter().map(|p| p.pid).collect();
        assert_eq!(tree, [600.0, 601.0, 700.0], "children first, then theirs; never another's");
        assert_eq!(descendants(&table, 800.0), []);
        assert_eq!(procs_of(&Value::Null), None);
    }

    // A test instance stuck on its way out, on a made-up machine: the app
    // (501) with a helper (600) and the service it started (601), which
    // runs a shell (700); 800 is another app's.
    #[test]
    fn a_test_instances_hard_stop_takes_its_tree() {
        let i = Instance::new("test", &opts()).unwrap();
        let fake = Fake { table: RefCell::new(vec![(501u64, 1u64), (600, 501), (601, 501), (700, 601), (800, 1)]), ..Fake::default() };
        let out = ready(i.quit(&fake, 0.0));
        let (killed, table) = (&fake.killed, &fake.table);
        assert_eq!(out, Ok(json!({ "quit": true, "killed": 501, "ended": [600, 601, 700] })));
        assert_eq!(killed.borrow().iter().map(|(p, _)| *p).collect::<Vec<_>>(), [501, 600, 601, 700]);
        assert!(killed.borrow().iter().all(|(_, s)| s == "SIGKILL"));
        assert_eq!(*table.borrow(), [(800, 1)], "another app's process is left alone");
    }
}
