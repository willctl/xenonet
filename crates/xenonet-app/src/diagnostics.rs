//! core/diagnostics.js: what XenoNet can tell someone helping you, safe to
//! share (xenonet diagnose, the doctor panel's Copy diagnostics): its
//! versions and which XenoNet it is, the doctor's findings that aren't ok,
//! its errors from the last day by code, where its hooks point, its
//! launcher, and which XENONET_ settings are set. Never a transcript, a
//! title, a project's name, an identity, a token or a key; the home folder
//! is ~. It's scrubbed as it's made and checked once more at the end, and
//! refused if anything private is left.
//!
//! With --ask, xenonet diagnose hands it to an agent on this computer, in a
//! folder of its own, which is told never to file an issue or send
//! anything: what it can do for anyone else is write issue-draft.md.

use crate::fsx::{self, FsError};
use crate::jsv::{less, raw, truthy};
use crate::nodepath::NATIVE;
use serde_json::{json, Map, Value};
use xenonet_base::json::{number_string, stringify};
use xenonet_base::log::{scrub, summarize, unsafe_text};
use xenonet_base::text::{join, js_number, js_string};
use xenonet_base::time::iso;

pub const SCHEMA: &str = "diagnostics/bundle/v1";

/// XENONET_ settings whose values say nothing private.
pub const SHOWN: [&str; 12] = [
    "XENONET_PORT",
    "XENONET_INSTANCE",
    "XENONET_DEMO",
    "XENONET_ACTORS",
    "XENONET_ASSISTANT",
    "XENONET_HOSTS",
    "XENONET_IDENTITIES",
    "XENONET_CLAUDE_USAGE",
    "XENONET_POLL_MS",
    "XENONET_STANDALONE",
    "XENONET_HOT",
    "XENONET_DEBUG",
];

/// What the bundle is made from: what the caller gathered, and about this
/// machine.
pub struct Gathered<'a> {
    pub xenonet: Option<&'a Value>,
    pub doctor: Option<&'a Value>,
    pub errors: &'a [Value],
    pub connect: Option<&'a Value>,
    pub launcher: Option<&'a Value>,
    pub env: &'a Map<String, Value>,
    pub home: &'a str,
    pub now: f64,
    pub platform: &'a str,
    pub arch: &'a str,
    pub release: &'a str,
    pub node: &'a str,
    pub electron: Option<&'a str>,
}

// `v ?? null`.
fn or_null(v: Option<&Value>) -> Value {
    v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null)
}

// new Date(v).toISOString(), which throws for a time that isn't one.
fn iso_of(v: Option<&Value>) -> Result<String, String> {
    let ms = match v {
        Some(Value::String(s)) => xenonet_base::time::parse_iso(s).unwrap_or(f64::NAN),
        Some(Value::Null) => 0.0,
        other => js_number(other),
    };
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return Err("Invalid time value".into());
    }
    Ok(iso(ms))
}

// An object built in order, leaving out what JS had as undefined.
struct Obj(Map<String, Value>);

impl Obj {
    fn new() -> Self {
        Obj(Map::new())
    }
    fn put(mut self, k: &str, v: Option<Value>) -> Self {
        if let Some(v) = v {
            self.0.insert(k.into(), v);
        }
        self
    }
    fn set(self, k: &str, v: Value) -> Self {
        self.put(k, Some(v))
    }
    fn done(self) -> Value {
        Value::Object(self.0)
    }
}

/// The bundle, or why it's refused.
pub fn make_bundle(g: &Gathered<'_>) -> Result<Value, String> {
    let home = g.home;
    // Strings are scrubbed; anything else stays what it was.
    let s = |v: Option<&Value>| -> Option<Value> {
        v.map(|x| match x {
            Value::String(t) => Value::String(scrub(t, home)),
            other => other.clone(),
        })
    };
    let x = g.xenonet;
    let doctor = match g.doctor.filter(|d| truthy(Some(d))) {
        None => Value::Null,
        Some(d) => {
            let problems: Vec<Value> = match raw(Some(d), "results") {
                Some(Value::Array(rs)) => rs
                    .iter()
                    .filter(|r| matches!(raw(Some(r), "status").and_then(Value::as_str), Some("warn" | "fail")))
                    .map(|r| {
                        let r = Some(r);
                        Obj::new()
                            .put("id", raw(r, "id").cloned())
                            .put("status", raw(r, "status").cloned())
                            .put("title", s(raw(r, "title")))
                            .put("detail", s(raw(r, "detail")))
                            .put("fix", s(Some(&or_null(raw(r, "fix")))))
                            .done()
                    })
                    .collect(),
                _ => Vec::new(),
            };
            json!({ "at": or_null(raw(Some(d), "at")), "summary": or_null(raw(Some(d), "summary")), "problems": problems })
        }
    };
    let mut errors = Vec::new();
    for r in summarize(g.errors).into_iter().take(50) {
        let rr = Some(&r);
        errors.push(
            Obj::new()
                .put("code", raw(rr, "code").cloned())
                .put("component", raw(rr, "component").cloned())
                .put("level", raw(rr, "level").cloned())
                .put("count", raw(rr, "count").cloned())
                .set("first", Value::String(iso_of(raw(rr, "first"))?))
                .set("last", Value::String(iso_of(raw(rr, "last"))?))
                .put("message", s(raw(rr, "message")))
                .put("fix", s(raw(rr, "fix")))
                .put("version", raw(rr, "version").cloned())
                .done(),
        );
    }
    let list = |v: Option<&Value>| match v {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let integrations = match g.connect.filter(|c| truthy(Some(c))) {
        None => Value::Null,
        Some(c) => {
            let apps: Vec<Value> = list(raw(Some(c), "apps"))
                .iter()
                .map(|a| {
                    let a = Some(a);
                    let changes: Vec<Value> =
                        list(raw(a, "changes")).iter().map(|ch| Obj::new().put("id", raw(Some(ch), "id").cloned()).put("state", raw(Some(ch), "state").cloned()).done()).collect();
                    let scrubbed = |k: &str| -> Vec<Value> { list(raw(a, k)).iter().map(|v| s(Some(v)).unwrap_or(Value::Null)).collect() };
                    Obj::new()
                        .put("app", raw(a, "app").cloned())
                        .set("changes", json!(changes))
                        .set("broken", json!(scrubbed("broken")))
                        .set("notes", json!(scrubbed("notes")))
                        .done()
                })
                .collect();
            Obj::new().put("port", raw(Some(c), "port").cloned()).set("apps", json!(apps)).done()
        }
    };
    let launcher = match g.launcher.filter(|l| truthy(Some(l))) {
        None => Value::Null,
        Some(l) => {
            let said = raw(Some(l), "said");
            json!({ "version": or_null(raw(Some(l), "version")), "ships": or_null(raw(Some(l), "ships")), "tools": or_null(raw(said, "tools")), "node": or_null(raw(said, "node")) })
        }
    };
    let mut keys: Vec<&String> = g.env.keys().filter(|k| k.starts_with("XENONET_")).collect();
    keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let settings: Vec<String> =
        keys.into_iter().map(|k| if SHOWN.contains(&k.as_str()) { format!("{k}={}", g.env.get(k).map_or_else(|| "undefined".into(), js_string)) } else { k.clone() }).collect();
    let bundle = json!({
        "schema": SCHEMA,
        "at": iso_of(Some(&json!(g.now)))?,
        "xenonet": { "version": or_null(raw(x, "version")), "instance": or_null(raw(x, "instance")), "packaged": or_null(raw(x, "packaged")), "running": or_null(raw(x, "running")) },
        "system": { "platform": g.platform, "arch": g.arch, "release": g.release, "node": g.node, "electron": g.electron },
        "doctor": doctor,
        "errors": errors,
        "integrations": integrations,
        "launcher": launcher,
        "settings": settings,
    });
    if let Some(why) = unsafe_text(&stringify(&bundle, 0), home) {
        return Err(format!("the diagnostics still held {why} after scrubbing, so XenoNet won't show them; xenonet logs --json shows what's there"));
    }
    Ok(bundle)
}

// A value in a template literal: String(v), and "undefined" for one missing.
fn t(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

fn at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = Some(v);
    for k in path {
        cur = raw(cur, k);
    }
    cur
}

fn items(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// The bundle as Markdown, for an issue or an agent.
pub fn bundle_markdown(b: &Value) -> String {
    let mut lines: Vec<String> = vec!["# XenoNet diagnostics".into(), String::new()];
    let version = at(b, &["xenonet", "version"]).filter(|v| !v.is_null()).map_or_else(|| "?".into(), js_string);
    let instance = at(b, &["xenonet", "instance"]).filter(|v| truthy(Some(v))).map_or(String::new(), |i| format!(" ({})", js_string(i)));
    let running = if at(b, &["xenonet", "running"]) == Some(&Value::Bool(false)) { ", not running" } else { "" };
    let electron = at(b, &["system", "electron"]).filter(|v| truthy(Some(v))).map_or(String::new(), |e| format!(" · Electron {}", js_string(e)));
    lines.push(format!(
        "{} · XenoNet {version}{instance}{running} · {} {} {} · Node {}{electron}",
        t(raw(Some(b), "at")),
        t(at(b, &["system", "platform"])),
        t(at(b, &["system", "arch"])),
        t(at(b, &["system", "release"])),
        t(at(b, &["system", "node"]))
    ));
    lines.push(String::new());
    lines.push("## Doctor".into());
    lines.push(String::new());
    let doctor = raw(Some(b), "doctor").filter(|d| truthy(Some(d)));
    match doctor {
        None => lines.push("Not run.".into()),
        Some(d) => {
            let problems = items(raw(Some(d), "problems"));
            if problems.is_empty() {
                lines.push("Every check passed.".into());
            }
            for p in &problems {
                let p = Some(p);
                let fix = raw(p, "fix").filter(|f| truthy(Some(f))).map_or(String::new(), |f| format!("\n  Fix: {}", js_string(f)));
                lines.push(format!("- **{}** `{}` {}: {}{fix}", t(raw(p, "status")), t(raw(p, "id")), t(raw(p, "title")), t(raw(p, "detail"))));
            }
        }
    }
    lines.push(String::new());
    lines.push("## Errors in the last day".into());
    lines.push(String::new());
    let errors = items(raw(Some(b), "errors"));
    if errors.is_empty() {
        lines.push("None.".into());
    }
    for e in &errors {
        let e = Some(e);
        let version = raw(e, "version").filter(|v| truthy(Some(v))).map_or(String::new(), |v| format!(", XenoNet {}", js_string(v)));
        let fix = raw(e, "fix").filter(|f| truthy(Some(f))).map_or(String::new(), |f| format!("\n  Fix: {}", js_string(f)));
        lines.push(format!(
            "- **{}** `{}` ({}, {}×, last {}{version}): {}{fix}",
            t(raw(e, "level")),
            t(raw(e, "code")),
            t(raw(e, "component")),
            t(raw(e, "count")),
            t(raw(e, "last")),
            t(raw(e, "message"))
        ));
    }
    lines.push(String::new());
    lines.push("## Integrations".into());
    lines.push(String::new());
    match raw(Some(b), "integrations").filter(|i| truthy(Some(i))) {
        None => lines.push("Unknown: XenoNet isn't running.".into()),
        Some(i) => {
            for a in items(raw(Some(i), "apps")) {
                let a = Some(&a);
                let changes: Vec<String> = items(raw(a, "changes")).iter().map(|c| format!("{} {}", t(raw(Some(c), "id")), t(raw(Some(c), "state")))).collect();
                let broken = items(raw(a, "broken"));
                let notes = items(raw(a, "notes"));
                let broken = if broken.is_empty() { String::new() } else { format!("; {}", join(broken.iter().map(Some), "; ")) };
                let notes = if notes.is_empty() { String::new() } else { format!("\n  {}", join(notes.iter().map(Some), " ")) };
                lines.push(format!("- {} (port {}): {}{broken}{notes}", t(raw(a, "app")), t(raw(Some(i), "port")), changes.join(", ")));
            }
        }
    }
    if let Some(l) = raw(Some(b), "launcher").filter(|l| truthy(Some(l))) {
        let l = Some(l);
        if less(raw(l, "version"), raw(l, "ships")) {
            lines.push(format!("- launcher {}, older than this XenoNet's {}, which replaces it when it next starts", t(raw(l, "version")), t(raw(l, "ships"))));
        } else {
            let or_word = |v: Option<&Value>, w: &str| v.filter(|x| !x.is_null()).map_or_else(|| w.to_string(), js_string);
            lines.push(format!("- launcher {}, Tools {}, Node {}", t(raw(l, "version")), or_word(raw(l, "tools"), "missing"), or_word(raw(l, "node"), "?")));
        }
    }
    lines.push(String::new());
    lines.push("## Settings".into());
    lines.push(String::new());
    let settings = items(raw(Some(b), "settings"));
    lines.push(if settings.is_empty() { "None of XENONET_ set.".into() } else { settings.iter().map(|s| format!("- `{}`", js_string(s))).collect::<Vec<_>>().join("\n") });
    lines.push(String::new());
    lines.join("\n")
}

/// What the agent is told, in the folder XenoNet makes for it: AGENTS.md
/// for Codex, CLAUDE.md for Claude Code.
pub fn agent_brief(sources: &[String]) -> String {
    let mut lines: Vec<String> = vec![
        "# Helping with XenoNet".into(),
        String::new(),
        "The person here runs XenoNet, a desktop console that maps their Claude Code and Codex sessions. Something isn't working, and bundle.md in this folder is what XenoNet could tell about it: its doctor's findings, its errors from the last day by code, where its hooks point, and its versions. It's scrubbed: the home folder is ~, and there are no transcripts, titles or keys.".into(),
        String::new(),
        "What to do:".into(),
        String::new(),
        "1. Read bundle.md, then explain in plain words what most likely went wrong and why, starting with what affects them most.".into(),
        "2. Walk them through fixing it, one step at a time, checking with them as you go. Prefer the fixes the doctor and the log already name. `xenonet doctor`, `xenonet logs --errors` and `xenonet status` say more, and are safe to run.".into(),
    ];
    if !sources.is_empty() {
        lines.push(format!(
            "3. XenoNet's own docs and code are here to read, if you need to know how something works: {}. Start with docs/guide/troubleshooting.md and docs/guide/development.md.",
            sources.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", ")
        ));
    }
    lines.extend([
        String::new(),
        "Rules:".into(),
        String::new(),
        "- Never file an issue, open a pull request, push, post or send anything anywhere, and don't run gh. XenoNet's maintainers want reports a person chose to send.".into(),
        "- If it looks like a bug in XenoNet rather than something to fix here, write issue-draft.md in this folder: what happened, what the bundle shows, and how to reproduce it, with no personal details. Tell them it's there; sending it is up to them.".into(),
        "- Don't change XenoNet's files, its settings, or Claude Code's or Codex's settings without asking first, and say what each change does.".into(),
        String::new(),
    ]);
    lines.join("\n")
}

const WEEK_MS: f64 = 7.0 * 86400e3;

/// The folder for one look, `<state>/diagnostics/<now>-<random>`, with
/// bundle.md, bundle.json and the brief; looks older than a week go when
/// the next is made. Returns the folder.
pub fn write_look(state_dir: &str, bundle: &Value, sources: &[String], now: f64) -> Result<String, FsError> {
    let root = NATIVE.join(&[state_dir, "diagnostics"]);
    fsx::mkdir_p(&root, Some(0o700))?;
    for name in fsx::read_dir_names(&root)? {
        let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || name.as_bytes().get(digits.len()) != Some(&b'-') {
            continue;
        }
        let at: f64 = digits.parse().unwrap_or(f64::NAN);
        if at != 0.0 && !at.is_nan() && now - at > WEEK_MS {
            fsx::rm_rf(&NATIVE.join(&[&root, &name]))?;
        }
    }
    let dir = NATIVE.join(&[&root, &format!("{}-{}", number_string(now), fsx::random_hex(3))]);
    fsx::mkdir(&dir, Some(0o700))?;
    let write = |name: &str, text: &str| fsx::write_file(&NATIVE.join(&[&dir, name]), text.as_bytes(), None);
    write("bundle.md", &bundle_markdown(bundle))?;
    write("bundle.json", &format!("{}\n", stringify(bundle, 2)))?;
    let brief = agent_brief(sources);
    write("AGENTS.md", &brief)?;
    write("CLAUDE.md", &brief)?;
    Ok(dir)
}

/// The command that starts the agent in that folder: Codex read-only and
/// asking before it does anything, or Claude Code in plan mode; with the
/// model chosen, and XenoNet's docs and code to read.
pub fn ask_command(app: &str, model: Option<&str>, dir: &Value, sources: &[String]) -> Value {
    let prompt = "Read bundle.md and AGENTS.md here, then tell me what's wrong with XenoNet and how to fix it.";
    let model = model.filter(|m| !m.is_empty());
    let add_dirs = sources.iter().flat_map(|s| ["--add-dir".to_string(), s.clone()]);
    if app == "claude" {
        let mut args: Vec<Value> = vec![json!("--permission-mode"), json!("plan")];
        if let Some(m) = model {
            args.extend([json!("--model"), json!(m)]);
        }
        args.extend(add_dirs.map(Value::String));
        args.push(json!(prompt.replace("AGENTS.md", "CLAUDE.md")));
        return json!({ "file": "claude", "args": args, "cwd": dir });
    }
    let mut args: Vec<Value> = vec![json!("-C"), dir.clone(), json!("-s"), json!("read-only"), json!("-a"), json!("on-request")];
    if let Some(m) = model {
        args.extend([json!("-m"), json!(m)]);
    }
    args.extend(add_dirs.map(Value::String));
    args.push(json!(prompt));
    json!({ "file": "codex", "args": args, "cwd": dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "C:\\Users\\nova";

    fn gathered<'a>(x: &'a Value, d: Option<&'a Value>, e: &'a [Value], env: &'a Map<String, Value>) -> Gathered<'a> {
        Gathered {
            xenonet: Some(x),
            doctor: d,
            errors: e,
            connect: None,
            launcher: None,
            env,
            home: HOME,
            now: 1791374400000.0,
            platform: "win32",
            arch: "x64",
            release: "10",
            node: "22",
            electron: None,
        }
    }

    #[test]
    fn bundles_whats_wrong_and_refuses_whats_private() {
        let doctor = json!({ "at": 1, "results": [{ "id": "agents.codex", "status": "fail", "title": "T", "detail": "connected, but C:\\Users\\nova\\old\\x.js is gone", "fix": "Run it." }, { "id": "ok", "status": "ok" }] });
        let errors = [json!({ "at": 1791367200000.0, "component": "service", "level": "error", "code": "service.port-in-use", "message": "port in use" })];
        let env: Map<String, Value> = json!({ "XENONET_PORT": "47840", "XENONET_STATE_DIR": "C:\\x", "PATH": "x" }).as_object().unwrap().clone();
        let x = json!({ "version": "0.8.0" });
        let b = make_bundle(&gathered(&x, Some(&doctor), &errors, &env)).unwrap();
        assert_eq!(b["doctor"]["problems"][0]["detail"], "connected, but ~\\old\\x.js is gone");
        assert_eq!(b["settings"], json!(["XENONET_PORT=47840", "XENONET_STATE_DIR"]));
        assert_eq!(b["errors"][0]["first"], "2026-10-07T10:00:00.000Z");
        let md = bundle_markdown(&b);
        assert!(md.contains("`agents.codex`") && md.contains("port in use") && !md.contains("nova"));
        let key = json!({ "instance": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789" });
        assert!(make_bundle(&gathered(&key, None, &[], &Map::new())).unwrap_err().contains("looks like a key"));
    }

    #[test]
    fn asks_read_only() {
        let c = ask_command("codex", Some("gpt-5.1-codex"), &json!("/d"), &["/opt/x".into()]);
        assert_eq!(
            c["args"].as_array().unwrap()[..8],
            [json!("-C"), json!("/d"), json!("-s"), json!("read-only"), json!("-a"), json!("on-request"), json!("-m"), json!("gpt-5.1-codex")]
        );
        assert_eq!(ask_command("claude", None, &json!("/d"), &[])["args"][1], "plan");
    }
}
