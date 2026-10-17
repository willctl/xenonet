//! xenonet actor, and xenonet actors for its list (cli/commands/actor.js):
//! actors on this machine (core/actors; docs/design/actors/interface.md,
//! section 3). Anyone may list them or look at one. You, or a program's
//! setup script, ask to register one, which waits for you to allow it in
//! XenoNet's window. An actor reports with its token, from
//! XENONET_ACTOR_TOKEN_FILE (or XENONET_ACTOR_TOKEN), never from an
//! argument: heartbeats, what it did, what it acts on, its inbox and
//! messages, listening for them, and leaving. Removing actors, switching
//! observers and seeing or testing how an actor is woken are yours, from
//! your own terminal. There's no allow, no new token and no webhook here:
//! those happen in XenoNet's window.

use crate::args::Flags;
use crate::js::{encode, have, list, num, on, str, t};
use crate::out::{ago, c, clip, eprint, json, now, print, table, write, Done, Fail, Layout};
use crate::products::known;
use crate::prompt::confirm;
use crate::reach::{bearer, need};
use crate::sys;
use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use xenonet_actors::token_file::{run_token_command, token_file_refusal, token_from_env, write_token_file};
use xenonet_base::client::{name_of, Server};
use xenonet_base::json::{number, stringify};

pub const HELP: &str = include_str!("help/actor.txt");

fn dot(presence: &str) -> String {
    match presence {
        "working" => c::green("●"),
        "idle" => c::dim("○"),
        "away" => c::yellow("◐"),
        "offline" => c::dim("·"),
        _ => c::dim("?"),
    }
}

/// A node of the map as a person reads it: a place by its folder's name, a
/// session by its app.
pub fn node_words(id: &str) -> String {
    let Some((kind, rest)) = id.split_once(':') else { return id.to_string() };
    if !matches!(kind, "project" | "zone" | "native" | "claude" | "codex") || rest.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return id.to_string();
    }
    match kind {
        "claude" => "a Claude Code session".into(),
        "codex" => "a Codex session".into(),
        "native" => "a project".into(),
        _ => Some(sys::base_name(rest)).filter(|b| !b.is_empty()).unwrap_or_else(|| rest.to_string()),
    }
}

fn slug_of(s: &str) -> &str {
    s.strip_prefix("actor:").unwrap_or(s)
}

// Shells, by the name ps gives their program: its path, or a login
// shell's -name.
const SHELLS: [&str; 10] = ["zsh", "bash", "sh", "dash", "fish", "ksh", "tcsh", "csh", "nu", "pwsh"];

pub fn is_shell(program: &str) -> bool {
    let base = program.rsplit('/').next().unwrap_or(program);
    let base = base.strip_prefix('-').unwrap_or(base);
    let base = if base.len() >= 4 && base[base.len() - 4..].eq_ignore_ascii_case(".exe") { &base[..base.len() - 4] } else { base };
    SHELLS.contains(&base.to_lowercase().as_str())
}

// The program a process runs, as ps names it, or None where that can't be
// told (no ps, as on Windows, or a process that's gone).
fn program_of(pid: u32) -> Option<String> {
    let out = xenonet_base::private_file::run_command("ps", &["-o", "comm=", "-p", &pid.to_string()], Duration::from_secs(3)).ok()?;
    Some(xenonet_base::text::js_trim(&out).to_string()).filter(|p| !p.is_empty())
}

fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        // SAFETY: getppid has no preconditions.
        Some(unsafe { libc::getppid() } as u32)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// The process a heartbeat names as the actor's own when --pid doesn't
/// say: this command's parent, when that's a program (Python, Node and the
/// like), so the actor goes offline when that program ends. Not when the
/// parent is a shell, which is your terminal's when you run it by hand,
/// and a shell script's too: the sessions you start there would then pass
/// for ones the actor spawned, so a script names its process with --pid.
/// Nor when what the parent runs can't be told.
pub fn default_pid(ppid: Option<u32>, program: impl Fn(u32) -> Option<String>) -> Option<u32> {
    let ppid = ppid.filter(|p| *p > 1)?;
    program(ppid).filter(|runs| !is_shell(runs)).map(|_| ppid)
}

/// A product as a person reads it: one XenoNet knows by its id, maker and
/// version, and any other as the actor's own words, quoted and said to be,
/// since any program can send any id, maker and version.
fn product_words(p: &Value) -> String {
    let id = t(p, "id");
    if known(&id) {
        let mut parts = vec![id];
        if on(p, "vendor") {
            parts.push(format!("by {}", t(p, "vendor")));
        }
        if on(p, "version") {
            parts.push(t(p, "version"));
        }
        return parts.join(" ");
    }
    let mut parts = vec![format!("“{id}”")];
    if on(p, "vendor") {
        parts.push(format!("by “{}”", t(p, "vendor")));
    }
    if on(p, "version") {
        parts.push(t(p, "version"));
    }
    format!("{} {}", parts.join(" "), c::dim(format!("(its own word{})", if parts.len() > 1 { "s" } else { "" })))
}

/// The token this command runs with, as an actor.
fn token_of() -> Result<String, Fail> {
    let env = |name: &str| std::env::var(name).ok();
    let mut run = |f: &str, a: &[&str]| run_token_command(f, a);
    match token_from_env(&env, cfg!(windows), &mut run) {
        Err(e) => Err(Fail::new(e.message(), 1)),
        Ok(None) => {
            Err(Fail::new("this is for an actor: say where its token is with XENONET_ACTOR_TOKEN_FILE=<file> (or XENONET_ACTOR_TOKEN); a token never goes on the command line", 2))
        }
        Ok(Some((token, _))) => Ok(token),
    }
}

fn as_actor(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    let token = token_of()?;
    Ok(s.request(method, route, body, &bearer(&token))?)
}

fn unread_words(me: &Value) -> String {
    if !on(me, "unread") {
        return String::new();
    }
    let n = num(me, "unread");
    format!("; {} (xenonet actor inbox)", if n == 1.0 { "a message waits".to_string() } else { format!("{} messages wait", t(me, "unread")) })
}

// ---------- for anyone ----------

fn list_actors(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let answer = s.get("/api/actors")?;
    let actors = answer.get("actors").cloned().unwrap_or(Value::Null);
    if flags.on("json") {
        json(&actors);
        return Ok(0);
    }
    // You, outside XenoNet's window, are only a placeholder.
    let shown: Vec<&Value> = actors.as_array().into_iter().flatten().filter(|a| str(a, "id") != Some("actor:you") || str(a, "presence") != Some("unknown")).collect();
    if shown.is_empty() {
        print(c::dim("No actors yet. A program asks with xenonet actor register, and you allow it in XenoNet's window."));
        return Ok(0);
    }
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|a| {
            let doing = match have(a, "status") {
                Some(st) => crate::js::text(Some(st)),
                None => {
                    let targets = list(a, "targets");
                    if targets.is_empty() {
                        String::new()
                    } else {
                        let mut words: Vec<String> = Vec::new();
                        for w in targets.iter().map(|x| node_words(&t(x, "id"))) {
                            if !words.contains(&w) {
                                words.push(w);
                            }
                        }
                        format!("on {}", words.into_iter().take(3).collect::<Vec<_>>().join(", "))
                    }
                }
            };
            let via: Vec<String> = list(a, "via").iter().map(|v| crate::js::text(Some(v))).filter(|v| v != "demo").collect();
            vec![dot(&t(a, "presence")), c::bold(t(a, "name")), c::dim(t(a, "id")), c::dim(if via.is_empty() { "demo".into() } else { via.join(", ") }), t(a, "presence"), doing]
        })
        .collect();
    print(table(&rows, Layout::default()));
    Ok(0)
}

fn show(args: &[String], flags: &Flags) -> Done {
    let Some(slug) = args.first().filter(|s| !s.is_empty()) else { return Err(Fail::new("say which actor: xenonet actor show <slug>", 2)) };
    let s = need(flags)?;
    let a = s.get(&format!("/api/actors/{}", encode(slug_of(slug))))?;
    if flags.on("json") {
        json(&a);
        return Ok(0);
    }
    print(format!("{} {}  {}", dot(&t(&a, "presence")), c::bold(t(&a, "name")), c::dim(t(&a, "id"))));
    let since = if on(&a, "presenceSince") { c::dim(format!(" since {}", ago(num(&a, "presenceSince"), now()))) } else { String::new() };
    let heard = if on(&a, "lastSeen") { c::dim(format!(" · last heard {}", ago(num(&a, "lastSeen"), now()))) } else { String::new() };
    let join = |k: &str| xenonet_base::text::join(list(&a, k).iter().map(Some), ", ");
    let mut facts: Vec<(&str, String)> = vec![
        ("Presence", format!("{}{since}{heard}", t(&a, "presence"))),
        ("Status", have(&a, "status").map_or_else(|| c::dim("none"), |st| crate::js::text(Some(st)))),
        ("Known by", join("via")),
    ];
    if let Some(p) = crate::js::some(&a, "product") {
        facts.push(("Product", product_words(p)));
    }
    facts.push(("Allowed", if list(&a, "capabilities").is_empty() { c::dim("nothing beyond reporting itself") } else { join("capabilities") }));
    if on(&a, "heartbeatMs") {
        facts.push(("Heartbeat", format!("every {} s", xenonet_base::json::number_string(xenonet_base::units::js_round(num(&a, "heartbeatMs") / 1000.0)))));
    }
    let targets = list(&a, "targets");
    facts.push((
        "Acts on",
        if targets.is_empty() {
            c::dim("nothing lately")
        } else {
            targets.iter().map(|x| format!("{} ({})", node_words(&t(x, "id")), t(x, "relation"))).collect::<Vec<_>>().join(", ")
        },
    ));
    print(table(&facts.into_iter().map(|(k, v)| vec![c::dim(k), v]).collect::<Vec<_>>(), Layout::wrapped(2)));
    let recent = list(&a, "recent");
    if !recent.is_empty() {
        print(format!("\n{}", c::bold("Lately")));
        let rows: Vec<Vec<String>> =
            recent.iter().rev().take(10).map(|e| vec![c::dim(ago(num(e, "at"), now())), have(e, "detail").map_or_else(|| t(e, "kind"), |d| crate::js::text(Some(d)))]).collect();
        print(table(&rows, Layout::wrapped(2)));
    }
    Ok(0)
}

/// The products XenoNet observes, or one turned on or off (yours).
fn observe(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let Some(product) = args.first().filter(|p| !p.is_empty()) else {
        let answer = s.get("/api/actors/observers")?;
        let observers = answer.get("observers").cloned().unwrap_or(Value::Null);
        if flags.on("json") {
            json(&observers);
            return Ok(0);
        }
        let rows: Vec<Vec<String>> =
            observers.as_array().into_iter().flatten().map(|o| vec![if on(o, "on") { c::green("on") } else { c::dim("off") }, t(o, "name"), c::dim(t(o, "product"))]).collect();
        print(table(&rows, Layout::default()));
        return Ok(0);
    };
    let state = args.get(1).map(String::as_str);
    if state != Some("on") && state != Some("off") {
        return Err(Fail::new("xenonet actor observe <product> on|off", 2));
    }
    let out = s.request("POST", "/api/actors/observers", Some(&json!({ "product": product, "on": state == Some("on") })), &[])?;
    if flags.on("json") {
        json(&out);
        return Ok(0);
    }
    print(format!("{} {} {} {product}.", c::green("✓"), name_of(&s), if on(&out, "on") { "observes" } else { "no longer observes" }));
    Ok(0)
}

// ---------- registering ----------

fn register(_args: &[String], flags: &Flags) -> Done {
    let Some(name) = flags.str("name").filter(|n| !xenonet_base::text::js_trim(n).is_empty()) else {
        return Err(Fail::new("say what it's called: xenonet actor register --name <name>", 2));
    };
    let mut body = Map::new();
    body.insert("schema".into(), json!("actor/registration/v1"));
    body.insert("name".into(), json!(name));
    if let Some(slug) = flags.str("slug") {
        body.insert("slug".into(), json!(slug));
    }
    if let Some(caps) = flags.str("capabilities") {
        let caps: Vec<&str> = caps.split(',').map(xenonet_base::text::js_trim).filter(|x| !x.is_empty()).collect();
        body.insert("capabilities".into(), json!(caps));
    }
    if flags.has("heartbeat") {
        let seconds = flags.number("heartbeat");
        if seconds.is_nan() || seconds <= 0.0 {
            return Err(Fail::new("--heartbeat is a number of seconds", 2));
        }
        body.insert("heartbeatMs".into(), number(xenonet_base::units::js_round(seconds * 1000.0)));
    }
    if let Some(product) = flags.str("product") {
        body.insert("product".into(), json!({ "id": product }));
    }
    let s = need(flags)?;
    // The file is checked before asking, so no token is made for nothing.
    let file = flags.str("tokenFile").map(sys::resolve);
    let projects_dir = s.get("/api/workspace/root").ok().and_then(|r| str(&r, "path").map(str::to_string));
    let force = flags.on("force");
    if let Some(why) = file.as_deref().and_then(|f| token_file_refusal(f, force, projects_dir.as_deref())) {
        return Err(Fail::new(why, 1));
    }

    let creds = s.request("POST", "/api/actors/register", Some(&Value::Object(body)), &[])?;
    if !flags.on("json") {
        print(format!("Waiting for you to allow \"{name}\" in {}'s window (it asks for 10 minutes)…", name_of(&s)));
    }
    // How often to ask whether you've answered; tests ask faster.
    let every = sys::var("XENONET_ACTOR_WAIT_MS").map(|v| xenonet_base::text::js_number(Some(&Value::String(v)))).filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(2000.0);
    let until = have(&creds, "expiresAt").map_or(now() + 10.0 * 60e3, |e| xenonet_base::text::js_number(Some(e))) + 5000.0;
    let token = t(&creds, "token");
    loop {
        std::thread::sleep(Duration::from_millis(every.max(0.0) as u64));
        match s.request("GET", "/api/actors/me", None, &bearer(&token)) {
            Ok(me) if str(&me, "state") == Some("allowed") => break,
            Ok(_) => {}
            Err(e) if e.status == Some(401) => return Err(Fail::new(format!("{} wasn't allowed: {}.", t(&creds, "id"), e.message), 1)),
            Err(e) if e.status.is_some() => return Err(Fail::new(e.message, 1)),
            // XenoNet restarting: keep waiting.
            Err(_) => {}
        }
        if now() > until {
            return Err(Fail::new("No answer in 10 minutes, so the request lapsed. Ask again when you're at XenoNet's window.", 1));
        }
    }
    if let Some(file) = &file {
        let mut run = |f: &str, a: &[&str]| run_token_command(f, a);
        let saved = write_token_file(file, &token, cfg!(windows), &mut run, force, projects_dir.as_deref()).map_err(|e| Fail::error(e.message()))?;
        if flags.on("json") {
            let mut out = Map::new();
            out.insert("id".into(), creds.get("id").cloned().unwrap_or(Value::Null));
            out.insert("state".into(), json!("allowed"));
            out.insert("file".into(), json!(saved));
            if let Some(ignored) = creds.get("ignored") {
                out.insert("ignored".into(), ignored.clone());
            }
            json(&Value::Object(out));
            return Ok(0);
        }
        print(format!("{} {} is allowed. Its token is in {}, readable only by you.", c::green("✓"), t(&creds, "id"), sys::tilde(&saved)));
    } else {
        if flags.on("json") {
            let mut out = creds.as_object().cloned().unwrap_or_default();
            out.insert("state".into(), json!("allowed"));
            out.shift_remove("expiresAt");
            json(&Value::Object(out));
            return Ok(0);
        }
        print(format!("{} {} is allowed. Its token, shown once:\n{token}", c::green("✓"), t(&creds, "id")));
        print(c::dim("Keep it in a file only you can read (chmod 600) and pass it as XENONET_ACTOR_TOKEN_FILE; never on a command line."));
    }
    let ignored = list(&creds, "ignored");
    if !ignored.is_empty() {
        print(c::dim(format!("XenoNet doesn't know these capabilities, so it left them out: {}", ignored.iter().map(|v| crate::js::text(Some(v))).collect::<Vec<_>>().join(", "))));
    }
    Ok(0)
}

// ---------- for an actor, with its token ----------

fn heartbeat(_args: &[String], flags: &Flags) -> Done {
    let mut body = Map::new();
    body.insert("schema".into(), json!("actor/report/v1"));
    if let Some(p) = flags.get("presence") {
        if !matches!(p.as_str(), Some("working" | "idle" | "offline")) {
            return Err(Fail::new("--presence is working, idle or offline", 2));
        }
        body.insert("presence".into(), p.clone());
    }
    for k in ["status", "instance"] {
        if let Some(v) = flags.str(k) {
            body.insert(k.into(), json!(v));
        }
    }
    body.insert("cwd".into(), json!(sys::resolve(&flags.str("cwd").map_or_else(sys::cwd, str::to_string))));
    // Its own long-lived process: the one you name, or else this command's
    // parent when that isn't a shell (default_pid).
    let pid = if flags.has("pid") { flags.number("pid") } else { default_pid(parent_pid(), program_of).map_or(f64::NAN, f64::from) };
    if flags.has("pid") && (pid.fract() != 0.0 || pid.is_nan() || pid < 1.0) {
        return Err(Fail::new("--pid is a process id", 2));
    }
    if pid > 1.0 {
        body.insert("pid".into(), number(pid));
    }
    let s = need(flags)?;
    let me = as_actor(&s, "POST", "/api/actors/heartbeat", Some(&Value::Object(body)))?;
    if flags.on("json") {
        json(&me);
        return Ok(0);
    }
    print(format!("{} {} is {}{}", dot(&t(&me, "presence")), t(&me, "id"), t(&me, "presence"), unread_words(&me)));
    Ok(0)
}

fn status(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let me = as_actor(&s, "GET", "/api/actors/me", None)?;
    if flags.on("json") {
        json(&me);
        return Ok(0);
    }
    if str(&me, "state") == Some("waiting") {
        print(format!("{} {} is waiting for you to allow it in {}'s window.", c::yellow("…"), t(&me, "id"), name_of(&s)));
        return Ok(0);
    }
    print(format!("{} {} is allowed, and {}{}", dot(&t(&me, "presence")), t(&me, "id"), t(&me, "presence"), unread_words(&me)));
    Ok(0)
}

fn event(args: &[String], flags: &Flags) -> Done {
    let Some(kind) = args.first().filter(|k| !k.is_empty()) else {
        return Err(Fail::new("say what it did: xenonet actor event <spawned|edited|ran|messaged|finished|failed>", 2));
    };
    if flags.str("target").is_some() && flags.str("path").is_some() {
        return Err(Fail::new("say where with --target or --path, not both", 2));
    }
    let mut e = Map::new();
    e.insert("kind".into(), json!(kind));
    if let Some(target) = flags.str("target") {
        e.insert("target".into(), json!(target));
    }
    if let Some(p) = flags.str("path") {
        e.insert("path".into(), json!(sys::resolve(p)));
    }
    if flags.has("count") {
        e.insert("count".into(), number(flags.number("count")));
    }
    if flags.on("paths") {
        e.insert("paths".into(), json!(flags.list("paths").iter().map(|p| sys::resolve(p)).collect::<Vec<_>>()));
    }
    for k in ["platform", "subject", "ref", "app"] {
        if let Some(v) = flags.str(k) {
            e.insert(k.into(), json!(v));
        }
    }
    let s = need(flags)?;
    let me = as_actor(&s, "POST", "/api/actors/events", Some(&json!({ "schema": "actor/report/v1", "events": [Value::Object(e)] })))?;
    if flags.on("json") {
        json(&me);
        return Ok(0);
    }
    if let Some(first) = list(&me, "dropped").first() {
        return Err(Fail::new(format!("XenoNet didn't take it: {}", t(first, "reason")), 1));
    }
    print(format!("{} {kind}", c::green("✓")));
    Ok(0)
}

/// --add and --remove take "<id or path>=<relation>": a node id
/// (project:…, a session's id) or a folder, which XenoNet maps to its
/// project.
pub fn target_of(spec: &str, fallback: Option<&str>) -> Value {
    let (node, relation) = match spec.rfind('=').filter(|i| *i > 0) {
        Some(i) => (&spec[..i], Some(&spec[i + 1..])),
        None => (spec, fallback),
    };
    let id = ["project:", "zone:", "native:", "claude:", "codex:"].iter().any(|p| node.starts_with(p));
    let mut out = Map::new();
    if id {
        out.insert("id".into(), json!(node));
    } else {
        out.insert("path".into(), json!(sys::resolve(node)));
    }
    if let Some(r) = relation {
        out.insert("relation".into(), json!(r));
    }
    Value::Object(out)
}

fn targets(_args: &[String], flags: &Flags) -> Done {
    let add: Vec<Value> = flags.list("add").iter().map(|x| target_of(x, Some("works-on"))).collect();
    let remove: Vec<Value> = flags.list("remove").iter().map(|x| target_of(x, None)).collect();
    if add.is_empty() && remove.is_empty() && !flags.on("replace") {
        return Err(Fail::new("say what it acts on: xenonet actor targets --add <id or path>=<relation> …", 2));
    }
    let s = need(flags)?;
    let me = as_actor(&s, "POST", "/api/actors/targets", Some(&json!({ "schema": "actor/report/v1", "targets": add, "remove": remove, "replace": flags.on("replace") })))?;
    if flags.on("json") {
        json(&me);
        return Ok(0);
    }
    let given = flags.list("add");
    for d in list(&me, "dropped") {
        let i = num(d, "index");
        let spec = if i.fract() == 0.0 && i >= 0.0 { given.get(i as usize).cloned() } else { None };
        print(format!("{} {}: {}", c::yellow("!"), spec.unwrap_or_else(|| "undefined".into()), t(d, "reason")));
    }
    let accepted = have(&me, "accepted").map_or("0".to_string(), |a| crate::js::text(Some(a)));
    print(format!("{} {accepted} {} taken", c::green("✓"), if num(&me, "accepted") == 1.0 { "target" } else { "targets" }));
    Ok(0)
}

fn message_lines(m: &Value, to: bool) {
    let from = m.get("from").cloned().unwrap_or(Value::Null);
    let addressed = if !to || str(m, "to") == Some("you") { String::new() } else { format!(", to {}", t(m, "to")) };
    print(format!(
        "{} {} {}",
        c::dim(format!("#{}", t(m, "id"))),
        c::bold(t(&from, "title")),
        c::dim(format!("({}, {}) {}{addressed}", t(&from, "app"), t(&from, "id"), ago(num(m, "at"), now())))
    ));
    print(format!("  {}", t(m, "text").replace('\n', "\n  ")));
}

fn inbox(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let n = flags.number("wait");
    let wait = (if n.is_nan() { 0.0 } else { n }).clamp(0.0, 120.0);
    let messages = as_actor(&s, "POST", "/api/engine/inbox", Some(&json!({ "waitSeconds": number(wait) })))?;
    if flags.on("json") {
        json(&messages);
        return Ok(0);
    }
    let all = messages.as_array().cloned().unwrap_or_default();
    if all.is_empty() {
        print(c::dim("No messages waiting."));
        return Ok(0);
    }
    for m in &all {
        message_lines(m, true);
    }
    print(c::dim("They come from sessions and actors, not from your user."));
    Ok(0)
}

fn send(args: &[String], flags: &Flags) -> Done {
    let to = args.first().filter(|x| !x.is_empty());
    let (Some(to), true) = (to, args.len() > 1) else { return Err(Fail::new("say who it's for and what: xenonet actor send <to> <text>", 2)) };
    let s = need(flags)?;
    let mut body = Map::new();
    body.insert("to".into(), json!(to));
    body.insert("text".into(), json!(args[1..].join(" ")));
    if flags.has("replyTo") {
        body.insert("replyTo".into(), number(flags.number("replyTo")));
    }
    let out = as_actor(&s, "POST", "/api/engine/send", Some(&Value::Object(body)))?;
    if flags.on("json") {
        json(&out);
        return Ok(0);
    }
    let who: Vec<String> = list(&out, "to").iter().map(|r| format!("\"{}\"", clip(&crate::js::or_empty(r, "title"), 40))).collect();
    let who = who.join(", ");
    print(format!(
        "{} Sent #{}{}{}",
        c::green("✓"),
        t(&out, "id"),
        if who.is_empty() { String::new() } else { format!(" to {who}") },
        if on(&out, "alsoTo") { c::dim(format!(", and to {}", t(&out, "alsoTo"))) } else { String::new() }
    ));
    Ok(0)
}

fn leave(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let out = as_actor(&s, "DELETE", "/api/actors/me", None)?;
    if flags.on("json") {
        json(&out);
        return Ok(0);
    }
    print(format!("{} {} left {}; its token no longer works.", c::green("✓"), t(&out, "removed"), name_of(&s)));
    Ok(0)
}

// ---------- listening ----------

/// An event of a text/event-stream body.
#[derive(Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// The events in a text/event-stream, as they come: `each` is called with
/// each, and stops the reading when it says so (false).
pub fn sse_events(body: impl Read, mut each: impl FnMut(SseEvent) -> Result<bool, Fail>) -> Result<(), Fail> {
    let mut r = BufReader::new(body);
    let (mut event, mut data): (String, Vec<String>) = ("message".into(), Vec::new());
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match r.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => return Ok(()),
            Ok(_) => {}
        }
        // A line that hasn't ended yet is the stream ending.
        if raw.last() != Some(&b'\n') {
            return Ok(());
        }
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
        let line = String::from_utf8_lossy(&raw).into_owned();
        if line.is_empty() {
            if !data.is_empty() && !each(SseEvent { event: std::mem::replace(&mut event, "message".into()), data: data.join("\n") })? {
                return Ok(());
            }
            event = "message".into();
            data.clear();
        } else if !line.starts_with(':') {
            let (field, value) = match line.find(':') {
                Some(at) => (&line[..at], line[at + 1..].strip_prefix(' ').unwrap_or(&line[at + 1..])),
                None => (line.as_str(), ""),
            };
            if field == "event" {
                event = value.to_string();
            } else if field == "data" {
                data.push(value.to_string());
            }
        }
    }
}

/// A body sent in chunks (Transfer-Encoding: chunked), read as it comes.
struct Chunked<R> {
    inner: R,
    left: usize,
    done: bool,
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        if self.left == 0 {
            let mut line = String::new();
            if self.inner.read_line(&mut line)? == 0 {
                self.done = true;
                return Ok(0);
            }
            if line.trim().is_empty() {
                line.clear();
                if self.inner.read_line(&mut line)? == 0 {
                    self.done = true;
                    return Ok(0);
                }
            }
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a bad chunk"))?;
            if size == 0 {
                self.done = true;
                return Ok(0);
            }
            self.left = size;
        }
        let n = buf.len().min(self.left);
        let got = self.inner.read(&mut buf[..n])?;
        if got == 0 {
            self.done = true;
        }
        self.left -= got;
        Ok(got)
    }
}

/// An answer that's still coming: its status, and its body as it comes.
struct Stream {
    status: u16,
    body: Box<dyn Read>,
}

// GET a route with these headers, and read the answer's head; the body is
// read as it comes.
fn open_stream(port: u16, route: &str, headers: &[(String, String)]) -> std::io::Result<Stream> {
    let s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_nodelay(true)?;
    let mut head = format!("GET {route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    (&s).write_all(head.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status = line.split(' ').nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let (mut length, mut chunked) = (None, false);
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            break;
        }
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse::<u64>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let body: Box<dyn Read> = if chunked {
        Box::new(Chunked { inner: r, left: 0, done: false })
    } else if let Some(n) = length {
        Box::new(r.take(n))
    } else {
        Box::new(r)
    };
    Ok(Stream { status, body })
}

const EXEC_MS: u64 = 10 * 60 * 1000;

// A program run for a message: the message on its standard input, and this
// command's environment without the actor's token, which the program
// doesn't need.
fn run_for(exec: &str, m: &Value, note: &dyn Fn(&str)) {
    let name = sys::base_name(exec);
    let mut cmd = std::process::Command::new(exec);
    cmd.env_remove("XENONET_ACTOR_TOKEN").env_remove("XENONET_ACTOR_TOKEN_FILE");
    cmd.env("XENONET_MESSAGE_ID", t(m, "id")).env("XENONET_MESSAGE_FROM", m.get("from").and_then(|f| have(f, "id")).map_or(String::new(), |id| crate::js::text(Some(id))));
    cmd.stdin(std::process::Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return note(&format!("{name} didn't start: {e}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{}\n", stringify(m, 0)).as_bytes());
    }
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                if let Some(code) = st.code().filter(|c| *c != 0) {
                    note(&format!("{name} exited with code {code} for message #{}", t(m, "id")));
                }
                return;
            }
            Ok(None) if started.elapsed() > Duration::from_millis(EXEC_MS) => {
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return,
        }
    }
}

/// An actor's program that stays running hears of each message as it
/// arrives, over its stream (GET /api/actors/me/stream), and reads it then.
/// XenoNet going away or restarting is waited out; a token that stops
/// working ends it.
fn listen(_args: &[String], flags: &Flags) -> Done {
    let auth = bearer(&token_of()?);
    let mut exec = None;
    if flags.has("exec") {
        let Some(program) = flags.str("exec").filter(|p| !p.is_empty()) else { return Err(Fail::new("say which program: xenonet actor listen --exec <program>", 2)) };
        let full = sys::resolve(program);
        if !xenonet_base::paths::can_run(&full) {
            return Err(Fail::new(format!("{} isn't a program you can run", sys::tilde(&full)), 2));
        }
        exec = Some(full);
    }
    let s = need(flags)?;
    let quiet = flags.on("json");
    let note = |line: &str| {
        if !quiet {
            eprint(c::dim(line));
        }
    };
    let drain = || -> Result<(), Fail> {
        let messages = s.request("POST", "/api/engine/inbox", Some(&json!({})), &auth)?;
        for m in messages.as_array().into_iter().flatten() {
            if let Some(exec) = &exec {
                run_for(exec, m, &note);
            } else if quiet {
                write(&format!("{}\n", stringify(m, 0)));
            } else {
                message_lines(m, false);
            }
        }
        Ok(())
    };
    let mut headers = auth.clone();
    headers.push(("accept".into(), "text/event-stream".into()));
    headers.push(("x-xenonet-api-version".into(), "1".into()));
    let mut delay: u64 = 1000;
    loop {
        if let Ok(mut res) = open_stream(s.port, "/api/actors/me/stream", &headers) {
            if res.status == 401 || res.status == 403 {
                let mut text = Vec::new();
                let _ = res.body.read_to_end(&mut text);
                let said =
                    xenonet_base::json::parse(String::from_utf8_lossy(&text).trim_start_matches('\u{feff}')).ok().and_then(|v| have(&v, "error").map(|e| crate::js::text(Some(e))));
                return Err(Fail::new(said.unwrap_or_else(|| format!("{} refused this actor's stream", name_of(&s))), 1));
            }
            if (200..300).contains(&res.status) {
                delay = 1000;
                note(&format!("Listening for this actor's messages on {}. Ctrl-C stops.", name_of(&s)));
                let read = sse_events(res.body, |ev| {
                    if ev.event == "inbox" {
                        drain()?;
                    }
                    if ev.event == "closed" {
                        let why = xenonet_base::json::parse(&ev.data).ok().map_or("undefined".into(), |d| t(&d, "error"));
                        return Err(Fail::new(format!("{} closed the stream: {why}", name_of(&s)), 1));
                    }
                    Ok(true)
                });
                // A Fail ends it; anything else (XenoNet going away) is waited out.
                if let Err(f) = read {
                    if f.expected {
                        return Err(f);
                    }
                }
            }
        }
        note(&format!("{} isn't answering; trying again in {} s", name_of(&s), delay / 1000));
        std::thread::sleep(Duration::from_millis(delay));
        delay = (delay * 2).min(30_000);
    }
}

// ---------- yours ----------

fn wake_words(kind: &str) -> String {
    match kind {
        "webhook" => "a webhook".into(),
        "notify" => "a notification to you".into(),
        "none" => "nothing: it reads its inbox when it checks".into(),
        _ => "undefined".into(),
    }
}

fn wake(args: &[String], flags: &Flags) -> Done {
    let Some(slug) = args.first().filter(|s| !s.is_empty()) else { return Err(Fail::new("say which actor: xenonet actor wake <slug> [test|off]", 2)) };
    let id = slug_of(slug);
    let s = need(flags)?;
    let route = format!("/api/actors/{}/wake", encode(id));
    let what = args.get(1).map(String::as_str).filter(|w| !w.is_empty());
    if what == Some("test") {
        let r = s.request("POST", &format!("{route}/test"), None, &[])?;
        if flags.on("json") {
            json(&r);
            return Ok(0);
        }
        let host = r.get("wake").map_or("undefined".into(), |w| t(w, "host"));
        print(if on(&r, "ok") { format!("{} Woke actor:{id}: {host} answered HTTP {}", c::green("✓"), t(&r, "status")) } else { format!("{} {}", c::red("✗"), t(&r, "why")) });
        return Ok(0);
    }
    if what == Some("off") {
        let r = s.request("DELETE", &route, None, &[])?;
        if flags.on("json") {
            json(&r);
        } else {
            print(format!("{} actor:{id} is woken no more; it reads its inbox when it checks", c::green("✓")));
        }
        return Ok(0);
    }
    if let Some(what) = what {
        return Err(Fail::new(format!("xenonet actor wake takes test or off, not {what}"), 2));
    }
    let answer = s.get("/api/actors/wakes")?;
    let w = list(&answer, "wakes")
        .iter()
        .find(|x| str(x, "actor") == Some(&format!("actor:{id}")))
        .cloned()
        .unwrap_or_else(|| json!({ "actor": format!("actor:{id}"), "kind": "none", "listening": 0, "last": null }));
    if flags.on("json") {
        json(&w);
        return Ok(0);
    }
    let how = if str(&w, "kind") == Some("webhook") {
        format!("a webhook to {}, {} (kept in your credential store)", t(&w, "host"), if on(&w, "signed") { "signed" } else { "with its key" })
    } else {
        wake_words(&t(&w, "kind"))
    };
    print(format!("{} is woken by {how}", c::bold(t(&w, "actor"))));
    if on(&w, "listening") {
        print(format!("  {} a program of its own is listening, so messages reach it at once", c::green("●")));
    }
    if let Some(last) = crate::js::some(&w, "last") {
        let ok = on(last, "ok");
        print(format!(
            "  {} last wake {}: {}",
            if ok { c::green("✓") } else { c::yellow("!") },
            ago(num(last, "at"), now()),
            if ok { format!("answered HTTP {}", t(last, "status")) } else { t(last, "why") }
        ));
    }
    if str(&w, "kind") != Some("webhook") {
        print(c::dim("  A webhook is set in XenoNet's window: Actors, then Wake… on the actor."));
    }
    Ok(0)
}

fn remove(args: &[String], flags: &Flags) -> Done {
    let Some(slug) = args.first().filter(|s| !s.is_empty()) else { return Err(Fail::new("say which actor: xenonet actor remove <slug>", 2)) };
    let id = slug_of(slug);
    if id == "you" {
        return Err(Fail::new("You can't be removed: turning off Show me on the map, in XenoNet's Actors panel, deletes your record.", 1));
    }
    let s = need(flags)?;
    let name = name_of(&s);
    // One XenoNet only observes isn't XenoNet's to remove (it would refuse):
    // said before asking, with the switch that takes it off the map.
    let a = s.get(&format!("/api/actors/{}", encode(id)))?;
    let via: Vec<&str> = list(&a, "via").iter().filter_map(Value::as_str).collect();
    let observed = via.iter().any(|v| *v == "observed" || *v == "attributed");
    if observed && !via.contains(&"registered") {
        let product = a.get("product").and_then(|p| crate::js::some(p, "id")).map_or(String::new(), |p| format!("xenonet actor observe {} off, or ", crate::js::text(Some(p))));
        return Err(Fail::new(
            format!("{name} only observes {}, so it can't be removed. Turn off its observer instead: {product}its switch in {name}'s Actors panel.", t(&a, "name")),
            1,
        ));
    }
    let question = format!(
        "Remove {} from {name}? Its token stops working{}.",
        t(&a, "id"),
        if observed { format!(", and {name} keeps observing it") } else { " and its record goes".into() }
    );
    if !confirm(&question, flags)? {
        print(c::dim("Nothing changed."));
        return Ok(0);
    }
    let out = s.request("DELETE", &format!("/api/actors/{}", encode(id)), None, &[])?;
    if flags.on("json") {
        json(&out);
        return Ok(0);
    }
    print(format!("{} Removed {}{}.", c::green("✓"), t(&out, "removed"), if on(&out, "observed") { format!("; {name} still observes it") } else { String::new() }));
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let sub = args.first().map_or("list", String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "list" => list_actors(rest, flags),
        "show" => show(rest, flags),
        "observe" => observe(rest, flags),
        "register" => register(rest, flags),
        "heartbeat" | "beat" => heartbeat(rest, flags),
        "status" => status(rest, flags),
        "event" => event(rest, flags),
        "targets" => targets(rest, flags),
        "inbox" => inbox(rest, flags),
        "listen" => listen(rest, flags),
        "send" => send(rest, flags),
        "leave" => leave(rest, flags),
        "remove" => remove(rest, flags),
        "wake" => wake(rest, flags),
        _ => Err(Fail::new(format!("no \"xenonet actor {sub}\". xenonet actor --help lists what it does."), 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same cases as test/cli-actor.test.js's.
    #[test]
    fn shells_by_the_name_ps_gives_them() {
        for p in ["zsh", "/bin/zsh", "-zsh", "bash", "-bash", "/bin/sh", "dash", "/opt/homebrew/bin/fish", "ksh", "/bin/tcsh", "csh", "nu", "pwsh", "pwsh.exe"] {
            assert!(is_shell(p), "{p}");
        }
        for p in ["/usr/local/bin/node", "python3", "/home/nova/bin/wren", "zshell-helper", "bashful", "-", ""] {
            assert!(!is_shell(p), "{p}");
        }
        assert_eq!(default_pid(Some(4242), |_| Some("/usr/local/bin/node".into())), Some(4242));
        assert_eq!(default_pid(Some(4242), |_| Some("-zsh".into())), None);
        assert_eq!(default_pid(Some(4242), |_| Some("/bin/bash".into())), None);
        assert_eq!(default_pid(Some(4242), |_| None), None);
        assert_eq!(default_pid(Some(1), |_| Some("/sbin/launchd".into())), None);
    }

    #[test]
    fn nodes_and_targets_as_people_read_them() {
        assert_eq!(node_words("project:/users/nova/projects/docs-site"), "docs-site");
        assert_eq!(node_words("claude:abc"), "a Claude Code session");
        assert_eq!(node_words("native:x"), "a project");
        assert_eq!(node_words("actor:wren"), "actor:wren");
        assert_eq!(target_of("claude:x=spawned", Some("works-on")), json!({ "id": "claude:x", "relation": "spawned" }));
        assert_eq!(target_of("project:/a", None), json!({ "id": "project:/a" }));
        assert_eq!(target_of("=x", None)["path"], json!(sys::resolve("=x")));
    }

    #[test]
    fn events_of_a_stream() {
        let text = "retry: 5000\n\n: still here\n\nevent: inbox\ndata: {\"unread\":1}\n\ndata: a\r\ndata:b\n\nevent: x\n";
        let mut got = Vec::new();
        sse_events(text.as_bytes(), |e| {
            got.push(e);
            Ok(true)
        })
        .unwrap();
        assert_eq!(got, [SseEvent { event: "inbox".into(), data: "{\"unread\":1}".into() }, SseEvent { event: "message".into(), data: "a\nb".into() }]);
    }

    #[test]
    fn chunks_read_as_they_come() {
        let mut body = String::new();
        Chunked { inner: BufReader::new("5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".as_bytes()), left: 0, done: false }.read_to_string(&mut body).unwrap();
        assert_eq!(body, "hello world");
    }
}
