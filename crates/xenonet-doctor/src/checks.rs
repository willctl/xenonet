//! core/doctor/index.js's checks. Each returns { status, detail, fix? },
//! where status is ok, info (a fact, not a problem), warn, fail or skip.
//! A check is a task: what it reads of the machine it asks for, as the JS
//! called `ctx` for it, and the twin answers from the same `ctx`.

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::{Style, NATIVE};
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;
use xenonet_base::units::{duration, plural};

pub const STATUSES: [&str; 5] = ["ok", "info", "warn", "fail", "skip"];
const MIN_NODE: (f64, f64) = (22.0, 13.0);
const BLOCK_GRACE_MS: f64 = 60e3;

/// The checks, in order: id, group and title.
pub const CHECKS: [(&str, &str, &str); 27] = [
    ("xenonet.server", "XenoNet", "XenoNet is running"),
    ("xenonet.sources", "XenoNet", "XenoNet can read what it shows"),
    ("xenonet.blocks", "XenoNet", "XenoNet's parts are working"),
    ("xenonet.state", "XenoNet", "XenoNet can save its state"),
    ("contracts.drift", "XenoNet", "The apps' data is the shape XenoNet expects"),
    ("claude.cli", "Claude Code", "Claude Code is installed"),
    ("claude.auth", "Claude Code", "Claude Code is signed in"),
    ("claude.transcripts", "Claude Code", "XenoNet can read Claude Code's sessions"),
    ("claude.usage", "Claude Code", "Claude plan usage is current"),
    ("codex.cli", "Codex", "Codex is installed"),
    ("codex.auth", "Codex", "Codex is signed in"),
    ("codex.sessions", "Codex", "XenoNet can read Codex's sessions"),
    ("system.projects", "This computer", "XenoNet can read your projects folder"),
    ("system.git", "This computer", "git is available"),
    ("system.command", "This computer", "The xenonet command is installed"),
    ("system.runtime", "This computer", "The runtime is new enough"),
    ("xenonet.errors", "XenoNet", "No errors in XenoNet's log lately"),
    ("xenonet.launcher", "XenoNet", "Hooks and MCP servers reach XenoNet through its launcher"),
    ("agents.claude", "Sessions", "Claude Code sessions are connected to XenoNet"),
    ("agents.codex", "Sessions", "Codex sessions are connected to XenoNet"),
    ("agents.checked", "Sessions", "Sessions can't pass for each other"),
    ("actors.repos", "Actors", "No actor's token is in a repo"),
    ("actors.expired", "Actors", "Registered actors' tokens are current"),
    ("actors.gateway", "Actors", "The actor gateway reaches the internet"),
    ("actors.wakes", "Actors", "Actors with a webhook can be woken"),
    ("actors.observers", "Actors", "XenoNet can read what its observers look at"),
    ("actors.checked", "Actors", "Actors' tokens are held to their processes"),
];

fn verdict(status: &str, detail: impl Into<String>, fix: Option<String>) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), json!(status));
    m.insert("detail".into(), json!(detail.into()));
    if let Some(f) = fix.filter(|f| !f.is_empty()) {
        m.insert("fix".into(), json!(f));
    }
    Value::Object(m)
}
fn ok(d: impl Into<String>) -> Value {
    verdict("ok", d, None)
}
fn info(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("info", d, fix.map(String::from))
}
fn warn(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("warn", d, fix.map(String::from))
}
fn fail(d: impl Into<String>, fix: Option<&str>) -> Value {
    verdict("fail", d, fix.map(String::from))
}
fn skip(d: impl Into<String>) -> Value {
    verdict("skip", d, None)
}

/// What the checks read of `ctx` as data: XenoNet's own state when it's
/// running, the folders, and the runtime.
pub struct Facts {
    pub data: Value,
}

impl Facts {
    fn get(&self, k: &str) -> Option<&Value> {
        self.data.get(k).filter(|v| !v.is_null())
    }
    fn home(&self) -> String {
        self.get("home").map(js_string).unwrap_or_default()
    }
    fn platform(&self) -> String {
        self.get("platform").map(js_string).unwrap_or_default()
    }
    fn path(&self, k: &str) -> Option<String> {
        self.get("paths").and_then(|p| p.get(k)).filter(|v| truthy(Some(v))).map(js_string)
    }
    // ctx.actors: "off" (null), "none" (missing), or "on".
    fn actors(&self) -> &str {
        self.data.get("actors").and_then(Value::as_str).unwrap_or("none")
    }
}

/// How a check asks the machine: ctx[fn](...args), or None when ctx has no
/// such function (as `?.()` reads it).
struct Ctx<'a> {
    io: &'a Io,
    f: &'a Facts,
}

impl Ctx<'_> {
    async fn call(&self, f: &str, args: Value) -> Result<Option<Value>, Value> {
        let r = self.io.ask(json!({ "op": "call", "fn": f, "args": args })).await?;
        Ok(if r.get("missing") == Some(&json!(true)) { None } else { Some(r.get("value").cloned().unwrap_or(Value::Null)) })
    }
    // A call whose throw ends the check, as the JS let it.
    async fn must(&self, f: &str, args: Value) -> Result<Value, Failure> {
        match self.call(f, args).await {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Err(Failure::type_error(format!("ctx.{f} is not a function"))),
            Err(_) => Err(Failure::Rethrow),
        }
    }
    async fn which(&self, cmd: &str) -> Result<Option<String>, Failure> {
        Ok(Some(self.must("which", json!([cmd])).await?).filter(|v| truthy(Some(v))).map(|v| js_string(&v)))
    }
    async fn exists(&self, p: &str) -> Result<bool, Failure> {
        Ok(truthy(Some(&self.must("exists", json!([p])).await?)))
    }
    async fn run(&self, file: &str, args: &[&str], ms: f64) -> Result<Value, Failure> {
        self.must("run", json!([file, args, ms])).await
    }
    fn tilde(&self, p: &str) -> String {
        xenonet_base::paths::tilde(p, &self.f.home())
    }
    async fn now(&self) -> Result<f64, Failure> {
        self.io.ask(json!({ "op": "now" })).await.map(|v| xenonet_base::text::js_number(Some(&v))).map_err(|_| Failure::Rethrow)
    }
}

enum Folder {
    Missing,
    Ok(usize),
    Denied(String),
}

async fn folder(c: &Ctx<'_>, dir: Option<&str>) -> Result<Folder, Failure> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else { return Ok(Folder::Missing) };
    if !c.exists(dir).await? {
        return Ok(Folder::Missing);
    }
    Ok(match c.call("readdir", json!([dir])).await {
        Ok(Some(v)) => Folder::Ok(v.as_array().map_or_else(|| xenonet_app::jsv::key_count(&v), Vec::len)),
        Ok(None) => Folder::Denied("undefined".into()),
        Err(e) => Folder::Denied(e.get("code").filter(|v| !v.is_null()).map_or_else(|| "undefined".into(), js_string)),
    })
}

fn no_actors(f: &Facts) -> Option<Value> {
    match f.actors() {
        "off" => Some(skip("actors are off here (XENONET_ACTORS=off)")),
        "on" => None,
        _ => Some(skip("needs XenoNet running")),
    }
}

fn unreadable(tool: &str) -> String {
    format!("{tool} answered in a way XenoNet doesn't read; update XenoNet, or report it with xenonet diagnose.")
}

fn privacy_fix(f: &Facts, what: &str) -> String {
    if f.platform() == "darwin" {
        format!("Let XenoNet read {what}: System Settings › Privacy & Security › Files and Folders (or Full Disk Access), then restart XenoNet.")
    } else {
        format!("Check the permissions on {what}.")
    }
}

fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn failed_sources(health: Option<&Value>) -> Vec<Value> {
    list(health.and_then(|h| h.get("sources"))).into_iter().filter(|s| !truthy(s.get("ok")) && !matches!(s.get("name").and_then(Value::as_str), Some("wiki" | "identities"))).collect()
}

fn n(v: Option<&Value>) -> f64 {
    xenonet_base::text::js_number(v)
}

fn s(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

fn version_in(text: &str, re: &'static OnceLock<Regex>, pattern: &str) -> Option<String> {
    re.get_or_init(|| Regex::new(pattern).unwrap()).find(text).map(|m| m.as_str().to_string())
}

/// One check's verdict.
pub async fn run(io: Io, id: String, facts: Facts) -> Result<Value, Failure> {
    let c = Ctx { io: &io, f: &facts };
    let f = &facts;
    match id.as_str() {
        "xenonet.server" => {
            if let Some(me) = f.get("self").filter(|v| truthy(Some(v))) {
                let name = me.get("name").filter(|v| !v.is_null()).map_or_else(|| "XenoNet".into(), js_string);
                return Ok(ok(format!("{name} on port {}", s(me.get("port")))));
            }
            let Some(base) = f.get("base").filter(|v| truthy(Some(v))) else { return Ok(warn("XenoNet isn't running, so the map, its alerts and the tools sessions use are off", Some("Open XenoNet (or XenoNet Dev)."))) };
            let name = f.get("baseName").map_or_else(|| "XenoNet".into(), js_string);
            let b = js_string(base);
            Ok(ok(format!("{name} on {}", b.strip_prefix("http://").unwrap_or(&b))))
        }
        "xenonet.sources" => {
            let Some(health) = f.get("health").filter(|v| truthy(Some(v))) else { return Ok(skip("needs XenoNet running")) };
            let bad = failed_sources(Some(health));
            if !bad.is_empty() {
                let said: Vec<String> = bad.iter().map(|x| format!("{}: {}", s(x.get("name")), x.get("error").filter(|v| !v.is_null()).map_or_else(|| "failed".into(), js_string))).collect();
                return Ok(fail(said.join("; "), Some("Run xenonet doctor again after fixing the checks below; if it stays, xenonet logs --errors says more.")));
            }
            let mut counts: Map<String, Value> = Map::new();
            for x in list(health.get("sources")).into_iter().filter(|x| truthy(x.get("ok"))) {
                counts.insert(s(x.get("name")), x.get("count").cloned().unwrap_or(Value::Null));
            }
            let part = |k: &str, word: &str| counts.get(k).filter(|v| !v.is_null()).map(|v| plural(n(Some(v)), word, None));
            let parts: Vec<String> = [part("projects", "project"), part("claude", "Claude session"), part("codex", "Codex thread")].into_iter().flatten().collect();
            Ok(ok(format!("reading {}", parts.join(", "))))
        }
        "xenonet.blocks" => {
            let Some(health) = f.get("health").filter(|v| truthy(Some(v))) else { return Ok(skip("needs XenoNet running")) };
            let Some(blocks) = health.get("blocks").filter(|v| truthy(Some(v))) else { return Ok(skip("this XenoNet doesn't report its parts")) };
            let blocks = list(Some(blocks));
            let reported: Vec<Value> = failed_sources(Some(health)).iter().map(|x| x.get("name").cloned().unwrap_or(Value::Null)).collect();
            let has = |b: &Value| b.get("source").is_some_and(|src| reported.iter().any(|r| xenonet_app::jsv::strict_eq(Some(r), Some(src))));
            let wrong: Vec<&Value> = blocks.iter().filter(|b| matches!(b.get("state").and_then(Value::as_str), Some("failed" | "degraded"))).collect();
            let mut bad: Vec<&Value> = wrong.iter().copied().filter(|b| !has(b)).collect();
            bad.sort_by_key(|b| u8::from(b.get("state").and_then(Value::as_str) != Some("failed")));
            let mut left: Vec<String> = Vec::new();
            for b in wrong.iter().filter(|b| has(b)) {
                let src = s(b.get("source"));
                if !left.contains(&src) {
                    left.push(src);
                }
            }
            if bad.is_empty() {
                let besides = if left.is_empty() { String::new() } else { format!(" besides those reading for {}, which XenoNet can't read", left.join(" and ")) };
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
            let now = c.now().await?;
            let lasting: Vec<&Value> = bad.iter().copied().filter(|b| now - n(b.get("since")) >= BLOCK_GRACE_MS).collect();
            if lasting.is_empty() {
                return Ok(info(format!("{}, a moment ago; XenoNet tries again by itself", say(&bad)), None));
            }
            Ok(warn(say(&lasting), Some("Most of these clear up once what they read is back, and xenonet doctor's other checks say what's missing. If one stays, restart XenoNet; xenonet logs --errors says more.")))
        }
        "xenonet.state" => state(&c).await,
        "contracts.drift" => {
            let d = c.call("drift", json!([])).await.map_err(|_| Failure::Rethrow)?;
            let Some(d) = d.filter(|d| truthy(Some(d))) else { return Ok(skip("no cache folder configured")) };
            let records = list(d.get("records"));
            let looked = d.get("lookedAt").filter(|v| !v.is_null()).map_or(f64::NEG_INFINITY, |v| n(Some(v)));
            let fresh: Vec<&Value> = records.iter().filter(|r| n(r.get("firstSeen")) > looked).collect();
            if fresh.is_empty() {
                return Ok(ok(if records.is_empty() { "no differences from XenoNet's schemas".to_string() } else { format!("nothing new since you last looked ({} known)", plural(records.len() as f64, "difference", None)) }));
            }
            let addition = |r: &Value| r.get("kind").and_then(Value::as_str).is_some_and(|k| xenonet_schema::drift::ADDITIONS.contains(&k));
            let changes: Vec<&Value> = fresh.iter().copied().filter(|r| !addition(r)).collect();
            let shown: Vec<&Value> = changes.iter().copied().chain(fresh.iter().copied().filter(|r| addition(r))).collect();
            let named: Vec<String> = shown.iter().take(2).map(|r| format!("{}: {}", xenonet_schema::drift::source_name(&s(r.get("source"))), xenonet_schema::drift::describe(r))).collect();
            let more = if shown.len() > 2 { format!("; and {} more", shown.len() - 2) } else { String::new() };
            let say = format!("{}{more}", named.join("; "));
            let file = d.get("file").filter(|v| !v.is_null()).map_or_else(|| xenonet_schema::drift::DRIFT_FILE.to_string(), js_string);
            let fix = format!("xenonet drift lists them by source and marks them seen. XenoNet keeps them in {}.", c.tilde(&file));
            if !changes.is_empty() {
                return Ok(warn(format!("{} to what XenoNet reads since you last looked: {say}", plural(changes.len() as f64, "change", None)), Some(&fix)));
            }
            Ok(info(format!("the apps' data has {} since you last looked: {say}", plural(fresh.len() as f64, "addition", None)), Some(&fix)))
        }
        "claude.cli" => {
            let Some(bin) = c.which("claude").await? else { return Ok(warn("the claude command isn't on your PATH, and handing off to Claude and connecting Claude Code need it", Some("Install Claude Code: https://claude.com/claude-code"))) };
            let v = c.run(&bin, &["--version"], 4000.0).await?;
            static R: OnceLock<Regex> = OnceLock::new();
            let version = version_in(&s(v.get("stdout")), &R, r"\d+\.\d+\.\d+");
            Ok(ok(format!("{}{}", version.map_or(String::new(), |v| format!("version {v}, ")), c.tilde(&bin))))
        }
        "claude.auth" => {
            let Some(bin) = c.which("claude").await? else { return Ok(skip("Claude Code isn't installed")) };
            let r = c.run(&bin, &["auth", "status"], 6000.0).await?;
            if truthy(r.get("timedOut")) {
                return Ok(warn("claude auth status didn't answer in 6 seconds", None));
            }
            let st = xenonet_base::json::parse(&s(r.get("stdout"))).ok();
            if let Some(st) = &st {
                if truthy(st.get("loggedIn")) {
                    let by = st.get("authMethod").filter(|v| truthy(Some(v))).map_or(String::new(), |m| format!(" with {}", js_string(m)));
                    return Ok(ok(format!("signed in{by}")));
                }
                if truthy(Some(st)) && st.get("loggedIn") == Some(&json!(false)) {
                    return Ok(fail("not signed in: XenoNet can't read Claude plan usage, and new sessions can't start", Some("Run claude and sign in with /login.")));
                }
            }
            Ok(warn("couldn't tell from claude auth status", Some("Run claude auth status to see why.")))
        }
        "claude.transcripts" => {
            let dir = f.path("claudeProjectsDir");
            let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
            Ok(match folder(&c, dir.as_deref()).await? {
                Folder::Missing => info(format!("no {shown} yet: no Claude Code sessions on this machine"), None),
                Folder::Denied(code) => fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(f, &shown))),
                Folder::Ok(count) => ok(format!("{} of transcripts in {shown}", plural(count as f64, "project", None))),
            })
        }
        "claude.usage" => {
            let Some(live) = f.get("live").filter(|v| truthy(Some(v))) else { return Ok(skip("needs XenoNet running")) };
            let Some(cl) = live.get("limits").and_then(|l| l.get("claude")).filter(|v| truthy(Some(v))) else { return Ok(info("no plan usage reading yet", None)) };
            if !truthy(cl.get("stale")) {
                return Ok(ok(format!("read {}", if cl.get("source").and_then(Value::as_str) == Some("anthropic") { "from Anthropic" } else { "from Claude desktop" })));
            }
            let since = match cl.get("observedAt").filter(|v| truthy(Some(v))) {
                Some(at) => {
                    let shown = io.ask(json!({ "op": "localTime", "ms": at })).await.map_err(|_| Failure::Rethrow)?;
                    format!("no reading since {}", js_string(&shown))
                }
                None => "no reading yet".into(),
            };
            let reason = cl.get("reason").filter(|v| !v.is_null()).map_or_else(String::new, js_string);
            let any = |words: &[&str]| words.iter().any(|w| reason.contains(w));
            if any(&["expired", "refused"]) {
                return Ok(warn(format!("{reason}, so there's {since}"), Some("Open Claude Code once; it refreshes its sign-in, and XenoNet reads it on its next try.")));
            }
            if any(&["asked to wait", "couldn't reach", "no Claude Code sign-in"]) {
                return Ok(info(format!("{reason}, so there's {since}. XenoNet tries again by itself."), None));
            }
            Ok(warn(if truthy(cl.get("reason")) { format!("{reason}, so there's {since}") } else { format!("there's {since}") }, None))
        }
        "codex.cli" => {
            let mut bins: Vec<String> = Vec::new();
            if let Some(b) = f.get("codexBin").filter(|v| truthy(Some(v))) {
                bins.push(js_string(b));
            }
            if let Some(b) = c.which("codex").await? {
                bins.push(b);
            }
            let mut bin = None;
            for b in &bins {
                if NATIVE.is_absolute(b) && c.exists(b).await? {
                    bin = Some(b.clone());
                    break;
                }
            }
            let bin = bin.or_else(|| bins.first().cloned());
            let missing = match &bin {
                None => true,
                Some(b) if NATIVE.is_absolute(b) => !c.exists(b).await?,
                Some(b) => c.which(b).await?.is_none(),
            };
            if missing {
                return Ok(warn("no Codex: neither the ChatGPT app's codex nor a codex command", Some("Install the ChatGPT desktop app or the Codex CLI.")));
            }
            let bin = bin.unwrap();
            let v = c.run(&bin, &["--version"], 4000.0).await?;
            static R: OnceLock<Regex> = OnceLock::new();
            let version = version_in(&s(v.get("stdout")), &R, r"\d+\.\d+\.\d+[A-Za-z0-9_.-]*");
            Ok(ok(format!("{}{}", version.map_or(String::new(), |v| format!("version {v}, ")), c.tilde(&bin))))
        }
        "codex.auth" => {
            let bin = match c.which("codex").await? {
                Some(b) => Some(b),
                None => f.get("codexBin").map(js_string),
            };
            let Some(bin) = bin.filter(|b| !b.is_empty()) else { return Ok(skip("Codex isn't installed")) };
            let r = c.run(&bin, &["login", "status"], 6000.0).await?;
            let text = xenonet_base::text::js_trim(&format!("{}{}", s(r.get("stdout")), s(r.get("stderr")))).to_string();
            let lower = text.to_ascii_lowercase();
            let first = text.split('\n').next().unwrap_or("").to_string();
            if lower.contains("logged in") && !lower.contains("not logged in") {
                return Ok(ok(match first.strip_prefix("Logged in") {
                    Some(rest) => format!("signed in{rest}"),
                    None => first,
                }));
            }
            if lower.contains("not logged in") {
                return Ok(fail("not signed in", Some("Run codex login.")));
            }
            Ok(warn(if text.is_empty() { "couldn't tell from codex login status".to_string() } else { first }, None))
        }
        "codex.sessions" => {
            let home = f.get("paths").and_then(|p| p.get("codexHome")).filter(|v| !v.is_null()).map(js_string);
            let dir = home.as_ref().map(|h| if h.is_empty() { String::new() } else { style_of(h).join(&[h, "sessions"]) });
            let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
            Ok(match folder(&c, dir.as_deref()).await? {
                Folder::Missing => info(format!("no {shown} yet: no Codex sessions on this machine"), None),
                Folder::Denied(code) => fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(f, &shown))),
                Folder::Ok(_) => ok(shown),
            })
        }
        "system.projects" => {
            let dir = f.path("projectsDir");
            let shown = c.tilde(&dir.clone().unwrap_or_else(|| "undefined".into()));
            Ok(match folder(&c, dir.as_deref()).await? {
                Folder::Missing => fail(
                    format!("{shown} doesn't exist"),
                    Some(if truthy(f.data.get("projectsDirSet")) { "Make it, or set XENONET_PROJECTS_DIR to the folder that holds your projects." } else { "Make it, or choose the folder that holds your projects: Change the projects folder… in XenoNet's palette, or xenonet root <folder>." }),
                ),
                Folder::Denied(code) => fail(format!("can't read {shown} ({code})"), Some(&privacy_fix(f, &shown))),
                Folder::Ok(count) => ok(format!("{shown}, {} {}", xenonet_base::units::locale(count as f64, 0, 3), if count == 1 { "entry" } else { "entries" })),
            })
        }
        "system.git" => {
            if let Some(avail) = c.call("gitAvailable", json!([])).await.map_err(|_| Failure::Rethrow)? {
                if !truthy(Some(&avail)) {
                    return Ok(warn("git isn't installed, so repo status (uncommitted work, branches) is off", Some(if f.platform() == "darwin" { "Run xcode-select --install, or install git with Homebrew, then restart XenoNet." } else { "Install git, then restart XenoNet." })));
                }
            }
            Ok(match c.which("git").await? {
                Some(b) => ok(c.tilde(&b)),
                None => warn("git isn't on your PATH", None),
            })
        }
        "system.command" => command(&c).await,
        "system.runtime" => {
            if let Some(e) = f.get("electron").filter(|v| truthy(Some(v))) {
                return Ok(ok(format!("Electron {} (Node {})", js_string(e), s(f.get("node")))));
            }
            let node = s(f.get("node"));
            let mut parts = node.split('.').map(|p| xenonet_base::text::js_number(Some(&json!(p))));
            let (major, minor) = (parts.next().unwrap_or(f64::NAN), parts.next().unwrap_or(f64::NAN));
            if major < MIN_NODE.0 || (major == MIN_NODE.0 && minor < MIN_NODE.1) {
                return Ok(fail(format!("Node {node}; XenoNet needs 22.13 or later"), Some("Update Node.js.")));
            }
            Ok(ok(format!("Node {node}")))
        }
        "xenonet.errors" => {
            let entries = c.call("errors", json!([])).await.map_err(|_| Failure::Rethrow)?;
            let Some(entries) = entries.filter(|e| truthy(Some(e))) else { return Ok(skip("needs XenoNet's state folder")) };
            let kept: Vec<Value> = list(Some(&entries)).into_iter().filter(|e| !s(e.get("code")).starts_with("doctor.")).collect();
            let rows = xenonet_base::log::summarize(&kept);
            if rows.is_empty() {
                return Ok(ok("none in the last day"));
            }
            let total: f64 = rows.iter().map(|r| n(r.get("count"))).sum();
            let shown: Vec<String> = rows.iter().take(3).map(|r| format!("{}{}", s(r.get("code")), if n(r.get("count")) > 1.0 { format!(" ×{}", s(r.get("count"))) } else { String::new() })).collect();
            let others = if rows.len() > 3 { format!(", and {}", plural((rows.len() - 3) as f64, "other kind", None)) } else { String::new() };
            Ok(warn(format!("{} in the last day: {}{others}", plural(total, "error", None), shown.join(", ")), Some("xenonet logs --errors shows them, with a fix for each when there's one. xenonet diagnose --ask has a Codex or Claude Code session look at them with you.")))
        }
        "xenonet.launcher" => {
            let l = c.call("launcher", json!([])).await.map_err(|_| Failure::Rethrow)?;
            let Some(l) = l.filter(|v| truthy(Some(v))) else { return Ok(skip("no launcher here: XenoNet runs from a checkout")) };
            if xenonet_app::jsv::less(l.get("version"), l.get("ships")) {
                return Ok(warn(format!("launcher {}; this XenoNet ships {}", s(l.get("version")), s(l.get("ships"))), Some("XenoNet replaces it the next time it starts.")));
            }
            let Some(said) = l.get("said").filter(|v| truthy(Some(v))) else {
                return Ok(fail(format!("the launcher in {} doesn't run", c.tilde(&s(l.get("dir")))), Some("Open XenoNet, which writes it again, or install XenoNet again.")));
            };
            if !truthy(said.get("tools")) {
                return Ok(fail("the launcher can't find XenoNet's Tools, so the xenonet command and MCP servers don't start (hooks still reach XenoNet)", Some("Open XenoNet to repair them, or install XenoNet again.")));
            }
            let node = s(said.get("node"));
            Ok(ok(format!("launcher {}, running Tools {} on Node {}", s(l.get("version")), s(said.get("tools")), node.strip_prefix('v').unwrap_or(&node))))
        }
        "agents.claude" | "agents.codex" => {
            let app = &id["agents.".len()..];
            let connect = f.get("connect").filter(|v| truthy(Some(v)));
            let a = connect.and_then(|cn| cn.get("apps")).and_then(Value::as_array).and_then(|apps| apps.iter().find(|x| x.get("app").and_then(Value::as_str) == Some(app)));
            let Some(a) = a else { return Ok(skip("needs XenoNet running")) };
            let changes = list(a.get("changes"));
            if changes.iter().all(|x| x.get("state").and_then(Value::as_str) == Some("missing")) {
                return Ok(info("not connected, so these sessions can't see or message each other through XenoNet", Some("Connect them from XenoNet's palette (Connect Claude Code and Codex to XenoNet), or run xenonet connect.")));
            }
            let stale: Vec<String> = changes.iter().filter(|x| x.get("state").and_then(Value::as_str) != Some("present")).map(|x| format!("{} {}", s(x.get("id")), s(x.get("state")))).collect();
            if !stale.is_empty() {
                return Ok(warn(format!("partly connected ({})", stale.join(", ")), Some("Run xenonet connect again.")));
            }
            let broken = list(a.get("broken"));
            if !broken.is_empty() {
                return Ok(fail(format!("connected, but {}", broken.iter().map(js_string).collect::<Vec<_>>().join("; ")), Some("Run xenonet connect again.")));
            }
            Ok(ok(format!("connected to XenoNet on port {}", s(connect.and_then(|cn| cn.get("port"))))))
        }
        "agents.checked" => {
            if f.platform() != "darwin" {
                return Ok(info("XenoNet can't see yet which process sends a request on this system, so it takes each session at its word", None));
            }
            let t = c.must("procs", json!([])).await?;
            if !truthy(t.get("peer")) {
                return Ok(warn("netstat doesn't say which process sends a request, so XenoNet takes each session at its word", Some(&unreadable("netstat"))));
            }
            if !truthy(t.get("chain")) {
                return Ok(warn("ps doesn't say which processes a request comes from, so XenoNet refuses every request in a session's name, and sessions can't use its tools", Some(&unreadable("ps"))));
            }
            if !c.exists("/usr/bin/codesign").await? {
                return Ok(warn("codesign is missing, so Claude Code's own hooks can't settle which process holds a session", None));
            }
            Ok(ok("XenoNet checks each request in a session's name against the process that sent it"))
        }
        "actors.repos" => repos(&c).await,
        "actors.expired" => {
            if let Some(off) = no_actors(f) {
                return Ok(off);
            }
            let expired = list(c.call("actors.expired", json!([])).await.map_err(|_| Failure::Rethrow)?.as_ref());
            if expired.is_empty() {
                return Ok(ok("no actor's token has expired"));
            }
            let names: Vec<String> = expired.iter().map(|a| format!("\"{}\" (actor:{})", s(a.get("name")), s(a.get("slug")))).collect();
            let said = if names.len() > 2 { format!("{} and {} more", names[..2].join(", "), names.len() - 2) } else { names.join(" and ") };
            let one = expired.len() == 1;
            Ok(warn(format!("{said} {} expired after 30 days without a call, so {} offline", if one { "has a token that" } else { "have tokens that" }, if one { "it stays" } else { "they stay" }), Some("Give each a new token with New token in XenoNet's Actors panel, or remove the ones you don't use there or with xenonet actor remove <slug>.")))
        }
        "actors.gateway" => gateway(&c).await,
        "actors.wakes" => {
            if let Some(off) = no_actors(f) {
                return Ok(off);
            }
            let wakes = c.call("actors.wakes", json!([])).await.map_err(|_| Failure::Rethrow)?;
            let Some(wakes) = wakes.filter(|v| truthy(Some(v))) else { return Ok(skip("waking actors isn't in this XenoNet")) };
            let hooks: Vec<Value> = list(Some(&wakes)).into_iter().filter(|w| w.get("kind").and_then(Value::as_str) == Some("webhook")).collect();
            if hooks.is_empty() {
                return Ok(ok("no actor is woken by a webhook"));
            }
            let failing: Vec<String> = hooks.iter().filter(|w| w.get("last").is_some_and(|l| truthy(Some(l)) && !truthy(l.get("ok")))).map(|w| format!("{}'s last wake failed: {}", s(w.get("actor")), w["last"].get("reason").filter(|v| !v.is_null()).map_or_else(|| "it didn't answer".into(), js_string))).collect();
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
                return Ok(info("XenoNet can't see yet which process sends a request on this system, so an actor's token alone decides", None));
            }
            let t = c.must("procs", json!([])).await?;
            let blind = if !truthy(t.get("peer")) {
                Some(("netstat doesn't say which process sends a request", "netstat"))
            } else if !truthy(t.get("chain")) {
                Some(("ps doesn't say which processes a request comes from", "ps"))
            } else {
                None
            };
            if let Some((why, tool)) = blind {
                return Ok(warn(format!("{why}, so XenoNet refuses every actor's token, and what only you or its window may do, such as allowing or removing actors"), Some(&unreadable(tool))));
            }
            if t.get("exe") == Some(&json!(false)) {
                return Ok(warn("lsof doesn't say which program a process runs, so XenoNet refuses Hermes's actor, and what only its window may do, such as allowing actors", Some(&unreadable("lsof"))));
            }
            let observed = if f.actors() == "on" { c.call("actors.observers", json!([])).await.map_err(|_| Failure::Rethrow)? } else { None };
            let unknown: Vec<String> = list(observed.as_ref())
                .iter()
                .filter(|o| truthy(o.get("on")) && o.get("recognized").is_some_and(|r| truthy(Some(r)) && n(r.get("found")) < n(r.get("expected"))))
                .map(|o| format!("{} runs {} processes XenoNet doesn't know as {}'s, so its actor's token would be refused from under them", s(o.get("name")), list(o["recognized"].get("purposes")).iter().map(js_string).collect::<Vec<_>>().join(", "), s(o.get("name"))))
                .collect();
            if !unknown.is_empty() {
                return Ok(warn(unknown.join("; "), Some("The product may have moved where it keeps its programs; update XenoNet, or report it.")));
            }
            let said = "an actor's token never works from under a session or an actor app's own process, and a product's only from under that product";
            let verify = c.call("verify", json!([])).await.map_err(|_| Failure::Rethrow)?;
            let unseen = verify.as_ref().and_then(|v| v.get("unseen")).filter(|v| !v.is_null()).map_or(0.0, |v| n(Some(v)));
            if unseen != 0.0 && !unseen.is_nan() {
                let them = if unseen == 1.0 { "it" } else { "them" };
                return Ok(info(format!("{said}; since XenoNet started, it refused {} because it couldn't see which program sent {them}", plural(unseen, "request", None)), Some("A program that sends a request and exits before the answer can't be seen; one that waits for it can. If this keeps growing, netstat or ps may be answering in a way XenoNet doesn't read: update XenoNet, or report it.")));
            }
            Ok(ok(said))
        }
        _ => Err(Failure::msg(format!("no such check: {id}"))),
    }
}

fn style_of(p: &str) -> Style {
    match xenonet_base::paths::path_style(p) {
        xenonet_base::paths::PathStyle::Windows => Style::Win32,
        xenonet_base::paths::PathStyle::Posix => Style::Posix,
        xenonet_base::paths::PathStyle::Native => NATIVE,
    }
}

async fn state(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    let Some(dir) = f.path("stateDir") else { return Ok(skip("no state folder configured")) };
    let shown = c.tilde(&dir);
    if !c.exists(&dir).await? {
        return Ok(info(format!("{shown} doesn't exist yet; XenoNet makes it on first save"), None));
    }
    if !matches!(c.call("access", json!([dir, "W_OK"])).await, Ok(Some(_))) {
        return Ok(fail(format!("can't write to {shown}"), Some("Fix the folder's permissions, or set XENONET_STATE_DIR to a folder you own.")));
    }
    // The file itself: one moved aside, one it can't read now, one from a
    // newer XenoNet, one it isn't saving to, or parts that don't fit.
    let file = c.call("stateFile", json!([dir])).await.ok().flatten().filter(|v| truthy(Some(v)));
    let loaded = f.get("stateLoad");
    let file_get = |k: &str| file.as_ref().and_then(|x| x.get(k)).filter(|v| !v.is_null());
    let broken = list(file_get("broken"));
    if let Some(last) = broken.last() {
        return Ok(warn(format!("{} is a state.json XenoNet couldn't read, so it moved it aside and started afresh", c.tilde(&js_string(last))), Some("To get back what was in it, fix it and put it back as state.json while XenoNet is closed; delete it once you don't need it.")));
    }
    if let Some(u) = file_get("unreadable").filter(|v| truthy(Some(v))) {
        return Ok(warn(format!("state.json {}, so XenoNet will move it aside and start afresh", js_string(u)), Some("To keep what's in it, fix it before XenoNet next starts.")));
    }
    if file.is_some() && xenonet_app::jsv::less(file_get("known"), file_get("version")) && file_get("version").is_some() && file_get("known").is_some() {
        return Ok(warn(format!("state.json is from a newer XenoNet (version {}), so this one reads it but doesn't save changes to it", s(file_get("version"))), Some("Use the newer XenoNet, or update this one.")));
    }
    if let Some(l) = loaded.filter(|l| truthy(l.get("readOnly"))) {
        return Ok(warn(format!("{}; changes last only until XenoNet quits", s(l.get("problem"))), Some("Fix the file or its folder, then restart XenoNet.")));
    }
    let issues = list(file_get("issues"));
    if let Some(first) = issues.first() {
        let path = c.tilde(&first.get("path").map(js_string).unwrap_or_default());
        let message = s(first.get("message"));
        return Ok(warn(format!("state.json has {} XenoNet can't use, such as {}: {}", plural(issues.len() as f64, "part", None), if path.is_empty() { "the whole file".into() } else { path }, message.strip_prefix("Invalid input: ").unwrap_or(&message)), Some("Fix that part, or move state.json aside while XenoNet is closed to start afresh.")));
    }
    if let Ok(Some(st)) = c.call("statfs", json!([dir])).await {
        let free = n(st.get("bavail")) * n(st.get("bsize"));
        if free < 1e9 {
            return Ok(warn(format!("{} GB free where XenoNet keeps its state", xenonet_base::units::to_fixed(free / 1e9, 1)), Some("Free some disk space; posters and replay clips need room too.")));
        }
    }
    Ok(ok(shown))
}

async fn command(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    let st = c.must("command", json!([])).await?;
    let Some(file) = st.get("file").filter(|v| truthy(Some(v))).map(js_string) else { return Ok(skip("not on Windows yet")) };
    let place = c.tilde(&file);
    if !truthy(st.get("installed")) {
        return Ok(info("not installed; once it is, xenonet doctor, xenonet agents and the rest work in a terminal", Some("Install it from the palette (Install the xenonet command), or run node cli/xenonet.js install in a checkout of XenoNet.")));
    }
    if !truthy(st.get("ours")) {
        return Ok(info(format!("{place} is another program's"), None));
    }
    if truthy(st.get("broken")) {
        let runs: Vec<String> = list(st.get("runs")).iter().map(|r| c.tilde(&js_string(r))).collect();
        return Ok(warn(format!("{place} runs {}, which is gone", runs.join(" ")), Some("Install it again from the palette (Install the xenonet command).")));
    }
    let dir = c.tilde(&NATIVE.dirname(&file));
    let win = f.platform() == "win32";
    if !truthy(st.get("onPath")) && win {
        let saved = c.call("savedPath", json!([])).await.map_err(|_| Failure::Rethrow)?;
        let target = Style::Win32.dirname(&file);
        if list(saved.as_ref()).iter().any(|d| xenonet_app::command::same_place(&js_string(d), &target, "win32")) {
            return Ok(ok(format!("{place}, on your Path for new terminals")));
        }
    }
    if !truthy(st.get("onPath")) {
        let fix = if win { format!("Run XenoNet's Setup again, which adds it, or add {dir} to your user Path in Windows' Environment Variables. Then open a new terminal.") } else { "Add it in your shell's profile: export PATH=\"$HOME/.local/bin:$PATH\"".to_string() };
        return Ok(warn(format!("{place} is there, but {dir} isn't on your PATH"), Some(&fix)));
    }
    Ok(ok(place))
}

async fn repos(c: &Ctx<'_>) -> Result<Value, Failure> {
    if let Some(off) = no_actors(c.f) {
        return Ok(off);
    }
    let started = c.now().await?;
    let by_hand = c.f.data.get("byHand") == Some(&json!(true));
    let found = match c.call("actors.tokenFiles", json!([{ "fresh": by_hand }])).await.map_err(|_| Failure::Rethrow)? {
        Some(v) => v,
        None => {
            let repos = list(c.call("actors.repos", json!([])).await.map_err(|_| Failure::Rethrow)?.as_ref());
            let mut m = Map::new();
            m.insert("at".into(), Value::Null);
            m.insert("repos".into(), json!(repos.len()));
            let more = if repos.is_empty() { json!({ "checked": 0, "files": [] }) } else { c.io.ask(json!({ "op": "findTokenFiles", "repos": repos })).await.map_err(|_| Failure::Rethrow)? };
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
        let shown: Vec<String> = files.iter().take(3).map(|x| c.tilde(&js_string(x))).collect();
        let more = if files.len() > 3 { format!(" and {} more", files.len() - 3) } else { String::new() };
        return Ok(warn(
            format!("{} in a repo {} an actor's token: {}{more}{when}", plural(files.len() as f64, "file", None), if files.len() == 1 { "holds" } else { "hold" }, shown.join(", ")),
            Some("Move each token out of the repo, whether it's in a file of its own or a line of a .env. Keep it in a file only you can read, outside any repo (for example ~/.config/<name>/xenonet.token, named in XENONET_ACTOR_TOKEN_FILE). Give its actor a new token with New token in XenoNet's Actors panel, since a session may have read it. If it was committed, the old token has leaked."),
        ));
    }
    let checked = n(found.get("checked"));
    if checked < count {
        return Ok(info(format!("none in the {} looked in; the other {} didn't answer in time{when}", plural(checked, "repo", None), xenonet_base::json::number_string(count - checked)), None));
    }
    Ok(ok(format!("none in {}{when}", plural(count, "repo", None))))
}

async fn gateway(c: &Ctx<'_>) -> Result<Value, Failure> {
    if let Some(off) = no_actors(c.f) {
        return Ok(off);
    }
    let st = c.call("actors.gateway", json!([])).await.map_err(|_| Failure::Rethrow)?;
    let Some(st) = st.filter(|v| truthy(Some(v))) else { return Ok(skip("the actor gateway isn't in this XenoNet")) };
    if !truthy(st.get("on")) {
        return Ok(ok("it's off, so nothing outside this computer reaches XenoNet through it"));
    }
    let fix = "In XenoNet's Actors panel, Repair sets the tunnel up again and Change… picks another; xenonet gateway check checks again from a terminal.";
    if let Some(p) = st.get("problem").filter(|v| truthy(Some(v))) {
        return Ok(warn(js_string(p), Some(fix)));
    }
    if st.get("reach").and_then(|r| r.get("state")).and_then(Value::as_str) != Some("ok") {
        return Ok(info(format!("it's on, at {}; XenoNet hasn't checked that address yet", st.get("host").filter(|v| !v.is_null()).map_or_else(|| "no address yet".into(), js_string)), None));
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
            said.push(format!("{name}'s gateway token stops working on {} unless it calls before then", &xenonet_base::time::iso(t)[..10]));
        }
    }
    if !said.is_empty() {
        return Ok(warn(said.join("; "), Some("In XenoNet's Actors panel, Connect… on the actor gives its app the address and a token again.")));
    }
    let through = st.get("tunnel").and_then(|t| t.get("through")).filter(|v| truthy(Some(v))).map_or(String::new(), |t| format!(" through {}", js_string(t)));
    let local = if truthy(st["reach"].get("local")) { ", checked on this computer alone" } else { "" };
    Ok(ok(format!("reachable at {}{through}{local}", s(st.get("url")))))
}

async fn observers(c: &Ctx<'_>) -> Result<Value, Failure> {
    let f = c.f;
    if let Some(off) = no_actors(f) {
        return Ok(off);
    }
    let all = list(c.call("actors.observers", json!([])).await.map_err(|_| Failure::Rethrow)?.as_ref());
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
        return Ok(info(format!("{} {} on this computer, so there's nothing to observe", if named.is_empty() { "No product XenoNet observes".to_string() } else { named }, if all.len() == 1 { "isn't" } else { "aren't" }), None));
    }
    let on: Vec<&Value> = installed.iter().copied().filter(|o| truthy(o.get("on"))).collect();
    if on.is_empty() {
        let one = installed.len() == 1;
        return Ok(info(format!("the {} of {} {} off", if one { "observer" } else { "observers" }, names(&installed), if one { "is" } else { "are" }), Some("Turn one on in XenoNet's Actors panel, or with xenonet actor observe <product> on.")));
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
                let file = src.get("file").filter(|v| truthy(Some(v))).map_or(String::new(), |x| format!(" ({})", js_string(x)));
                bad.push(format!("{}'s {}{file} {w}", s(o.get("name")), s(src.get("name"))));
            }
        }
    }
    if !bad.is_empty() {
        let fix = if f.platform() == "darwin" { "A file that can't be read is usually macOS privacy or its permissions: check them, then restart XenoNet. One in another shape means the product changed it: update XenoNet, and npm run contracts in a checkout lists what changed." } else { "XenoNet observes these products on macOS for now." };
        return Ok(warn(bad.join("; "), Some(fix)));
    }
    Ok(ok(on.iter().map(|o| format!("{}: {} read", s(o.get("name")), plural(list(o.get("sources")).iter().filter(|x| x.get("state").and_then(Value::as_str) == Some("ok")).count() as f64, "source", None))).collect::<Vec<_>>().join("; ")))
}
