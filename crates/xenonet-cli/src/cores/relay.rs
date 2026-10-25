//! core/hosts/relay.js's runRelay for the command: `xenonet relay`, this
//! XenoNet's map streamed to a console on another machine over the ssh
//! connection the console opened. What leaves this machine is
//! xenonet-hosts' (the scrubbing, the shaping, the allowlist, what waits
//! while the console reads slowly); the HTTP calls to the XenoNet here,
//! the pipes, the timers and the signals are done here, on tokio, as the
//! twin did them with Node's.
//!
//! One loop owns the relay's state and does all the shaping; the reads
//! (the console's gets, a fresh snapshot), the event stream and standard
//! output are tasks that report back to it.

use super::native::now;
use crate::out::{eprint, Fail};
use crate::sys;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read as _, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt};
use tokio::sync::{mpsc, Notify};
use xenonet_base::process::Background as _;
use xenonet_hosts::frames::{encode, parse_frame, LineSplitter, FROM_CONSOLE, MAX_FRAME, MAX_REQUEST, PROTOCOL};
use xenonet_hosts::relay::{self as core, Outbox, Producer, Shape, EXIT_DONE, EXIT_NO_XENONET, EXIT_XENONET_GONE};

const VERSION: &str = env!("XENONET_VERSION");
const GET_MS: f64 = core::GET_TIMEOUT_MS;
// Past this much written and not yet taken, standard output is full, as a
// stream's write() says false past its high-water mark.
const HIGH_WATER: usize = 64 << 10;

fn log(line: &str) {
    eprint(format!("xenonet relay: {line}"));
}

// ---------- the XenoNet on this machine ----------

/// Why a read failed: its words, and the status to answer with.
struct GetError {
    message: String,
    status: Option<u16>,
}

/// getJson(port, route): GET 127.0.0.1:port/route as JSON, within the time
/// given and no bigger than a frame: { status, body }.
async fn get_json(port: u16, route: String, timeout_ms: f64) -> Result<(u16, Value), GetError> {
    let r = tokio::task::spawn_blocking(move || {
        xenonet_base::http::request(port, "GET", &route, &[("accept".into(), "application/json".into())], None, Some(Duration::from_millis(timeout_ms as u64)))
    })
    .await
    .map_err(|e| GetError { message: e.to_string(), status: None })?;
    let a = r.map_err(|e| match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
            GetError { message: format!("no answer in {} s", xenonet_base::json::number_string(timeout_ms / 1000.0)), status: Some(504) }
        }
        std::io::ErrorKind::ConnectionRefused => GetError { message: format!("connect ECONNREFUSED 127.0.0.1:{port}"), status: None },
        _ => GetError { message: e.to_string(), status: None },
    })?;
    let body = a.body.map_err(|e| GetError { message: e.to_string(), status: None })?;
    if body.len() > MAX_FRAME {
        return Err(GetError { message: "too big to relay".into(), status: Some(413) });
    }
    let text = String::from_utf8_lossy(&body);
    if text.is_empty() {
        return Ok((a.status, Value::Null));
    }
    Ok(match xenonet_base::json::parse(&text) {
        Ok(v) => (a.status, v),
        Err(_) => (502, json!({ "error": "XenoNet answered with something that isn't JSON" })),
    })
}

/// The health of the XenoNet here, or None.
async fn health(port: u16) -> Option<Value> {
    get_json(port, "/api/health".into(), 2000.0).await.ok().map(|(_, b)| b).filter(|b| !b.is_null())
}

/// What the event stream says: an event, or that it ended.
enum Event {
    One(String, Value),
    End,
}

/// followEvents(port, route): the event stream, each event as it comes, on
/// a thread of its own; Err when it doesn't start.
fn follow_events(port: u16, route: &str, to: mpsc::UnboundedSender<Event>) -> Result<(), String> {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("connect {} 127.0.0.1:{port}", xenonet_base::fs::code_of(&e).unwrap_or("ECONNREFUSED")))?;
    let _ = s.set_nodelay(true);
    write!(s, "GET {route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n").map_err(|e| e.to_string())?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    let status: u16 = line.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    if status != 200 {
        return Err(format!("the event stream answered {status}"));
    }
    let mut chunked = false;
    loop {
        line.clear();
        if r.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            chunked |= k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked");
        }
    }
    std::thread::spawn(move || {
        let mut buf = String::new();
        let mut bytes = Vec::new();
        let take = |piece: &[u8], buf: &mut String, bytes: &mut Vec<u8>| -> bool {
            bytes.extend_from_slice(piece);
            // Whole characters only; the rest waits for the next piece.
            let cut = match std::str::from_utf8(bytes) {
                Ok(_) => bytes.len(),
                Err(e) => e.valid_up_to(),
            };
            buf.push_str(&String::from_utf8_lossy(&bytes[..cut]));
            bytes.drain(..cut);
            while let Some(at) = buf.find("\n\n") {
                let block: String = buf.drain(..at + 2).collect();
                let mut name = "message".to_string();
                let mut data: Vec<&str> = Vec::new();
                for l in block[..at].split('\n') {
                    if let Some(rest) = l.strip_prefix("event:") {
                        name = rest.trim().to_string();
                    } else if let Some(rest) = l.strip_prefix("data:") {
                        data.push(rest.strip_prefix(' ').unwrap_or(rest));
                    }
                }
                if data.is_empty() {
                    continue;
                }
                if let Ok(v) = xenonet_base::json::parse(&data.join("\n")) {
                    if to.send(Event::One(name, v)).is_err() {
                        return false;
                    }
                }
            }
            buf.len() <= MAX_FRAME * 2
        };
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let ok = if chunked {
                let mut size = String::new();
                match r.read_line(&mut size) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(n) = usize::from_str_radix(size.trim().split(';').next().unwrap_or(""), 16) else { break };
                if n == 0 {
                    break;
                }
                let mut piece = vec![0u8; n + 2];
                if r.read_exact(&mut piece).is_err() {
                    break;
                }
                take(&piece[..n], &mut buf, &mut bytes)
            } else {
                match r.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => take(&chunk[..n], &mut buf, &mut bytes),
                }
            };
            if !ok {
                break;
            }
        }
        let _ = to.send(Event::End);
    });
    Ok(())
}

/// Starts a XenoNet here for as long as the relay runs: this binary's
/// service.
fn start_xenonet(port: u16, demo: bool) -> Option<std::process::Child> {
    let mut args = vec!["serve".to_string()];
    if demo {
        args.push("--demo".into());
    }
    std::process::Command::new(std::env::current_exe().ok()?)
        .no_window()
        .args(args)
        .env("XENONET_PORT", port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()
}

// ---------- standard output ----------

/// Standard output, written by a task of its own: how much waits, and a
/// word when it has all been taken while the relay held back.
struct Out {
    to: Option<mpsc::UnboundedSender<String>>,
    queued: Arc<AtomicUsize>,
    waiting: Arc<AtomicBool>,
    drained: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl Out {
    fn new() -> Out {
        let (to, mut from) = mpsc::unbounded_channel::<String>();
        let queued = Arc::new(AtomicUsize::new(0));
        let waiting = Arc::new(AtomicBool::new(false));
        let drained = Arc::new(Notify::new());
        let (q, w, d) = (queued.clone(), waiting.clone(), drained.clone());
        let task = tokio::spawn(async move {
            let mut out = tokio::io::stdout();
            while let Some(text) = from.recv().await {
                if out.write_all(text.as_bytes()).await.is_err() || out.flush().await.is_err() {
                    break;
                }
                if q.fetch_sub(text.len(), Ordering::SeqCst) == text.len() && w.swap(false, Ordering::SeqCst) {
                    d.notify_one();
                }
            }
        });
        Out { to: Some(to), queued, waiting, drained, task }
    }

    /// Writes `text`; false when the output can take no more for now.
    fn write(&self, text: String) -> bool {
        let n = text.len();
        let Some(to) = &self.to else { return true };
        if to.send(text).is_err() {
            return true;
        }
        if self.queued.fetch_add(n, Ordering::SeqCst) + n > HIGH_WATER {
            self.waiting.store(true, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// Everything written, taken.
    async fn finish(mut self) {
        self.to = None;
        let _ = self.task.await;
    }
}

// ---------- the relay ----------

/// A read's answer, for the loop to shape: the console's get (its id and
/// shape), or a snapshot's three.
enum Read {
    Get { id: i64, shape: Shape, answer: Result<(u16, Value), GetError> },
    Snapshot(Result<[(u16, Value); 3], String>),
}

struct Relay {
    port: u16,
    since_ms: f64,
    detail: bool,
    /// The shaping, once the XenoNet here said where its home folder is.
    p: Producer,
    ready: bool,
    stopped: bool,
    code: i32,
    out: Outbox,
    stdout: Out,
    inflight: usize,
    reads: mpsc::UnboundedSender<Read>,
}

impl Relay {
    fn write(&mut self, text: String) {
        if !self.stdout.write(text) {
            self.out.full();
        }
    }

    fn send(&mut self, frame: &Value, coalesce: Option<&str>) {
        if self.stopped {
            return;
        }
        let text = encode(frame);
        if xenonet_app::jstext::utf16_len(&text) > MAX_FRAME {
            let t = frame.get("t").map(xenonet_base::text::js_string).unwrap_or_default();
            return log(&format!("left out a {t} too big to send ({} MB)", (text.len() as f64 / 1e6).round()));
        }
        if let Some(t) = self.out.send(text, coalesce, now()) {
            self.write(t);
        }
    }

    /// The output took what waited: what waited goes, as long as it keeps
    /// taking it, and a fresh snapshot when frames were dropped meanwhile.
    fn drain(&mut self) {
        let waiting = self.out.drain();
        for (i, t) in waiting.iter().enumerate() {
            if self.out.busy {
                self.out.left(waiting[i..].to_vec());
                break;
            }
            self.out.wrote(now());
            self.write(t.clone());
        }
        if self.out.take_gap() {
            self.snapshot();
        }
    }

    fn stop(&mut self, code: i32, bye: Option<Value>) {
        if self.stopped {
            return;
        }
        if let Some(b) = bye {
            let mut frame = serde_json::Map::new();
            frame.insert("t".into(), json!("bye"));
            for (k, v) in b.as_object().into_iter().flatten() {
                frame.insert(k.clone(), v.clone());
            }
            self.send(&Value::Object(frame), None);
        }
        self.stopped = true;
        self.code = code;
    }

    fn gone(&mut self, message: String) {
        self.stop(EXIT_XENONET_GONE, Some(json!({ "code": "xenonet-gone", "message": message })));
    }

    /// A fresh snapshot: the map, the live state and the engine's, read
    /// side by side.
    fn snapshot(&self) {
        let (port, to) = (self.port, self.reads.clone());
        tokio::spawn(async move {
            let _ = to.send(Read::Snapshot(read_snapshot(port).await));
        });
    }

    fn snapshot_frame(&mut self, r: Result<[(u16, Value); 3], String>) -> Result<(), String> {
        let frame = self.p.snapshot(r?, now())?;
        self.send(&frame, None);
        self.ready = true;
        Ok(())
    }

    fn event(&mut self, name: &str, data: &Value) {
        if self.stopped || !self.ready {
            return;
        }
        if let Some((frame, coalesce)) = self.p.event(name, data, now()) {
            self.send(&frame, coalesce);
        }
    }

    /// A frame from the console: the time, or a read from the allowlist.
    fn frame(&mut self, line: &str) {
        let Ok(f) = parse_frame(line, Some(&FROM_CONSOLE)) else { return };
        if self.stopped {
            return;
        }
        let id = f.get("id").and_then(Value::as_f64).filter(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0).map(|n| n as i64);
        match (f.get("t").and_then(Value::as_str), id) {
            (Some("time"), Some(id)) => self.send(&json!({ "t": "time", "id": id, "now": xenonet_base::json::number(now()) }), None),
            (Some("get"), Some(id)) => {
                let (shape, read) = match core::allowed(f.get("path")) {
                    Ok((shape, path)) => {
                        let read = core::route_to_read(shape, &path);
                        (shape, read)
                    }
                    Err(e) => return self.send(&json!({ "t": "res", "id": id, "status": 403, "body": { "error": e } }), None),
                };
                if self.inflight >= core::MAX_GETS {
                    return self.send(&json!({ "t": "res", "id": id, "status": 429, "body": { "error": "too many reads at once" } }), None);
                }
                self.inflight += 1;
                let (port, to) = (self.port, self.reads.clone());
                tokio::spawn(async move {
                    let answer = get_json(port, read, GET_MS).await;
                    let _ = to.send(Read::Get { id, shape, answer });
                });
            }
            _ => {}
        }
    }

    fn answered(&mut self, id: i64, shape: Shape, answer: Result<(u16, Value), GetError>) {
        self.inflight -= 1;
        match answer {
            Ok((status, body)) => {
                let frame = self.p.answer(id, shape, status, &body, now());
                self.send(&frame, None);
            }
            Err(e) => self.send(&json!({ "t": "res", "id": id, "status": e.status.unwrap_or(502), "body": { "error": e.message } }), None),
        }
    }
}

async fn read_snapshot(port: u16) -> Result<[(u16, Value); 3], String> {
    let (g, l, e) =
        tokio::join!(get_json(port, "/api/graph?hosts=0".into(), GET_MS), get_json(port, "/api/live?hosts=0".into(), GET_MS), get_json(port, "/api/engine/state".into(), GET_MS));
    Ok([g.map_err(|e| e.message)?, l.map_err(|e| e.message)?, e.map_err(|e| e.message)?])
}

/// The signals that end the relay: SIGTERM, SIGHUP and SIGINT (Ctrl-C on
/// Windows).
async fn ended() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut hup), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup()), signal(SignalKind::interrupt())) else {
            return std::future::pending().await;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = hup.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn relay(port: u16, since_ms: f64, detail: bool, start: bool, demo: bool) -> i32 {
    let stdout = Out::new();
    let (reads, mut read) = mpsc::unbounded_channel();
    let mut r = Relay {
        port,
        since_ms,
        detail,
        p: Producer::new(None, since_ms, detail, xenonet_state::identities::parse_ssh_aliases),
        ready: false,
        stopped: false,
        code: EXIT_DONE,
        out: Outbox::default(),
        stdout,
        inflight: 0,
        reads,
    };
    let code = run(&mut r, &mut read, start, demo).await;
    let Relay { stdout, .. } = r;
    stdout.finish().await;
    code
}

async fn run(r: &mut Relay, read: &mut mpsc::UnboundedReceiver<Read>, start: bool, demo: bool) -> i32 {
    let port = r.port;
    let mut child = None;
    let mut up = health(port).await;
    if up.is_none() && start {
        child = start_xenonet(port, demo);
        let until = now() + 60e3;
        while up.is_none() && now() < until && child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None))) {
            tokio::time::sleep(Duration::from_millis(300)).await;
            up = health(port).await;
        }
    }
    let code = serve(r, read, up).await;
    if let Some(mut c) = child {
        if matches!(c.try_wait(), Ok(None)) {
            terminate(&mut c);
        }
    }
    code
}

#[cfg_attr(unix, expect(unsafe_code, reason = "kill(2) with SIGTERM, which std's Child::kill (SIGKILL) isn't"))]
fn terminate(c: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) sends SIGTERM to the child this relay started,
        // which hasn't been waited for, so its pid is still its own.
        unsafe {
            libc::kill(c.id() as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = c.kill();
    }
}

async fn serve(r: &mut Relay, read: &mut mpsc::UnboundedReceiver<Read>, up: Option<Value>) -> i32 {
    let port = r.port;
    let Some(up) = up else {
        r.stop(EXIT_NO_XENONET, Some(json!({ "code": "no-xenonet", "message": format!("no XenoNet answers on 127.0.0.1:{port} on this machine") })));
        return r.code;
    };
    let me = get_json(port, "/api/hosts/self".into(), GET_MS).await.unwrap_or_else(|e| (0, json!({ "error": e.message })));
    if me.0 != 200 || !xenonet_base::text::truthy(me.1.get("key")) {
        let (code, bye) = core::self_bye(f64::from(me.0), &me.1, up.get("version"));
        r.stop(code, Some(bye));
        return r.code;
    }
    r.p = Producer::new(me.1.get("home").and_then(Value::as_str), r.since_ms, r.detail, xenonet_state::identities::parse_ssh_aliases);
    let hello = json!({
        "t": "hello", "schema": "host/hello/v1", "protocol": PROTOCOL, "version": VERSION,
        "host": { "id": me.1.get("id").cloned().unwrap_or(Value::Null), "key": me.1["key"] },
        "platform": sys::platform(), "demo": xenonet_base::text::truthy(up.get("demo")), "now": xenonet_base::json::number(now()),
        "sinceMs": xenonet_base::json::number(r.since_ms), "detail": r.detail,
    });
    r.send(&hello, None);

    let (events_to, mut events) = mpsc::unbounded_channel();
    let started = tokio::task::spawn_blocking(move || follow_events(port, "/api/events?hosts=0", events_to)).await.unwrap_or_else(|e| Err(e.to_string()));
    let first = match started {
        Ok(()) => read_snapshot(port).await,
        Err(e) => Err(e),
    };
    if let Err(e) = r.snapshot_frame(first) {
        r.gone(format!("couldn't read the XenoNet on 127.0.0.1:{port}: {e}"));
        return r.code;
    }
    // What came before the snapshot is in it.
    while let Ok(e) = events.try_recv() {
        if matches!(e, Event::End) {
            r.gone(format!("the XenoNet on 127.0.0.1:{port} stopped"));
            return r.code;
        }
    }

    let mut lines = LineSplitter::new(MAX_REQUEST);
    let mut stdin = tokio::io::stdin();
    let mut chunk = vec![0u8; 64 << 10];
    let mut stdin_open = true;
    let mut pinger = tokio::time::interval(Duration::from_millis(5000));
    pinger.tick().await;
    let drained = r.stdout.drained.clone();
    let signals = ended();
    tokio::pin!(signals);
    while !r.stopped {
        tokio::select! {
            n = stdin.read(&mut chunk), if stdin_open => match n {
                Ok(0) | Err(_) => {
                    stdin_open = false;
                    r.stop(EXIT_DONE, None);
                }
                Ok(n) => {
                    for line in lines.push(&chunk[..n]).lines {
                        r.frame(&line);
                    }
                }
            },
            e = events.recv() => match e {
                Some(Event::One(name, data)) => r.event(&name, &data),
                Some(Event::End) | None => r.gone(format!("the XenoNet on 127.0.0.1:{port} stopped")),
            },
            got = read.recv() => match got {
                Some(Read::Get { id, shape, answer }) => r.answered(id, shape, answer),
                Some(Read::Snapshot(s)) => {
                    let _ = r.snapshot_frame(s);
                }
                None => {}
            },
            () = drained.notified() => r.drain(),
            _ = pinger.tick() => {
                if r.out.ping_due(now()) {
                    r.send(&json!({ "t": "ping", "now": xenonet_base::json::number(now()) }), None);
                }
            }
            () = &mut signals => r.stop(EXIT_DONE, None),
        }
    }
    r.code
}

/// runRelay({ port, sinceMs, detail, start, demo }): the exit code, once
/// whatever was on its way to the console has gone.
pub fn run_relay(opts: &Value) -> Result<i32, Fail> {
    let port = opts.get("port").and_then(Value::as_f64).unwrap_or(0.0) as u16;
    let since = opts.get("sinceMs").and_then(Value::as_f64).unwrap_or(core::DEFAULT_SINCE_MS);
    let on = |k: &str| opts.get(k) == Some(&Value::Bool(true));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Fail::error(e.to_string()))?;
    let code = rt.block_on(relay(port, since, on("detail"), on("start"), on("demo")));
    // The reads and the event stream's thread may still be at work: they
    // end with the process.
    rt.shutdown_background();
    Ok(code)
}
