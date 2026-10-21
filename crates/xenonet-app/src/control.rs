//! core/control.js: drives XenoNet Dev from scripts, tests and agents. It
//! starts an instance (see instances.rs), talks to its control channel
//! (POST /api/dev/control on the instance's own server) and quits it. The
//! console MCP server, xenonet console and the end-to-end tests all go
//! through here.
//!
//! Everything it waits on is asked of whoever runs its tasks (task.rs):
//! `fetch` (to the instance's server), `alive`, `kill` and `processes` (the
//! machine's processes, the last as parse_processes reads ps, or null
//! where it can't list them), `command` and `spawn` (starting XenoNet
//! Dev), `exited`, and `env`. Tests stand in for the machine that way, as they did for the JS.

use crate::fsx;
use crate::instances::{instance, INSTANCES};
use crate::jsv::{raw, truthy};
use crate::nodepath::NATIVE;
use crate::task::{now_ms, Failure, Io};
use serde_json::{json, Map, Value};
use xenonet_base::json::{number_string, stringify};
use xenonet_base::text::js_string;

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
            if text.contains(&crate::jstext::path_to_file_url(&NATIVE.join(&[root, "electron", "main.js"]))) {
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
pub async fn lock_holder(io: &Io, dir: &str) -> Result<Value, Failure> {
    let Ok(link) = std::fs::read_link(NATIVE.join(&[dir, "SingletonLock"])) else { return Ok(Value::Null) };
    let Some(pid) = lock_pid(&link.to_string_lossy()) else { return Ok(Value::Null) };
    let alive = io.ask(json!({ "op": "alive", "pid": pid })).await.map_err(|_| Failure::Rethrow)?;
    if !truthy(Some(&alive)) {
        return Ok(Value::Null);
    }
    // The socket's link points into a temporary folder: a path short
    // enough for a socket, where the data folder's might not be.
    let socket = NATIVE.join(&[dir, "SingletonSocket"]);
    let socket = fsx::realpath(&socket).unwrap_or(socket);
    Ok(if connects(&socket) { xenonet_base::json::number(pid) } else { Value::Null })
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
    id == "test" && !(47840.0..=47842.0).contains(&xenonet_base::text::js_number(Some(port)))
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
        let base = format!("http://127.0.0.1:{}", js_string(&port));
        let lock_dir = match o.user_data {
            Some(d) => d.to_string(),
            None if id == "test" => NATIVE.join(&[o.tmpdir, &format!("xenonet-demo-{}", js_string(&port)), "electron"]),
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
        js_string(&self.port)
    }

    // What's on the port: "up" (a XenoNet answers), "down" (nothing
    // listens, so the connection is refused) or "busy" (something listens
    // but doesn't answer, such as a server on its way out).
    async fn probe(&self, io: &Io) -> &'static str {
        match io.ask(json!({ "op": "fetch", "url": format!("{}/api/health", self.base), "timeoutMs": 2000, "read": "none" })).await {
            Ok(r) if r["ok"] == json!(true) => "up",
            Ok(_) => "busy",
            Err(e) if e["causeCode"] == json!("ECONNREFUSED") => "down",
            Err(_) => "busy",
        }
    }

    pub async fn health(&self, io: &Io) -> bool {
        self.probe(io).await == "up"
    }

    // Until `pid` has exited, for `ms` at most; whether it did.
    async fn gone(&self, io: &Io, pid: f64, ms: f64) -> Result<bool, Failure> {
        let until = now_ms() + ms;
        loop {
            if !self.alive(io, pid).await? {
                return Ok(true);
            }
            if now_ms() >= until {
                return Ok(false);
            }
            io.sleep(100.0).await;
        }
    }

    async fn alive(&self, io: &Io, pid: f64) -> Result<bool, Failure> {
        let a = io.ask(json!({ "op": "alive", "pid": pid })).await.map_err(|_| Failure::Rethrow)?;
        Ok(truthy(Some(&a)))
    }

    async fn kill(&self, io: &Io, pid: f64, signal: &str) -> Result<(), Failure> {
        io.ask(json!({ "op": "kill", "pid": pid, "signal": signal })).await.map_err(|_| Failure::Rethrow)?;
        Ok(())
    }

    // What's said of a copy that holds the lock without answering here.
    fn held(&self, pid: &Value) -> String {
        let (pid, name, port) = (js_string(pid), &self.name, self.port_text());
        format!("process {pid} holds {name}'s single-instance lock but doesn't answer on port {port}: it's stuck (on its way out, or on an error page) or running on another port, and a new copy would quit at once. Launch with restart to end it, or end it yourself: kill {pid}, then kill -9 {pid} if it stays.")
    }

    /// One control request. While the server restarts, the request waits
    /// for it to come back, for up to `wait` ms; it's sent only once, so
    /// nothing runs twice.
    pub async fn call(&self, io: &Io, body: &Value, wait: f64) -> Result<Value, Failure> {
        let until = now_ms() + wait;
        loop {
            let res = io.ask(json!({ "op": "fetch", "url": format!("{}/api/dev/control", self.base), "method": "POST", "headers": { "content-type": "application/json" }, "body": stringify(body, 0), "read": "json" })).await;
            let res = match res {
                Ok(r) => r,
                Err(e) => {
                    let refused = e["causeCode"] == json!("ECONNREFUSED");
                    if refused && now_ms() < until {
                        io.sleep(400.0).await;
                        continue;
                    }
                    // Sent, then cut off: it may have run, so it isn't sent again.
                    return Err(Failure::msg(if refused {
                        format!("{} isn't running (nothing on port {}); launch it first", self.name, self.port_text())
                    } else {
                        let why = raw(Some(&e), "causeMessage").filter(|m| !m.is_null()).or_else(|| raw(Some(&e), "message")).map(js_string).unwrap_or_default();
                        format!("lost the connection to {} mid-request (its server restarted?): {why}", self.name)
                    }));
                }
            };
            let status_line = format!("{} {}", js_string(&res["status"]), res.get("statusText").map(js_string).unwrap_or_default());
            let data = if res.get("bodyError").is_some() { json!({ "error": status_line }) } else { res.get("body").cloned().unwrap_or(Value::Null) };
            if res["ok"] != json!(true) {
                let why = raw(Some(&data), "error").filter(|e| !e.is_null()).map_or(status_line, js_string);
                return Err(Failure::msg(why));
            }
            return Ok(data);
        }
    }

    /// The instance and its window, or null if it isn't running.
    pub async fn ping(&self, io: &Io) -> Result<Value, Failure> {
        if !self.health(io).await {
            return Ok(Value::Null);
        }
        match self.call(io, &json!({ "op": "ping" }), 0.0).await {
            Ok(v) => Ok(v),
            Err(e) => Ok(json!({ "instance": self.id, "name": self.name, "port": self.port, "controlled": false, "error": e.message() })),
        }
    }

    pub async fn ready(&self, io: &Io, timeout: f64) -> Result<Value, Failure> {
        let until = now_ms() + timeout;
        let mut last = Value::Null;
        while now_ms() < until {
            last = self.ping(io).await?;
            if truthy(raw(raw(Some(&last), "page"), "ready")) {
                return Ok(last);
            }
            if !last.is_null() && raw(Some(&last), "controlled") == Some(&json!(false)) {
                return Err(Failure::msg(format!(
                    "port {} has a XenoNet server with no window to control ({})",
                    self.port_text(),
                    raw(Some(&last), "error").map_or_else(|| "undefined".into(), js_string)
                )));
            }
            io.sleep(500.0).await;
        }
        let page = if last.is_null() { String::new() } else { format!(": {}", raw(Some(&last), "page").map_or_else(|| "undefined".into(), |p| stringify(p, 0))) };
        Err(Failure::msg(format!("{} wasn't ready after {}s{page}", self.name, number_string(timeout / 1000.0))))
    }

    /// Starts the instance unless it's running, and waits until its page is
    /// ready. `show`: "inactive" brings the window up without taking focus,
    /// "front" focuses it, "hidden" never shows it. `restart` quits it
    /// first, and ends a copy stuck holding its lock. What it returns says
    /// what it ended on the way, if anything.
    pub async fn launch(&self, io: &Io, show: Option<&Value>, restart: bool, timeout: f64) -> Result<Value, Failure> {
        let show = show.filter(|s| !s.is_null()).map_or_else(|| if self.id == "test" { "hidden".to_string() } else { "inactive".to_string() }, js_string);
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
        let mut running = self.ping(io).await?;
        if !running.is_null() && restart {
            self.quit(io, 20000.0).await?;
            running = Value::Null;
        }
        if raw(Some(&running), "controlled") == Some(&json!(false)) {
            return Err(Failure::msg(format!("port {} has a XenoNet server with no window (npm run dev or npm run demo?); stop it, or use another instance", self.port_text())));
        }
        if running.is_null() {
            running = self.settle(io, restart, &mut ended).await?;
        }
        if !running.is_null() {
            if show != "hidden" && !truthy(raw(raw(Some(&running), "window"), "visible")) {
                self.call(io, &json!({ "op": "window", "show": show }), 20000.0).await?;
            }
            let r = self.ready(io, timeout).await?;
            return Ok(done(r, &ended));
        }
        let log_file = NATIVE.join(&[&self.log_dir, &format!("{}.log", self.id)]);
        fsx::mkdir_p(&self.log_dir, None)?;
        let env = io.ask(json!({ "op": "env" })).await.map_err(|_| Failure::Rethrow)?;
        let mut env = match env {
            Value::Object(m) => m,
            _ => Map::new(),
        };
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
        let cmd = io.ask(json!({ "op": "command" })).await.map_err(|_| Failure::Rethrow)?;
        io.ask(json!({ "op": "spawn", "command": cmd.get("command").cloned().unwrap_or(Value::Null), "args": cmd.get("args").cloned().unwrap_or(json!([])), "cwd": self.root, "env": env, "logFile": log_file })).await.map_err(|_| Failure::Rethrow)?;
        let relative = NATIVE.relative(&self.root, &log_file);
        let until = now_ms() + timeout;
        while now_ms() < until {
            let p = self.ping(io).await?;
            if truthy(raw(raw(Some(&p), "page"), "ready")) {
                return Ok(done(p, &ended));
            }
            let exited = io.ask(json!({ "op": "exited" })).await.map_err(|_| Failure::Rethrow)?;
            if !exited.is_null() && p.is_null() {
                // It handed off to a copy that took the lock in the meantime.
                let holder = lock_holder(io, &self.lock_dir).await?;
                if !holder.is_null() {
                    return Err(Failure::msg(format!("{} quit at once: {}", self.name, self.held(&holder))));
                }
                let text = fsx::read_text(&log_file)?;
                let lines: Vec<&str> = xenonet_base::text::js_trim(&text).split('\n').collect();
                let tail = lines[lines.len().saturating_sub(12)..].join("\n");
                return Err(Failure::msg(format!("{} exited ({}) before it was ready. The end of {relative}:\n{tail}", self.name, js_string(&exited))));
            }
            io.sleep(500.0).await;
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
    async fn settle(&self, io: &Io, restart: bool, ended: &mut Vec<Value>) -> Result<Value, Failure> {
        let mut until = now_ms() + self.settle_ms;
        loop {
            let state = self.probe(io).await;
            if state == "up" {
                let p = self.ping(io).await?;
                if !p.is_null() && raw(Some(&p), "controlled") != Some(&json!(false)) {
                    return Ok(p);
                }
            }
            let holder = lock_holder(io, &self.lock_dir).await?;
            if state == "down" && holder.is_null() {
                return Ok(Value::Null);
            }
            let late = now_ms() >= until;
            if !holder.is_null() && (restart || (late && self.made_up)) {
                self.end(io, holder.as_f64().unwrap_or(f64::NAN)).await?;
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
            io.sleep(300.0).await;
        }
    }

    // Ends a copy holding the lock: SIGTERM asks it to quit, which one
    // stuck on its way out ignores, so SIGKILL follows.
    async fn end(&self, io: &Io, pid: f64) -> Result<(), Failure> {
        self.kill(io, pid, "SIGTERM").await?;
        if self.gone(io, pid, self.term_ms).await? {
            return Ok(());
        }
        self.kill(io, pid, "SIGKILL").await?;
        if !self.gone(io, pid, 5000.0).await? {
            return Err(Failure::msg(format!("process {} holds {}'s single-instance lock and is still there after kill -9", number_string(pid), self.name)));
        }
        Ok(())
    }

    // The machine's processes, or None where it can't list them.
    async fn processes(&self, io: &Io) -> Result<Option<Vec<Proc>>, Failure> {
        let v = io.ask(json!({ "op": "processes" })).await.map_err(|_| Failure::Rethrow)?;
        Ok(procs_of(&v))
    }

    // A test instance's process tree, but the app itself: its helpers (such
    // as Chromium's GPU and network processes) and the service it started,
    // with what they started. None for any other instance, or where the
    // machine can't list its processes.
    async fn tree_of(&self, io: &Io, pid: Option<f64>) -> Result<Option<Vec<Proc>>, Failure> {
        let Some(pid) = pid.filter(|_| owns_tree(&self.id, &self.port)) else { return Ok(None) };
        Ok(self.processes(io).await?.map(|t| descendants(&t, pid)))
    }

    // The pids of a tree taken earlier that are still there, each the same
    // process (started when it was), with the app's descendants now while
    // it's there.
    async fn left_of(&self, io: &Io, pid: f64, before: &[Proc]) -> Result<Vec<f64>, Failure> {
        let table = self.processes(io).await?.unwrap_or_default();
        let mut all = before.to_vec();
        if self.alive(io, pid).await? {
            all.extend(descendants(&table, pid));
        }
        let mut left: Vec<f64> = Vec::new();
        for p in all {
            if p.pid != pid && !left.contains(&p.pid) && table.iter().any(|q| q.pid == p.pid && q.start == p.start) {
                left.push(p.pid);
            }
        }
        Ok(left)
    }

    // Kills each of `pids`, and whether all have gone within 5 seconds.
    async fn killed(&self, io: &Io, pids: &[f64]) -> Result<bool, Failure> {
        for p in pids {
            self.kill(io, *p, "SIGKILL").await?;
        }
        let until = now_ms() + 5000.0;
        for p in pids {
            if !self.gone(io, *p, (until - now_ms()).max(0.0)).await? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Quits the instance, and returns once its process has exited and its
    /// server has let go of the port. A process still there `timeout` ms
    /// after its server went is stuck on its way out and is killed. For a
    /// test instance, that takes its whole tree, as it stood before it was
    /// told to quit and as it stands then: killed alone, the app would
    /// leave its GPU and network processes and its service running
    /// (`ended`).
    pub async fn quit(&self, io: &Io, timeout: f64) -> Result<Value, Failure> {
        let running = self.ping(io).await?;
        if running.is_null() {
            return Ok(json!({ "quit": false, "running": false }));
        }
        let pid = raw(Some(&running), "pid").and_then(Value::as_f64).filter(|p| p.fract() == 0.0);
        let tree = self.tree_of(io, pid).await?;
        let _ = self.call(io, &json!({ "op": "quit" }), 0.0).await;
        let until = now_ms() + timeout;
        let pids = |l: &[f64]| Value::Array(l.iter().map(|p| xenonet_base::json::number(*p)).collect());
        loop {
            let state = self.probe(io).await;
            let alive = match pid {
                Some(p) => self.alive(io, p).await?,
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
                    Some(t) => self.left_of(io, pid.unwrap_or(f64::NAN), t).await?,
                    None => Vec::new(),
                };
                if !alive {
                    // The app has gone, and what still holds the port may be
                    // the service it started, stuck on its way out.
                    if !left.is_empty() && self.killed(io, &left).await? && self.probe(io).await == "down" {
                        return Ok(json!({ "quit": true, "ended": pids(&left) }));
                    }
                    return Err(Failure::msg(format!("{} quit, but something on port {} still doesn't answer", self.name, self.port_text())));
                }
                let pid = pid.unwrap_or(f64::NAN);
                let all: Vec<f64> = std::iter::once(pid).chain(left.iter().copied()).collect();
                if self.killed(io, &all).await? {
                    let mut out = json!({ "quit": true, "killed": xenonet_base::json::number(pid) });
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
            io.sleep(300.0).await;
        }
    }

    /// A call into the page's driver. If the page is reloading (it does
    /// after a server restart), it waits for it, then tries once more.
    pub async fn driver(&self, io: &Io, method: &Value, args: &[Value]) -> Result<Value, Failure> {
        let timeout = args.iter().find(|a| matches!(a, Value::Object(m) if m.contains_key("timeout"))).and_then(|a| a.get("timeout")).cloned();
        let mut body = Map::new();
        body.insert("op".into(), json!("driver"));
        body.insert("method".into(), method.clone());
        body.insert("args".into(), json!(args));
        if let Some(t) = timeout {
            body.insert("timeout".into(), t);
        }
        let body = Value::Object(body);
        match self.call(io, &body, 20000.0).await {
            Err(e) if e.message().contains("has no driver") => {
                self.ready(io, 60000.0).await?;
                self.call(io, &body, 20000.0).await
            }
            r => r,
        }
    }

    pub async fn simulate(&self, io: &Io, kind: &Value, a: Option<&Value>, b: Option<&Value>) -> Result<Value, Failure> {
        let mut body = Map::new();
        body.insert("kind".into(), kind.clone());
        if let Some(a) = a {
            body.insert("a".into(), a.clone());
        }
        if let Some(b) = b {
            body.insert("b".into(), b.clone());
        }
        let res = io.ask(json!({ "op": "fetch", "url": format!("{}/api/dev/simulate", self.base), "method": "POST", "headers": { "content-type": "application/json" }, "body": stringify(&Value::Object(body), 0), "read": "json!" })).await.map_err(|_| Failure::Rethrow)?;
        let data = res.get("body").cloned().unwrap_or(Value::Null);
        if res["ok"] != json!(true) {
            let why = match &data {
                Value::Null => return Err(Failure::type_error("Cannot read properties of null (reading 'error')")),
                d => raw(Some(d), "error").filter(|e| !e.is_null()).map_or_else(|| res.get("statusText").map(js_string).unwrap_or_default(), js_string),
            };
            return Err(Failure::msg(why));
        }
        Ok(data)
    }

    /// Any read-only API route, such as /api/live.
    pub async fn api(&self, io: &Io, route: &str) -> Result<Value, Failure> {
        let res = io.ask(json!({ "op": "fetch", "url": format!("{}{route}", self.base), "read": "json!" })).await.map_err(|_| Failure::Rethrow)?;
        Ok(res.get("body").cloned().unwrap_or(Value::Null))
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
    use crate::task::{run_with, Task};

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
        let t = Task::new(move |io| async move { i.call(&io, &json!({ "op": "ping" }), 0.0).await });
        let out = run_with(t, |_| Err(json!({ "message": "fetch failed", "causeCode": "ECONNREFUSED" })));
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
        use std::cell::RefCell;
        let i = Instance::new("test", &opts()).unwrap();
        let t = Task::new(move |io| async move { i.quit(&io, 0.0).await });
        let table = RefCell::new(vec![(501u64, 1u64), (600, 501), (601, 501), (700, 601), (800, 1)]);
        let killed = RefCell::new(Vec::new());
        // Its server answers until it's told to quit, then is gone; the app
        // stays.
        let quitting = RefCell::new(false);
        let out = run_with(t, |q| {
            let pid = q["pid"].as_f64().unwrap_or(0.0) as u64;
            match q["op"].as_str().unwrap_or("") {
                "fetch" if q["url"].as_str().unwrap_or("").ends_with("/api/dev/control") => {
                    *quitting.borrow_mut() |= q["body"].as_str().unwrap_or("").contains("\"quit\"");
                    Ok(json!({ "ok": true, "status": 200, "body": { "instance": "test", "pid": 501 } }))
                }
                "fetch" if !*quitting.borrow() => Ok(json!({ "ok": true, "status": 200 })),
                "fetch" => Err(json!({ "message": "fetch failed", "causeCode": "ECONNREFUSED" })),
                "processes" => Ok(Value::Array(table.borrow().iter().map(|(p, pp)| json!({ "pid": p, "ppid": pp, "start": format!("at {p}") })).collect())),
                "alive" => Ok(json!(table.borrow().iter().any(|(p, _)| *p == pid))),
                "kill" => {
                    killed.borrow_mut().push((pid, q["signal"].as_str().unwrap_or("").to_string()));
                    table.borrow_mut().retain(|(p, _)| *p != pid);
                    Ok(Value::Null)
                }
                _ => Ok(Value::Null),
            }
        });
        assert_eq!(out, Ok(json!({ "quit": true, "killed": 501, "ended": [600, 601, 700] })));
        assert_eq!(killed.borrow().iter().map(|(p, _)| *p).collect::<Vec<_>>(), [501, 600, 601, 700]);
        assert!(killed.borrow().iter().all(|(_, s)| s == "SIGKILL"));
        assert_eq!(*table.borrow(), [(800, 1)], "another app's process is left alone");
    }
}
