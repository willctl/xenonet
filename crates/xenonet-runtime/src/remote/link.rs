//! One network host's link: the console's end of a subscription to
//! another machine's map, through the network's gateway. It starts the
//! subscription, reads the frames, keeps the clock offset, sends gets
//! with time limits, and keeps the host's state:
//!
//!   connecting  starting
//!   live        frames are arriving
//!   stale       no frame for 45 seconds: three missed pings
//!   offline     the subscription ended or never came up; tried again
//!               with backoff, from 2 to 60 seconds, with jitter
//!   error       it needs you (a protocol this XenoNet doesn't speak,
//!               another XenoNet answering in the host's place, removed
//!               from the network); tried again every minute
//!
//! Everything a host sends is untrusted: frames are capped in size and
//! checked. Nothing a link reads can make it write anywhere but its own
//! subscription.
//!
//! The link here is its state and decisions. Each call says what the
//! hub is to do, in order (`Effect`): start the subscription, write a
//! frame, end it, emit an event, set or clear a timer, settle a get.

use crate::remote::frames::{encode, parse_frame, shallow, FROM_RELAY, MAX_DEPTH, PROTOCOL};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use xenonet_model::ids::is_host_id;
use xenonet_text::text::display;
use xenonet_text::text::truthy;

pub const HELLO_MS: f64 = 20e3;
pub const STALE_MS: f64 = 45e3;
pub const BACKOFF_MIN: f64 = 2e3;
pub const BACKOFF_MAX: f64 = 60e3;
pub const GET_MS: f64 = 20e3;
pub const TIME_EVERY_MS: f64 = 5.0 * 60e3;

/// The host a link subscribes to: its entry's id, when it's a host id.
pub fn host_of(entry: &Value) -> Result<String, String> {
    entry.get("id").and_then(Value::as_str).filter(|i| is_host_id(i)).map(str::to_string).ok_or_else(|| format!("not a host id: {}", show(entry.get("id"))))
}

// JSON.stringify(v), with undefined as JS writes it in a template.
fn show(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), |v| xenonet_text::json::stringify(v, 0))
}

/// Why a subscription ended: its state, code, message and fix.
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

/// How a subscription ended: the host's bye, why it couldn't start, or
/// what the network said.
pub struct Ended<'a> {
    pub bye: Option<&'a Value>,
    pub spawn_error: Option<&'a str>,
    /// offline (the host is, in presence), removed (from the roster),
    /// lapsed (no frame came in time), link (this machine's own link to
    /// the gateway is down) or closed.
    pub network: Option<&'a str>,
}

/// Why a subscription ended, from the host's bye or what the network said.
pub fn why_ended(entry: &Value, e: &Ended<'_>) -> Why {
    let id = entry.get("id").map(display).unwrap_or_default();
    if let Some(err) = e.spawn_error {
        return why("offline", "no-network", format!("this machine can't reach {id} over the network: {err}"), None);
    }
    let bye_code = e.bye.and_then(|b| b.get("code")).map(display).filter(|c| !c.is_empty());
    let message = || e.bye.and_then(|b| b.get("message")).filter(|m| !m.is_null()).map_or_else(String::new, |m| xenonet_text::utf16::head(&display(m), 200));
    match bye_code.as_deref() {
        Some("not-window") => {
            return why(
                "error",
                "not-window",
                format!("{id} shows its map only to a window host's console"),
                Some("Use this machine's XenoNet window as a window host: from the founder's window, Replace this machine with I'll use a XenoNet window on it ticked.".into()),
            )
        }
        Some("xenonet-gone") => return why("offline", "xenonet-gone", format!("the XenoNet on {id} stopped"), None),
        Some(code) => {
            let m = message();
            return why("offline", "relay", format!("the relay on {id} stopped: {}", if m.is_empty() { code.to_string() } else { m }), None);
        }
        None => {}
    }
    match e.network {
        Some("offline") => why("offline", "host-offline", format!("{id} is offline in the network"), None),
        Some("removed") => why("error", "removed", format!("{id} was removed from the network"), None),
        Some("lapsed") => why("offline", "lapsed", format!("{id} stopped sending its map"), None),
        Some("link") => why("offline", "link-down", "this machine's link to the network's gateway is down".into(), None),
        _ => why("offline", "ended", format!("the map from {id} stopped"), None),
    }
}

/// What the hub is to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Start run `run`: its subscription to `host`'s map.
    Start {
        run: u64,
        host: String,
    },
    Write {
        run: u64,
        text: String,
    },
    /// End the run's subscription, and tell the link it ended (`end`).
    Fail {
        run: u64,
    },
    /// End the run's subscription if it's still there.
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

/// What a link asks of the hub around it: a random number for the
/// jitter, and the schema's check of a hello, with what's wrong.
pub trait Asks {
    fn random(&mut self) -> f64;
    fn check_hello(&mut self, frame: &Value) -> Option<String>;
}

struct Run {
    id: u64,
    hello: bool,
    snapshot: bool,
    bye: Option<Value>,
    problem: Option<Why>,
    ended: bool,
    pending: Vec<u64>,
    times: HashMap<u64, f64>,
}

/// One host's link.
pub struct Link {
    pub entry: Value,
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
    pub fn new(entry: Value, now: f64, hello_ms: f64, stale_ms: f64, backoff: (f64, f64), get_ms: f64) -> Self {
        Link {
            entry,
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
        self.entry.get("id").map(display).unwrap_or_default()
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
        let message = format!("{} didn't answer in {} s", self.id(), xenonet_text::json::number_string(xenonet_text::units::round(timeout_ms / 1000.0)));
        self.fx.push(Effect::Reject { id, message, status: 504 });
        self.take()
    }

    /// Where the host stands, for the hub's view of it.
    pub fn info(&self) -> Value {
        let e = &self.entry;
        let mut m = Map::new();
        m.insert("id".into(), e.get("id").cloned().unwrap_or(Value::Null));
        if let Some(t) = e.get("transport") {
            m.insert("transport".into(), t.clone());
        }
        m.insert("state".into(), json!(self.state));
        m.insert("since".into(), xenonet_text::json::number(self.since));
        m.insert("lastHeard".into(), self.last_heard.map_or(Value::Null, xenonet_text::json::number));
        m.insert("offsetMs".into(), if self.rtt_ms.is_none() { Value::Null } else { xenonet_text::json::number(self.offset_ms) });
        m.insert("rttMs".into(), self.rtt_ms.map_or(Value::Null, xenonet_text::json::number));
        m.insert("hello".into(), self.hello.clone());
        m.insert("error".into(), self.error.clone());
        m.insert("nextTryAt".into(), if matches!(self.state, "offline" | "error") { self.next_try_at.map_or(Value::Null, xenonet_text::json::number) } else { Value::Null });
        m.insert("counts".into(), json!({ "connects": self.connects, "frames": self.frames, "dropped": self.dropped }));
        m.insert("lastDrop".into(), self.last_drop.clone());
        Value::Object(m)
    }

    fn connect(&mut self, now: f64, a: &mut dyn Asks) {
        self.fx.push(Effect::Clear { name: "retry".into() });
        if self.stopped {
            return;
        }
        let host = match host_of(&self.entry) {
            Ok(h) => h,
            Err(err) => {
                let id = self.id();
                return self.ended(why("error", "bad-entry", format!("XenoNet can't reach {id}: {err}"), None), now, a);
            }
        };
        self.connects += 1.0;
        self.set("connecting", now);
        self.runs += 1;
        let run = Run { id: self.runs, hello: false, snapshot: false, bye: None, problem: None, ended: false, pending: Vec::new(), times: HashMap::new() };
        let run_id = run.id;
        self.run = Some(run);
        self.fx.push(Effect::Start { run: run_id, host });
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
                    why("offline", "no-snapshot", format!("{id} said hello but sent no map in {} s", xenonet_text::json::number_string(hello_ms / 1000.0)), None)
                } else {
                    why("offline", "no-hello", format!("{id} didn't answer in {} s", xenonet_text::json::number_string(hello_ms / 1000.0)), None)
                };
                self.fail(run, w, now, a);
            }
        }
        self.take()
    }

    /// A frame the host sent, as a line.
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
                let name_ok = frame.get("name").and_then(Value::as_str).is_some_and(|n| xenonet_text::utf16::len(n) <= 40);
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
                let code = xenonet_text::utf16::head(&frame.get("code").filter(|v| !v.is_null()).map_or_else(String::new, display), 40);
                let message = xenonet_text::utf16::head(&frame.get("message").filter(|v| !v.is_null()).map_or_else(String::new, display), 300);
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
            let first = xenonet_text::utf16::head(problem.split('\n').next().unwrap_or(""), 160);
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
            let p = f.get("protocol").map_or_else(|| "undefined".into(), display);
            return self.fail(
                run,
                why(
                    "error",
                    "protocol",
                    format!("{id} runs XenoNet {}, whose relay speaks protocol {p}; this XenoNet speaks {PROTOCOL}", f.get("version").map_or_else(|| "undefined".into(), display)),
                    Some("Update the older of the two.".into()),
                ),
                now,
                a,
            );
        }
        let demo = truthy(f.get("demo"));
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
                        Some(format!("If {id} was set up again, remove it from the network and add it again.")),
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
            self.entry["name"] = if truthy(Some(&host_id)) && host_id != json!(id) { host_id } else { Value::Null };
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
        let at = f.get("at").and_then(Value::as_f64).map_or(Value::Null, xenonet_text::json::number);
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
            self.offset_ms = xenonet_text::units::round(host_now - f64::midpoint(sent, now));
            self.rtt_ms = Some(self.rtt_ms.map_or(rtt, |r| r.min(rtt)));
            let o = self.offset_ms;
            self.emit("offset", xenonet_text::json::number(o));
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
        self.last_drop = json!({ "at": xenonet_text::json::number(now), "why": xenonet_text::utf16::head(why, 120) });
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

    /// The run's subscription ended (or couldn't start).
    pub fn end(&mut self, run: u64, e: &Ended<'_>, now: f64, a: &mut dyn Asks) -> Vec<Effect> {
        let Some(mut r) = self.run.take() else { return self.take() };
        if r.id != run || r.ended {
            self.run = Some(r);
            return self.take();
        }
        r.ended = true;
        self.settle(&mut r, "the connection ended");
        let w = match r.problem.take() {
            Some(p) => p,
            None => {
                let bye = r.bye.clone();
                why_ended(&self.entry, &Ended { bye: bye.as_ref(), spawn_error: e.spawn_error, network: e.network })
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
        if !self.stopped {
            let (min, max) = self.backoff;
            let delay = if w.state == "error" { max } else { (min * 2f64.powi(self.attempt as i32)).min(max) * (0.75 + a.random() * 0.5) };
            self.attempt = (self.attempt + 1).min(16);
            self.next_try_at = Some(now + xenonet_text::units::round(delay));
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

    struct NoAsks;

    impl Asks for NoAsks {
        fn random(&mut self) -> f64 {
            0.5
        }
        fn check_hello(&mut self, _frame: &Value) -> Option<String> {
            None
        }
    }

    #[test]
    fn a_network_host_starts_a_subscription_and_says_why_it_ended() {
        let e = json!({ "id": "laptop", "transport": "network" });
        assert_eq!(host_of(&e).unwrap(), "laptop");
        assert!(host_of(&json!({ "id": "Laptop!", "transport": "network" })).is_err());
        let mut l = Link::new(e.clone(), 0.0, HELLO_MS, STALE_MS, (BACKOFF_MIN, BACKOFF_MAX), GET_MS);
        let fx = l.start(0.0, &mut NoAsks);
        assert!(fx.contains(&Effect::Start { run: 1, host: "laptop".into() }), "{fx:?}");
        let ended = |network: Option<&'static str>, bye: Option<&Value>| {
            let w = why_ended(&e, &Ended { bye, spawn_error: None, network });
            (w.state, w.code)
        };
        assert_eq!(ended(Some("offline"), None), ("offline", "host-offline"));
        assert_eq!(ended(Some("removed"), None), ("error", "removed"));
        assert_eq!(ended(Some("lapsed"), None), ("offline", "lapsed"));
        assert_eq!(ended(Some("link"), None), ("offline", "link-down"));
        assert_eq!(ended(Some("lapsed"), Some(&json!({ "code": "not-window" }))), ("error", "not-window"), "the host's bye says more");
        // An end retries, with backoff.
        let fx = l.end(1, &Ended { bye: None, spawn_error: None, network: Some("offline") }, 10.0, &mut NoAsks);
        assert_eq!(l.state, "offline");
        assert!(fx.iter().any(|f| matches!(f, Effect::Timer { name, .. } if name == "retry")), "{fx:?}");
    }
}
