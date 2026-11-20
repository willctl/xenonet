//! Starting a session: core/delivery/starts.js. You, a session or an
//! actor asks a machine for a new session of Claude Code, Codex or Grok
//! Build, in a folder there, with a first prompt. Whether the machine
//! takes it is `reach`'s (its start setting); what a request may say, how
//! its first prompt is framed, the limits and what the answers say are
//! here. The command that starts each app is its adapter's (`apps`, the
//! `start` way in). Running it is the shared runner's (`runs`), and
//! carrying it to another machine the network's: a message frame to
//! `TARGET`, answered by receipts keyed by its id.
//!
//! The prompt goes on standard input (or in a file of XenoNet's, for an
//! app that reads it from one), never as an argument. A session never
//! starts bypassing its permission checks.

use super::apps::{self, Start};
use serde_json::{json, Value};
pub use xenonet_adapter::delivery::Finish;

/// The reserved target a start request goes to on another machine.
pub const TARGET: &str = "xenonet:start";

/// The apps XenoNet starts sessions of: those whose adapter has a start,
/// in the registry's order, then those it starts over ACP alone (the
/// registry's `acp`, as an actor product's entry gives it). The first is
/// the one a request that names no app gets.
pub fn apps() -> Vec<&'static xenonet_adapter::apps::App> {
    let mut out: Vec<&'static xenonet_adapter::apps::App> = apps::apps().into_iter().filter(|(a, w)| w.start.is_some() || a.acp.is_some()).map(|(a, _)| a).collect();
    for p in xenonet_adapter::platforms().iter() {
        let a = p.app();
        if a.acp.is_some() && !out.iter().any(|o| o.id == a.id) {
            out.push(a);
        }
    }
    out
}

/// Whether XenoNet starts an app's sessions over ACP alone: it has an ACP
/// command and no headless start to fall back to.
pub fn acp_only(app: &xenonet_adapter::apps::App) -> bool {
    app.acp.is_some() && apps::of(&app.id).is_none_or(|w| w.start.is_none())
}

/// Them as a person reads the list: "Claude Code (claude), Codex (codex)
/// or Grok Build (grok)".
fn apps_words(started: &[&xenonet_adapter::apps::App]) -> String {
    let each: Vec<String> = started.iter().map(|a| format!("{} ({})", a.name, a.id)).collect();
    match each.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => each.join(""),
    }
}

/// Permission modes a request may ask for; manual is the person's own
/// settings. bypassPermissions is never taken.
pub const MODES: [&str; 5] = ["manual", "plan", "acceptEdits", "auto", "dontAsk"];
pub const PROMPT_MAX: usize = 16000;
/// Starts an hour, on one machine and from one sending machine.
pub const PER_HOUR: usize = 20;
pub const PER_HOST_HOUR: usize = 10;
const HOUR_MS: f64 = 3600e3;
/// A start request older than this when it arrives isn't started: a start
/// is never kept for later.
pub const FRESH_MS: f64 = 120e3;

/// What a request says of how confined the run is: nothing (`Unset`, which
/// the sender fills in by who asks and where, and a receiver takes as
/// nothing said), `off`, `vendor` with the domains the run may reach, or
/// `cell`. It never holds a wall: a cell start runs behind the wall of the
/// identity that signs it, which the machine that runs it works out from
/// the verified act (`xenonet_authz::Confine` is what is signed).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConfineAsk {
    #[default]
    Unset,
    Off,
    Vendor(Vec<String>),
    Cell,
}

impl ConfineAsk {
    /// The domains of a run held to its app's sandbox, None for any other.
    pub fn domains(&self) -> Option<&[String]> {
        match self {
            ConfineAsk::Vendor(d) => Some(d),
            _ => None,
        }
    }
}

/// A domain a sandboxed run may reach: a host name, or `*.` and one.
fn domain(d: &str) -> Result<String, String> {
    let d = d.trim().to_ascii_lowercase();
    let host = d.strip_prefix("*.").unwrap_or(&d);
    let ok = !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && host.split('.').all(|l| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    if ok {
        Ok(d)
    } else {
        Err(format!("{d} isn't a domain a sandboxed run can be let reach: a host name like github.com, or *.example.org"))
    }
}

/// The domains the person lets runs they didn't start themselves reach, as
/// they gave them: each a host name or `*.` and one, in lower case.
pub fn allowed_domains(given: &[String]) -> Result<Vec<String>, String> {
    let mut all = given.iter().map(|d| domain(d)).collect::<Result<Vec<_>, _>>()?;
    all.sort();
    all.dedup();
    Ok(all)
}

/// Why a run started by anyone but the person may not reach `wanted`, given
/// the domains the person allows such runs (`allowed`): the first domain
/// outside them, in words. A name is within an entry that is the same name,
/// or a `*.` entry that holds it as a subdomain (`*.example.org` holds
/// `api.example.org` and `*.api.example.org`, not `example.org`).
pub fn domains_within(wanted: &[String], allowed: &[String]) -> Result<(), String> {
    let within = |d: &str| {
        allowed.iter().any(|a| {
            let (core, wild) = (d.strip_prefix("*.").unwrap_or(d), d.starts_with("*."));
            match a.strip_prefix("*.") {
                Some(zone) => a == d || core.ends_with(&format!(".{zone}")),
                None => !wild && a == d,
            }
        })
    };
    match wanted.iter().find(|d| !within(d)) {
        None => Ok(()),
        Some(d) if allowed.is_empty() => Err(format!(
            "{d} can't be reached by a run you didn't start yourself: this machine lets such runs reach no domain, and you can allow some with xenonet runs domains set"
        )),
        Some(d) => {
            Err(format!("{d} is not among the domains this machine lets a run you didn't start yourself reach ({}): change them with xenonet runs domains", allowed.join(", ")))
        }
    }
}

/// What a request asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// A project or folder id from the machine's map, a path it reports,
    /// or ~.
    pub folder: String,
    pub app: String,
    /// None: the person's own settings.
    pub mode: Option<String>,
    pub prompt: String,
    /// Asked as delegated work: the machine starts it only when it's set to
    /// delegate (a session or actor asking; you asking is never refused).
    pub delegate: bool,
    /// How confined the run is: the level, and for `vendor` the domains it
    /// may reach.
    pub confine: ConfineAsk,
}

impl Request {
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "folder": self.folder, "app": self.app, "mode": self.mode, "prompt": self.prompt });
        if self.delegate {
            v["delegate"] = json!(true);
        }
        // A level is always said when it is known: a sender that says
        // nothing is an older one, and a receiver takes nothing as off.
        match &self.confine {
            ConfineAsk::Unset => {}
            ConfineAsk::Off => v["confine"] = json!("off"),
            ConfineAsk::Vendor(domains) => {
                v["confine"] = json!("vendor");
                v["domains"] = json!(domains);
            }
            ConfineAsk::Cell => v["confine"] = json!("cell"),
        }
        v
    }
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

/// A request as it came (`{ folder?, app?, mode?, prompt, delegate? }`), or why it
/// can't be started anywhere.
pub fn request(v: &Value) -> Result<Request, String> {
    let started = apps();
    let app = text(v, "app").filter(|a| !a.is_empty()).or_else(|| started.first().map(|a| a.id.clone())).unwrap_or_default();
    if !started.iter().any(|a| a.id == app) {
        return Err(format!("XenoNet starts {}, not {app}", apps_words(&started)));
    }
    let mode = text(v, "mode").filter(|m| !m.is_empty() && m != "manual" && m != "default");
    if let Some(m) = &mode {
        if m == "bypassPermissions" {
            return Err("XenoNet never starts a session that bypasses its permission checks".into());
        }
        if !MODES.contains(&m.as_str()) {
            return Err(format!("the mode is one of {}, not {m}", MODES.join(", ")));
        }
        if let Some(why) = apps::of(&app).and_then(|w| w.no_mode) {
            return Err(why.into());
        }
        // ACP has no word for XenoNet's modes, and an app with no headless
        // start has nothing else to start with one.
        if let Some(a) = started.iter().find(|a| a.id == app && acp_only(a)) {
            return Err(format!("{} starts under its own settings, so no mode", a.name));
        }
    }
    let prompt = text(v, "prompt").map(|p| p.trim().to_string()).unwrap_or_default();
    if prompt.is_empty() {
        return Err("say what the new session should do: a prompt".into());
    }
    if prompt.chars().count() > PROMPT_MAX {
        return Err(format!("a first prompt is at most {PROMPT_MAX} characters"));
    }
    let folder = text(v, "folder").map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).unwrap_or_else(|| "~".into());
    if folder.chars().count() > 1000 {
        return Err("that folder is too long to be one".into());
    }
    let delegate = v.get("delegate").and_then(Value::as_bool).unwrap_or(false);
    let confine = confine_of(v)?;
    Ok(Request { folder, app, mode, prompt, delegate, confine })
}

/// A request's level: `confine` (`off`, `vendor` or `cell`), or the older
/// `sandbox` (true for vendor, false for off, or `{ domains }`), and its
/// `domains`. Domains alone ask for vendor. A request that names a wall is
/// refused: the wall is the signer's, worked out where the start runs.
fn confine_of(v: &Value) -> Result<ConfineAsk, String> {
    if v.get("wall").is_some_and(|w| !w.is_null()) {
        return Err("a start names no wall: a start in a cell runs behind the wall of the identity that signs it".into());
    }
    let list = |x: Option<&Value>| -> Result<Option<Vec<String>>, String> {
        match x {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(a)) => a.iter().map(|d| d.as_str().ok_or("a domain is text".to_string()).and_then(domain)).collect::<Result<Vec<_>, _>>().map(Some),
            Some(_) => Err("domains is a list of host names".into()),
        }
    };
    let given = list(v.get("domains"))?;
    // The older words, kept: `sandbox` is true, false or `{ domains }`.
    let (old, inside) = match v.get("sandbox") {
        None | Some(Value::Null) => (None, None),
        Some(Value::Bool(b)) => (Some(if *b { "vendor" } else { "off" }), None),
        Some(o @ Value::Object(_)) => (Some(if o.get("on").and_then(Value::as_bool).unwrap_or(true) { "vendor" } else { "off" }), list(o.get("domains"))?),
        Some(_) => return Err("sandbox is true, false or an object with domains".into()),
    };
    let level = match v.get("confine") {
        None | Some(Value::Null) => None,
        Some(Value::String(l)) if ["off", "vendor", "cell"].contains(&l.as_str()) => Some(l.as_str()),
        Some(other) => return Err(format!("{other} isn't a confinement level: use off, vendor or cell")),
    };
    if let (Some(a), Some(b)) = (level, old) {
        if a != b {
            return Err(format!("confine says {a} and sandbox says {b}: say it once, with confine"));
        }
    }
    let mut domains: Vec<String> = given.into_iter().chain(inside).flatten().collect();
    domains.sort();
    domains.dedup();
    match (level.or(old), domains.is_empty()) {
        (None, true) => Ok(ConfineAsk::Unset),
        (Some("off"), true) => Ok(ConfineAsk::Off),
        (Some("off"), false) => Err("domains are for a run in its app's sandbox: they reach nothing with confinement off".into()),
        (Some("cell"), true) => Ok(ConfineAsk::Cell),
        (Some("cell"), false) => Err("a cell reaches no domain, so it takes none".into()),
        // Domains alone ask for the sandbox.
        (_, _) => Ok(ConfineAsk::Vendor(domains)),
    }
}

/// The sandbox's words in the app's own terms, or why the app can't start
/// sandboxed here: no mapping for its sandbox (Grok Build, and the apps
/// started over ACP alone), or a limit of its own (native Windows, domains).
/// `windows` is whether this machine is native Windows.
pub fn sandbox_plan(r: &Request, plan: &xenonet_adapter::delivery::Plan<'_>, windows: bool) -> Result<String, String> {
    let name = apps().into_iter().find(|a| a.id == r.app).map_or(r.app.clone(), |a| a.name.clone());
    let ways = apps::of(&r.app).and_then(|w| w.sandbox).ok_or_else(|| {
        format!("{name} has no sandbox XenoNet can drive, so it isn't started sandboxed: start it without the sandbox from your own console, or use Claude Code or Codex")
    })?;
    if let Some(why) = (ways.unavailable)(plan, windows) {
        return Err(why);
    }
    Ok((ways.words)(plan))
}

/// Whether `app`'s sandbox is enough for a run started by `by_person` (or
/// not): an app whose sandbox lets its commands read the whole disk (Codex)
/// is not enough for a run that isn't the person's own, since its last words
/// can carry any file it read. Why not, in words.
pub fn sandbox_enough(app: &str, by_person: bool) -> Result<(), String> {
    let reads_all = apps::of(app).and_then(|w| w.sandbox).is_some_and(|s| s.reads_whole_disk);
    if reads_all && !by_person {
        let name = apps().into_iter().find(|a| a.id == app).map_or(app.to_string(), |a| a.name.clone());
        return Err(format!(
            "{name}'s sandbox can't keep what a run reads from the rest of the disk out of what it says, so it isn't started for anyone but you: start it from your own console, or use Claude Code"
        ));
    }
    Ok(())
}

/// One-line text for a title, without what could break the framing.
fn title_of(t: &str) -> String {
    let one = super::policy::sanitize(t, true, 120.0);
    one.replace(['«', '»'], "\"")
}

/// What a delegated session is told after who asked: what XenoNet does
/// for it, and what stays off limits whatever the delegation.
pub const DELEGATED_RULES: &str = "Within that delegation XenoNet answers your permission prompts for reading, for editing files under this folder and for routine commands, and sends anything else to your user's console as a question, so you needn't ask first. Never delete data, print or move secrets, or spend money.";

/// The first prompt: who asked, then the prompt. `by` is `{ kind, id,
/// title }`; `from` is the machine it came from, None for this one.
/// `delegation` is the machine that put this one on standing delegation
/// and the day (a session or actor starts under it): the prompt then says
/// the asker's authority is your user's, checked here, not claimed.
pub fn framed(by: &Value, from: Option<&str>, prompt: &str, delegation: Option<(&str, &str)>, sandbox: Option<&str>) -> String {
    let kind = by.get("kind").and_then(Value::as_str).unwrap_or("session");
    let mut rules = String::new();
    let line = if kind == "you" {
        match from {
            Some(h) => format!("[XenoNet] Started by you, at {h}'s console."),
            None => "[XenoNet] Started by you, at this machine's console.".to_string(),
        }
    } else {
        let id = super::policy::sanitize(by.get("id").and_then(Value::as_str).unwrap_or(""), true, 200.0);
        let title = title_of(by.get("title").and_then(Value::as_str).unwrap_or(&id));
        let what = if kind == "actor" { "the actor" } else { "the session" };
        let tail = match delegation {
            Some((host, date)) => {
                rules = format!("{DELEGATED_RULES}\n\n");
                format!(
                    " Your user delegated it: they put this machine on standing delegation at {host}'s console on {date}, which XenoNet here checked; the asker can't claim it."
                )
            }
            None => " It came from an agent, not from your user.".to_string(),
        };
        match from {
            Some(h) => format!("[XenoNet] {h} says this was asked by {what} «{title}» ({id}).{tail}"),
            None => format!("[XenoNet] This was asked by {what} «{title}» ({id}) on this machine.{tail}"),
        }
    };
    // A sandboxed run is told it is, and what it may reach, so it doesn't
    // spend its turns on what the sandbox will refuse.
    let sandboxed = sandbox.map(|w| format!("[XenoNet] This run is sandboxed. {w}\n\n")).unwrap_or_default();
    format!("{line}\n\n{sandboxed}{rules}{prompt}")
}

/// Whether the app reads a turn's prompt from a file XenoNet writes, as its
/// adapter says, rather than from standard input.
pub fn prompt_in_file(app: &str) -> bool {
    apps::of(app).is_some_and(|w| w.prompt_file)
}

/// The app and arguments that start a new session `id` (a UUID XenoNet
/// picks, so the answer can name it at once), in `cwd`, as the app's
/// adapter gives them. `settings` is the listen file a session waits for
/// messages with after each turn; `server` the xenonet MCP server's name
/// in its settings, whose permission tool answers its prompts;
/// `prompt_file` where the prompt was written for an app that reads it
/// from a file. Or why there's no such command: the app's adapter has no
/// start.
pub fn command(r: &Request, id: &str, cwd: &str, settings: Option<&str>, server: &str, prompt_file: Option<&str>) -> Result<(String, Vec<String>), String> {
    let start = apps::of(&r.app).and_then(|w| w.start).ok_or_else(|| format!("XenoNet has no way to start a {} session here", r.app))?;
    Ok(start(&Start { id, cwd, mode: r.mode.as_deref(), settings, server, prompt_file }))
}

/// The arguments of a turn of `app` run in its sandbox, given the files of
/// the run: a start's, a resumed turn's or a copy's alike.
pub fn sandboxed(app: &str, files: &xenonet_adapter::delivery::SandboxFiles<'_>, args: Vec<String>) -> Result<Vec<String>, String> {
    let ways = apps::of(app).and_then(|w| w.sandbox).ok_or_else(|| format!("{app} has no sandbox XenoNet can drive"))?;
    Ok((ways.turn)(files, args))
}

/// Who asked a start here, in the starts' history: one asker on this
/// machine (a session, an actor, you) by its id, apart from the others.
pub fn asker(id: &str) -> String {
    format!("here:{id}")
}

/// Whether a start may go now, by the starts in the last hour (when, and
/// who asked: another machine by its name, or one asker here, `asker`,
/// each with its own ten): None, or why not.
pub fn admit(history: &[(f64, String)], from: &str, now: f64) -> Option<String> {
    let recent: Vec<&(f64, String)> = history.iter().filter(|(at, _)| now - at < HOUR_MS).collect();
    if recent.len() >= PER_HOUR {
        return Some(format!("this machine starts at most {PER_HOUR} sessions an hour"));
    }
    if recent.iter().filter(|(_, h)| h == from).count() < PER_HOST_HOUR {
        return None;
    }
    let who = match from.strip_prefix("here:") {
        Some("you") => "you ask".to_string(),
        Some(id) => format!("{id} asks"),
        None if from.is_empty() => "this machine asks".to_string(),
        None => format!("{from} asks"),
    };
    Some(format!("{who} for at most {PER_HOST_HOUR} sessions an hour here"))
}

/// The history worth keeping: the last hour's.
pub fn kept(history: &[(f64, String)], now: f64) -> Vec<(f64, String)> {
    history.iter().filter(|(at, _)| now - at < HOUR_MS).cloned().collect()
}

fn trim_end(p: &str) -> &str {
    let t = p.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        p
    } else {
        t
    }
}

/// The folder a request names, as a path on this machine: ~ (home), a
/// project or folder by its id on the map (`known`: [id, path]), or a path
/// the machine reports (a project's or folder's path, or a session's
/// folder, in `reported`), with ~ for home. On Windows (`windows`) a path
/// takes its separator, and paths compare as its file system does, either
/// slash and any case alike. Or why it's none of those.
pub fn folder(asked: &str, home: &str, known: &[(String, String)], reported: &[String], windows: bool) -> Result<String, String> {
    let asked = asked.trim();
    if asked.is_empty() || asked == "~" {
        return Ok(home.to_string());
    }
    if let Some((_, path)) = known.iter().find(|(id, _)| id == asked) {
        return Ok(path.clone());
    }
    let expand = |p: &str| match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) if windows => format!("{}\\{}", trim_end(home), rest.replace('/', "\\")),
        Some(rest) => format!("{}/{rest}", trim_end(home)),
        None => p.to_string(),
    };
    let key = |p: &str| {
        let t = trim_end(&expand(p)).to_string();
        if windows {
            t.replace('/', "\\").to_lowercase()
        } else {
            t
        }
    };
    let want = key(asked);
    let same = |p: &str| key(p) == want;
    if let Some((_, path)) = known.iter().find(|(_, p)| same(p)) {
        return Ok(path.clone());
    }
    if let Some(p) = reported.iter().find(|p| same(p)) {
        return Ok(expand(p));
    }
    Err(format!("{asked} isn't a project or folder on this machine's map: name one by its id or path, or ~"))
}

/// Whether a sandbox may be made of `cwd`: a sandbox is a project folder, so
/// never the home folder (the folder `~` and an empty one name), the root of
/// the disk, or any folder that holds home, since the run may write all of
/// what the sandbox holds. Checked by the machine that runs the start, on
/// the folder as it resolved there.
pub fn sandbox_folder(cwd: &str, home: &str, windows: bool) -> Result<(), String> {
    let parts = |p: &str| -> Vec<String> {
        p.replace('\\', "/").split('/').filter(|x| !x.is_empty() && *x != ".").map(|x| if windows { x.to_lowercase() } else { x.to_string() }).collect()
    };
    let (folder, home_parts) = (parts(cwd), parts(home));
    if folder.len() <= home_parts.len() && home_parts[..folder.len()] == folder[..] {
        return Err("a sandboxed run needs a project folder, not your home folder, the root of the disk or a folder that holds your home folder: name the project's folder".into());
    }
    // Nor one under a hidden folder of home or its Library (AppData): a run
    // that writes there changes what outlives it, as the config and startup
    // files of the machine do.
    if super::delegation::in_dot_or_library(cwd, cwd, home, windows) {
        return Err("a sandboxed run needs a project folder, not one inside a hidden folder (.config, .ssh) of your home folder or its Library: name the project's folder".into());
    }
    Ok(())
}

/// The deepest a folder is looked into for `.claude` folders, and the most
/// entries looked at: past either, the check says so and refuses.
const HOOK_WALK_DEPTH: usize = 16;
const HOOK_WALK_ENTRIES: usize = 50_000;

/// Why a Claude Code run mustn't be sandboxed in `dir`, if a `.claude`
/// folder in it, at any depth, holds skills, agents, commands or plugins
/// that carry hooks: Claude Code loads what is in `.claude` (the folder it
/// starts in, and any under it) whatever `--setting-sources` leaves out, and
/// a hook, or a skill that is a plugin, runs as Claude Code's own process,
/// outside the sandbox that wraps its commands. A `.claude` that is a link is
/// read where it points, and one that points out of the folder is refused,
/// as is a link inside one that does. Past what it will look at (too deep, or
/// too many entries) it refuses in words. Files that don't read are taken as
/// holding none, since the run can't read them either.
pub fn project_hooks(dir: &std::path::Path) -> Option<String> {
    project_hooks_within(dir, HOOK_WALK_DEPTH, HOOK_WALK_ENTRIES)
}

/// `project_hooks` with its limits given.
fn project_hooks_within(dir: &std::path::Path, max_depth: usize, max_entries: usize) -> Option<String> {
    use std::path::{Path, PathBuf};
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut seen = 0usize;
    let shown = |p: &Path| p.strip_prefix(dir).unwrap_or(p).to_string_lossy().into_owned();
    let words = |what: String| {
        format!("{what} carries hooks or is a plugin, which Claude Code runs outside the sandbox that holds its commands, so a sandboxed run isn't started in this folder: use another folder, or take them out")
    };
    let too_much = || {
        format!("this folder has too much to check for hooks in its .claude folders (more than {max_entries} entries, or folders more than {max_depth} deep), so a sandboxed run isn't started in it: use a smaller folder")
    };
    let outside = |what: String| {
        format!("{what} is a link that leads outside the folder, where it can't be checked for hooks, so a sandboxed run isn't started in this folder: take the link out")
    };
    let has_hooks_key = |f: &Path| -> bool {
        let Ok(text) = std::fs::read_to_string(f) else { return false };
        let mut lines = text.lines();
        lines.next().is_some_and(|l| l.trim() == "---") && lines.take_while(|l| l.trim() != "---").any(|l| l.starts_with("hooks:") || l.starts_with("\"hooks\":"))
    };
    let sorted = |d: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(d).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v
    };
    // What one `.claude` folder (read at `at`, shown as `shown_at`) holds.
    let claude = |at: &Path, shown_at: &Path, seen: &mut usize| -> Option<String> {
        let mut todo = vec![(at.to_path_buf(), shown_at.to_path_buf(), 0usize)];
        let mut visited: Vec<PathBuf> = Vec::new();
        while let Some((d, shown_d, depth)) = todo.pop() {
            let real = d.canonicalize().unwrap_or_else(|_| d.clone());
            if visited.contains(&real) {
                continue;
            }
            visited.push(real);
            for f in sorted(&d) {
                *seen += 1;
                if *seen > max_entries {
                    return Some(too_much());
                }
                let name = f.file_name()?.to_string_lossy().into_owned();
                let shown_f = shown_d.join(&name);
                let linked = std::fs::symlink_metadata(&f).is_ok_and(|m| m.file_type().is_symlink());
                if linked && !f.canonicalize().is_ok_and(|t| t.starts_with(&root)) {
                    return Some(outside(shown(&shown_f)));
                }
                if matches!(name.as_str(), "hooks" | "hooks.json" | "plugin.json" | ".claude-plugin") {
                    return Some(words(shown(&shown_f)));
                }
                if f.is_dir() {
                    if depth + 1 > max_depth {
                        return Some(too_much());
                    }
                    todo.push((f, shown_f, depth + 1));
                } else if name.to_lowercase().ends_with(".md") && has_hooks_key(&f) {
                    return Some(words(shown(&shown_f)));
                }
            }
        }
        None
    };
    // Every folder under `dir` is looked into for `.claude`, links to
    // folders aside, and node_modules only two levels down (a package, or a
    // scope's package), which is where a package's own .claude is.
    let mut todo = vec![(dir.to_path_buf(), 0usize, usize::MAX)];
    while let Some((d, depth, nm)) = todo.pop() {
        for f in sorted(&d) {
            seen += 1;
            if seen > max_entries {
                return Some(too_much());
            }
            let name = f.file_name()?.to_string_lossy().into_owned();
            let meta = std::fs::symlink_metadata(&f).ok()?;
            if name == ".claude" {
                let target = if meta.file_type().is_symlink() {
                    let Ok(t) = f.canonicalize() else { continue };
                    if !t.starts_with(&root) {
                        return Some(outside(shown(&f)));
                    }
                    t
                } else {
                    f.clone()
                };
                if target.is_dir() {
                    if let Some(why) = claude(&target, &f, &mut seen) {
                        return Some(why);
                    }
                }
                continue;
            }
            if !meta.is_dir() || name == ".git" {
                continue;
            }
            if depth + 1 > max_depth {
                return Some(too_much());
            }
            let nm = if name == "node_modules" { 2 } else { nm };
            if nm == 0 {
                continue;
            }
            todo.push((f, depth + 1, if nm == usize::MAX { nm } else { nm - 1 }));
        }
    }
    None
}

/// Whether a run's output says the app isn't signed in, for an app whose
/// runs sign in with the token you gave them (Claude Code).
pub fn signed_out(app: &str, log: &str) -> bool {
    apps::of(app).and_then(|w| w.sign_in).is_some_and(|s| (s.signed_out)(log))
}

/// The session a run's output names, as the app's adapter reads it:
/// Claude Code's session_id (its first stream-json event, or its result),
/// Codex's thread_id (thread.started).
pub fn session_of(app: &str, log: &str) -> Option<String> {
    let id = (apps::of(app)?.session_of?)(log)?;
    Some(format!("{app}:{id}"))
}

/// The last turn that ended in these lines of a run's output (a Claude Code
/// stream's result, Codex's last message at a turn.completed, or the
/// result Grok Build prints), if any ended, as the app's adapter reads it.
pub fn finish(app: &str, log: &str) -> Option<Finish> {
    (apps::of(app)?.finish?)(log)
}

/// Whether the session called XenoNet's send tool in these lines of its
/// output: it told someone what it found itself.
pub fn reported(app: &str, log: &str) -> bool {
    apps::of(app).and_then(|w| w.reported).is_some_and(|r| r(log))
}

/// Whether a final message asks something and waits: a question in its last
/// lines, or an offer to go on when told to.
fn asks(text: &str) -> bool {
    let tail: String = text.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
    tail.contains('?') || crate::re!(r"(?i)(?-u:\b)(want me to|shall i|should i|let me know|say so and i|tell me (if|whether|which))(?-u:\b)").is_match(&tail)
}

/// What a turn came to, from its end: failed, denied a tool, asked
/// something, finished (and reported back itself, or only said so in its
/// last message), or silent when it left nothing.
pub fn outcome(f: Option<&Finish>, reported: bool) -> &'static str {
    let Some(f) = f else { return "silent" };
    if f.error {
        "failed"
    } else if !f.denied.is_empty() {
        "denied"
    } else if f.text.is_empty() {
        "silent"
    } else if asks(&f.text) {
        "asked"
    } else if reported {
        "reported"
    } else {
        "finished"
    }
}

/// The most of a session's last words a notice carries.
pub const LAST_MAX: usize = 1200;

/// The words clipped for a notice, to whole characters.
pub fn clipped(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= LAST_MAX {
        return t.to_string();
    }
    let head: String = t.chars().take(LAST_MAX).collect();
    format!("{}…", head.trim_end())
}

// What the requester is told of an answer, and the Allow card's warning,
// are the data model's: the routes' made-up network says them too.
pub use xenonet_model::starts::{answer_words, ENV_WARNING};

/// Where your computer's credential store keeps the long-lived Claude Code
/// token XenoNet's runs sign in with (`claude setup-token`'s), which goes
/// to them alone as CLAUDE_CODE_OAUTH_TOKEN. The store takes names without
/// quotes or backslashes (`secret_store::TARGET`).
pub const SIGN_IN_TARGET: &str = "XenoNet/Claude Code runs";

/// A token as you pasted it, trimmed, or why it isn't one: what `claude
/// setup-token` prints, one line with no spaces.
pub fn sign_in_token(pasted: &str) -> Result<String, String> {
    let t = pasted.trim();
    if t.is_empty() {
        return Err("paste the token claude setup-token printed".into());
    }
    if !t.starts_with("sk-ant-") || t.len() < 20 || t.len() > 400 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("that isn't a Claude Code token: run claude setup-token, and paste the line it prints".into());
    }
    Ok(t.to_string())
}

/// Why a start that ended or was refused needs a sign-in, in words.
pub fn sign_in_words(host: &str) -> String {
    format!("Claude Code on {host} needs a sign-in for XenoNet's runs: run xenonet runs sign-in there")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_says_its_app_mode_folder_and_prompt() {
        xenonet_platforms::install();
        let r = request(&json!({ "prompt": "  Fix the login form  " })).unwrap();
        assert_eq!(r, Request { folder: "~".into(), app: "claude".into(), mode: None, prompt: "Fix the login form".into(), delegate: false, confine: ConfineAsk::Unset });
        assert_eq!(request(&json!({ "app": "grok", "mode": "plan", "folder": "project:atlas", "prompt": "x" })).unwrap().mode.as_deref(), Some("plan"));
        assert_eq!(request(&json!({ "mode": "manual", "prompt": "x" })).unwrap().mode, None, "manual is the person's own settings");
        assert_eq!(request(&json!({ "mode": "bypassPermissions", "prompt": "x" })), Err("XenoNet never starts a session that bypasses its permission checks".into()));
        assert!(request(&json!({ "mode": "yolo", "prompt": "x" })).is_err());
        assert!(request(&json!({ "app": "codex", "mode": "plan", "prompt": "x" })).is_err(), "Codex takes its own config's");
        assert_eq!(
            request(&json!({ "app": "opencode", "prompt": "x" })),
            Err("XenoNet starts Claude Code (claude), Codex (codex), Grok Build (grok) or Hermes Agent (hermes-agent), not opencode".into())
        );
        // Hermes starts over ACP alone, under its own settings.
        assert_eq!(request(&json!({ "app": "hermes-agent", "prompt": "x" })).unwrap().app, "hermes-agent");
        assert_eq!(request(&json!({ "app": "hermes-agent", "mode": "plan", "prompt": "x" })), Err("Hermes Agent starts under its own settings, so no mode".into()));
        assert_eq!(
            request(&json!({ "app": "codex", "mode": "plan", "prompt": "x" })),
            Err("Codex runs with its own config.toml's sandbox and approvals: leave the mode out".into())
        );
        assert_eq!(request(&json!({ "prompt": " " })), Err("say what the new session should do: a prompt".into()));
        assert!(request(&json!({ "prompt": "x".repeat(PROMPT_MAX + 1) })).is_err());
        let back = request(&r.to_json()).unwrap();
        assert_eq!(back, r);
        // Delegated work says so, and a plain request doesn't carry the word.
        let d = request(&json!({ "prompt": "x", "delegate": true })).unwrap();
        assert!(d.delegate && d.to_json()["delegate"] == json!(true));
        assert!(!r.delegate && r.to_json().get("delegate").is_none());
        assert_eq!(request(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn a_request_says_how_confined_the_run_is_and_where_it_may_reach() {
        xenonet_platforms::install();
        let ask = |v: Value| request(&v).unwrap().confine;
        assert_eq!(ask(json!({ "prompt": "x" })), ConfineAsk::Unset);
        assert_eq!(ask(json!({ "prompt": "x", "confine": "off" })), ConfineAsk::Off);
        assert_eq!(ask(json!({ "prompt": "x", "confine": "vendor" })), ConfineAsk::Vendor(vec![]));
        assert_eq!(ask(json!({ "prompt": "x", "confine": "cell" })), ConfineAsk::Cell);
        assert_eq!(
            ask(json!({ "prompt": "x", "confine": "vendor", "domains": ["GitHub.com", "github.com", "*.npmjs.org"] })),
            ConfineAsk::Vendor(vec!["*.npmjs.org".into(), "github.com".into()])
        );
        assert_eq!(ask(json!({ "prompt": "x", "domains": ["github.com"] })), ConfineAsk::Vendor(vec!["github.com".into()]), "domains alone ask for it");
        for bad in ["localhost", "https://github.com", "a b.com", "-x.com", "git hub.com", ""] {
            assert!(request(&json!({ "prompt": "x", "domains": [bad] })).is_err(), "{bad}");
        }
        for bad in [json!("strict"), json!("Cell"), json!(true), json!(["vendor"])] {
            assert!(request(&json!({ "prompt": "x", "confine": bad })).is_err(), "{bad}");
        }
        assert!(request(&json!({ "prompt": "x", "confine": "off", "domains": ["github.com"] })).is_err());
        assert!(request(&json!({ "prompt": "x", "confine": "cell", "domains": ["github.com"] })).is_err(), "a cell takes no domains");
        // A request's own wall is never taken: the wall is the signer's.
        for level in ["cell", "vendor", "off"] {
            let why = request(&json!({ "prompt": "x", "confine": level, "wall": "wall_nova_0123456789abcdef" })).unwrap_err();
            assert!(why.contains("names no wall"), "{why}");
        }
        // The older words still mean the same, and a clash is refused.
        assert_eq!(ask(json!({ "prompt": "x", "sandbox": false })), ConfineAsk::Off);
        assert_eq!(ask(json!({ "prompt": "x", "sandbox": true })), ConfineAsk::Vendor(vec![]));
        assert_eq!(ask(json!({ "prompt": "x", "sandbox": { "domains": ["github.com"] } })), ConfineAsk::Vendor(vec!["github.com".into()]));
        assert!(request(&json!({ "prompt": "x", "sandbox": false, "domains": ["github.com"] })).is_err());
        assert!(request(&json!({ "prompt": "x", "sandbox": false, "confine": "vendor" })).is_err());
        // What is known says so on the wire, and comes back as it went.
        for (v, level) in [
            (json!({ "prompt": "x", "domains": ["github.com"] }), "vendor"),
            (json!({ "prompt": "x", "confine": "off" }), "off"),
            (json!({ "prompt": "x", "confine": "cell" }), "cell"),
        ] {
            let r = request(&v).unwrap();
            assert_eq!(r.to_json()["confine"], json!(level));
            assert_eq!(request(&r.to_json()).unwrap(), r);
        }
        assert!(request(&json!({ "prompt": "x" })).unwrap().to_json().get("confine").is_none());
    }

    #[test]
    fn an_app_without_a_sandbox_refuses_a_sandboxed_start_in_words() {
        xenonet_platforms::install();
        let plan = |d: &'static [String]| xenonet_adapter::delivery::Plan { folder: "/w/atlas", home: "/h", domains: d, port: "47840", bin: "xenonet" };
        let r = |app: &str| request(&json!({ "app": app, "prompt": "x", "confine": "vendor" })).unwrap();
        let none: &'static [String] = &[];
        assert!(sandbox_plan(&r("claude"), &plan(none), false).unwrap().contains("no network"));
        assert!(sandbox_plan(&r("claude"), &plan(none), true).unwrap_err().contains("WSL2"));
        for app in ["grok", "hermes-agent"] {
            let why = sandbox_plan(&r(app), &plan(none), false).unwrap_err();
            assert!(why.contains("has no sandbox XenoNet can drive"), "{app}: {why}");
        }
        assert!(sandbox_plan(&r("codex"), &plan(none), false).unwrap().contains("no network"));
        let some: &'static [String] = Box::leak(vec!["github.com".to_string()].into_boxed_slice());
        assert!(sandbox_plan(&r("codex"), &plan(some), false).unwrap_err().contains("named domains"));
        assert!(sandbox_plan(&r("claude"), &plan(some), false).unwrap().contains("github.com"));
    }

    #[test]
    fn a_sandboxed_turn_runs_in_the_apps_sandbox_and_loads_none_of_the_repos() {
        xenonet_platforms::install();
        use xenonet_adapter::delivery::{Plan, SandboxFiles};
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let r = |app: &str| Request { folder: "~".into(), app: app.into(), mode: None, prompt: "x".into(), delegate: false, confine: ConfineAsk::Vendor(vec![]) };
        let plan = Plan { folder: "/w/atlas", home: "/h", domains: &[], port: "47840", bin: "xenonet" };
        let files = SandboxFiles { plan, settings: "/s/runs/a.settings.json", mcp: "/s/runs/a.mcp.json", server: "xenonet" };
        let (_, args) = command(&r("claude"), id, "/w/atlas", Some("/s/listen.json"), "xenonet", None).unwrap();
        let args = sandboxed("claude", &files, args).unwrap();
        let at = args.iter().position(|a| a == "--settings").unwrap();
        assert_eq!(args[at + 1], "/s/runs/a.settings.json", "the run's own file, not the listening one: {args:?}");
        assert!(!args.iter().any(|a| a == "/s/listen.json"));
        assert!(args.contains(&"--setting-sources=".to_string()) && args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(args[args.iter().position(|a| a == "--mcp-config").unwrap() + 1], "/s/runs/a.mcp.json");
        let (_, codex) = command(&r("codex"), id, "/w/atlas", None, "xenonet", None).unwrap();
        let codex = sandboxed("codex", &files, codex).unwrap();
        assert_eq!(codex[..5], ["exec", "--sandbox", "workspace-write", "--ignore-user-config", "--ignore-rules"]);
        assert!(codex.contains(&"approval_policy=\"never\"".to_string()), "{codex:?}");
        assert!(codex.contains(&"sandbox_workspace_write.network_access=false".to_string()), "{codex:?}");
        assert_eq!(codex.last().map(String::as_str), Some("-"));
        assert!(sandboxed("grok", &files, vec![]).is_err());
    }

    #[test]
    fn a_run_you_didnt_start_reaches_only_the_domains_you_allowed() {
        let list = |d: &[&str]| allowed_domains(&d.iter().map(ToString::to_string).collect::<Vec<_>>()).unwrap();
        let ask = |d: &[&str]| d.iter().map(ToString::to_string).collect::<Vec<_>>();
        let allowed = list(&["GitHub.com", "*.npmjs.org", "github.com"]);
        assert_eq!(allowed, ["*.npmjs.org", "github.com"]);
        assert!(domains_within(&ask(&["github.com", "registry.npmjs.org", "*.registry.npmjs.org"]), &allowed).is_ok());
        assert!(domains_within(&[], &[]).is_ok(), "no domain asked, none to allow");
        for bad in ["evil.com", "api.github.com", "npmjs.org", "*.npmjs.org.evil.com", "xnpmjs.org"] {
            assert!(domains_within(&ask(&[bad]), &allowed).is_err(), "{bad}");
        }
        assert!(domains_within(&ask(&["*.npmjs.org"]), &allowed).is_ok(), "the entry itself");
        assert!(domains_within(&ask(&["*.github.com"]), &allowed).is_err(), "a wildcard is not within a name");
        let why = domains_within(&ask(&["github.com"]), &[]).unwrap_err();
        assert!(why.contains("github.com") && why.contains("xenonet runs domains"), "{why}");
        assert!(allowed_domains(&["localhost".to_string()]).is_err());
    }

    #[test]
    fn a_sandbox_is_a_project_folder_never_the_home_folder_the_root_or_what_holds_them() {
        for (folder, home) in [("/home/me", "/home/me"), ("/home/me/", "/home/me"), ("/home", "/home/me"), ("/", "/home/me"), ("/Users", "/Users/me")] {
            let why = sandbox_folder(folder, home, false).unwrap_err();
            assert!(why.contains("project folder") && why.contains("home folder"), "{folder}: {why}");
        }
        assert!(sandbox_folder("C:\\Users\\Me", "c:/users/me/", true).is_err());
        assert!(sandbox_folder("C:\\", "C:\\Users\\Me", true).is_err());
        assert!(sandbox_folder("/home/me/atlas", "/home/me", false).is_ok());
        assert!(sandbox_folder("/home/mel", "/home/me", false).is_ok(), "a sibling that starts the same is not home");
        assert!(sandbox_folder("/srv/atlas", "/home/me", false).is_ok());
        // Under a dot-folder or Library/AppData in home, where a change outlives the run.
        for folder in ["/home/me/.ssh", "/home/me/.config/atlas", "/home/me/.local/share/x", "/Users/me/Library/LaunchAgents", "/Users/me/Library/atlas"] {
            let home = if folder.starts_with("/Users") { "/Users/me" } else { "/home/me" };
            let why = sandbox_folder(folder, home, false).unwrap_err();
            assert!(why.contains("hidden folder") && why.contains("Library"), "{folder}: {why}");
        }
        assert!(sandbox_folder("C:\\Users\\Me\\AppData\\Roaming\\x", "C:\\Users\\Me", true).is_err());
        assert!(sandbox_folder("/home/me/work/.github", "/home/me", false).is_ok(), "a dot-folder further down a project is fine");
        assert!(sandbox_folder("/srv/.cache/atlas", "/home/me", false).is_ok(), "and one outside home");
    }

    #[test]
    fn a_project_with_skills_or_agents_that_carry_hooks_is_not_sandboxed_for_claude_code() {
        let base = std::env::temp_dir().join(format!("xenonet-hooks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let write = |name: &str, text: &str| {
            let f = base.join(name);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        // Skills and agents without hooks load beside the sandbox harmlessly.
        write(".claude/skills/review/SKILL.md", "---\nname: review\ndescription: Reads a diff\n---\nLook at the diff.\n");
        write(".claude/agents/helper.md", "---\nname: helper\n---\nHelp.\n");
        write("README.md", "hooks: are described here\n");
        assert_eq!(project_hooks(&base), None, "no hooks in them");
        assert_eq!(project_hooks(&base.join("nowhere")), None);
        for (name, text) in [
            (".claude/skills/fmt/SKILL.md", "---\nname: fmt\nhooks:\n  PostToolUse:\n    - command: ./fmt.sh\n---\nFormat.\n"),
            (".claude/agents/guard.md", "---\nname: guard\ntools: Read\nhooks:\n  Stop: []\n---\nGuard.\n"),
            (".claude/commands/ship.md", "---\nhooks:\n  Stop: []\n---\nShip.\n"),
            (".claude/skills/tool/hooks/hooks.json", "{}"),
            (".claude/skills/pack/.claude-plugin/plugin.json", "{\"name\":\"pack\"}"),
            (".claude/skills/pack2/plugin.json", "{\"name\":\"pack2\"}"),
        ] {
            let _ = std::fs::remove_dir_all(base.join(".claude"));
            write(".claude/skills/review/SKILL.md", "---\nname: review\n---\nok\n");
            write(name, text);
            let why = project_hooks(&base).unwrap_or_default();
            assert!(why.starts_with(".claude/"), "{name}: {why}");
            assert!(why.contains("hooks") && why.contains("outside the sandbox"), "{name}: {why}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    #[cfg(unix)]
    fn hooks_are_found_in_every_claude_folder_at_any_depth_and_in_a_linked_one() {
        let base = std::env::temp_dir().join(format!("xenonet-hooks-deep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let write = |name: &str, text: &str| {
            let f = base.join(name);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        let hooked = "---\nname: x\nhooks:\n  Stop: []\n---\nx\n";
        let plain = "---\nname: x\n---\nx\n";
        // Plain skills, nested and not, run.
        write(".claude/skills/review/SKILL.md", plain);
        write("pkg/web/.claude/skills/fmt/SKILL.md", plain);
        write(".claude/skills/review/scripts/run.sh", "echo hi\n");
        assert_eq!(project_hooks(&base), None, "plain skills in nested folders run");
        for (name, text, shown) in [
            (".claude/commands/sub/deeper/x.md", hooked, ".claude/commands/sub/deeper/x.md"),
            ("pkg/web/.claude/skills/fmt/SKILL.md", hooked, "pkg/web/.claude/skills/fmt/SKILL.md"),
            ("pkg/web/.claude/agents/team/guard.md", hooked, "pkg/web/.claude/agents/team/guard.md"),
            (".claude/plugins/tools/plugin.json", "{}", ".claude/plugins/tools/plugin.json"),
            (".claude/skills/review/refs/hooks/on-stop.sh", "echo\n", ".claude/skills/review/refs/hooks"),
        ] {
            write(name, text);
            let why = project_hooks(&base).unwrap_or_default();
            assert!(why.starts_with(shown) && why.contains("outside the sandbox"), "{name}: {why}");
            let _ = std::fs::remove_file(base.join(name));
            let _ = std::fs::remove_dir_all(base.join(".claude/skills/review/refs"));
            let _ = std::fs::remove_dir_all(base.join(".claude/plugins"));
            let _ = std::fs::remove_dir_all(base.join(".claude/commands"));
            let _ = std::fs::remove_dir_all(base.join("pkg/web/.claude/agents"));
            write("pkg/web/.claude/skills/fmt/SKILL.md", plain);
            assert_eq!(project_hooks(&base), None, "{name}: back to plain");
        }
        // A `.claude` that is a link: to a folder outside is a refusal, to one inside is read.
        let outside = std::env::temp_dir().join(format!("xenonet-hooks-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(outside.join("skills/ok")).unwrap();
        std::fs::write(outside.join("skills/ok/SKILL.md"), plain).unwrap();
        std::fs::create_dir_all(base.join("linked")).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("linked/.claude")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.starts_with("linked/.claude") && why.contains("outside the folder"), "{why}");
        std::fs::remove_file(base.join("linked/.claude")).unwrap();
        write("shared/skills/ok/SKILL.md", plain);
        std::os::unix::fs::symlink(base.join("shared"), base.join("linked/.claude")).unwrap();
        assert_eq!(project_hooks(&base), None, "a link to a plain folder inside runs");
        write("shared/commands/x.md", hooked);
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.starts_with("linked/.claude/commands/x.md"), "a link to a hooked folder inside is read: {why}");
        // A link inside a `.claude` that leaves the folder is a refusal too.
        std::fs::remove_dir_all(base.join("shared/commands")).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("shared/skills/away")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.contains("away") && why.contains("outside the folder"), "{why}");
        std::fs::remove_file(base.join("shared/skills/away")).unwrap();
        assert_eq!(project_hooks(&base), None);
        // Past what it will look at, it says so and refuses.
        write("a/b/c/d/e/f/g.txt", "x");
        let why = project_hooks_within(&base, 3, 100_000).unwrap_or_default();
        assert!(why.contains("too much to check"), "deeper than it looks: {why}");
        let why = project_hooks_within(&base, 16, 5).unwrap_or_default();
        assert!(why.contains("too much to check"), "more files than it looks at: {why}");
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_sandbox_that_reads_the_whole_disk_is_enough_for_the_person_alone() {
        xenonet_platforms::install();
        assert!(sandbox_enough("codex", true).is_ok());
        let why = sandbox_enough("codex", false).unwrap_err();
        assert!(why.contains("Codex") && why.contains("anyone but you"), "{why}");
        assert!(sandbox_enough("claude", false).is_ok());
        assert!(sandbox_enough("claude", true).is_ok());
    }

    #[test]
    fn a_sandboxed_run_is_told_so_in_its_first_prompt() {
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(
            framed(&you, Some("desk"), "Go.", None, Some("It reaches nothing.")),
            "[XenoNet] Started by you, at desk's console.\n\n[XenoNet] This run is sandboxed. It reaches nothing.\n\nGo."
        );
    }

    #[test]
    fn the_first_prompt_says_who_asked() {
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(framed(&you, Some("desk"), "Go.", None, None), "[XenoNet] Started by you, at desk's console.\n\nGo.");
        assert_eq!(framed(&you, None, "Go.", None, None), "[XenoNet] Started by you, at this machine's console.\n\nGo.");
        let s = json!({ "kind": "session", "id": "claude:aaaaaaaa-0001", "title": "Fix «it»\n\u{1b}[31mnow" });
        assert_eq!(
            framed(&s, Some("desk"), "Go.", None, None),
            "[XenoNet] desk says this was asked by the session «Fix \"it\" now» (claude:aaaaaaaa-0001). It came from an agent, not from your user.\n\nGo."
        );
        let a = json!({ "kind": "actor", "id": "actor:muse" });
        assert!(framed(&a, None, "Go.", None, None).starts_with("[XenoNet] This was asked by the actor «actor:muse» (actor:muse) on this machine."));
    }

    #[test]
    fn a_delegated_start_says_your_user_delegated_it_and_what_stays_off_limits() {
        let s = json!({ "kind": "session", "id": "claude:aaaaaaaa-0001", "title": "Update" });
        let text = framed(&s, Some("desk"), "Go.", Some(("desk", "2026-11-01")), None);
        assert!(text.starts_with("[XenoNet] desk says this was asked by the session «Update» (claude:aaaaaaaa-0001). Your user delegated it: they put this machine on standing delegation at desk's console on 2026-11-01, which XenoNet here checked; the asker can't claim it.\n\n"));
        assert!(text.ends_with(&format!("{DELEGATED_RULES}\n\nGo.")));
        assert!(!text.contains("not from your user"), "the agent line is gone: the delegation is your user's");
        // A start by you says what it said, delegation or not.
        let you = json!({ "kind": "you", "id": "actor:you" });
        assert_eq!(framed(&you, Some("desk"), "Go.", Some(("desk", "2026-11-01")), None), "[XenoNet] Started by you, at desk's console.\n\nGo.");
    }

    #[test]
    fn each_app_starts_its_own_way_and_the_prompt_never_goes_as_an_argument() {
        xenonet_platforms::install();
        let id = "0190d3e0-0000-7000-8000-00000000ab01";
        let r = |app: &str, mode: Option<&str>| Request {
            folder: "~".into(),
            app: app.into(),
            mode: mode.map(str::to_string),
            prompt: "-rm -rf".into(),
            delegate: false,
            confine: ConfineAsk::Unset,
        };
        let (app, args) = command(&r("claude", Some("plan")), id, "/w/atlas", Some("/s/listen.json"), "xenonet", None).unwrap();
        assert_eq!(app, "claude");
        assert_eq!(
            args,
            [
                "-p",
                "--session-id",
                id,
                "--output-format",
                "stream-json",
                "--verbose",
                "--settings",
                "/s/listen.json",
                "--permission-mode",
                "plan",
                "--permission-prompt-tool",
                "mcp__xenonet__permission"
            ]
        );
        assert_eq!(
            command(&r("claude", None), id, "/w", None, "xenonet-dev", None).unwrap().1,
            ["-p", "--session-id", id, "--output-format", "stream-json", "--verbose", "--permission-prompt-tool", "mcp__xenonet-dev__permission"]
        );
        assert_eq!(command(&r("codex", None), id, "/w/atlas", None, "xenonet", None).unwrap().1, ["exec", "--json", "-C", "/w/atlas", "-"]);
        assert_eq!(
            command(&r("grok", Some("manual")), id, "/w", None, "xenonet", Some("/s/runs/p.txt")).unwrap().1,
            ["--prompt-file", "/s/runs/p.txt", "-s", id, "--output-format", "json", "--permission-mode", "default"]
        );
        for app in apps().iter().filter(|a| !acp_only(a)).map(|a| a.id.as_str()) {
            assert!(!command(&r(app, None), id, "/w", None, "xenonet", Some("/p")).unwrap().1.iter().any(|a| a.contains("rm -rf")), "{app}");
        }
        assert!(prompt_in_file("grok") && !prompt_in_file("claude") && !prompt_in_file("codex"));
        assert!(command(&r("opencode", None), id, "/w", None, "xenonet", None).is_err(), "no start in its adapter");
    }

    #[test]
    fn twenty_starts_an_hour_here_and_ten_from_one_machine() {
        let now = 10_000_000.0;
        let mut h: Vec<(f64, String)> = (0..10).map(|i| (now - f64::from(i) * 1000.0, "desk".to_string())).collect();
        assert_eq!(admit(&h, "desk", now).as_deref(), Some("desk asks for at most 10 sessions an hour here"));
        assert_eq!(admit(&h, "", now), None, "this machine has its own ten");
        h.extend((0..10).map(|_| (now - 5.0, String::new())));
        assert_eq!(admit(&h, "studio", now).as_deref(), Some("this machine starts at most 20 sessions an hour"));
        assert_eq!(admit(&h, "studio", now + HOUR_MS), None, "an hour on");
        assert_eq!(kept(&h, now + HOUR_MS - 6000.0).len(), 16, "six of desk's and this machine's ten");
        // Each asker here has its own ten: one session's don't hold
        // another's, or yours.
        let a = asker("claude:a");
        let mine: Vec<(f64, String)> = (0..10).map(|_| (now, a.clone())).collect();
        assert_eq!(admit(&mine, &a, now).as_deref(), Some("claude:a asks for at most 10 sessions an hour here"));
        assert_eq!(admit(&mine, &asker("claude:b"), now), None);
        assert_eq!(admit(&mine, &asker("you"), now), None);
        let yours: Vec<(f64, String)> = (0..10).map(|_| (now, asker("you"))).collect();
        assert_eq!(admit(&yours, &asker("you"), now).as_deref(), Some("you ask for at most 10 sessions an hour here"));
    }

    #[test]
    fn a_folder_is_home_a_map_id_or_a_path_the_machine_reports() {
        let known = vec![("project:atlas".to_string(), "/home/jo/src/atlas".to_string()), ("zone:notes".to_string(), "/home/jo/notes".to_string())];
        let reported = vec!["~/work/ledger".to_string()];
        let f = |a: &str| folder(a, "/home/jo", &known, &reported, false);
        assert_eq!(f("~"), Ok("/home/jo".into()));
        assert_eq!(f(""), Ok("/home/jo".into()));
        assert_eq!(f("project:atlas"), Ok("/home/jo/src/atlas".into()));
        assert_eq!(f("~/src/atlas/"), Ok("/home/jo/src/atlas".into()));
        assert_eq!(f("/home/jo/notes"), Ok("/home/jo/notes".into()));
        assert_eq!(f("~/work/ledger"), Ok("/home/jo/work/ledger".into()));
        assert_eq!(f("/home/jo/work/ledger"), Ok("/home/jo/work/ledger".into()));
        assert_eq!(f("/etc"), Err("/etc isn't a project or folder on this machine's map: name one by its id or path, or ~".into()));
        assert!(f("~/src").is_err(), "a folder above a project isn't one");
        assert!(f("/HOME/jo/notes").is_err(), "case counts, off Windows");
        // On Windows: its separator, and either slash and any case alike.
        let known = vec![("project:atlas".to_string(), "C:\\Users\\jo\\src\\Atlas".to_string())];
        let reported = vec!["~\\work\\Ledger".to_string()];
        let w = |a: &str| folder(a, "C:\\Users\\jo", &known, &reported, true);
        assert_eq!(w("~/work/ledger"), Ok("C:\\Users\\jo\\work\\Ledger".into()));
        assert_eq!(w("c:/users/jo/src/atlas/"), Ok("C:\\Users\\jo\\src\\Atlas".into()));
        assert_eq!(w("~\\SRC\\atlas"), Ok("C:\\Users\\jo\\src\\Atlas".into()));
        assert!(w("~/src").is_err());
    }

    #[test]
    fn a_run_says_its_session_and_whether_claude_code_is_signed_out() {
        xenonet_platforms::install();
        let claude = "{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"0190d3e0-0000-7000-8000-00000000ab02\"}\n{\"type\":\"result\"}";
        assert_eq!(session_of("claude", claude).as_deref(), Some("claude:0190d3e0-0000-7000-8000-00000000ab02"));
        let codex = "{\"type\":\"thread.started\",\"thread_id\":\"019a0000-aaaa-bbbb-cccc-000000000001\"}";
        assert_eq!(session_of("codex", codex).as_deref(), Some("codex:019a0000-aaaa-bbbb-cccc-000000000001"));
        assert_eq!(session_of("claude", "not json\n{\"session_id\":\"../x\"}"), None);
        assert!(signed_out("claude", "Error: OAuth session expired. Please run /login"));
        assert!(signed_out("claude", "Not logged in · Please run /login"));
        assert!(!signed_out("claude", "{\"type\":\"result\",\"subtype\":\"success\"}"));
        assert!(!signed_out("codex", "Not logged in"), "only an app whose runs XenoNet signs in");
    }

    #[test]
    fn the_answers_in_words() {
        let a = json!({ "session": "claude:0190d3e0-0000-7000-8000-00000000ab01", "folder": "~/src/atlas" });
        assert_eq!(
            answer_words("started", "tower", &a),
            "[XenoNet] Started claude:0190d3e0-0000-7000-8000-00000000ab01 on tower, in ~/src/atlas. Message it as you would any session."
        );
        assert_eq!(
            answer_words("refused", "tower", &json!({ "reason": "only you start sessions here" })),
            "[XenoNet] No session was started on tower: only you start sessions here."
        );
        let ended = json!({ "session": "claude:x", "exit": 0, "minutes": 4 });
        assert_eq!(
            answer_words("ended", "this machine", &ended),
            "[XenoNet] claude:x ended after 4 min (exit 0). That machine's XenoNet is older and can't say whether it finished or stopped to ask something, so don't take it for done: update XenoNet there (xenonet update tools, then xenonet update service)."
        );
        let ended = |outcome: &str, extra: Value| {
            let mut a = json!({ "session": "claude:x", "exit": 0, "minutes": 5, "outcome": outcome, "last": "All set." });
            a.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            answer_words("ended", "tower", &a)
        };
        assert_eq!(ended("asked", json!({ "last": "Want me to go ahead?" })), "[XenoNet] claude:x on tower stopped to ask something, and has done nothing more after 5 min. Answer it by messaging it: it takes up from there.\nIts last words: «Want me to go ahead?»");
        assert_eq!(ended("finished", json!({})), "[XenoNet] claude:x on tower finished after 5 min, but sent no report of its own.\nIts last words: «All set.»");
        assert_eq!(ended("reported", json!({})), "[XenoNet] claude:x on tower finished after 5 min and reported back itself.\nIts last words: «All set.»");
        assert_eq!(
            ended("denied", json!({ "denied": ["Bash", "Edit"] })),
            "[XenoNet] claude:x on tower stopped after 5 min for want of permission to use Bash, Edit, which it was denied.\nIts last words: «All set.»"
        );
        assert_eq!(
            ended("failed", json!({ "exit": 1, "reason": "Claude Code needs a sign-in" })),
            "[XenoNet] claude:x on tower failed after 5 min (exit 1): Claude Code needs a sign-in.\nIts last words: «All set.»"
        );
        assert_eq!(ended("silent", json!({ "last": "" })), "[XenoNet] claude:x on tower ended after 5 min without a word: it sent no report and left no last message (exit 0).");
        assert_eq!(sign_in_words("tower"), "Claude Code on tower needs a sign-in for XenoNet's runs: run xenonet runs sign-in there");
    }

    #[test]
    fn a_sign_in_token_is_what_claude_setup_token_prints() {
        let token = format!("sk-ant-oat01-{}", "a".repeat(80));
        assert_eq!(sign_in_token(&format!("  {token}\n")), Ok(token.clone()));
        assert_eq!(sign_in_token(""), Err("paste the token claude setup-token printed".into()));
        assert!(sign_in_token("hunter2").is_err());
        assert!(sign_in_token(&format!("{token} extra")).is_err(), "one line, no spaces");
    }

    #[test]
    fn a_turns_end_is_read_from_the_runs_output() {
        xenonet_platforms::install();
        let claude = r#"{"type":"system","subtype":"init","session_id":"s"}
{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}
{"type":"result","subtype":"success","is_error":false,"result":"Want me to go ahead? I'll stop here.","permission_denials":[]}
"#;
        let f = finish("claude", claude).unwrap();
        assert_eq!(f, Finish { text: "Want me to go ahead? I'll stop here.".into(), error: false, denied: vec![] });
        assert_eq!(outcome(Some(&f), false), "asked");
        assert!(finish("claude", "{\"type\":\"assistant\"}\n").is_none(), "no end yet");
        assert!(finish("claude", r#"{"result":"bare"}"#).is_none() && finish("grok", r#"{"result":"bare"}"#).is_some_and(|f| f.text == "bare"), "Grok Build prints a bare result");
        let denied = r#"{"type":"result","subtype":"success","is_error":false,"result":"I could not run it.","permission_denials":[{"tool_name":"Bash"},{"tool_name":"Edit"}]}"#;
        let d = finish("claude", denied).unwrap();
        assert_eq!((d.denied.clone(), outcome(Some(&d), true)), (vec!["Bash".to_string(), "Edit".to_string()], "denied"));
        let failed = finish("claude", r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":""}"#).unwrap();
        assert_eq!(outcome(Some(&failed), false), "failed");
        let done = Finish { text: "Updated to 0.6.1.".into(), error: false, denied: vec![] };
        assert_eq!((outcome(Some(&done), false), outcome(Some(&done), true)), ("finished", "reported"));
        assert_eq!(outcome(Some(&Finish { text: String::new(), error: false, denied: vec![] }), false), "silent");
        assert_eq!(outcome(None, true), "silent");
        for q in ["Let me know which one.", "Shall I continue", "ok, say so and I'll do it"] {
            assert!(asks(q), "{q}");
        }
        assert!(!asks("Done. The service restarted."));
    }

    #[test]
    fn codex_ends_a_turn_with_its_last_message() {
        xenonet_platforms::install();
        let log = r#"{"type":"thread.started","thread_id":"t"}
{"type":"item.completed","item":{"id":"1","type":"agent_message","text":"Which folder?"}}
{"type":"turn.completed"}
"#;
        assert_eq!(finish("codex", log), Some(Finish { text: "Which folder?".into(), error: false, denied: vec![] }));
        assert_eq!(finish("codex", r#"{"type":"turn.failed","error":{"message":"rate limited"}}"#).map(|f| (f.text, f.error)), Some(("rate limited".into(), true)));
    }

    #[test]
    fn reporting_back_means_calling_the_send_tool() {
        xenonet_platforms::install();
        let sent = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__xenonet__send","input":{"to":"claude:a","text":"done"}}]}}"#;
        assert!(reported("claude", sent));
        assert!(reported("claude", &sent.replace("mcp__xenonet__send", "mcp__xenonet-dev__send")));
        assert!(!reported("claude", &sent.replace("__send", "__inbox")));
        assert!(!reported("claude", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"mcp__xenonet__send"}]}}"#));
        assert!(reported("codex", r#"{"type":"item.completed","item":{"type":"mcp_tool_call","server":"xenonet","tool":"send"}}"#));
    }

    #[test]
    fn last_words_are_clipped_to_whole_characters() {
        assert_eq!(clipped("  hi  "), "hi");
        let long = "é".repeat(LAST_MAX + 5);
        assert_eq!(clipped(&long).chars().count(), LAST_MAX + 1);
    }
}
