//! The doctor's checks. Each gives { status, detail, fix? }, where status
//! is ok, info (a fact, not a problem), warn, fail or skip. A check is a
//! task: what it reads of the machine it asks for through `ctx`, and the
//! host answers from the machine it runs on. XenoNet's own checks are
//! here; each app's come from its adapter (`xenonet_adapter::platforms`),
//! and take their place in the list by slot.

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::fmt::Write as _;
use xenonet_adapter::check::{self, CheckRow, Slot, Status, Stop, Verdict};
use xenonet_adapter::Check;
use xenonet_os::error::Failure;
use xenonet_os::nodepath::{Style, NATIVE};
use xenonet_text::text::display;
use xenonet_text::text::truthy;
use xenonet_text::units::{duration, plural};

pub const STATUSES: [&str; 5] = ["ok", "info", "warn", "fail", "skip"];
const MIN_NODE: (f64, f64) = (22.0, 13.0);
const BLOCK_GRACE_MS: f64 = 60e3;

/// A check's place in the list: id, group and title.
pub type Row = (&'static str, &'static str, &'static str);

/// XenoNet's own checks, before the apps' own.
const XENONET: [Row; 5] = [
    ("xenonet.server", "XenoNet", "XenoNet is running"),
    ("xenonet.sources", "XenoNet", "XenoNet can read what it shows"),
    ("xenonet.blocks", "XenoNet", "XenoNet's parts are working"),
    ("xenonet.state", "XenoNet", "XenoNet can save its state"),
    ("contracts.drift", "XenoNet", "The apps' data is the shape XenoNet expects"),
];

/// This computer's, after the apps' own and before the sessions'.
const MACHINE: [Row; 6] = [
    ("system.projects", "This computer", "XenoNet can read your projects folder"),
    ("system.git", "This computer", "git is available"),
    ("system.command", "This computer", "The xenonet command is installed"),
    ("system.runtime", "This computer", "The runtime is new enough"),
    ("xenonet.errors", "XenoNet", "No errors in XenoNet's log lately"),
    ("xenonet.launcher", "XenoNet", "Hooks and MCP servers reach XenoNet through its launcher"),
];

/// The sessions' and the actors', after the apps' sessions checks; the
/// adapters' actors checks (a product's plugin) come after the observers'.
const ACTORS: [Row; 6] = [
    ("agents.checked", "Sessions", "Sessions can't pass for each other"),
    ("actors.repos", "Actors", "No actor's token is in a repo"),
    ("actors.expired", "Actors", "Registered actors' tokens are current"),
    ("actors.gateway", "Actors", "The actor gateway reaches the internet"),
    ("actors.wakes", "Actors", "Actors with a webhook can be woken"),
    ("actors.observers", "Actors", "XenoNet can read what its observers look at"),
];
const ACTORS_CHECKED: [Row; 1] = [("actors.checked", "Actors", "Actors' tokens are held to their processes")];

/// Every check, in the order they run and show: XenoNet's own, the
/// installed adapters' app checks, this computer's, the adapters' sessions
/// checks, the sessions' and actors' (with the adapters' actors checks
/// after the observers'), the service started at login's (login.rs), then
/// the gateway's firewall's (firewall.rs).
pub fn all() -> Vec<Row> {
    all_with(&xenonet_adapter::platforms().check_rows())
}

/// The list with these adapter rows in their slots.
pub fn all_with(adapters: &[CheckRow]) -> Vec<Row> {
    let slot = |s: Slot| adapters.iter().filter(move |r| r.slot == s).map(|r| (r.id, r.group, r.title));
    XENONET
        .into_iter()
        .chain(slot(Slot::App))
        .chain(MACHINE)
        .chain(slot(Slot::Sessions))
        .chain(ACTORS)
        .chain(slot(Slot::Actors))
        .chain(ACTORS_CHECKED)
        .chain(crate::login::CHECKS)
        .chain(crate::firewall::CHECKS)
        .collect()
}

fn verdict(status: &str, detail: impl Into<String>, fix: Option<String>) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), json!(status));
    m.insert("detail".into(), json!(detail.into()));
    if let Some(f) = fix.filter(|f| !f.is_empty()) {
        m.insert("fix".into(), json!(f));
    }
    Value::Object(m)
}
pub(crate) fn ok(d: impl Into<String>) -> Value {
    verdict("ok", d, None)
}
pub(crate) fn info(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("info", d, fix.map(String::from))
}
pub(crate) fn warn(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("warn", d, fix.map(String::from))
}
pub(crate) fn fail(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("fail", d, fix.map(String::from))
}
pub(crate) fn skip(d: impl Into<String>) -> Value {
    verdict("skip", d, None)
}
fn value(v: Verdict) -> Value {
    let status = match v.status {
        Status::Ok => "ok",
        Status::Info => "info",
        Status::Warn => "warn",
        Status::Fail => "fail",
        Status::Skip => "skip",
    };
    verdict(status, v.detail, v.fix)
}

/// What the checks read as data, gathered before they run: XenoNet's own state when it's
/// running, the folders, and the runtime.
pub struct Facts {
    pub data: Value,
}

impl Facts {
    fn get(&self, k: &str) -> Option<&Value> {
        self.data.get(k).filter(|v| !v.is_null())
    }
    fn home(&self) -> String {
        self.get("home").map(display).unwrap_or_default()
    }
    pub(crate) fn platform(&self) -> String {
        self.get("platform").map(display).unwrap_or_default()
    }
    fn path(&self, k: &str) -> Option<String> {
        self.get("paths").and_then(|p| p.get(k)).filter(|v| truthy(Some(v))).map(display)
    }
    // ctx.actors: "off" (null), "none" (missing), or "on".
    fn actors(&self) -> &str {
        self.data.get("actors").and_then(Value::as_str).unwrap_or("none")
    }
}

/// What the doctor's checks read of the machine they run on, and of the
/// XenoNet running there. The lookups that end in a default are what only
/// a running XenoNet has (its actors, its network, its process checks): a
/// host without one answers None, and the check says what it needs.
#[async_trait(?Send)]
pub trait DoctorHost {
    /// The time now, in ms.
    async fn now(&self) -> f64;
    /// A time in ms, written in the machine's local time.
    async fn local_time(&self, ms: f64) -> String;
    /// A command on PATH.
    async fn which(&self, cmd: &str) -> Option<String>;
    async fn exists(&self, path: &str) -> bool;
    /// A program run with these arguments, for at most `ms`: `{ code,
    /// stdout, stderr, timedOut }`, whatever it came to.
    async fn run(&self, file: &str, args: &[&str], ms: f64) -> Value;
    /// A small file's text, or None when it can't be read.
    async fn read_text(&self, path: &str) -> Option<String>;
    /// A folder's entries, in order.
    async fn readdir(&self, path: &str) -> Result<Vec<String>, Failure>;
    /// Whether a folder can be written to.
    async fn writable(&self, dir: &str) -> bool;
    /// The file system's numbers where a folder is (`bavail`, `bsize`…),
    /// or None when they can't be read.
    async fn statfs(&self, dir: &str) -> Option<Value>;
    /// state.json in a folder: its version, whether it fits its schema,
    /// and the files moved aside.
    async fn state_file(&self, dir: &str) -> Value;
    /// Why the store in a folder can't be used (data from an earlier
    /// build, or another version's store), or None when it can.
    async fn store(&self, _dir: &str) -> Option<String> {
        None
    }
    /// The xenonet command on PATH: its file, whether it's installed and
    /// ours, what it runs, and whether that's gone.
    async fn command(&self) -> Value;
    /// The folders of the user's saved Path on Windows, or null elsewhere.
    async fn saved_path(&self) -> Value;
    /// The process lookups' self-test: whether a request's process, the
    /// processes above it and its program can be seen.
    async fn procs(&self) -> Value;
    /// Whether git runs here.
    async fn git_available(&self) -> bool;
    /// The launcher hooks and MCP servers run, or null when there's none.
    async fn launcher(&self) -> Result<Value, Failure>;
    /// The last day's errors in XenoNet's log, or null with no state folder.
    async fn errors(&self) -> Value;
    /// The drift log's records, or null with no cache folder.
    async fn drift(&self) -> Value;
    /// How the service started at login stands.
    async fn login(&self) -> Option<Value> {
        None
    }
    /// This machine's standing in its network (null when it's in none).
    async fn network(&self) -> Option<Value> {
        None
    }
    /// The program the service runs from after a restart.
    async fn restart_exe(&self) -> Option<Value> {
        None
    }
    /// A path with its links followed (null when it can't be).
    async fn realpath(&self, _path: &str) -> Option<Value> {
        None
    }
    /// The process checks' counts since the service started.
    async fn verify(&self) -> Option<Value> {
        None
    }
    /// The actors whose tokens expired.
    async fn actors_expired(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    /// What the actors' observers look at, and what they found.
    async fn actors_observers(&self) -> Option<Value> {
        None
    }
    /// The files in repos that hold an actor's token, looked for afresh
    /// when `fresh`.
    async fn actors_token_files(&self, _fresh: bool) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    /// The repos to look in for actors' tokens.
    async fn actors_repos(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    /// The actor gateway's state.
    async fn actors_gateway(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    /// The actors' wakes.
    async fn actors_wakes(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    /// The token files in these repos, looked for here.
    async fn find_token_files(&self, _repos: &[Value]) -> Result<Value, Failure> {
        Ok(json!({ "checked": 0, "files": [] }))
    }
}

/// A check's view of the machine: its host, and the facts.
pub(crate) struct Ctx<'a> {
    pub(crate) host: &'a dyn DoctorHost,
    pub(crate) f: &'a Facts,
}

impl Ctx<'_> {
    pub(crate) async fn which(&self, cmd: &str) -> Option<String> {
        self.host.which(cmd).await
    }
    pub(crate) async fn exists(&self, p: &str) -> bool {
        self.host.exists(p).await
    }
    pub(crate) async fn run(&self, file: &str, args: &[&str], ms: f64) -> Value {
        self.host.run(file, args, ms).await
    }
    pub(crate) fn tilde(&self, p: &str) -> String {
        xenonet_os::paths::tilde(p, &self.f.home())
    }
    async fn now(&self) -> f64 {
        self.host.now().await
    }
}

/// The same machine, as an adapter's check asks it.
#[async_trait(?Send)]
impl check::CheckCtx for Ctx<'_> {
    fn facts(&self) -> &Value {
        &self.f.data
    }
    async fn which(&self, cmd: &str) -> Result<Option<String>, Stop> {
        Ok(Ctx::which(self, cmd).await)
    }
    async fn exists(&self, path: &str) -> Result<bool, Stop> {
        Ok(Ctx::exists(self, path).await)
    }
    async fn run(&self, file: &str, args: &[&str], ms: f64) -> Result<Value, Stop> {
        Ok(Ctx::run(self, file, args, ms).await)
    }
    async fn read_text(&self, path: &str) -> String {
        self.host.read_text(path).await.unwrap_or_default()
    }
    async fn readdir(&self, path: &str) -> Result<Option<Vec<String>>, Option<String>> {
        self.host.readdir(path).await.map(Some).map_err(|e| e.field("code").map(display))
    }
    async fn now(&self) -> Result<f64, Stop> {
        Ok(Ctx::now(self).await)
    }
    async fn local_time(&self, ms: &Value) -> Result<String, Stop> {
        Ok(self.host.local_time(ms.as_f64().unwrap_or(f64::NAN)).await)
    }
    fn tilde(&self, path: &str) -> String {
        Ctx::tilde(self, path)
    }
}

fn no_actors(f: &Facts) -> Option<Value> {
    match f.actors() {
        "off" => Some(skip("actors are off here (XENONET_ACTORS=off)")),
        "on" => None,
        _ => Some(skip("needs XenoNet running")),
    }
}

pub(crate) fn unreadable(tool: &str) -> String {
    format!("{tool} answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose.")
}

fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn failed_sources(health: Option<&Value>, absent: &[&str]) -> Vec<Value> {
    list(health.and_then(|h| h.get("sources")))
        .into_iter()
        .filter(|s| {
            let name = s.get("name").and_then(Value::as_str);
            !truthy(s.get("ok")) && !matches!(name, Some("wiki" | "identities")) && !name.is_some_and(|n| absent.contains(&n))
        })
        .collect()
}

/// The map's sources for apps that aren't installed, by app id: what
/// they can't read isn't a failure. Only an app whose source runs a
/// program of the app's own (its adapter's `presence`) can be absent.
async fn absent_sources(c: &Ctx<'_>) -> Vec<&'static str> {
    let mut absent = Vec::new();
    for p in xenonet_adapter::platforms().iter() {
        let Some(presence) = p.presence() else { continue };
        // The doctor's machine answers every ask, so there's no error to
        // pass on.
        if !presence.installed(c).await.unwrap_or(false) {
            absent.push(p.id());
        }
    }
    absent
}

/// The names of these apps, as the registry has them.
fn names(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| xenonet_adapter::apps::app(id).map_or_else(|| (*id).to_string(), |a| a.name.clone())).collect()
}

fn n(v: Option<&Value>) -> f64 {
    xenonet_text::text::to_number(v)
}

fn s(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), display)
}

/// One check's verdict: XenoNet's own here, an app's through its adapter.
pub async fn run(host: &dyn DoctorHost, id: String, facts: Facts) -> Result<Value, Failure> {
    run_with(host, id, facts, xenonet_adapter::platforms().checks()).await
}

/// One check's verdict, with these adapter checks to hand an app's id to.
pub async fn run_with(host: &dyn DoctorHost, id: String, facts: Facts, adapters: Vec<Box<dyn Check>>) -> Result<Value, Failure> {
    let c = Ctx { host, f: &facts };
    let f = &facts;
    match id.as_str() {
        "xenonet.server" => {
            if let Some(me) = f.get("self").filter(|v| truthy(Some(v))) {
                let name = me.get("name").filter(|v| !v.is_null()).map_or_else(|| "XenoNet".into(), display);
                return Ok(ok(format!("{name} on port {}", s(me.get("port")))));
            }
            let Some(base) = f.get("base").filter(|v| truthy(Some(v))) else {
                return Ok(warn("XenoNet isn't running, so the map, its alerts and the tools sessions use are off", Some("Open XenoNet, or run xenonet service start.")));
            };
            let name = f.get("baseName").map_or_else(|| "XenoNet".into(), display);
            let b = display(base);
            Ok(ok(format!("{name} on {}", b.strip_prefix("http://").unwrap_or(&b))))
        }
        "xenonet.sources" => {
            let Some(health) = f.get("health").filter(|v| truthy(Some(v))) else { return Ok(skip("needs XenoNet running")) };
            let absent = absent_sources(&c).await;
            let bad = failed_sources(Some(health), &absent);
            if !bad.is_empty() {
                let said: Vec<String> =
                    bad.iter().map(|x| format!("{}: {}", s(x.get("name")), x.get("error").filter(|v| !v.is_null()).map_or_else(|| "failed".into(), display))).collect();
                return Ok(fail(said.join("; "), Some("Run xenonet doctor again after fixing the checks below; if it stays, xenonet logs --errors says more.")));
            }
            let mut counts: Map<String, Value> = Map::new();
            for x in list(health.get("sources")).into_iter().filter(|x| truthy(x.get("ok")) && !x.get("name").and_then(Value::as_str).is_some_and(|n| absent.contains(&n))) {
                counts.insert(s(x.get("name")), x.get("count").cloned().unwrap_or(Value::Null));
            }
            let projects = counts.get("projects").filter(|v| !v.is_null()).map(|v| plural(n(Some(v)), "project", None));
            let apps = xenonet_adapter::platforms().iter().filter_map(|p| counts.get(p.id()).and_then(|v| p.counted_words(v)));
            let parts: Vec<String> = projects.into_iter().chain(apps).collect();
            let missing: String = names(&absent).iter().fold(String::new(), |mut out, name| {
                let _ = write!(out, "; {name} isn't installed");
                out
            });
            Ok(ok(format!("reading {}{missing}", parts.join(", "))))
        }
        "xenonet.blocks" => {
            let Some(health) = f.get("health").filter(|v| truthy(Some(v))) else { return Ok(skip("needs XenoNet running")) };
            let Some(blocks) = health.get("blocks").filter(|v| truthy(Some(v))) else { return Ok(skip("this XenoNet doesn't report its parts")) };
            let blocks = list(Some(blocks));
            // One that reads for an app that isn't installed has nothing to
            // read.
            let absent = absent_sources(&c).await;
            let gone = |b: &Value| b.get("source").and_then(Value::as_str).is_some_and(|src| absent.contains(&src));
            let reported: Vec<Value> = failed_sources(Some(health), &absent).iter().map(|x| x.get("name").cloned().unwrap_or(Value::Null)).collect();
            let has = |b: &Value| b.get("source").is_some_and(|src| reported.iter().any(|r| xenonet_text::value::same(Some(r), Some(src))));
            let wrong: Vec<&Value> = blocks.iter().filter(|b| matches!(b.get("state").and_then(Value::as_str), Some("failed" | "degraded"))).collect();
            let mut bad: Vec<&Value> = wrong.iter().copied().filter(|b| !has(b) && !gone(b)).collect();
            bad.sort_by_key(|b| u8::from(b.get("state").and_then(Value::as_str) != Some("failed")));
            let mut left: Vec<String> = Vec::new();
            for b in wrong.iter().filter(|b| has(b)) {
                let src = s(b.get("source"));
                if !left.contains(&src) {
                    left.push(src);
                }
            }
            if bad.is_empty() {
                let mut besides: Vec<String> = Vec::new();
                if !left.is_empty() {
                    besides.push(format!("those reading for {}, which XenoNet can't read", left.join(" and ")));
                }
                let missing: Vec<&str> = absent.iter().copied().filter(|id| wrong.iter().any(|b| gone(b) && b.get("source").and_then(Value::as_str) == Some(*id))).collect();
                if !missing.is_empty() {
                    let which = if missing.len() == 1 { "isn't" } else { "aren't" };
                    besides.push(format!("those reading for {}, which {which} installed", names(&missing).join(" and ")));
                }
                let besides = if besides.is_empty() { String::new() } else { format!(" besides {}", besides.join(", and ")) };
                return Ok(ok(format!("{}, none with a problem{besides}", plural(blocks.len() as f64, "part", None))));
            }
            let say = |l: &[&Value]| -> String {
                l.iter()
                    .map(|b| {
                        let how = if b.get("state").and_then(Value::as_str) == Some("failed") { "isn't working" } else { "is partly working" };
                        let err = b.get("lastError").filter(|e| truthy(Some(e))).map_or(String::new(), |e| format!(" ({})", c.tilde(&s(e.get("message")))));
                        format!("{} {how}{err}", s(b.get("name")))
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            let now = c.now().await;
            let lasting: Vec<&Value> = bad.iter().copied().filter(|b| now - n(b.get("since")) >= BLOCK_GRACE_MS).collect();
            if lasting.is_empty() {
                return Ok(info(format!("{}, a moment ago; XenoNet tries again by itself", say(&bad)), None));
            }
            Ok(warn(say(&lasting), Some("Most of these clear up once what they read is back, and xenonet doctor's other checks say what's missing. If one stays, restart XenoNet; xenonet logs --errors says more.")))
        }
        "xenonet.state" => state(&c).await,
        "contracts.drift" => {
            let d = c.host.drift().await;
            let Some(d) = Some(d).filter(|d| truthy(Some(d))) else { return Ok(skip("no cache folder configured")) };
            let records = list(d.get("records"));
            let looked = d.get("lookedAt").filter(|v| !v.is_null()).map_or(f64::NEG_INFINITY, |v| n(Some(v)));
            let fresh: Vec<&Value> = records.iter().filter(|r| n(r.get("firstSeen")) > looked).collect();
            if fresh.is_empty() {
                return Ok(ok(if records.is_empty() {
                    "no differences from XenoNet's schemas".to_string()
                } else {
                    format!("nothing new since you last looked ({} known)", plural(records.len() as f64, "difference", None))
                }));
            }
            let addition = |r: &Value| r.get("kind").and_then(Value::as_str).is_some_and(|k| xenonet_schema::drift::ADDITIONS.contains(&k));
            let changes: Vec<&Value> = fresh.iter().copied().filter(|r| !addition(r)).collect();
            let shown: Vec<&Value> = changes.iter().copied().chain(fresh.iter().copied().filter(|r| addition(r))).collect();
            let named: Vec<String> =
                shown.iter().take(2).map(|r| format!("{}: {}", xenonet_schema::drift::source_name(&s(r.get("source"))), xenonet_schema::drift::describe(r))).collect();
            let more = if shown.len() > 2 { format!("; and {} more", shown.len() - 2) } else { String::new() };
            let say = format!("{}{more}", named.join("; "));
            let file = d.get("file").filter(|v| !v.is_null()).map_or_else(|| xenonet_schema::drift::DRIFT_FILE.to_string(), display);
            let fix = format!("xenonet drift lists them by source and marks them seen. XenoNet keeps them in {}.", c.tilde(&file));
            if !changes.is_empty() {
                return Ok(warn(format!("{} to what XenoNet reads since you last looked: {say}", plural(changes.len() as f64, "change", None)), Some(&fix)));
            }
            Ok(info(format!("the apps' data has {} since you last looked: {say}", plural(fresh.len() as f64, "addition", None)), Some(&fix)))
        }
        "system.projects" => {
            let dir = f.path("projectsDir");
            let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
            Ok(match check::folder(&c, dir.as_deref()).await? {
                check::Folder::Missing => fail(
                    format!("{shown} doesn't exist"),
                    Some(if truthy(f.data.get("projectsDirSet")) {
                        "Make it, or set XENONET_PROJECTS_DIR to the folder that holds your projects."
                    } else {
                        "Make it, or choose the folder that holds your projects: Change the projects folder… in XenoNet's palette, or xenonet root <folder>."
                    }),
                ),
                check::Folder::Denied(code) => fail(format!("can't read {shown} ({code})"), Some(&check::privacy_fix(&c, &shown))),
                check::Folder::Ok(count) => ok(format!("{shown}, {} {}", xenonet_text::units::locale(count as f64, 0, 3), if count == 1 { "entry" } else { "entries" })),
            })
        }
        "system.git" => {
            if !c.host.git_available().await {
                return Ok(warn(
                    "git isn't installed, so repo status (uncommitted work, branches) is off",
                    Some(if f.platform() == "darwin" {
                        "Run xcode-select --install, or install git with Homebrew, then restart XenoNet."
                    } else {
                        "Install git, then restart XenoNet."
                    }),
                ));
            }
            Ok(match c.which("git").await {
                Some(b) => ok(c.tilde(&b)),
                None => warn("git isn't on your PATH", None),
            })
        }
        "system.command" => command(&c).await,
        "system.runtime" => {
            if let Some(e) = f.get("electron").filter(|v| truthy(Some(v))) {
                return Ok(ok(format!("Electron {} (Node {})", display(e), s(f.get("node")))));
            }
            let node = s(f.get("node"));
            let mut parts = node.split('.').map(|p| xenonet_text::text::to_number(Some(&json!(p))));
            let (major, minor) = (parts.next().unwrap_or(f64::NAN), parts.next().unwrap_or(f64::NAN));
            if major < MIN_NODE.0 || (major == MIN_NODE.0 && minor < MIN_NODE.1) {
                return Ok(fail(format!("Node {node}; XenoNet needs 22.13 or later"), Some("Update Node.js.")));
            }
            Ok(ok(format!("Node {node}")))
        }
        "xenonet.errors" => {
            let entries = c.host.errors().await;
            let Some(entries) = Some(entries).filter(|e| truthy(Some(e))) else { return Ok(skip("needs XenoNet's state folder")) };
            let kept: Vec<Value> = list(Some(&entries)).into_iter().filter(|e| !s(e.get("code")).starts_with("doctor.")).collect();
            let rows = xenonet_os::log::summarize(&kept);
            if rows.is_empty() {
                return Ok(ok("none in the last day"));
            }
            let total: f64 = rows.iter().map(|r| n(r.get("count"))).sum();
            let shown: Vec<String> =
                rows.iter().take(3).map(|r| format!("{}{}", s(r.get("code")), if n(r.get("count")) > 1.0 { format!(" ×{}", s(r.get("count"))) } else { String::new() })).collect();
            let others = if rows.len() > 3 { format!(", and {}", plural((rows.len() - 3) as f64, "other kind", None)) } else { String::new() };
            Ok(warn(
                format!("{} in the last day: {}{others}", plural(total, "error", None), shown.join(", ")),
                Some("xenonet logs --errors shows them, with a fix for each when there's one. xenonet diagnose --ask has a Codex or Claude Code session look at them with you."),
            ))
        }
        "xenonet.launcher" => {
            let l = c.host.launcher().await?;
            let Some(l) = Some(l).filter(|v| truthy(Some(v))) else { return Ok(skip("no launcher here: XenoNet runs from a checkout")) };
            if xenonet_text::value::less(l.get("version"), l.get("ships")) {
                return Ok(warn(format!("launcher {}; this XenoNet ships {}", s(l.get("version")), s(l.get("ships"))), Some("XenoNet replaces it the next time it starts.")));
            }
            let Some(said) = l.get("said").filter(|v| truthy(Some(v))) else {
                return Ok(fail(format!("the launcher in {} doesn't run", c.tilde(&s(l.get("dir")))), Some("Open XenoNet, which writes it again, or install XenoNet again.")));
            };
            if !truthy(said.get("tools")) {
                return Ok(fail(
                    "the launcher can't find XenoNet's Tools, so the xenonet command and MCP servers don't start (hooks still reach XenoNet)",
                    Some("Open XenoNet to repair them, or install XenoNet again."),
                ));
            }
            // The xenonet binary has no node of its own to name.
            let node = if truthy(said.get("node")) { format!(" on Node {}", s(said.get("node")).trim_start_matches('v')) } else { String::new() };
            Ok(ok(format!("launcher {}, running Tools {}{node}", s(l.get("version")), s(said.get("tools")))))
        }
        "agents.checked" => {
            if f.platform() != "darwin" {
                return Ok(info("XenoNet doesn't check on this system yet that a request in a session's name comes from that session's process, so it takes each session at its word; what's yours to do is still refused from under an agent", None));
            }
            let t = c.host.procs().await;
            if !truthy(t.get("peer")) {
                return Ok(warn("netstat doesn't say which process sends a request, so XenoNet takes each session at its word", Some(&unreadable("netstat"))));
            }
            if !truthy(t.get("chain")) {
                return Ok(warn(
                    "ps doesn't say which processes a request comes from, so XenoNet refuses every request in a session's name, and sessions can't use its tools",
                    Some(&unreadable("ps")),
                ));
            }
            if !c.exists("/usr/bin/codesign").await {
                return Ok(warn("codesign is missing, so Claude Code's own hooks can't settle which process holds a session", None));
            }
            Ok(ok("XenoNet checks each request in a session's name against the process that sent it"))
        }
        "actors.repos" => repos(&c).await,
        "actors.expired" => {
            if let Some(off) = no_actors(f) {
                return Ok(off);
            }
            let expired = list(c.host.actors_expired().await?.as_ref());
            if expired.is_empty() {
                return Ok(ok("no actor's token has expired"));
            }
            let names: Vec<String> = expired.iter().map(|a| format!("\"{}\" (actor:{})", s(a.get("name")), s(a.get("slug")))).collect();
            let said = if names.len() > 2 { format!("{} and {} more", names[..2].join(", "), names.len() - 2) } else { names.join(" and ") };
            let one = expired.len() == 1;
            Ok(warn(
                format!(
                    "{said} {} expired after 30 days without a call, so {} offline",
                    if one { "has a token that" } else { "have tokens that" },
                    if one { "it stays" } else { "they stay" }
                ),
                Some("Give each a new token with New token in XenoNet's Actors panel, or remove the ones you don't use there or with xenonet actor remove <slug>."),
            ))
        }
        "actors.gateway" => gateway(&c).await,
        "actors.wakes" => {
            if let Some(off) = no_actors(f) {
                return Ok(off);
            }
            let wakes = c.host.actors_wakes().await?;
            let Some(wakes) = wakes.filter(|v| truthy(Some(v))) else { return Ok(skip("waking actors isn't in this XenoNet")) };
            let hooks: Vec<Value> = list(Some(&wakes)).into_iter().filter(|w| w.get("kind").and_then(Value::as_str) == Some("webhook")).collect();
            if hooks.is_empty() {
                return Ok(ok("no actor is woken by a webhook"));
            }
            let failing: Vec<String> = hooks
                .iter()
                .filter(|w| w.get("last").is_some_and(|l| truthy(Some(l)) && !truthy(l.get("ok"))))
                .map(|w| {
                    format!("{}'s last wake failed: {}", s(w.get("actor")), w["last"].get("reason").filter(|v| !v.is_null()).map_or_else(|| "it didn't answer".into(), display))
                })
                .collect();
            if !failing.is_empty() {
                return Ok(warn(failing.join("; "), Some("In XenoNet's Actors panel, Wake… on the actor sets its webhook again; xenonet actor wake <slug> test tries it now.")));
            }
            Ok(ok(format!("{} woken by a webhook, the last wake of each answered", plural(hooks.len() as f64, "actor", None))))
        }
        "actors.observers" => observers(&c).await,
        "actors.checked" => {
            if f.actors() == "off" {
                return Ok(skip("actors are off here (XENONET_ACTORS=off)"));
            }
            if f.platform() != "darwin" {
                return Ok(info("XenoNet doesn't hold actors' tokens to their processes on this system yet, so an actor's token alone decides", None));
            }
            let t = c.host.procs().await;
            let blind = if !truthy(t.get("peer")) {
                Some(("netstat doesn't say which process sends a request", "netstat"))
            } else if !truthy(t.get("chain")) {
                Some(("ps doesn't say which processes a request comes from", "ps"))
            } else {
                None
            };
            if let Some((why, tool)) = blind {
                return Ok(warn(
                    format!("{why}, so XenoNet refuses every actor's token, and what only you or its window may do, such as allowing or removing actors"),
                    Some(&unreadable(tool)),
                ));
            }
            if t.get("exe") == Some(&json!(false)) {
                return Ok(warn(
                    "lsof doesn't say which program a process runs, so XenoNet refuses Hermes's actor, and what only its window may do, such as allowing actors",
                    Some(&unreadable("lsof")),
                ));
            }
            let observed = if f.actors() == "on" { c.host.actors_observers().await } else { None };
            let unknown: Vec<String> = list(observed.as_ref())
                .iter()
                .filter(|o| truthy(o.get("on")) && o.get("recognized").is_some_and(|r| truthy(Some(r)) && n(r.get("found")) < n(r.get("expected"))))
                .map(|o| {
                    format!(
                        "{} runs {} processes XenoNet doesn't know as {}'s, so its actor's token would be refused from under them",
                        s(o.get("name")),
                        list(o["recognized"].get("purposes")).iter().map(display).collect::<Vec<_>>().join(", "),
                        s(o.get("name"))
                    )
                })
                .collect();
            if !unknown.is_empty() {
                return Ok(warn(unknown.join("; "), Some("The product may have moved where it keeps its programs; update XenoNet, or report it.")));
            }
            let said = "an actor's token never works from under a session or an actor app's own process, and a product's only from under that product";
            let verify = c.host.verify().await;
            let unseen = verify.as_ref().and_then(|v| v.get("unseen")).filter(|v| !v.is_null()).map_or(0.0, |v| n(Some(v)));
            if unseen != 0.0 && !unseen.is_nan() {
                let them = if unseen == 1.0 { "it" } else { "them" };
                return Ok(info(format!("{said}; since XenoNet started, it refused {} because it couldn't see which program sent {them}", plural(unseen, "request", None)), Some("A program that sends a request and exits before the answer can't be seen; one that waits for it can. If this keeps growing, netstat or ps may be answering in a way XenoNet doesn't read: update XenoNet, or report it.")));
            }
            Ok(ok(said))
        }
        id if crate::login::CHECKS.iter().any(|(c, ..)| *c == id) => crate::login::run(&c, id).await,
        id if crate::firewall::CHECKS.iter().any(|(c, ..)| *c == id) => crate::firewall::run(&c, id).await,
        id => match adapters.iter().find(|k| k.row().id == id) {
            Some(k) => k.run(&c).await.map(value),
            None => Err(Failure::msg(format!("no such check: {id}"))),
        },
    }
}

async fn state(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    let Some(dir) = f.path("stateDir") else { return Ok(skip("no state folder configured")) };
    let shown = c.tilde(&dir);
    if !c.exists(&dir).await {
        return Ok(info(format!("{shown} doesn't exist yet; XenoNet makes it on first save"), None));
    }
    if !c.host.writable(&dir).await {
        return Ok(fail(format!("can't write to {shown}"), Some("Fix the folder's permissions, or set XENONET_STATE_DIR to a folder you own.")));
    }
    // A store this build doesn't read: nothing is migrated, so the machine
    // is set up again.
    if let Some(why) = c.host.store(&dir).await {
        return Ok(fail(c.tilde(&why), Some("Set this machine up again: quit XenoNet, move its state folder aside, start XenoNet, and join your network again.")));
    }
    // The file itself: one moved aside, one it can't read now, one from a
    // newer XenoNet, one it isn't saving to, or parts that don't fit.
    let file = Some(c.host.state_file(&dir).await).filter(|v| truthy(Some(v)));
    let loaded = f.get("stateLoad");
    let file_get = |k: &str| file.as_ref().and_then(|x| x.get(k)).filter(|v| !v.is_null());
    let broken = list(file_get("broken"));
    if let Some(last) = broken.last() {
        return Ok(warn(
            format!("{} is a state.json XenoNet couldn't read, so it moved it aside and started afresh", c.tilde(&display(last))),
            Some("To get back what was in it, fix it and put it back as state.json while XenoNet is closed; delete it once you don't need it."),
        ));
    }
    if let Some(u) = file_get("unreadable").filter(|v| truthy(Some(v))) {
        return Ok(warn(format!("state.json {}, so XenoNet will move it aside and start afresh", display(u)), Some("To keep what's in it, fix it before XenoNet next starts.")));
    }
    if file.is_some() && xenonet_text::value::less(file_get("known"), file_get("version")) && file_get("version").is_some() && file_get("known").is_some() {
        return Ok(warn(
            format!("state.json is from a newer XenoNet (version {}), so this one reads it but doesn't save changes to it", s(file_get("version"))),
            Some("Use the newer XenoNet, or update this one."),
        ));
    }
    if let Some(l) = loaded.filter(|l| truthy(l.get("readOnly"))) {
        return Ok(warn(format!("{}; changes last only until XenoNet quits", s(l.get("problem"))), Some("Fix the file or its folder, then restart XenoNet.")));
    }
    let issues = list(file_get("issues"));
    if let Some(first) = issues.first() {
        let path = c.tilde(&first.get("path").map(display).unwrap_or_default());
        let message = s(first.get("message"));
        return Ok(warn(
            format!(
                "state.json has {} XenoNet can't use, such as {}: {}",
                plural(issues.len() as f64, "part", None),
                if path.is_empty() { "the whole file".into() } else { path },
                message.strip_prefix("Invalid input: ").unwrap_or(&message)
            ),
            Some("Fix that part, or move state.json aside while XenoNet is closed to start afresh."),
        ));
    }
    if let Some(st) = c.host.statfs(&dir).await {
        let free = n(st.get("bavail")) * n(st.get("bsize"));
        if free < 1e9 {
            return Ok(warn(
                format!("{} GB free where XenoNet keeps its state", xenonet_text::units::to_fixed(free / 1e9, 1)),
                Some("Free some disk space; posters and replay clips need room too."),
            ));
        }
    }
    Ok(ok(shown))
}

async fn command(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    let st = c.host.command().await;
    let Some(file) = st.get("file").filter(|v| truthy(Some(v))).map(display) else { return Ok(skip("not on Windows yet")) };
    let place = c.tilde(&file);
    if !truthy(st.get("installed")) {
        return Ok(info(
            "not installed; once it is, xenonet doctor, xenonet agents and the rest work in a terminal",
            Some("Install it from the palette (Install the xenonet command), or run node cli/xenonet.js install in a checkout of XenoNet."),
        ));
    }
    if !truthy(st.get("ours")) {
        return Ok(info(format!("{place} is another program's"), None));
    }
    if truthy(st.get("broken")) {
        let runs: Vec<String> = list(st.get("runs")).iter().map(|r| c.tilde(&display(r))).collect();
        return Ok(warn(format!("{place} runs {}, which is gone", runs.join(" ")), Some("Install it again from the palette (Install the xenonet command).")));
    }
    let dir = c.tilde(&NATIVE.dirname(&file));
    let win = f.platform() == "win32";
    if !truthy(st.get("onPath")) && win {
        let saved = Some(c.host.saved_path().await);
        let target = Style::Win32.dirname(&file);
        if list(saved.as_ref()).iter().any(|d| xenonet_components::command::same_place(&display(d), &target, "win32")) {
            return Ok(ok(format!("{place}, on your Path for new terminals")));
        }
    }
    if !truthy(st.get("onPath")) {
        let fix = if win {
            format!("Run XenoNet's Setup again, which adds it, or add {dir} to your user Path in Windows' Environment Variables. Then open a new terminal.")
        } else {
            "Add it in your shell's profile: export PATH=\"$HOME/.local/bin:$PATH\"".to_string()
        };
        return Ok(warn(format!("{place} is there, but {dir} isn't on your PATH"), Some(&fix)));
    }
    Ok(ok(place))
}

async fn repos(c: &Ctx<'_>) -> Result<Value, Failure> {
    if let Some(off) = no_actors(c.f) {
        return Ok(off);
    }
    let started = c.now().await;
    let by_hand = c.f.data.get("byHand") == Some(&json!(true));
    let found = match c.host.actors_token_files(by_hand).await? {
        Some(v) => v,
        None => {
            let repos = list(c.host.actors_repos().await?.as_ref());
            let mut m = Map::new();
            m.insert("at".into(), Value::Null);
            m.insert("repos".into(), json!(repos.len()));
            let more = if repos.is_empty() { json!({ "checked": 0, "files": [] }) } else { c.host.find_token_files(&repos).await? };
            for (k, v) in more.as_object().into_iter().flatten() {
                m.insert(k.clone(), v.clone());
            }
            Value::Object(m)
        }
    };
    let at = found.get("at").filter(|v| !v.is_null()).map(|v| n(Some(v)));
    let when = match at {
        Some(a) if a < started => format!(" (XenoNet looked {} ago, as it does once a day and whenever you run the doctor)", duration(Some(started - a))),
        _ => String::new(),
    };
    let count = n(found.get("repos"));
    if count == 0.0 || count.is_nan() || !truthy(found.get("repos")) {
        return Ok(ok(format!("no repos to look in{when}")));
    }
    let files = list(found.get("files"));
    if !files.is_empty() {
        let shown: Vec<String> = files.iter().take(3).map(|x| c.tilde(&display(x))).collect();
        let more = if files.len() > 3 { format!(" and {} more", files.len() - 3) } else { String::new() };
        let one = files.len() == 1;
        // Only a token XenoNet issued counts. When it couldn't read its own
        // tokens, every token-shaped string did, a made-up one too.
        let unchecked = truthy(found.get("unchecked"));
        let what = if unchecked { "what looks like an actor's token" } else { "an actor's token" };
        let note = if unchecked { format!("; XenoNet couldn't check {} against the tokens it issued", if one { "it" } else { "them" }) } else { String::new() };
        return Ok(warn(
            format!("{} in a repo {} {what}: {}{more}{when}{note}", plural(files.len() as f64, "file", None), if one { "holds" } else { "hold" }, shown.join(", ")),
            Some("Move each token out of the repo, whether it's in a file of its own or a line of a .env. Keep it in a file only you can read, outside any repo (for example ~/.config/<name>/xenonet.token, named in XENONET_ACTOR_TOKEN_FILE). Give its actor a new token with New token in XenoNet's Actors panel, since a session may have read it. If it was committed, the old token has leaked."),
        ));
    }
    let checked = n(found.get("checked"));
    if checked < count {
        return Ok(info(
            format!("none in the {} looked in; the other {} didn't answer in time{when}", plural(checked, "repo", None), xenonet_text::json::number_string(count - checked)),
            None,
        ));
    }
    Ok(ok(format!("none in {}{when}", plural(count, "repo", None))))
}

async fn gateway(c: &Ctx<'_>) -> Result<Value, Failure> {
    if let Some(off) = no_actors(c.f) {
        return Ok(off);
    }
    let st = c.host.actors_gateway().await?;
    let Some(st) = st.filter(|v| truthy(Some(v))) else { return Ok(skip("the actor gateway isn't in this XenoNet")) };
    if !truthy(st.get("on")) {
        return Ok(ok("it's off, so nothing outside this computer reaches XenoNet through it"));
    }
    let fix = "In XenoNet's Actors panel, Repair sets the tunnel up again and Change… picks another; xenonet gateway check checks again from a terminal.";
    if let Some(p) = st.get("problem").filter(|v| truthy(Some(v))) {
        return Ok(warn(display(p), Some(fix)));
    }
    // Funnel publishes /mcp alone: any other path it puts on the internet
    // is a warning.
    let tunnel = st.get("tunnel").cloned().unwrap_or(Value::Null);
    let funnel = tunnel.get("kind").and_then(Value::as_str) == Some("tailscale");
    let public: Vec<String> = list(tunnel.get("public")).iter().map(display).collect();
    if funnel && public.iter().any(|p| p != "/mcp") {
        return Ok(warn(
            format!("Tailscale Funnel puts more than the gateway's /mcp on the internet: {}", public.join(", ")),
            Some("In XenoNet's Actors panel, Repair sets Funnel back to /mcp alone. A path another program serves on port 443 has to move first."),
        ));
    }
    if st.get("reach").and_then(|r| r.get("state")).and_then(Value::as_str) != Some("ok") {
        return Ok(info(
            format!("it's on, at {}; XenoNet hasn't checked that address yet", st.get("host").filter(|v| !v.is_null()).map_or_else(|| "no address yet".into(), display)),
            None,
        ));
    }
    let words = |r: &str| match r {
        "rotated" => "replaced",
        "removed" => "taken away",
        "expired" => "expired",
        _ => "refused",
    };
    let mut said: Vec<String> = Vec::new();
    for a in list(st.get("actors")) {
        let name = s(a.get("name"));
        if truthy(a.get("oldAddress")) {
            said.push(format!("{name} last called {}, an older address", s(a.get("lastHost"))));
        }
        if let Some(r) = a.get("refused").filter(|v| truthy(Some(v))) {
            said.push(format!("{name} calls with a gateway token that was {}", words(&s(r.get("reason")))));
        }
        if truthy(a.get("token")) && truthy(a.get("expiring")) {
            let t = n(a.get("expiresAt"));
            if !t.is_finite() {
                return Err(Failure::with("Invalid time value", Map::new()));
            }
            said.push(format!("{name}'s gateway token stops working on {} unless it calls before then", &xenonet_text::time::iso(t)[..10]));
        }
    }
    if !said.is_empty() {
        return Ok(warn(said.join("; "), Some("In XenoNet's Actors panel, Connect… on the actor gives its app the address and a token again.")));
    }
    let through = st.get("tunnel").and_then(|t| t.get("through")).filter(|v| truthy(Some(v))).map_or(String::new(), |t| format!(" through {}", display(t)));
    let local = if truthy(st["reach"].get("local")) { ", checked on this computer alone" } else { "" };
    // What's public: Funnel's /mcp alone, or a port tunnel's everything.
    let public = match tunnel.get("kind").and_then(Value::as_str) {
        Some("tailscale") if tunnel.get("public").is_some_and(Value::is_array) => ", with /mcp alone public",
        Some("cloudflare" | "cloudflare-quick" | "ngrok") => ", with the gateway's whole port public",
        _ => "",
    };
    Ok(ok(format!("reachable at {}{through}{public}{local}", s(st.get("url")))))
}

async fn observers(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    if let Some(off) = no_actors(f) {
        return Ok(off);
    }
    let all = list(c.host.actors_observers().await.as_ref());
    let names = |os: &[&Value]| -> String {
        match os.len() {
            0 => String::new(),
            1 => s(os[0].get("name")),
            k => format!("{} and {}", os[..k - 1].iter().map(|o| s(o.get("name"))).collect::<Vec<_>>().join(", "), s(os[k - 1].get("name"))),
        }
    };
    let every: Vec<&Value> = all.iter().collect();
    let installed: Vec<&Value> = all.iter().filter(|o| truthy(o.get("installed"))).collect();
    if installed.is_empty() {
        let named = names(&every);
        return Ok(info(
            format!(
                "{} {} on this computer, so there's nothing to observe",
                if named.is_empty() { "No product XenoNet observes".to_string() } else { named },
                if all.len() == 1 { "isn't" } else { "aren't" }
            ),
            None,
        ));
    }
    let on: Vec<&Value> = installed.iter().copied().filter(|o| truthy(o.get("on"))).collect();
    if on.is_empty() {
        let one = installed.len() == 1;
        return Ok(info(
            format!("the {} of {} {} off", if one { "observer" } else { "observers" }, names(&installed), if one { "is" } else { "are" }),
            Some("Turn one on in XenoNet's Actors panel, or with xenonet actor observe <product> on."),
        ));
    }
    let word = |state: &str| match state {
        "denied" => Some("can't be read"),
        "unreadable" => Some("isn't in a shape XenoNet reads"),
        "unsupported" => Some("can't be looked at on this system"),
        _ => None,
    };
    let mut bad: Vec<String> = Vec::new();
    for o in &on {
        for src in list(o.get("sources")) {
            if let Some(w) = word(&s(src.get("state"))) {
                let file = src.get("file").filter(|v| truthy(Some(v))).map_or(String::new(), |x| format!(" ({})", display(x)));
                bad.push(format!("{}'s {}{file} {w}", s(o.get("name")), s(src.get("name"))));
            }
        }
    }
    if !bad.is_empty() {
        let fix = if f.platform() == "darwin" {
            "A file that can't be read is usually macOS privacy or its permissions: check them, then restart XenoNet. One in another shape means the product changed it: update XenoNet, and npm run contracts in a checkout lists what changed."
        } else {
            "XenoNet observes these products on macOS for now."
        };
        return Ok(warn(bad.join("; "), Some(fix)));
    }
    Ok(ok(on
        .iter()
        .map(|o| {
            format!(
                "{}: {} read",
                s(o.get("name")),
                plural(list(o.get("sources")).iter().filter(|x| x.get("state").and_then(Value::as_str) == Some("ok")).count() as f64, "source", None)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{ready, Fake};

    fn row(id: &'static str, group: &'static str, slot: Slot) -> CheckRow {
        CheckRow { id, group, title: "", slot }
    }

    #[test]
    fn the_adapters_rows_take_their_slots_in_the_list() {
        let ids = |rows: &[CheckRow]| all_with(rows).into_iter().map(|(id, ..)| id).collect::<Vec<_>>();
        let own = ids(&[]);
        assert_eq!(&own[..6], &["xenonet.server", "xenonet.sources", "xenonet.blocks", "xenonet.state", "contracts.drift", "system.projects"]);
        assert_eq!(&own[11..13], &["agents.checked", "actors.repos"]);
        assert_eq!(own.last(), Some(&"network.firewall"));
        let with = ids(&[
            row("claude.cli", "Claude Code", Slot::App),
            row("agents.claude", "Sessions", Slot::Sessions),
            row("codex.cli", "Codex", Slot::App),
            row("agents.codex", "Sessions", Slot::Sessions),
        ]);
        assert_eq!(&with[4..8], &["contracts.drift", "claude.cli", "codex.cli", "system.projects"]);
        assert_eq!(&with[12..16], &["xenonet.launcher", "agents.claude", "agents.codex", "agents.checked"]);
        assert_eq!(with.len(), own.len() + 4);
    }

    /// A check an adapter might bring, asking the machine for all it can.
    struct Asks;

    #[async_trait(?Send)]
    impl Check for Asks {
        fn row(&self) -> CheckRow {
            row("ask.everything", "Ask", Slot::App)
        }
        async fn run(&self, c: &dyn check::CheckCtx) -> Result<Verdict, Stop> {
            if check::Facts(c.facts()).get("stop").is_some() {
                return Err(Stop::msg("stopped on purpose"));
            }
            let which = c.which("tool").await?;
            let here = c.exists("/x/here").await?;
            let ran = c.run("/x/tool", &["--v"], 100.0).await?;
            let dirs = (c.readdir("/x/dir").await, c.readdir("/x/locked").await);
            let text = c.read_text("/x/note").await;
            let when = c.local_time(&json!(1.0)).await?;
            Ok(Verdict::info(
                format!("{which:?} {here} {} {dirs:?} {text} {when} {} {}", ran["stdout"].as_str().unwrap_or(""), c.now().await?, c.tilde("/Users/ada/x")),
                Some("do nothing"),
            ))
        }
    }

    fn drive(facts: Value) -> Result<Value, Failure> {
        let host = Fake::default();
        ready(run_with(&host, "ask.everything".into(), Facts { data: facts }, vec![Box::new(Asks)]))
    }

    #[test]
    fn an_adapters_check_asks_the_machine_through_the_doctor() {
        let v = drive(json!({ "home": "/Users/ada" })).unwrap();
        assert_eq!(v["status"], json!("info"));
        assert_eq!(v["detail"], json!("Some(\"/x/tool\") true ran /x/tool --v (Ok(Some([\"a\", \"b\"])), Err(Some(\"EACCES\"))) hi at 1.0 42 ~/x"));
        assert_eq!(v["fix"], json!("do nothing"));
        assert_eq!(drive(json!({ "stop": true })), Err(Failure::msg("stopped on purpose")));
        let host = Fake::default();
        assert_eq!(ready(run_with(&host, "no.such".into(), Facts { data: json!({}) }, Vec::new())), Err(Failure::msg("no such check: no.such")));
    }
}
