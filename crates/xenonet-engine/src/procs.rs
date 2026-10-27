//! Which process is on the other end of a connection to XenoNet, and the
//! processes above it (core/procs.js), so the engine can check that a
//! request made in an agent's name comes from that agent, and an actor's
//! from where its token may be used (see `verify`).
//!
//! macOS only for now: netstat -anv names the process that owns each TCP
//! socket, ps gives each process's parent, start time and program, codesign
//! says who signed a running program, lsof where a process works and the
//! executable it runs, plutil an app bundle's id, and pgrep a process's
//! children. One ps of the whole table costs about a tenth of a second
//! whatever it asks for, so the table is shared for a moment, and a
//! connection's process, a program's signature, its executable, a bundle's
//! id and what a Node or Python process runs are looked up once.
//!
//! The program ps names is `argv[0]`, which a process sets itself (exec -a).
//! Where a check rests on a path (Hermes's hosts, XenoNet's own bundle), the
//! executable's real path decides instead (chain's `exe`).
//!
//! The commands run through `Io`: the system in the binary, and the twin's
//! `exec` behind core/procs.js. A command asked for while the same one is
//! already running (`Call::share`) waits for that one's answer, as JS's
//! shared promises do.

use crate::js::{is_integer, template, truthy};
use crate::re;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use xenonet_base::json::number_string;
use xenonet_base::nodepath::Style;
use xenonet_base::paths::normalize_posix;
use xenonet_base::sync::Locked;
use xenonet_base::text::{js_number, js_string, js_trim};

const TABLE_MS: f64 = 1500.0;
// New Hermes.app bundles a census reads the ids of, at most.
const BUNDLES_AT_ONCE: usize = 16;

/// The developer team that signs each agent app (codesign's
/// TeamIdentifier), as the app registry has it: Anthropic's for Claude
/// Code, OpenAI's for Codex.
pub fn team_of_agent(agent: &str) -> Option<&'static str> {
    xenonet_base::apps::team(agent)
}

/// The teams that sign the actor apps whose makers sign them, by product:
/// Grok Bot is Cursor's (Anysphere), Muse is Meta's. Their actors register
/// and report only from under the app as its maker built it (verify).
/// Hermes's Python is signed ad hoc, so it has none.
pub fn team_of_product(product: &str) -> Option<&'static str> {
    match product {
        "grok-bot" => Some("DCNK4UB866"),
        "muse" => Some("V9WTTPBFK9"),
        _ => None,
    }
}

/// A command to run, and the key under which a run of it already under way
/// is shared.
#[derive(Clone, Debug)]
pub struct Call {
    pub file: String,
    pub args: Vec<String>,
    pub share: Option<String>,
}

impl Call {
    pub fn new(file: &str, args: &[&str]) -> Self {
        Call { file: file.into(), args: args.iter().map(ToString::to_string).collect(), share: None }
    }
    fn shared(mut self, key: String) -> Self {
        self.share = Some(key);
        self
    }
}

/// A command that failed, with what it wrote anyway: lsof says 1 when one
/// of the processes it's asked about can't be read, and still lists the
/// rest.
#[derive(Clone, Debug, Default)]
pub struct Failure {
    pub message: String,
    pub stdout: Option<String>,
}

pub trait Io {
    fn exec(&mut self, call: Call) -> Result<String, Failure>;
    /// Several commands at once, their answers in order.
    fn exec_all(&mut self, calls: Vec<Call>) -> Vec<Result<String, Failure>> {
        calls.into_iter().map(|c| self.exec(c)).collect()
    }
    fn now(&mut self) -> f64;
}

/// A process as ps lists it: its pid, its parent's, when it started (ms,
/// UTC) and its program (`argv[0]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub pid: f64,
    pub ppid: f64,
    pub start: f64,
    pub program: String,
}

/// The actor product a program belongs to, and its role there (see
/// `product_of`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Product {
    pub product: &'static str,
    pub role: &'static str,
}

/// A process of a chain: its row, its executable's real path when asked
/// for (`exe`: None when not asked, Some(None) when it couldn't be read),
/// and what it is.
#[derive(Clone, Debug)]
pub struct Link {
    pub row: Row,
    pub exe: Option<Option<String>>,
    pub agent: Option<&'static str>,
    pub product: Option<Product>,
}

/// A map by pid, in the order its pids were first set, as a JS Map keyed
/// by numbers.
#[derive(Clone, Debug)]
pub struct PidMap<V> {
    items: Vec<(f64, V)>,
    at: HashMap<u64, usize>,
}

impl<V> Default for PidMap<V> {
    fn default() -> Self {
        PidMap { items: Vec::new(), at: HashMap::new() }
    }
}

// JS Maps take 0 and -0 for the same key.
fn bits(pid: f64) -> u64 {
    if pid == 0.0 {
        0
    } else {
        pid.to_bits()
    }
}

impl PidMap<Row> {
    /// A table of one row.
    pub fn from_row(row: Row) -> Self {
        let mut t = PidMap::default();
        t.set(row.pid, row);
        t
    }
}

impl<V> PidMap<V> {
    pub fn set(&mut self, pid: f64, v: V) {
        match self.at.get(&bits(pid)) {
            Some(&i) => self.items[i].1 = v,
            None => {
                self.at.insert(bits(pid), self.items.len());
                self.items.push((pid, v));
            }
        }
    }
    pub fn get(&self, pid: f64) -> Option<&V> {
        self.at.get(&bits(pid)).map(|&i| &self.items[i].1)
    }
    pub fn has(&self, pid: f64) -> bool {
        self.at.contains_key(&bits(pid))
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &(f64, V)> {
        self.items.iter()
    }
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.items.iter().map(|(_, v)| v)
    }
}

pub type Table = PidMap<Row>;

pub(crate) fn slashes(p: &str) -> String {
    p.replace('\\', "/")
}

/// The agent app a program is, from its path: Claude Code, Codex or
/// OpenCode, each by its own rules (`processes`). Programs run by Node or
/// Bun are told by their arguments, when given.
pub use crate::processes::agent_of;

/// The apps of actors whose own processes could be relaying for a cloud,
/// by their bundle's name: Hermes.app (the desktop app, and its setup
/// app), Grok Bot, Muse, and the ChatGPT app, where OpenAI's dots run.
fn actor_app(bundle: &str) -> Option<&'static str> {
    match bundle {
        "Hermes" => Some("hermes-agent"),
        "Grok Bot" => Some("grok-bot"),
        "Muse" => Some("muse"),
        "ChatGPT" => Some("dots"),
        _ => None,
    }
}

/// Whether a program's file name is Node's or Bun's (node.exe and bun.exe
/// on Windows).
pub fn is_node(base: &str) -> bool {
    matches!(base, "node" | "bun" | "node.exe" | "bun.exe")
}

fn is_python(base: &str) -> bool {
    re!(r"^python(\d+(\.\d+)*)?$", i).is_match(base)
}

// Where Hermes keeps the Pythons that run it: its own folder (the managed
// Python, its venv) or the inside of Hermes.app.
fn hermes_path(p: &str) -> bool {
    p.contains("/.hermes/") || p.contains("/Hermes.app/")
}

/// Whether a program needs its arguments read to say what it is: Node and
/// Bun (which agent, or Hermes's terminal UI), and the Pythons Hermes runs.
pub fn wants_args(program: &str) -> bool {
    let p = slashes(program);
    let base = &Style::Posix.basename(&p, None);
    is_node(base) || (is_python(base) && hermes_path(&p))
}

/// The actor product a program belongs to, from its path and, for Node and
/// Python, its arguments. Roles:
///
///   host  runs the agent, so what runs under it is the product's: Hermes's
///         managed Python (or its venv's, or one inside Hermes.app) running
///         its CLI, its terminal UI's backend or its agent
///   tui   Hermes's terminal UI, whose child is the host
///   app   an actor app's own process, its main one or a helper
///
/// No code signature vouches for Hermes's processes (its Python is signed
/// ad hoc), so this rests on paths and arguments, and on Hermes.app's
/// bundle id where a path lies in one (`Procs` checks it). An agent's own
/// program (Claude Code or Codex inside the ChatGPT app) is an agent, not
/// this.
pub fn product_of(program: &str, args: &str) -> Option<Product> {
    let p = slashes(program);
    let base = &Style::Posix.basename(&p, None);
    if is_python(base) && hermes_path(&p) && re!(r"\b(hermes_cli|tui_gateway|run_agent)\b").is_match(args) {
        return Some(Product { product: "hermes-agent", role: "host" });
    }
    if is_node(base) && re!(r"\/(ui-tui\/dist|hermes_cli\/tui_dist)\/entry\.js\b").is_match(args) {
        return Some(Product { product: "hermes-agent", role: "tui" });
    }
    let bundle = re!(r"\/([^/]+)\.app\/").captures(&p).and_then(|c| c.get(1)).map(|m| m.as_str());
    bundle.and_then(actor_app).map(|product| Product { product, role: "app" })
}

// A Hermes.app is Hermes's, a Grok Bot.app Grok Bot's and a Muse.app
// Meta's Muse, only when its Info.plist says so: other makers' apps can
// have the same names (there are other Mac apps called Muse), and anyone
// can name a bundle Grok Bot.app. For each, the outermost bundle of that
// name a path lies in, and the bundle ids that are the product's.
fn vouched_bundle_of(product: &str, program: &str) -> Option<String> {
    let r = match product {
        "hermes-agent" => re!(r"^(.*?\/Hermes\.app)\/"),
        "grok-bot" => re!(r"^(.*?\/Grok Bot\.app)\/"),
        "muse" => re!(r"^(.*?\/Muse\.app)\/"),
        _ => return None,
    };
    r.captures(program).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

fn vouched_id(product: &str, id: &str) -> bool {
    match product {
        "hermes-agent" => re!(r"^com\.nousresearch\.hermes(\.|$)").is_match(id),
        "grok-bot" => re!(r"^com\.anysphere\.sand(\.|$)").is_match(id),
        "muse" => re!(r"^com\.meta\.endo(\.|$)").is_match(id),
        _ => false,
    }
}

const VOUCHED: [&str; 3] = ["hermes-agent", "grok-bot", "muse"];

/// The bundle of a vouched product's name a program lies in, or None.
pub fn vouched_bundle(program: &str) -> Option<String> {
    VOUCHED.iter().find_map(|v| vouched_bundle_of(v, program))
}

fn bundle_for(product: Option<Product>, program: &str) -> Option<String> {
    product.and_then(|p| vouched_bundle_of(p.product, program))
}

/// A product a process was taken for, once the vouched app its path lies
/// in (if any) has been asked: kept when that bundle is the product's,
/// and dropped when it's another maker's. One whose id can't be read stays
/// the product's for an app's process, which only ever counts against it,
/// but not for an agent host, which Hermes's token may use.
fn vouch(product: Option<Product>, program: &str, id: Option<&str>) -> Option<Product> {
    let p = product?;
    if bundle_for(product, program).is_none() {
        return product;
    }
    match id {
        None => (p.role != "host").then_some(p),
        Some(id) => vouched_id(p.product, id).then_some(p),
    }
}

fn js_num(s: &str) -> f64 {
    js_number(Some(&Value::String(s.to_string())))
}

// Number(s) || null: what the "p" lines of lsof's answers name.
fn pid_of(s: &str) -> Option<f64> {
    let n = js_num(s);
    (n != 0.0 && !n.is_nan()).then_some(n)
}

/// What `lsof -a -p <pids> -d txt -Fn` says of several processes: each one's
/// executable, by pid, as its real path. lsof lists a process's program
/// text in the order it's mapped, and the executable is mapped first,
/// before dyld and the libraries.
pub fn parse_texts(text: &str) -> PidMap<String> {
    let mut out = PidMap::default();
    let (mut pid, mut fd): (Option<f64>, Option<&str>) = (None, None);
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix('p') {
            pid = pid_of(rest);
            fd = None;
        } else if let Some(rest) = line.strip_prefix('f') {
            fd = Some(rest);
        } else if line.starts_with("n/") {
            if let (Some(p), Some("txt")) = (pid, fd) {
                if !out.has(p) {
                    out.set(p, line[1..].to_string());
                }
            }
        }
    }
    out
}

/// What `lsof -a -p <pid> -d cwd -Fn` says of a process's folder: its "n"
/// line, or None.
pub fn parse_cwd(text: &str) -> Option<String> {
    text.split('\n').find(|l| l.starts_with("n/")).map(|l| l[1..].to_string())
}

/// What `lsof -a -p <pids> -d cwd -Fpn` says of several processes' folders.
pub fn parse_cwds(text: &str) -> PidMap<String> {
    let mut out = PidMap::default();
    let mut pid = None;
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix('p') {
            pid = pid_of(rest);
        } else if line.starts_with("n/") {
            if let Some(p) = pid {
                out.set(p, line[1..].to_string());
            }
        }
    }
    out
}

/// What `ps -o pid=,command= -p <pids>` says: each process's command line.
pub fn parse_commands(text: &str) -> PidMap<String> {
    let mut out = PidMap::default();
    for line in text.split('\n') {
        if let Some(c) = re!(r"^\s*(\d+)\s(.*)$").captures(line) {
            out.set(js_num(&c[1]), c[2].to_string());
        }
    }
    out
}

fn month(m: &str) -> Option<f64> {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|x| *x == m).map(|i| i as f64)
}

// Date.UTC, with its fields rolling over as JS's do, and TimeClip.
fn date_utc(year: f64, month: f64, day: f64, h: f64, m: f64, s: f64) -> f64 {
    // Years 0 to 99 are 1900 to 1999, as Date.UTC reads them.
    let year = if (0.0..100.0).contains(&year.trunc()) { 1900.0 + year.trunc() } else { year };
    let ym = year + (month / 12.0).floor();
    let mn = month.rem_euclid(12.0);
    if !ym.is_finite() || ym.abs() > 400_000.0 {
        return f64::NAN;
    }
    let days = xenonet_base::time::days_from_civil(ym as i64, mn as u32 + 1, 1) as f64 + day - 1.0;
    let t = days * 86_400_000.0 + h * 3_600_000.0 + m * 60_000.0 + s * 1000.0;
    if t.abs() > 8.64e15 {
        f64::NAN
    } else {
        t + 0.0
    }
}

/// ps -axo pid=,ppid=,lstart=,comm= as a table by pid. lstart reads like
/// "Sat Oct  3 16:34:29 2026", in UTC (ps runs with TZ=UTC); the program's
/// path runs to the end of the line.
pub fn parse_ps(text: &str) -> Table {
    let mut table = PidMap::default();
    for line in text.split('\n') {
        let Some(m) = re!(r"^\s*(\d+)\s+(\d+)\s+\w{3}\s+(\w{3})\s+(\d+)\s+(\d+):(\d+):(\d+)\s+(\d{4})\s+(.*)$").captures(line) else {
            continue;
        };
        let start = match month(&m[3]) {
            Some(mo) => date_utc(js_num(&m[8]), mo, js_num(&m[4]), js_num(&m[5]), js_num(&m[6]), js_num(&m[7])),
            None => 0.0,
        };
        let pid = js_num(&m[1]);
        table.set(pid, Row { pid, ppid: js_num(&m[2]), start, program: js_trim(&m[9]).to_string() });
    }
    table
}

/// The process holding the client's end of a connection to 127.0.0.1:
/// `netstat -anv -p tcp` lines read `tcp4 … <local> <foreign> <state>
/// <4 counters> <name>:<pid> <flags> …`, and the name may have spaces, or
/// be empty once the process has exited (the pid is still the last one
/// that held the socket). While XenoNet is answering, its own end is open,
/// so the client's end is ESTABLISHED, or FIN_WAIT_1 or FIN_WAIT_2 when the
/// client has closed its side; TIME_WAIT on that pair is an earlier
/// connection's.
pub fn parse_netstat(text: &str, local_port: &str, remote_port: &str) -> Option<f64> {
    let client = format!("127.0.0.1.{remote_port}");
    let server = format!("127.0.0.1.{local_port}");
    let mut closing = None;
    for line in text.split('\n') {
        let Some(m) = re!(r"^tcp4\s+\d+\s+\d+\s+(\S+)\s+(\S+)\s+([A-Z_0-9]+)\s+\d+\s+\d+\s+\d+\s+\d+\s+(.*):(\d+)\s+[0-9a-f]{5}\s").captures(line) else {
            continue;
        };
        let state = &m[3];
        if m[1] != *client || m[2] != *server || !matches!(state, "ESTABLISHED" | "FIN_WAIT_1" | "FIN_WAIT_2") {
            continue;
        }
        if state == "ESTABLISHED" {
            return Some(js_num(&m[5]));
        }
        closing = closing.or(Some(js_num(&m[5])));
    }
    closing
}

// ---------- Linux and Windows ----------
//
// There XenoNet reads the processes itself rather than through ps and
// lsof: on Linux from /proc, on Windows from a ToolHelp32 snapshot (see
// procs_native). They're asked for as these calls, which the system's
// `Io` answers natively and a test answers with a made-up table.

/// Every process: one line each, `pid ppid start program exe`, tab
/// separated, with its start time in ms (UTC), the program it says it is
/// (Linux: `argv[0]`; Windows: its image) and its executable's real path
/// (empty where it can't be read: another user's process).
pub const PROCESSES: &str = "xenonet:processes";
/// The process holding the client's end of a connection to 127.0.0.1, by
/// `[local port, remote port]` as the server sees them: its pid, or nothing.
/// Linux only; Windows asks netstat -ano.
pub const TCP_PEER: &str = "xenonet:tcp-peer";
/// A process's command line, by `[pid]`.
pub const COMMAND_LINE: &str = "xenonet:command-line";

/// What `PROCESSES` answers: the table, and each process's executable.
pub fn parse_table(text: &str) -> (Table, PidMap<String>) {
    let (mut table, mut exes) = (PidMap::default(), PidMap::default());
    for line in text.split('\n') {
        let f: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        let (pid, ppid, start) = (js_num(f[0]), js_num(f[1]), js_num(f[2]));
        if !pid.is_finite() || pid.trunc() != pid || f[3].is_empty() {
            continue;
        }
        table.set(pid, Row { pid, ppid: if ppid.is_finite() { ppid } else { 0.0 }, start: if start.is_finite() { start } else { 0.0 }, program: f[3].to_string() });
        if let Some(exe) = f.get(4).filter(|e| !e.is_empty()) {
            exes.set(pid, (*exe).to_string());
        }
    }
    (table, exes)
}

/// The process holding the client's end of a connection to 127.0.0.1, as
/// `netstat -ano -p TCP` lists it on Windows: `TCP <local> <foreign>
/// <state> <pid>`, the state in the system's language, so it isn't read;
/// a pair whose pid is 0 has been closed.
pub fn parse_netstat_windows(text: &str, local_port: &str, remote_port: &str) -> Option<f64> {
    let client = format!("127.0.0.1:{remote_port}");
    let server = format!("127.0.0.1:{local_port}");
    text.split('\n').find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let pid = js_num(f.last()?);
        (f.len() >= 4 && f[0].eq_ignore_ascii_case("TCP") && f[1] == client && f[2] == server && pid > 0.0).then_some(pid)
    })
}

/// The socket inode of the client's end of a connection to 127.0.0.1, as
/// /proc/net/tcp lists it on Linux: `sl local rem st … uid timeout inode`,
/// addresses as hex `ADDR:PORT` (127.0.0.1 is 0100007F), and an inode of 0
/// for a socket that's closing.
pub fn parse_proc_net_tcp(text: &str, local_port: u16, remote_port: u16) -> Option<String> {
    let client = format!("0100007F:{remote_port:04X}");
    let server = format!("0100007F:{local_port:04X}");
    text.split('\n').skip(1).find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        (f.len() > 9 && f[1].eq_ignore_ascii_case(&client) && f[2].eq_ignore_ascii_case(&server) && f[9] != "0").then(|| f[9].to_string())
    })
}

fn key(p: &Row) -> String {
    format!("{}:{}:{}", number_string(p.pid), number_string(p.start), p.program)
}

fn pids_list(pids: &[f64]) -> String {
    pids.iter().map(|p| number_string(*p)).collect::<Vec<_>>().join(",")
}

// The integers above `min` among these, each once, in order: a JS Set of
// them, filtered.
fn listed(pids: &[Value], min: f64) -> Vec<f64> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for v in pids {
        if let Some(n) = is_integer(Some(v)) {
            if n > min && seen.insert(bits(n)) {
                out.push(n);
            }
        }
    }
    out
}

// A bundle id's read: under way while plutil runs, then what it read
// (None when it couldn't).
enum BundleRead {
    Asked,
    Settled(Option<String>),
}

#[derive(Default)]
struct State {
    table: Option<Arc<Table>>,
    table_at: f64,
    // A signature's verdict by process and team; None while codesign runs.
    signatures: HashMap<String, Option<bool>>,
    // What each process of Node, Bun or Hermes's Pythons is, by pid, start
    // time and program, so its arguments are read once: only the verdict
    // is kept, never the arguments.
    verdicts: HashMap<String, (Option<&'static str>, Option<Product>)>,
    // Each process's executable, by pid, start time and program.
    executables: HashMap<String, String>,
    // The bundle id of each Hermes.app, Grok Bot.app and Muse.app, by its
    // folder: asked, and settled.
    bundle_reads: HashMap<String, BundleRead>,
    bundle_ids: HashMap<String, Option<String>>,
    // Where the table is read natively (Linux, Windows), each process's
    // executable as the last read said, by pid, start time and program.
    native_exes: HashMap<String, String>,
}

/// What the census says of a process: `of` in core/procs.js.
pub type Verdict = (Option<&'static str>, Option<Product>);

pub struct Procs {
    /// Everything procs does: macOS, where codesign can say who signed a
    /// running program, so requests in a session's name, actors' tokens and
    /// the window's routes are checked too.
    pub supported: bool,
    /// The process on the other end of a connection and the ones above it,
    /// each with the agent or actor product it is: macOS, Linux and
    /// Windows. What's yours to do (verify's `person`) rests on this alone.
    pub chains: bool,
    // Linux and Windows, where the table is read natively (PROCESSES).
    native: bool,
    platform: String,
    state: Mutex<State>,
}

impl Procs {
    pub fn new(platform: &str) -> Self {
        let native = platform == "linux" || platform == "win32";
        Procs { supported: platform == "darwin", chains: platform == "darwin" || native, native, platform: platform.to_string(), state: Mutex::new(State::default()) }
    }

    // The whole table as PROCESSES gives it, with each process's
    // executable kept for chain's `exe`.
    fn read_native(&self, io: &mut dyn Io, shared: bool) -> Result<Table, String> {
        let call = Call::new(PROCESSES, &[]);
        let text = io.exec(if shared { call.shared(PROCESSES.into()) } else { call }).map_err(|f| f.message)?;
        let (table, exes) = parse_table(&text);
        let mut s = self.s();
        if s.native_exes.len() > 4096 {
            s.native_exes.clear();
        }
        for (pid, exe) in exes.iter() {
            if let Some(row) = table.get(*pid) {
                s.native_exes.insert(key(row), exe.clone());
            }
        }
        Ok(table)
    }

    fn s(&self) -> MutexGuard<'_, State> {
        self.state.locked()
    }

    /// The process table, shared for TABLE_MS.
    pub fn snapshot(&self, io: &mut dyn Io) -> Result<Arc<Table>, String> {
        if self.s().table.is_some() {
            let now = io.now();
            let s = self.s();
            if let Some(t) = &s.table {
                if now - s.table_at < TABLE_MS {
                    return Ok(t.clone());
                }
            }
        }
        let table = if self.native {
            Arc::new(self.read_native(io, true)?)
        } else {
            let text = io.exec(Call::new("ps", &["-axo", "pid=,ppid=,lstart=,comm="]).shared("ps -axo".into())).map_err(|f| f.message)?;
            Arc::new(parse_ps(&text))
        };
        let now = io.now();
        let mut s = self.s();
        s.table = Some(table.clone());
        s.table_at = now;
        Ok(table)
    }

    /// The process on the other end of a socket (its ports, and the pid
    /// found for it before, `kept`), or null when it can't be told, and
    /// whether to keep it on the socket. Only a process found is kept, so
    /// a lookup that failed is tried again on its next request.
    pub fn peer(&self, io: &mut dyn Io, local_port: Option<&Value>, remote_port: Option<&Value>, kept: Option<&Value>) -> (Value, bool) {
        if !self.chains || !truthy(remote_port) {
            return (Value::Null, false);
        }
        if let Some(k) = kept.filter(|k| !k.is_null()) {
            return (k.clone(), false);
        }
        let (local, remote) = (template(local_port), template(remote_port));
        let found = match self.platform.as_str() {
            "win32" => parse_netstat_windows(&io.exec(Call::new("netstat", &["-ano", "-p", "TCP"])).unwrap_or_default(), &local, &remote),
            "linux" => io.exec(Call::new(TCP_PEER, &[&local, &remote])).ok().map(|t| js_num(js_trim(&t))).filter(|p| p.is_finite() && *p > 0.0),
            _ => parse_netstat(&io.exec(Call::new("netstat", &["-anv", "-p", "tcp"])).unwrap_or_default(), &local, &remote),
        };
        match found {
            Some(pid) => (xenonet_base::json::number(pid), true),
            None => (Value::Null, false),
        }
    }

    /// A process and the ones above it, nearest first, up to launchd, each
    /// with the agent app it is or the actor product it belongs to. A
    /// process of Node, Bun or one of Hermes's Pythons is asked for its
    /// arguments, to tell what it runs.
    ///
    /// A process started after the shared table was read isn't in it, so a
    /// pid the table doesn't have is read by itself, with its parents up to
    /// the first one the table has as it is, before it's taken for gone
    /// (`again`: false for a process seen before, such as a session's pin).
    ///
    /// `exe`: each process also gets its executable's real path, and what
    /// it is goes by that path rather than by `argv[0]`. An agent host is one
    /// only by its real path.
    pub fn chain(&self, io: &mut dyn Io, pid: Option<f64>, again: bool, exe: bool) -> Result<Vec<Link>, String> {
        let t = self.snapshot(io)?;
        let fresh = match pid {
            Some(p) if again && !t.has(p) => Some(self.newcomers(io, p, &t)),
            _ => None,
        };
        let get = |p: f64| fresh.as_ref().and_then(|f| f.get(p)).or_else(|| t.get(p)).cloned();
        let mut rows = Vec::new();
        let mut next = pid.and_then(get);
        while let Some(p) = next {
            if rows.len() >= 64 {
                break;
            }
            // Windows keeps a parent's pid after the parent is gone, and may
            // give it to a newer process: a "parent" that started after its
            // child isn't the one that started it.
            next = if p.ppid > 1.0 { get(p.ppid).filter(|q| !(self.native && q.start > 0.0 && p.start > 0.0 && q.start > p.start)) } else { None };
            rows.push(p);
        }
        let real = if exe { Some(self.executables_of(io, &rows)) } else { None };
        let mut out = Vec::new();
        for p in rows {
            let file: Option<String> = match &real {
                Some(m) => m.get(p.pid).cloned().flatten(),
                None => Some(p.program.clone()),
            };
            let named = file.as_deref().filter(|f| !f.is_empty());
            let of = |args: &str| agent_of(&p.program, args).or_else(|| named.and_then(|f| agent_of(f, args)));
            let mut agent = of("");
            let mut args = String::new();
            if agent.is_none() && (wants_args(&p.program) || named.is_some_and(wants_args)) {
                let pid = number_string(p.pid);
                args = io.exec(if self.native { Call::new(COMMAND_LINE, &[&pid]) } else { Call::new("ps", &["-o", "command=", "-p", &pid]) }).unwrap_or_default();
                agent = of(&args);
            }
            let program = file.as_deref().unwrap_or(&p.program).to_string();
            let mut product = if agent.is_some() { None } else { product_of(&program, &args) };
            // argv[0] alone never makes an agent host.
            if product.is_some_and(|x| x.role == "host") && named.is_none() {
                product = None;
            }
            let product = self.vouched(io, product, &program);
            out.push(Link { row: p, exe: real.as_ref().map(|_| file.clone()), agent, product });
        }
        Ok(out)
    }

    // The executables of these processes, by pid: those read before, and
    // the rest in one lsof. Kept by pid, start time and program, so a
    // process that runs another program under a new name is read again.
    fn executables_of(&self, io: &mut dyn Io, rows: &[Row]) -> PidMap<Option<String>> {
        if self.native {
            let s = self.s();
            let mut out = PidMap::default();
            for p in rows {
                out.set(p.pid, s.native_exes.get(&key(p)).cloned());
            }
            return out;
        }
        let missing: Vec<&Row> = {
            let s = self.s();
            rows.iter().filter(|p| !s.executables.contains_key(&key(p))).collect()
        };
        if !missing.is_empty() {
            let list = pids_list(&missing.iter().map(|p| p.pid).collect::<Vec<_>>());
            let text = match io.exec(Call::new("lsof", &["-a", "-p", &list, "-d", "txt", "-Fn"])) {
                Ok(t) => t,
                Err(f) => f.stdout.unwrap_or_default(),
            };
            let read = parse_texts(&text);
            let mut s = self.s();
            if s.executables.len() > 1024 {
                s.executables.clear();
            }
            // A process lsof said nothing about isn't kept, so it's asked
            // again.
            for p in missing {
                if let Some(x) = read.get(p.pid) {
                    s.executables.insert(key(p), x.clone());
                }
            }
        }
        let s = self.s();
        let mut out = PidMap::default();
        for p in rows {
            out.set(p.pid, s.executables.get(&key(p)).cloned());
        }
        out
    }

    // Starts reading a vouched app's bundle id, unless it's read or being
    // read: the call to make, if any.
    fn begin_bundle(s: &mut State, bundle: &str) -> Option<Call> {
        if s.bundle_reads.contains_key(bundle) {
            return None;
        }
        if s.bundle_reads.len() > 256 {
            s.bundle_reads.clear();
            s.bundle_ids.clear();
        }
        s.bundle_reads.insert(bundle.to_string(), BundleRead::Asked);
        Some(Self::plutil(bundle))
    }

    fn plutil(bundle: &str) -> Call {
        let plist = normalize_posix(&format!("{}/Contents/Info.plist", slashes(bundle)));
        Call::new("plutil", &["-extract", "CFBundleIdentifier", "raw", "-o", "-", &plist]).shared(format!("plutil {bundle}"))
    }

    fn settle_bundle(&self, bundle: &str, answer: Result<String, Failure>) -> Option<String> {
        let id = answer.ok().map(|out| js_trim(&out).to_string()).filter(|s| !s.is_empty());
        let mut s = self.s();
        if let Some(slot) = s.bundle_reads.get_mut(bundle) {
            *slot = BundleRead::Settled(id.clone());
        }
        s.bundle_ids.insert(bundle.to_string(), id.clone());
        id
    }

    /// A vouched app's bundle id: the CFBundleIdentifier of its Info.plist,
    /// that key alone, read once (None when it can't be read).
    fn bundle_id(&self, io: &mut dyn Io, bundle: &str) -> Option<String> {
        let call = {
            let mut s = self.s();
            match s.bundle_reads.get(bundle) {
                Some(BundleRead::Settled(id)) => return id.clone(),
                Some(BundleRead::Asked) => Self::plutil(bundle),
                None => Self::begin_bundle(&mut s, bundle).expect("not read yet"),
            }
        };
        let answer = io.exec(call);
        self.settle_bundle(bundle, answer)
    }

    fn vouched(&self, io: &mut dyn Io, product: Option<Product>, program: &str) -> Option<Product> {
        match bundle_for(product, program) {
            Some(bundle) => {
                let id = self.bundle_id(io, &bundle);
                vouch(product, program, id.as_deref())
            }
            None => product,
        }
    }

    // vouch, from the ids read so far, for the census.
    fn settled(s: &State, product: Option<Product>, program: &str) -> Option<Product> {
        let id = bundle_for(product, program).and_then(|b| s.bundle_ids.get(&b).cloned().flatten());
        vouch(product, program, id.as_deref())
    }

    // A process the shared table doesn't have, and its parents until one
    // the table has as it is (the same start and program, so not a reused
    // pid), by pid: each read with a ps of its own. Empty for a process
    // that's gone.
    fn newcomers(&self, io: &mut dyn Io, pid: f64, t: &Table) -> Table {
        let mut out = PidMap::default();
        // Natively, the whole table is read again: it's quick.
        let fresh = if self.native { self.read_native(io, false).ok() } else { None };
        let mut p = pid;
        while p > 1.0 && out.len() < 64 {
            let described = match &fresh {
                Some(f) => f.get(p).map(|r| PidMap::from_row(r.clone())).unwrap_or_default(),
                None => self.describe(io, &[Value::from(p)]),
            };
            let Some(row) = described.get(p) else { break };
            if t.get(p).is_some_and(|k| k.start == row.start && k.program == row.program) {
                break;
            }
            let next = row.ppid;
            out.set(p, row.clone());
            p = next;
        }
        out
    }

    /// The folder a process works in, or None when it can't be told.
    pub fn cwd(&self, io: &mut dyn Io, pid: Option<&Value>) -> Option<String> {
        let pid = is_integer(pid).filter(|p| *p >= 1.0)?;
        if !self.supported {
            return None;
        }
        let text = io.exec(Call::new("lsof", &["-a", "-p", &number_string(pid), "-d", "cwd", "-Fn"])).unwrap_or_default();
        parse_cwd(&text)
    }

    /// Whether a process is still the one that was seen (pids are reused).
    pub fn alive(&self, io: &mut dyn Io, pid: Option<&Value>, start: Option<&Value>) -> Result<bool, String> {
        let t = self.snapshot(io)?;
        let row = pid.and_then(Value::as_f64).and_then(|p| t.get(p));
        Ok(match (row, start.and_then(Value::as_f64)) {
            (Some(r), Some(s)) => r.start == s,
            _ => false,
        })
    }

    /// Every process on this machine with what it is, for the observers of
    /// actor products: the table, and `of` to say what each is. The
    /// arguments of the new processes of Node, Bun and Hermes's Pythons are
    /// read in one ps and matched there; the rest are told by their program
    /// alone, and a Hermes.app, Grok Bot.app or Muse.app by its bundle id.
    /// None where XenoNet can't look.
    pub fn census(&self, io: &mut dyn Io) -> Result<Option<Arc<Table>>, String> {
        if !self.supported {
            return Ok(None);
        }
        let t = self.snapshot(io)?;
        // The vouched apps' bundles programs name, read a few new ones at a
        // time: a program's name is its own word, and could name
        // thousands.
        let mut bundles: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for p in t.values() {
            if let Some(b) = vouched_bundle(&p.program) {
                if seen.insert(b.clone()) {
                    bundles.push(b);
                }
            }
        }
        let new: Vec<String> = {
            let s = self.s();
            bundles.iter().filter(|b| !s.bundle_reads.contains_key(*b)).take(BUNDLES_AT_ONCE).cloned().collect()
        };
        let calls: Vec<(String, Call)> = {
            let mut s = self.s();
            new.iter().filter_map(|b| Self::begin_bundle(&mut s, b).map(|c| (b.clone(), c))).collect()
        };
        self.settle_all(io, calls);
        // And those under way elsewhere, once they're done.
        let waiting: Vec<(String, Call)> = {
            let s = self.s();
            bundles.iter().filter(|b| matches!(s.bundle_reads.get(*b), Some(BundleRead::Asked))).map(|b| (b.clone(), Self::plutil(b))).collect()
        };
        self.settle_all(io, waiting);
        let fresh: Vec<Row> = {
            let s = self.s();
            t.values().filter(|p| wants_args(&p.program) && !s.verdicts.contains_key(&key(p))).cloned().collect()
        };
        if !fresh.is_empty() {
            let list = pids_list(&fresh.iter().map(|p| p.pid).collect::<Vec<_>>());
            let text = io.exec(Call::new("ps", &["-o", "pid=,command=", "-p", &list])).unwrap_or_default();
            let lines = parse_commands(&text);
            let mut s = self.s();
            // A process that's gone before ps looked gets no verdict, so
            // it's asked about again if it turns out it wasn't.
            for p in &fresh {
                let Some(line) = lines.get(p.pid) else { continue };
                let agent = agent_of(&p.program, line);
                s.verdicts.insert(key(p), (agent, if agent.is_some() { None } else { product_of(&p.program, line) }));
            }
        }
        // The verdicts of processes that are gone go once there are many.
        let mut s = self.s();
        if s.verdicts.len() > 256.max(2 * t.len()) {
            let live: HashSet<String> = t.values().map(key).collect();
            s.verdicts.retain(|k, _| live.contains(k));
        }
        Ok(Some(t))
    }

    fn settle_all(&self, io: &mut dyn Io, calls: Vec<(String, Call)>) {
        if calls.is_empty() {
            return;
        }
        let (names, calls): (Vec<String>, Vec<Call>) = calls.into_iter().unzip();
        for (b, answer) in names.iter().zip(io.exec_all(calls)) {
            self.settle_bundle(b, answer);
        }
    }

    /// What the census says a process is: its agent, or the product it's
    /// part of, from the verdicts and bundle ids read so far. `pid`,
    /// `start` and `program` as the row has them.
    pub fn of(&self, pid: Option<&Value>, start: Option<&Value>, program: Option<&Value>) -> Verdict {
        let k = format!("{}:{}:{}", template(pid), template(start), template(program));
        let program = match program {
            None | Some(Value::Null) => String::new(),
            Some(v) => js_string(v),
        };
        let s = self.s();
        if let Some((agent, product)) = s.verdicts.get(&k) {
            return (*agent, Self::settled(&s, *product, &program));
        }
        let agent = agent_of(&program, "");
        (agent, if agent.is_some() { None } else { Self::settled(&s, product_of(&program, ""), &program) })
    }

    /// The children of these processes, as pids. pgrep costs a few
    /// milliseconds of CPU where ps costs tens, so an observer can look at
    /// what an app runs about once a second.
    pub fn children(&self, io: &mut dyn Io, pids: &[Value]) -> Vec<f64> {
        let list = listed(pids, 1.0);
        if !self.supported || list.is_empty() {
            return Vec::new();
        }
        // pgrep says 1 when it finds none.
        let text = io.exec(Call::new("pgrep", &["-P", &pids_list(&list)])).unwrap_or_default();
        text.split('\n').map(js_num).filter(|n| n.is_finite() && n.trunc() == *n && *n > 0.0).collect()
    }

    /// These processes, as the table has them, without reading the whole
    /// table; the ones that are gone are left out.
    pub fn describe(&self, io: &mut dyn Io, pids: &[Value]) -> Table {
        let list = listed(pids, 0.0);
        if !self.supported || list.is_empty() {
            return PidMap::default();
        }
        parse_ps(&io.exec(Call::new("ps", &["-o", "pid=,ppid=,lstart=,comm=", "-p", &pids_list(&list)])).unwrap_or_default())
    }

    /// The folders these processes work in, by pid, in one lsof.
    pub fn cwds(&self, io: &mut dyn Io, pids: &[Value]) -> PidMap<String> {
        let list = listed(pids, 0.0);
        if !self.supported || list.is_empty() {
            return PidMap::default();
        }
        parse_cwds(&io.exec(Call::new("lsof", &["-a", "-p", &pids_list(&list), "-d", "cwd", "-Fpn"])).unwrap_or_default())
    }

    /// Whether a process from a chain runs its agent app's own code, or its
    /// actor app's (an app process of Grok Bot or Muse), signed by the
    /// app's maker. codesign asks about the running process, not a file, so
    /// another program named claude fails, and so does an agent built from
    /// source or run by Node, or a bundle that only calls itself Grok
    /// Bot.app. `pid`, `start` and `program` as the process has them.
    pub fn signed(&self, io: &mut dyn Io, pid: Option<&Value>, start: Option<&Value>, program: Option<&Value>, agent: Option<&str>, product: Option<(&str, &str)>) -> bool {
        let team = agent.and_then(team_of_agent).or_else(|| product.filter(|(_, role)| *role == "app").and_then(|(p, _)| team_of_product(p)));
        let Some(team) = team.filter(|_| self.supported) else {
            return false;
        };
        let k = format!("{}:{}:{}:{team}", template(pid), template(start), template(program));
        {
            let mut s = self.s();
            match s.signatures.get(&k) {
                Some(Some(v)) => return *v,
                Some(None) => {}
                None => {
                    if s.signatures.len() > 500 {
                        s.signatures.clear();
                    }
                    s.signatures.insert(k.clone(), None);
                }
            }
        }
        let requirement = format!("-R=anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
        let ok = io.exec(Call { file: "codesign".into(), args: vec!["--verify".into(), requirement, template(pid)], share: Some(format!("codesign {k}")) }).is_ok();
        if let Some(slot) = self.s().signatures.get_mut(&k) {
            *slot = Some(ok);
        }
        ok
    }
}

/// What the doctor's self-test found: whether a connection this process
/// made to itself came back as this process, with the executable it runs.
pub fn self_test(me: f64, peer: Option<f64>, first: Option<(f64, Option<&Value>)>) -> (bool, bool, bool) {
    let chain = first.is_some_and(|(pid, _)| pid == me);
    let exe = first.is_some_and(|(_, exe)| truthy(exe));
    (peer == Some(me), chain, exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made-up Linux or Windows machine: what each call answers, by its
    /// file and arguments, and every call asked.
    struct Machine {
        answers: HashMap<String, String>,
        asked: Vec<String>,
    }

    impl Io for Machine {
        fn exec(&mut self, call: Call) -> Result<String, Failure> {
            let asked = std::iter::once(call.file.clone()).chain(call.args).collect::<Vec<_>>().join(" ");
            self.asked.push(asked.clone());
            self.answers.get(&asked).cloned().ok_or(Failure { message: format!("no {asked} here"), stdout: None })
        }
        fn now(&mut self) -> f64 {
            1e12
        }
    }

    fn machine(answers: &[(&str, &str)]) -> Machine {
        Machine { answers: answers.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(), asked: Vec::new() }
    }

    #[test]
    fn on_windows_and_linux_the_processes_are_read_natively_and_a_reused_parent_pid_isnt_a_parent() {
        // Windows: Claude Code's claude.exe runs cmd, which runs the xenonet
        // command; Node runs the npm Claude Code, known by its command line.
        // 30's parent pid 9 was a process that's gone, and 9 now belongs to
        // one that started later.
        let table = [
            "4\t0\t0\tSystem\t",
            "5\t4\t1000\tC:\\Windows\\explorer.exe\tC:\\Windows\\explorer.exe",
            "20\t5\t2000\tC:\\Users\\me\\.local\\bin\\claude.exe\tC:\\Users\\me\\.local\\bin\\claude.exe",
            "21\t20\t3000\tC:\\Windows\\System32\\cmd.exe\tC:\\Windows\\System32\\cmd.exe",
            "22\t21\t4000\tC:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe\tC:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe",
            "23\t5\t2500\tC:\\Program Files\\nodejs\\node.exe\tC:\\Program Files\\nodejs\\node.exe",
            "9\t5\t9000\tC:\\Windows\\notepad.exe\tC:\\Windows\\notepad.exe",
            "30\t9\t5000\tC:\\Windows\\System32\\cmd.exe\t",
            "31\t1\t5000\tC:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\claude.exe\tC:\\Users\\me\\AppData\\Local\\AnthropicClaude\\app-1.0.0\\claude.exe",
        ]
        .join("\r\n");
        let netstat = "\r\nActive Connections\r\n\r\n  Proto  Local Address          Foreign Address        State           PID\r\n  TCP    127.0.0.1:47840        0.0.0.0:0              LISTENING       700\r\n  TCP    127.0.0.1:47840        127.0.0.1:51000        HERGESTELLT     700\r\n  TCP    127.0.0.1:51000        127.0.0.1:47840        HERGESTELLT     22\r\n  TCP    127.0.0.1:51001        127.0.0.1:47840        TIME_WAIT       0\r\n";
        let mut m = machine(&[
            (PROCESSES, &table),
            ("netstat -ano -p TCP", netstat),
            (&format!("{COMMAND_LINE} 23"), "\"C:\\Program Files\\nodejs\\node.exe\" C:\\Users\\me\\AppData\\Roaming\\npm\\node_modules\\@anthropic-ai\\claude-code\\cli.js"),
        ]);
        let p = Procs::new("win32");
        assert!(p.chains && !p.supported);
        let (pid, keep) = p.peer(&mut m, Some(&Value::from(47840)), Some(&Value::from(51000)), None);
        assert_eq!((pid, keep), (Value::from(22), true), "the client's end, whatever the state is called in this language");
        assert_eq!(p.peer(&mut m, Some(&Value::from(47840)), Some(&Value::from(51001)), None).0, Value::Null, "a closed pair's pid is 0");
        let chain = p.chain(&mut m, Some(22.0), true, true).unwrap();
        assert_eq!(chain.iter().map(|l| l.row.pid).collect::<Vec<_>>(), [22.0, 21.0, 20.0, 5.0, 4.0]);
        assert_eq!(chain.iter().map(|l| l.agent).collect::<Vec<_>>(), [None, None, Some("claude"), None, None]);
        assert_eq!(chain[0].exe.clone().flatten().as_deref(), Some("C:\\Users\\me\\AppData\\Local\\XenoNet\\runtime\\launcher\\xenonet.exe"));
        assert_eq!(p.chain(&mut m, Some(23.0), true, false).unwrap()[0].agent, Some("claude"), "node.exe running Claude Code's cli.js");
        assert_eq!(p.chain(&mut m, Some(30.0), true, true).unwrap().len(), 1, "9 started after 30, so it isn't 30's parent");
        assert_eq!(p.chain(&mut m, Some(30.0), true, true).unwrap()[0].exe, Some(None), "an executable that couldn't be read");
        assert_eq!(p.chain(&mut m, Some(31.0), true, false).unwrap()[0].agent, None, "Claude desktop's own claude.exe isn't Claude Code");
        assert!(!m.asked.iter().any(|a| a.starts_with("ps ") || a.starts_with("lsof ")), "{:?}", m.asked);

        // Linux: the connection's process from /proc, and a process the
        // table didn't have yet, read again.
        let first = "1\t0\t100\t/sbin/init\t/usr/lib/systemd/systemd\n50\t1\t200\t/bin/bash\t/usr/bin/bash\n";
        let mut m = machine(&[(PROCESSES, first), (&format!("{TCP_PEER} 47840 51000"), "60\n")]);
        let p = Procs::new("linux");
        assert_eq!(p.peer(&mut m, Some(&Value::from(47840)), Some(&Value::from(51000)), None).0, Value::from(60));
        assert_eq!(p.chain(&mut m, Some(50.0), true, false).unwrap().len(), 1, "up to init, as up to launchd");
        m.answers.insert(PROCESSES.into(), format!("{first}60\t50\t300\tclaude\t/home/me/.local/share/claude/versions/2.1.0\n"));
        let chain = p.chain(&mut m, Some(60.0), true, true).unwrap();
        assert_eq!(chain.iter().map(|l| (l.row.pid, l.agent)).collect::<Vec<_>>(), [(60.0, Some("claude")), (50.0, None)]);
        assert_eq!(chain[0].exe.clone().flatten().as_deref(), Some("/home/me/.local/share/claude/versions/2.1.0"));
        // Anywhere else, nothing is looked at.
        let mut m = machine(&[]);
        let p = Procs::new("freebsd");
        assert!(!p.chains && !p.supported);
        assert_eq!(p.peer(&mut m, Some(&Value::from(47840)), Some(&Value::from(51000)), None).0, Value::Null);
        assert_eq!(m.asked, Vec::<String>::new());
    }

    #[test]
    fn what_proc_net_tcp_and_a_table_line_say() {
        let tcp = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:BAB0 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0000000000000000 100 0 0 10 0\n   1: 0100007F:C738 0100007F:BAB0 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000000000000000 20 4 30 10 -1\n   2: 0100007F:C739 0100007F:BAB0 06 00000000:00000000 03:00000000 00000000     0        0 0 3 0000000000000000\n";
        assert_eq!(parse_proc_net_tcp(tcp, 47792, 51000).as_deref(), Some("22222"));
        assert_eq!(parse_proc_net_tcp(tcp, 47792, 51001), None, "a socket that's closing has no inode");
        let (t, exes) = parse_table("7\t1\t1.5e12\tpython3 -c x\t\n8\tx\tnope\tsh\t/bin/sh\nnot a line\n");
        assert_eq!(t.get(7.0).map(|r| (r.ppid, r.start, r.program.as_str())), Some((1.0, 1.5e12, "python3 -c x")));
        assert_eq!(t.get(8.0).map(|r| (r.ppid, r.start)), Some((0.0, 0.0)));
        assert_eq!((exes.get(7.0), exes.get(8.0).map(String::as_str)), (None, Some("/bin/sh")));
    }

    #[test]
    fn agents_by_program() {
        assert_eq!(agent_of("/Users/me/.local/bin/claude", ""), Some("claude"));
        assert_eq!(agent_of("/Users/me/.local/share/claude/versions/2.1.288", ""), Some("claude"));
        assert_eq!(agent_of("/path/codex-aarch64-apple-darwin", ""), Some("codex"));
        assert_eq!(agent_of("/usr/local/bin/node", "node /x/@anthropic-ai/claude-code/cli.js"), Some("claude"));
        assert_eq!(agent_of("/tmp/claude-decoy/run", ""), None);
        assert_eq!(agent_of("/usr/bin/codexterm", ""), None);
        assert_eq!(agent_of("", ""), None);
    }

    #[test]
    fn products_by_path() {
        let py = "/Users/nova/.hermes/tools/python-3.14.7/bin/python3.14";
        assert_eq!(product_of(py, "-m tui_gateway.entry"), Some(Product { product: "hermes-agent", role: "host" }));
        assert_eq!(product_of(py, "-m pip install requests"), None);
        assert_eq!(product_of("/Applications/Grok Bot.app/Contents/MacOS/Grok Bot", ""), Some(Product { product: "grok-bot", role: "app" }));
        assert_eq!(product_of("/Applications/XenoNet.app/Contents/MacOS/XenoNet", ""), None);
        assert!(wants_args(py) && !wants_args("/opt/homebrew/bin/python3"));
    }

    #[test]
    fn ps_and_netstat() {
        let t = parse_ps("  510     1 Fri Oct  3 16:34:29 2026 /Users/me/claude.app/Contents/MacOS/claude\nx\n");
        assert_eq!(t.len(), 1);
        assert_eq!(t.get(510.0).unwrap().start, 1_791_045_269_000.0);
        assert_eq!(parse_ps("  702     1 Sun Foo  1 07:30:00 2026 /bin/sh\n").get(702.0).unwrap().start, 0.0);
        let row =
            "tcp4       0      0  127.0.0.1.51000        127.0.0.1.47842        ESTABLISHED    111   2970  406912  146988 node:4242            00102 00000104 0000000003bca9cb";
        assert_eq!(parse_netstat(row, "47842", "51000"), Some(4242.0));
        assert_eq!(parse_netstat(row, "47842", "52000"), None);
    }

    /// A made-up codesign: the pids whose running code it vouches for, and
    /// what it was asked.
    struct Codesign {
        good: Vec<&'static str>,
        asked: Vec<Vec<String>>,
    }

    impl Io for Codesign {
        fn exec(&mut self, call: Call) -> Result<String, Failure> {
            assert_eq!(call.file, "codesign");
            self.asked.push(call.args.clone());
            if self.good.contains(&call.args[2].as_str()) {
                Ok(String::new())
            } else {
                Err(Failure { message: "code object is not signed at all".into(), stdout: None })
            }
        }
        fn now(&mut self) -> f64 {
            0.0
        }
    }

    #[test]
    fn signed_is_the_running_code_as_its_apps_maker_signed_it() {
        let p = Procs::new("darwin");
        let mut io = Codesign { good: vec!["10", "30"], asked: vec![] };
        let n = |x: f64| Value::from(x);
        let program = Value::String("/Users/me/.local/bin/claude".into());
        let signed = |io: &mut Codesign, pid: f64, start: f64, agent: Option<&str>, product: Option<(&str, &str)>| {
            p.signed(io, Some(&n(pid)), Some(&n(start)), Some(&program), agent, product)
        };
        // Each app by its maker's team: Anthropic, OpenAI, Cursor, Meta.
        assert!(signed(&mut io, 10.0, 1.0, Some("claude"), None));
        assert!(!signed(&mut io, 20.0, 1.0, Some("codex"), None), "codesign says it isn't");
        assert!(signed(&mut io, 30.0, 1.0, None, Some(("grok-bot", "app"))));
        assert!(!signed(&mut io, 40.0, 1.0, None, Some(("muse", "app"))));
        let teams: Vec<&str> = io.asked.iter().map(|a| a[1].as_str()).collect();
        assert_eq!(
            teams,
            [
                "-R=anchor apple generic and certificate leaf[subject.OU] = \"Q6L2SF6YDW\"",
                "-R=anchor apple generic and certificate leaf[subject.OU] = \"2DC432GLL2\"",
                "-R=anchor apple generic and certificate leaf[subject.OU] = \"DCNK4UB866\"",
                "-R=anchor apple generic and certificate leaf[subject.OU] = \"V9WTTPBFK9\"",
            ]
        );
        // Nothing to ask about: a product's helper rather than its app, an
        // ad hoc signed one, a program that's no agent's.
        let before = io.asked.len();
        assert!(!signed(&mut io, 30.0, 1.0, None, Some(("grok-bot", "host"))));
        assert!(!signed(&mut io, 30.0, 1.0, None, Some(("hermes-agent", "host"))));
        assert!(!signed(&mut io, 10.0, 1.0, Some("cursor"), None));
        assert!(!signed(&mut io, 10.0, 1.0, None, None));
        assert_eq!(io.asked.len(), before);
        // A verdict is kept for the process it was about, not its pid alone.
        assert!(signed(&mut io, 10.0, 1.0, Some("claude"), None));
        assert_eq!(io.asked.len(), before, "asked once");
        io.good.clear();
        assert!(!signed(&mut io, 10.0, 2.0, Some("claude"), None), "the pid, reused by another process");
        // And nothing at all where XenoNet can't look.
        let elsewhere = Procs::new("linux");
        assert!(!elsewhere.signed(&mut io, Some(&n(10.0)), Some(&n(1.0)), Some(&program), Some("claude"), None));
    }
}
