// What the JS service and CLI answered, pinned at the process boundary so
// the binary keeps answering the same once they're gone. The goldens were
// recorded from the JS service (`node server/index.js`) and the JS CLI
// (`node cli/xenonet.js`) on 2026-11-08, on the made-up home in
// fixtures/home, and kept only where the binary answered alike that day
// and two runs of each side agreed: golden/http, the reads of the HTTP API
// (route, status, body); golden/cli, commands that don't depend on live
// processes (each one's --help, --version, and the reads against the
// service), with their exit code and standard output.
//
// This copies the fixture home to a temporary folder (its placeholders
// filled in and its files' times set from fixtures/home.json), starts
// `xenonet serve` on a free port with that home and its own state, cache
// and runtime folders, asks each route and runs each command, and compares
// after setting aside what's bound to differ, as scripts/rust-diff.mjs
// does: times near now, process ids, the port and folders, the version.
// Beside rust-diff's, it sets aside the home's path as a path key
// (lowercased, without /private) and what says when today is (a date
// written out, a clock time, minutes ago). Nothing runs under Node.
//
// The run is made deterministic the way the recording was: the actors
// layer off (its observers look at the apps on the machine), no Codex,
// Claude Code or supervisor to ask, a PATH of the system's own folders, a
// time zone where it's about noon (so the fixture's hours fall on one
// day), and a home whose path is always 160 bytes long, so the
// transcripts holding it are of one size.

#![cfg(unix)]
#![expect(clippy::unwrap_used, clippy::panic, reason = "a test fails where its helpers unwrap or panic")]

use serde_json::{Map, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

const HOME_LEN: usize = 160;
const BIN: &str = env!("CARGO_BIN_EXE_xenonet");

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as f64
}

fn read_json(file: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()))).unwrap()
}

// ---------- the fixture home ----------

// A placeholder's seconds from the start, in `{{name:N}}` at `at`, and
// where it ends.
fn placeholder(text: &str, at: usize, name: &str) -> Option<(i64, usize)> {
    let rest = text[at..].strip_prefix("{{")?.strip_prefix(name)?.strip_prefix(':')?;
    let end = rest.find("}}")?;
    let n = rest[..end].parse().ok()?;
    Some((n, at + 2 + name.len() + 1 + end + 2))
}

fn fill(text: &str, home: &str, now: f64) -> String {
    let text = text.replace("{{home}}", home);
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(k) = text[i..].find("{{") {
        let at = i + k;
        out.push_str(&text[i..at]);
        if let Some((n, end)) = placeholder(&text, at, "iso") {
            out.push_str(&xenonet_base::time::iso(now + n as f64 * 1000.0));
            i = end;
        } else if let Some((n, end)) = placeholder(&text, at, "unix") {
            out.push_str(&(((now + n as f64 * 1000.0) / 1000.0).floor() as i64).to_string());
            i = end;
        } else {
            out.push_str("{{");
            i = at + 2;
        }
    }
    out.push_str(&text[i..]);
    out
}

fn copy_into(from: &Path, to: &Path, rel: &str, found: &mut Vec<(PathBuf, String)>) {
    let mut names: Vec<_> = std::fs::read_dir(from).unwrap().map(|e| e.unwrap()).collect();
    names.sort_by_key(std::fs::DirEntry::file_name);
    for e in names {
        let name = e.file_name().into_string().unwrap();
        let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let target = to.join(&name);
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir(&target).unwrap();
            copy_into(&e.path(), &target, &r, found);
        } else {
            std::fs::copy(e.path(), &target).unwrap();
        }
        found.push((target, r));
    }
}

// The fixture home, at `home`, as it stands `now`.
fn materialise(home: &Path, now: f64) {
    let manifest = read_json(&tests_dir().join("fixtures").join("home.json"));
    let mut all = Vec::new();
    copy_into(&tests_dir().join("fixtures").join("home"), home, "", &mut all);
    let home_text = home.to_str().unwrap();
    for (file, _) in &all {
        if file.is_file() {
            let text = std::fs::read_to_string(file).unwrap();
            let filled = fill(&text, home_text, now);
            if filled != text {
                std::fs::write(file, filled).unwrap();
            }
        }
    }
    // Deepest first, so a folder's time is set after what's in it.
    all.sort_by_key(|(_, r)| std::cmp::Reverse(r.split('/').count()));
    let default = manifest["default"].as_f64().unwrap();
    for (file, r) in &all {
        let secs = manifest["mtimes"].get(r).and_then(Value::as_f64).unwrap_or(default);
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis((now + secs * 1000.0) as u64);
        let f = std::fs::File::open(file).unwrap();
        f.set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t)).unwrap();
    }
}

// ---------- the service ----------

struct Service {
    base: PathBuf,
    home: PathBuf,
    tmp: String,
    port: u16,
    env: Vec<(String, String)>,
    child: Child,
}

fn free_port() -> u16 {
    loop {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        // The user's own XenoNet, its demo and XenoNet Dev are on these.
        if !(47840..=47842).contains(&port) {
            return port;
        }
    }
}

// A time zone where it's about noon now: Etc/GMT-3 is three hours ahead of
// UTC. Without the zone files, the POSIX form libc reads alone.
fn noon_zone(now: f64) -> String {
    let hour = (now / 3_600_000.0).floor() as i64 % 24;
    let off = 12 - hour;
    let name = if off >= 0 { format!("GMT-{off}") } else { format!("GMT+{}", -off) };
    if Path::new("/usr/share/zoneinfo/Etc").join(&name).exists() {
        format!("Etc/{name}")
    } else {
        format!("XNT{}", &name[3..])
    }
}

impl Service {
    fn start() -> Service {
        let now = (now_ms() / 1000.0).floor() * 1000.0;
        let tmp_dir = std::env::temp_dir().canonicalize().unwrap();
        let base = tmp_dir.join(format!("xenonet-golden-{}-{}", std::process::id(), now as u64));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let used = base.to_str().unwrap().len() + 1;
        assert!(used + 4 <= HOME_LEN, "the temporary folder's path is too long for a {HOME_LEN}-byte home: {}", base.display());
        let home = base.join(format!("{:_<width$}", "home", width = HOME_LEN - used));
        std::fs::create_dir(&home).unwrap();
        materialise(&home, now);
        let port = free_port();
        let h = |p: &str| home.join(p).to_str().unwrap().to_string();
        let home_text = home.to_str().unwrap().to_string();
        let mut env: Vec<(String, String)> = vec![
            ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
            ("HOME".into(), home_text.clone()),
            ("USERPROFILE".into(), home_text),
            ("TZ".into(), noon_zone(now)),
            ("LANG".into(), "C".into()),
            ("XENONET_PORT".into(), port.to_string()),
            ("XENONET_HOT".into(), "0".into()),
            ("XENONET_CONTROL".into(), String::new()),
            ("XENONET_INSTANCE".into(), String::new()),
            ("XENONET_EXEC".into(), String::new()),
            ("XENONET_STATE_DIR".into(), h(".xenonet-state")),
            ("XENONET_CACHE_DIR".into(), h(".xenonet-cache")),
            ("XENONET_RUNTIME_DIR".into(), h(".xenonet-runtime")),
            ("XENONET_CLAUDE_USAGE".into(), "off".into()),
            ("XENONET_ACTORS".into(), "off".into()),
            // One pass of the readers, so what they count doesn't depend on
            // how long the run takes.
            ("XENONET_POLL_MS".into(), "600000".into()),
            ("XENONET_SUPERVISOR_BIN".into(), h("no-supervisor")),
            ("XENONET_SUPERVISOR_DIR".into(), h(".xenonet-agents")),
            ("XENONET_APPLICATIONS_DIR".into(), h("Applications")),
            ("XENONET_CODEX_BIN".into(), h("no-codex")),
            ("XENONET_CLAUDE_BIN".into(), h("no-claude")),
        ];
        if let Ok(t) = std::env::var("TMPDIR") {
            env.push(("TMPDIR".into(), t));
        }
        let child = Command::new(BIN)
            .arg("serve")
            .current_dir(&home)
            .env_clear()
            .envs(env.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let s = Service { base, home, tmp: tmp_dir.to_str().unwrap().to_string(), port, env, child };
        s.wait_ready();
        s
    }

    fn wait_ready(&self) {
        let until = Instant::now() + Duration::from_secs(90);
        while Instant::now() < until {
            if let Some((200, Value::Object(g))) = get(self.port, "/api/graph") {
                let nodes = g.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
                if nodes.iter().any(|n| matches!(n["kind"].as_str(), Some("session" | "project"))) {
                    // A moment for the readers' first pass.
                    std::thread::sleep(Duration::from_secs(3));
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        panic!("xenonet serve didn't show the fixture's sessions within 90 s");
    }

    fn home_text(&self) -> &str {
        self.home.to_str().unwrap()
    }

    fn run(&self, args: &[String]) -> (i32, String) {
        let out = Command::new(BIN).args(args).current_dir(&self.home).env_clear().envs(self.env.iter().cloned()).stdin(Stdio::null()).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = Command::new("kill").arg("-TERM").arg(self.child.id().to_string()).status();
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until && matches!(self.child.try_wait(), Ok(None)) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

// GET a route: its status and its body, as JSON when it is.
fn get(port: u16, route: &str) -> Option<(u16, Value)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    write!(stream, "GET {route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: */*\r\nConnection: close\r\n\r\n").ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut body = raw[split + 4..].to_vec();
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    if head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked")) {
        body = dechunk(&body)?;
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    Some((status, serde_json::from_str(&text).unwrap_or(Value::String(text))))
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line = b.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..line]).split(';').next()?.trim(), 16).ok()?;
        b = &b[line + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

// ---------- setting aside what's bound to differ ----------

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// Each match of a pattern, found by `at` (the end of a match starting at
// i, or None), replaced by `with`, as String.replace(/…/g) does.
fn replace_matches(s: &str, with: &str, at: impl Fn(&[u8], usize) -> Option<usize>) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut from) = (0, 0);
    while i < b.len() {
        match at(b, i) {
            Some(end) => {
                out.push_str(&s[from..i]);
                out.push_str(with);
                i = end;
                from = end;
            }
            None => i += 1,
        }
    }
    out.push_str(&s[from..]);
    out
}

fn digits(b: &[u8], i: usize, n: usize) -> bool {
    b.len() >= i + n && b[i..i + n].iter().all(u8::is_ascii_digit)
}

fn boundary_before(b: &[u8], i: usize) -> bool {
    i == 0 || !word(b[i - 1])
}

// After a match ending in a word character.
fn boundary_after(b: &[u8], end: usize) -> bool {
    end == b.len() || !word(b[end])
}

// \d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(\.\d+)?Z
fn iso_at(b: &[u8], i: usize) -> Option<usize> {
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if b.len() < i + shape.len() + 1 {
        return None;
    }
    for (k, &c) in shape.iter().enumerate() {
        let ok = if c == b'd' { b[i + k].is_ascii_digit() } else { b[i + k] == c };
        if !ok {
            return None;
        }
    }
    let mut p = i + shape.len();
    if b[p] == b'.' && p + 1 < b.len() && b[p + 1].is_ascii_digit() {
        let mut q = p + 1;
        while q < b.len() && b[q].is_ascii_digit() {
            q += 1;
        }
        if q < b.len() && b[q] == b'Z' {
            return Some(q + 1);
        }
    }
    (b[p] == b'Z').then(|| {
        p += 1;
        p
    })
}

// xenonet-demo-\d+
fn demo_at(b: &[u8], i: usize) -> Option<usize> {
    let p = i + b"xenonet-demo-".len();
    if !b[i..].starts_with(b"xenonet-demo-") || !digits(b, p, 1) {
        return None;
    }
    Some(p + b[p..].iter().take_while(|c| c.is_ascii_digit()).count())
}

// \b(Mon|Tues|Wednes|Thurs|Fri|Satur|Sun)day, (January|…|December) \d{1,2}\b
fn date_at(b: &[u8], i: usize) -> Option<usize> {
    const DAYS: [&str; 7] = ["Mon", "Tues", "Wednes", "Thurs", "Fri", "Satur", "Sun"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    if !boundary_before(b, i) {
        return None;
    }
    let day = DAYS.iter().find(|d| b[i..].starts_with(d.as_bytes()))?;
    let mut p = i + day.len();
    p += b[p..].starts_with(b"day, ").then_some(5)?;
    let month = MONTHS.iter().find(|m| b[p..].starts_with(m.as_bytes()))?;
    p += month.len();
    p += (b.get(p) == Some(&b' ')).then_some(1)?;
    [2, 1].into_iter().filter(|&n| digits(b, p, n)).map(|n| p + n).find(|&end| boundary_after(b, end))
}

// \b(Jan|Feb|…|Dec) \d{1,2}\b
fn short_date_at(b: &[u8], i: usize) -> Option<usize> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    if !boundary_before(b, i) {
        return None;
    }
    let month = MONTHS.iter().find(|m| b[i..].starts_with(m.as_bytes()))?;
    let p = i + month.len() + (b.get(i + month.len()) == Some(&b' ')).then_some(1)?;
    [2, 1].into_iter().filter(|&n| digits(b, p, n)).map(|n| p + n).find(|&end| boundary_after(b, end))
}

// \b\d{1,2}:\d\d(:\d\d)?( [AP]M)?\b
fn clock_at(b: &[u8], i: usize) -> Option<usize> {
    if !boundary_before(b, i) {
        return None;
    }
    for h in [2, 1] {
        if !digits(b, i, h) || b.get(i + h) != Some(&b':') || !digits(b, i + h + 1, 2) {
            continue;
        }
        let p = i + h + 3;
        let secs = (b.get(p) == Some(&b':') && digits(b, p + 1, 2)).then_some(p + 3);
        let meridiem = |q: usize| (b.len() >= q + 3 && b[q] == b' ' && matches!(b[q + 1], b'A' | b'P') && b[q + 2] == b'M').then_some(q + 3);
        let ends = [secs.and_then(meridiem), secs, meridiem(p), Some(p)];
        if let Some(end) = ends.into_iter().flatten().find(|&end| boundary_after(b, end)) {
            return Some(end);
        }
    }
    None
}

// \b\d+ min ago\b
fn minutes_ago_at(b: &[u8], i: usize) -> Option<usize> {
    if !boundary_before(b, i) || !digits(b, i, 1) {
        return None;
    }
    let p = i + b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let end = p + b" min ago".len();
    (b[p..].starts_with(b" min ago") && boundary_after(b, end)).then_some(end)
}

// A path as a path key: without /private before /var, /tmp or /etc, and
// lowercased (core/paths.js pathKey).
fn unprivate(p: &str) -> String {
    for top in ["var", "tmp", "etc"] {
        let lead = format!("/private/{top}");
        if let Some(rest) = p.strip_prefix(&lead) {
            if rest.is_empty() || rest.starts_with('/') {
                return format!("/{top}{rest}");
            }
        }
    }
    p.to_string()
}

struct Normal {
    home: String,
    tmp: String,
    port: u16,
    version: String,
    now: f64,
}

impl Normal {
    fn swap(&self, text: &str) -> String {
        let mut s = text.to_string();
        for (path, to) in [(&self.home, "<home>"), (&self.tmp, "<tmp>")] {
            s = s.replace(path.as_str(), to).replace(&unprivate(path), to).replace(&unprivate(path).to_lowercase(), to);
        }
        s = s.replace(&self.version, "<version>").replace(&format!(":{}", self.port), ":<port>");
        s = replace_matches(&s, "xenonet-demo-<port>", demo_at);
        s = replace_matches(&s, "<iso>", iso_at);
        s = replace_matches(&s, "<date>", date_at);
        s = replace_matches(&s, "<date>", short_date_at);
        s = replace_matches(&s, "<clock>", clock_at);
        replace_matches(&s, "<n> min ago", minutes_ago_at)
    }

    fn walk(&self, v: &Value, key: Option<&str>) -> Value {
        match v {
            Value::Number(n) => {
                if let Some(k) = key.filter(|k| ["pid", "port", "ms", "took", "uptime", "elapsed"].contains(k)) {
                    return Value::String(format!("<{k}>"));
                }
                let x = n.as_f64().unwrap_or(0.0);
                if x > 1e12 && (x - self.now).abs() < 400.0 * 86_400_000.0 {
                    return Value::String("<time>".into());
                }
                v.clone()
            }
            Value::String(s) => Value::String(self.swap(s)),
            Value::Array(a) => Value::Array(a.iter().map(|y| self.walk(y, key)).collect()),
            Value::Object(o) => {
                let mut m = Map::new();
                for (k, y) in o {
                    m.insert(self.swap(k), self.walk(y, Some(k)));
                }
                Value::Object(m)
            }
            _ => v.clone(),
        }
    }
}

// ---------- comparing ----------

// The first differences between a golden and an answer, as paths: numbers
// compare as numbers, and an object's keys in their order, as rust-diff
// compares them.
fn differences(a: &Value, b: &Value, at: &str, out: &mut Vec<String>) {
    if out.len() >= 8 {
        return;
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) if x.as_f64() == y.as_f64() => {}
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{}: {} items in the golden, {} in the answer", if at.is_empty() { "." } else { at }, x.len(), y.len()));
            }
            for (k, (p, q)) in x.iter().zip(y).enumerate() {
                differences(p, q, &format!("{at}.{k}"), out);
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, p) in x {
                match y.get(k) {
                    Some(q) => differences(p, q, &format!("{at}.{k}"), out),
                    None => out.push(format!("{at}.{k}: only in the golden")),
                }
            }
            for k in y.keys().filter(|k| !x.contains_key(*k)) {
                out.push(format!("{at}.{k}: only in the answer"));
            }
            if x.len() == y.len() && x.keys().all(|k| y.contains_key(k)) && !x.keys().eq(y.keys()) {
                out.push(format!("{}: keys in another order", if at.is_empty() { "." } else { at }));
            }
        }
        (Value::String(x), Value::String(y)) if x.contains('\n') || y.contains('\n') => {
            for (k, (p, q)) in x.split('\n').zip(y.split('\n')).enumerate().filter(|(_, (p, q))| p != q).take(4) {
                out.push(format!("{at} line {}: {p:?} in the golden, {q:?} in the answer", k + 1));
            }
            if x.split('\n').count() != y.split('\n').count() {
                out.push(format!("{at}: {} lines in the golden, {} in the answer", x.split('\n').count(), y.split('\n').count()));
            }
        }
        _ if a == b => {}
        _ => {
            let cut = |v: &Value| v.to_string().chars().take(160).collect::<String>();
            out.push(format!("{}: {} in the golden, {} in the answer", if at.is_empty() { "." } else { at }, cut(a), cut(b)));
        }
    }
}

fn version() -> String {
    let package = read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../package.json"));
    package["version"].as_str().unwrap().to_string()
}

// The goldens' files, as tests/golden/sequence.json names them.
fn golden_files(kind: &str) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(tests_dir().join("golden").join(kind))
        .unwrap()
        .map(|e| format!("{kind}/{}", e.unwrap().file_name().to_string_lossy()))
        .filter(|f| Path::new(f).extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
        .collect();
    files.sort();
    files
}

#[test]
fn the_service_and_the_command_answer_as_the_js_did() {
    let sequence = read_json(&tests_dir().join("golden").join("sequence.json"));
    let steps = sequence["steps"].as_array().unwrap();
    let mut named: Vec<String> = steps.iter().filter_map(|s| s["golden"].as_str().map(str::to_string)).collect();
    named.sort();
    let mut files = golden_files("http");
    files.extend(golden_files("cli"));
    files.sort();
    assert_eq!(named, files, "every golden is a step of sequence.json, and every step's golden is there");
    let s = Service::start();
    let version = version();
    let normal = || Normal { home: s.home_text().to_string(), tmp: s.tmp.clone(), port: s.port, version: version.clone(), now: now_ms() };
    let mut failed = Vec::new();
    // Each read in the order the recording made them, the ones without a
    // golden too: some leave a trace a later one shows.
    for step in steps {
        let golden = step["golden"].as_str().map(|f| (f, read_json(&tests_dir().join("golden").join(f))));
        let mut d = Vec::new();
        if let Some(route) = step["get"].as_str() {
            let (status, body) = get(s.port, &route.replace("{{home}}", s.home_text())).unwrap_or((0, Value::Null));
            let Some((_, g)) = &golden else { continue };
            let answer = normal().walk(&serde_json::json!({ "status": status, "body": body }), None);
            differences(&serde_json::json!({ "status": g["status"], "body": g["body"] }), &answer, "", &mut d);
        } else {
            let args: Vec<String> = step["run"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().replace("{{home}}", s.home_text())).collect();
            let (code, stdout) = s.run(&args);
            let Some((_, g)) = &golden else { continue };
            let n = normal();
            if g["code"] != code {
                d.push(format!("exit code {code}, the golden's {}", g["code"]));
            }
            match (g.get("json"), serde_json::from_str::<Value>(&stdout)) {
                (Some(want), Ok(got)) => differences(want, &n.walk(&got, None), ".json", &mut d),
                (Some(_), Err(_)) => d.push(format!("JSON in the golden, and the answer isn't: {:?}", stdout.chars().take(200).collect::<String>())),
                (None, _) => differences(&g["stdout"], &Value::String(n.swap(&stdout)), ".stdout", &mut d),
            }
        }
        if let (Some((file, _)), false) = (&golden, d.is_empty()) {
            let what = step["get"]
                .as_str()
                .map_or_else(|| format!("xenonet {}", step["run"].as_array().unwrap().iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")), str::to_string);
            failed.push(format!("{file} ({what}):\n    {}", d.join("\n    ")));
        }
    }
    drop(s);
    assert!(failed.is_empty(), "{} of {} goldens differ:\n{}", failed.len(), files.len(), failed.join("\n"));
}

// The normaliser itself, against what the JS one makes of the same text.
#[test]
fn what_differs_from_run_to_run_is_set_aside_as_rust_diff_sets_it_aside() {
    let n = Normal {
        home: "/private/var/folders/x/T/xenonet-golden-1/Home".into(),
        tmp: "/private/var/folders/x/T".into(),
        port: 51234,
        version: "0.6.6".into(),
        now: 1_786_000_000_000.0,
    };
    let swap = |s: &str| n.swap(s);
    assert_eq!(swap("/private/var/folders/x/T/xenonet-golden-1/Home/Projects"), "<home>/Projects");
    assert_eq!(swap("project:/var/folders/x/t/xenonet-golden-1/home/projects/a"), "project:<home>/projects/a");
    assert_eq!(swap("/private/var/folders/x/T/other"), "<tmp>/other");
    assert_eq!(swap("XenoNet 0.6.6 on 127.0.0.1:51234"), "XenoNet <version> on 127.0.0.1:<port>");
    assert_eq!(swap("at 2026-11-08T12:00:00.000Z and 2026-11-08T12:00:00Z, not 2026-11-08T12:00:00.5"), "at <iso> and <iso>, not 2026-11-08T12:<clock>.5");
    assert_eq!(swap("Sunday, November 8 and Wednesday, March 18x"), "<date> and Wednesday, March 18x");
    assert_eq!(swap("Nov 8, 11:10:13 PM; Mayday 5; Dec 31x"), "<date>, <clock>; Mayday 5; Dec 31x");
    assert_eq!(swap("10:55 PM to 11:08 PM, 9:05:12 AM, 23:59, 1:234, a1:23, 12:30PM"), "<clock> to <clock>, <clock>, <clock>, 1:234, a1:23, 12:30PM");
    assert_eq!(swap("claude:0190d3e0 a000-000000000001:0190d3e0"), "claude:0190d3e0 a000-000000000001:0190d3e0");
    assert_eq!(swap("2 min ago, 12 min ago, x3 min ago, 3 h ago"), "<n> min ago, <n> min ago, x3 min ago, 3 h ago");
    assert_eq!(swap("/tmp/xenonet-demo-47841/home"), "/tmp/xenonet-demo-<port>/home");
    let v = serde_json::json!({ "pid": 12, "at": 1_786_000_100_000_u64, "old": 1_000_000_000_000_u64, "list": { "ms": [1, 2] }, "n": 3 });
    assert_eq!(n.walk(&v, None), serde_json::json!({ "pid": "<pid>", "at": "<time>", "old": 1_000_000_000_000_u64, "list": { "ms": ["<ms>", "<ms>"] }, "n": 3 }));
    let filled = fill("{{home}}/a {{iso:-60}} {{unix:-60}} {{other}}", "/h", 1_786_000_000_000.0);
    assert_eq!(filled, format!("/h/a {} 1785999940 {{{{other}}}}", xenonet_base::time::iso(1_785_999_940_000.0)));
}
