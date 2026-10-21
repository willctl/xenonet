//! core/digest.js: summaries built from the live event history: "while you
//! were away", the day's log, and handoff briefs. All deterministic; asking
//! Codex to narrate one is a child process, which the twin runs.

use serde_json::{json, Map, Value};
use std::collections::hash_map::Entry;
use std::fmt::Write as _;
use xenonet_base::json::{number, number_string};
use xenonet_base::jsstr;
use xenonet_base::text::{js_number, js_string, truthy};

fn base(p: &Value) -> String {
    let s = js_string(p);
    s.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

// The value a template literal writes: "undefined" for one that isn't
// there.
fn tpl(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

fn num_text(n: f64) -> String {
    number_string(n)
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

// A JS Map key for a value: values of different types stay apart.
fn map_key(v: Option<&Value>) -> String {
    v.map_or_else(|| "u".into(), |v| xenonet_base::json::stringify(v, 0))
}

struct Per {
    id: Value,
    app: Value,
    title: Value,
    turns: f64,
    prompts: f64,
    tools: Map<String, Value>,
    tool_calls: f64,
    files: Vec<String>,
    errors: f64,
    subagents: f64,
    // None when the first event had no time, which JSON then leaves out.
    first: Option<Value>,
    last: f64,
    last_message: Value,
}

/// summarize(events, { since, until, titleOf, live }): what each session
/// did in the window, the busiest first, and the totals. `title_of` is
/// asked once for each session, as it's first seen.
pub fn summarize<E>(events: &[Value], since: f64, until: f64, title_of: &mut dyn FnMut(&Value) -> Result<Value, E>, live: Option<&Value>) -> Result<Value, E> {
    let mut order: Vec<String> = Vec::new();
    let mut per: std::collections::HashMap<String, Per> = std::collections::HashMap::new();
    for e in events {
        let ts = js_number(get(e, "ts"));
        if ts < since || ts > until {
            continue;
        }
        let s = match per.entry(map_key(get(e, "sessionId"))) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let id = get(e, "sessionId").cloned().unwrap_or(Value::Null);
                let title = title_of(&id)?;
                order.push(v.key().clone());
                v.insert(Per {
                    id,
                    app: get(e, "app").cloned().unwrap_or(Value::Null),
                    title,
                    turns: 0.0,
                    prompts: 0.0,
                    tools: Map::new(),
                    tool_calls: 0.0,
                    files: Vec::new(),
                    errors: 0.0,
                    subagents: 0.0,
                    first: get(e, "ts").cloned(),
                    last: ts,
                    last_message: Value::Null,
                })
            }
        };
        s.last = if s.last.is_nan() || ts.is_nan() { f64::NAN } else { s.last.max(ts) };
        match get(e, "kind").and_then(Value::as_str) {
            Some("turn-end") => s.turns += 1.0,
            Some("prompt") => s.prompts += 1.0,
            Some("tool") => {
                s.tool_calls += 1.0;
                let cat = tpl(get(e, "category"));
                let n = s.tools.get(&cat).map_or(0.0, |v| js_number(Some(v)));
                s.tools.insert(cat, number(n + 1.0));
                if get(e, "category").and_then(Value::as_str) == Some("edit") {
                    for f in get(e, "files").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
                        let b = base(f);
                        if !s.files.contains(&b) {
                            s.files.push(b);
                        }
                    }
                }
            }
            Some("tool-error") => s.errors += 1.0,
            Some("subagent-start") => s.subagents += 1.0,
            Some("message") => s.last_message = get(e, "detail").cloned().unwrap_or(Value::Null),
            _ => {}
        }
    }
    let live_sessions = live.and_then(|l| get(l, "sessions")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let needs_you = |id: &Value| {
        // new Map(live.sessions.map(s => [s.id, s])): the last of an id wins.
        live_sessions.iter().rev().find(|s| get(s, "id").unwrap_or(&Value::Null) == id).and_then(|s| get(s, "needsYou")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null)
    };
    let mut sessions: Vec<(f64, Value)> = order
        .iter()
        .map(|k| {
            let s = &per[k];
            let mut m = Map::new();
            m.insert("id".into(), s.id.clone());
            m.insert("app".into(), s.app.clone());
            m.insert("title".into(), s.title.clone());
            m.insert("turns".into(), number(s.turns));
            m.insert("prompts".into(), number(s.prompts));
            m.insert("tools".into(), Value::Object(jsstr::js_key_order(s.tools.clone())));
            m.insert("toolCalls".into(), number(s.tool_calls));
            m.insert("files".into(), json!(s.files));
            m.insert("errors".into(), number(s.errors));
            m.insert("subagents".into(), number(s.subagents));
            if let Some(first) = &s.first {
                m.insert("first".into(), first.clone());
            }
            m.insert("last".into(), number(s.last));
            m.insert("lastMessage".into(), s.last_message.clone());
            m.insert("needsYou".into(), needs_you(&s.id));
            (s.tool_calls + s.turns * 5.0, Value::Object(m))
        })
        .collect();
    xenonet_base::sort::sort_by(&mut sessions, |a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut t = [0.0f64; 6];
    for (_, s) in &sessions {
        t[0] += 1.0;
        t[1] += js_number(s.get("turns"));
        t[2] += js_number(s.get("toolCalls"));
        t[3] += s["files"].as_array().map_or(0, Vec::len) as f64;
        t[4] += js_number(s.get("errors"));
        t[5] += js_number(s.get("subagents"));
    }
    Ok(json!({
        "since": number(since),
        "until": number(until),
        "totals": { "sessions": number(t[0]), "turns": number(t[1]), "toolCalls": number(t[2]), "files": number(t[3]), "errors": number(t[4]), "subagents": number(t[5]) },
        "sessions": sessions.into_iter().map(|(_, s)| s).collect::<Vec<_>>(),
    }))
}

const WEEKDAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A day as `toLocaleDateString(undefined, { weekday: "long", month:
/// "long", day: "numeric" })` writes it in English: "Saturday, October 3".
pub fn long_date(ms: f64) -> String {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return "Invalid Date".into();
    }
    let days = ((ms + jsstr::local_offset_ms(ms)) / 86_400_000.0).floor() as i64;
    let (_, m, d) = xenonet_base::time::civil_from_days(days);
    format!("{}, {} {d}", WEEKDAYS[days.rem_euclid(7) as usize], MONTHS[(m - 1) as usize])
}

// The local hour of a time, as getHours() gives it.
fn local_hour(ms: f64) -> Option<usize> {
    if !ms.is_finite() {
        return None;
    }
    let local = ms + jsstr::local_offset_ms(ms);
    Some((local.rem_euclid(86_400_000.0) / 3_600_000.0).floor() as usize)
}

/// A question a summary asks its caller about a value (an id): a title, a
/// home, a name. What the caller throws stops the summary.
pub type Ask<'a, E> = &'a mut dyn FnMut(&Value) -> Result<Value, E>;

/// The callbacks dayLog asks: a session's title, its home on the map, and
/// a home's name.
pub struct DayHooks<'a, E> {
    pub title_of: Option<Ask<'a, E>>,
    pub home_of: Option<Ask<'a, E>>,
    pub name_of: Option<Ask<'a, E>>,
}

/// dayLog(events, { now, titleOf, homeOf, nameOf, live, repos }): the
/// day so far, from local midnight.
pub fn day_log<E>(events: &[Value], now: f64, hooks: DayHooks<'_, E>, live: Option<&Value>, repos: &[Value]) -> Result<Value, E> {
    let (midnight, _) = jsstr::local_day_bounds(now);
    let mut own = |id: &Value| Ok::<Value, E>(id.clone());
    let title_of: &mut dyn FnMut(&Value) -> Result<Value, E> = match hooks.title_of {
        Some(f) => f,
        None => &mut own,
    };
    let sum = summarize(events, midnight, now, title_of, live)?;
    let mut hours = [0.0f64; 24];
    let mut projects: Vec<(Value, f64)> = Vec::new();
    let mut home_of = hooks.home_of;
    for e in events {
        let ts = js_number(get(e, "ts"));
        if ts < midnight || get(e, "kind").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(h) = local_hour(ts) {
            hours[h] += 1.0;
        }
        let home = match home_of.as_mut() {
            Some(f) => f(get(e, "sessionId").unwrap_or(&Value::Null))?,
            None => Value::Null,
        };
        if truthy(Some(&home)) {
            match projects.iter_mut().find(|(k, _)| *k == home) {
                Some((_, n)) => *n += 1.0,
                None => projects.push((home, 1.0)),
            }
        }
    }
    let max = hours.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let busiest = hours.iter().position(|&h| h == max).unwrap_or(0);
    let today: Vec<&Value> = events.iter().filter(|e| js_number(get(e, "ts")) >= midnight).collect();
    let ts_of = |e: Option<&&Value>| e.and_then(|e| get(e, "ts")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    xenonet_base::sort::sort_by(&mut projects, |a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut top = Vec::new();
    let mut name_of = hooks.name_of;
    for (id, n) in projects.into_iter().take(5) {
        let name = match name_of.as_mut() {
            Some(f) => f(&id)?,
            None => id.clone(),
        };
        top.push(json!({ "id": id, "name": name, "toolCalls": number(n) }));
    }
    let live_sessions = live.and_then(|l| get(l, "sessions")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let waiting = live_sessions.iter().filter(|s| truthy(get(s, "needsYou")) && get(&s["needsYou"], "kind").and_then(Value::as_str) != Some("review")).count();
    let mut out = Map::new();
    out.insert("date".into(), Value::String(long_date(now)));
    if let Some(fields) = sum.as_object() {
        out.extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    out.insert("tokens".into(), live.and_then(|l| get(l, "today")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::from(0)));
    out.insert("hours".into(), Value::Array(hours.iter().map(|&h| number(h)).collect()));
    out.insert("busiestHour".into(), if hours[busiest] != 0.0 { Value::from(busiest) } else { Value::Null });
    out.insert("firstAt".into(), ts_of(today.first()));
    out.insert("lastAt".into(), ts_of(today.last()));
    out.insert("topProjects".into(), Value::Array(top));
    out.insert("waiting".into(), Value::from(waiting));
    out.insert("uncommitted".into(), Value::Array(repos.iter().filter(|r| js_number(get(r, "dirty")) > 0.0).cloned().collect()));
    Ok(Value::Object(out))
}

/// A summary as text, for Codex to narrate.
pub fn summary_text(sum: &Value, label: &str) -> String {
    let t = |k: &str| tpl(sum.get("totals").and_then(|t| get(t, k)));
    let mut lines = vec![format!("{label}: {} sessions, {} turns, {} tool calls, {} files edited, {} errors.", t("sessions"), t("turns"), t("toolCalls"), t("files"), t("errors"))];
    for s in sum.get("sessions").and_then(Value::as_array).map_or(&[][..], Vec::as_slice).iter().take(12) {
        let tools = get(s, "tools").and_then(Value::as_object).map_or_else(String::new, |m| m.iter().map(|(k, v)| format!("{k} {}", js_string(v))).collect::<Vec<_>>().join(", "));
        let files: Vec<String> = get(s, "files").and_then(Value::as_array).map_or_else(Vec::new, |f| f.iter().map(|x| xenonet_base::text::join_item(Some(x))).collect());
        let mut line = format!(
            "- {} \"{}\": {} turns, {} tool calls ({})",
            tpl(get(s, "app")),
            tpl(get(s, "title")),
            tpl(get(s, "turns")),
            tpl(get(s, "toolCalls")),
            if tools.is_empty() { "none".into() } else { tools }
        );
        if !files.is_empty() {
            let _ = write!(line, ", edited {}", files.iter().take(6).cloned().collect::<Vec<_>>().join(", "));
        }
        if truthy(get(s, "errors")) {
            let _ = write!(line, ", {} errors", tpl(get(s, "errors")));
        }
        if truthy(get(s, "needsYou")) {
            let _ = write!(line, "; waiting for you: {}", tpl(get(&s["needsYou"], "text")));
        }
        if truthy(get(s, "lastMessage")) {
            let _ = write!(line, "; last said: {}", tpl(get(s, "lastMessage")));
        }
        lines.push(line);
    }
    let _ = num_text;
    lines.join("\n")
}

// s.slice(0, n - 1) + "…" for a string longer than n, else s ?? "".
fn clip_text(s: Option<&Value>, n: usize) -> String {
    match s {
        Some(Value::String(t)) if jsstr::len(t) > n => format!("{}…", jsstr::head(t, n - 1)),
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

fn last_text(list: Option<&Value>) -> Option<&Value> {
    list.and_then(Value::as_array).and_then(|a| a.last()).and_then(|x| get(x, "text"))
}

/// A brief for carrying a session's work on in another one.
pub fn handoff_brief(node: &Value, notes: &Value, target: Option<&str>) -> String {
    let from = match get(node, "app").and_then(Value::as_str) {
        Some("codex") => "a Codex session",
        Some("grok") => "a Grok Build session",
        _ => "a Claude Code session",
    };
    let goal = get(notes, "firstPrompt").and_then(|p| get(p, "text"));
    let latest = last_text(get(notes, "prompts"));
    let last = last_text(get(notes, "messages"));
    let mut files: Vec<String> = Vec::new();
    for list in ["edits", "files"] {
        for f in get(notes, list).and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            let f = js_string(f);
            if !files.contains(&f) {
                files.push(f);
            }
        }
    }
    files.truncate(12);
    let folder = ["worktree", "cwd"].iter().find_map(|k| get(node, k).filter(|v| truthy(Some(v)))).map_or_else(|| "unknown".to_string(), js_string);
    let branch = get(node, "branch").filter(|v| truthy(Some(v))).map_or_else(String::new, |b| format!(", on branch {}", js_string(b)));
    let mut parts = vec![format!("Continue the work from {from} titled \"{}\".", tpl(get(node, "title"))), String::new(), format!("Folder: {folder}{branch}")];
    if truthy(goal) {
        parts.extend([String::new(), format!("Original goal: {}", clip_text(goal, 700))]);
    }
    if truthy(latest) && latest != goal {
        parts.extend([String::new(), format!("Most recent request: {}", clip_text(latest, 600))]);
    }
    if truthy(last) {
        parts.extend([String::new(), "Where it left off:".into(), clip_text(last, if target == Some("codex") { 900 } else { 1500 })]);
    }
    if !files.is_empty() {
        parts.extend([String::new(), "Files it worked on:".into()]);
        parts.extend(files.iter().map(|f| format!("- {f}")));
    }
    if get(notes, "needsYou").and_then(|n| get(n, "kind")).and_then(Value::as_str) == Some("question") {
        parts.extend([String::new(), format!("It was waiting on this question from you: {}", clip_text(get(&notes["needsYou"], "text"), 400))]);
    }
    parts.extend([String::new(), "Before changing anything, check the repository's current state with git status and git diff. Then carry on from where it left off.".into()]);
    parts.join("\n")
}

/// What Codex is asked to narrate a summary with.
pub fn narrate_prompt(text: &str) -> String {
    format!("Write a short status note for the person running these Claude Code and Codex sessions. In 3 to 6 sentences, say what the sessions finished, what needs their attention first, and anything that looks stuck or risky. Refer to sessions by their titles. Write plainly: short sentences, active voice, everyday words, no em dashes. No headings, bullet points or preamble. Do not run any commands.\n\nActivity data:\n{text}")
}

/// What XenoNet's own `codex exec` runs leave out, as Xenolith's do: the
/// tools a narration or a sort has no use for, and everything a Codex home
/// brings in (plugins, apps, hooks, XenoNet's own among them, memories and
/// skills). Each goes as `-c features.<name>=false`, which a Codex that
/// doesn't know the name passes over, where `--disable` would refuse it.
const FEATURES_OFF: [&str; 16] = [
    "plugins",
    "apps",
    "hooks",
    "memories",
    "multi_agent",
    "goals",
    "shell_tool",
    "unified_exec",
    "view_image",
    "image_generation",
    "skill_search",
    "tool_suggest",
    "browser_use",
    "computer_use",
    "in_app_browser",
    "sleep_tool",
];

/// The arguments of one `codex exec` turn: ephemeral, read-only, in its own
/// folder, its last message written to `out`, in a schema's shape when
/// there's one. It runs in a Codex home of its own (codex_run_home), which
/// has no config.toml or rules to load, so it needs no
/// --ignore-user-config or --ignore-rules, which an older Codex would
/// refuse. It reads no AGENTS.md or skills, with no shell, web search or
/// plugins; sign-in stays in auth.json, never the keychain, so a token
/// refreshed during the run lands in the file it links to.
pub fn codex_exec_args(dir: &str, out: &str, schema_file: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["exec", "--ephemeral", "--skip-git-repo-check", "-s", "read-only", "--color", "never", "-C", dir].iter().map(ToString::to_string).collect();
    let settings = [
        "web_search=\"disabled\"",
        "project_doc_max_bytes=0",
        "history.persistence=\"none\"",
        "skills.include_instructions=false",
        "skills.bundled.enabled=false",
        "cli_auth_credentials_store=\"file\"",
    ];
    for kv in settings.iter().map(ToString::to_string).chain(FEATURES_OFF.iter().map(|f| format!("features.{f}=false"))) {
        args.extend(["-c".to_string(), kv]);
    }
    args.extend(["-o".to_string(), out.to_string()]);
    if let Some(f) = schema_file {
        args.extend(["--output-schema".to_string(), f.to_string()]);
    }
    args.push("-".into());
    args
}

/// The Codex home of one of XenoNet's own `codex exec` runs, `home` in the
/// run's folder, so the run loads none of the plugins, hooks, rules, skills
/// or AGENTS.md in yours (`user_home`, your CODEX_HOME). Only your sign-in
/// is shared: your auth.json is linked there, never copied, so a token
/// Codex refreshes during the run is written to yours and yours stays
/// signed in. A link that can't be made (a Windows account that may not
/// make one, another volume) leaves the run signed out, which it says.
pub fn codex_run_home(dir: &str, user_home: &str) -> std::io::Result<String> {
    let home = std::path::Path::new(dir).join("home");
    std::fs::create_dir_all(&home)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))?;
    }
    let auth = std::path::Path::new(user_home).join("auth.json");
    if auth.is_file() {
        let to = home.join("auth.json");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&auth, &to);
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&auth, &to);
        #[cfg(not(any(unix, windows)))]
        let linked: std::io::Result<()> = Err(std::io::Error::other("no links here"));
        if linked.is_err() {
            let _ = std::fs::hard_link(&auth, &to);
        }
    }
    Ok(home.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        let events = vec![
            json!({ "sessionId": "claude:a", "app": "claude", "kind": "prompt", "ts": 10 }),
            json!({ "sessionId": "claude:a", "app": "claude", "kind": "tool", "category": "edit", "files": ["/p/x/a.js", "C:\\p\\b.js"], "ts": 11 }),
            json!({ "sessionId": "codex:b", "app": "codex", "kind": "tool", "category": "shell", "ts": 12 }),
            json!({ "sessionId": "claude:a", "app": "claude", "kind": "turn-end", "ts": 13 }),
            json!({ "sessionId": "claude:c", "app": "claude", "kind": "turn-end", "ts": 99 }),
        ];
        let sum = summarize::<()>(&events, 0.0, 50.0, &mut |id| Ok(json!(format!("T {}", js_string(id)))), None).unwrap();
        assert_eq!(sum["totals"]["sessions"], 2);
        assert_eq!(sum["sessions"][0]["files"], json!(["a.js", "b.js"]));
        assert_eq!(sum["sessions"][0]["title"], "T claude:a");
        assert!(summary_text(&sum, "Today").starts_with("Today: 2 sessions, 1 turns, 2 tool calls, 2 files edited, 0 errors."));
        let brief = handoff_brief(
            &json!({ "app": "codex", "title": "Fix", "cwd": "/p" }),
            &json!({ "firstPrompt": { "text": "Go" }, "prompts": [{ "text": "Go" }], "edits": ["/p/a"], "files": ["/p/a", "/p/b"] }),
            None,
        );
        assert!(brief.contains("Folder: /p\n\nOriginal goal: Go\n\nFiles it worked on:\n- /p/a\n- /p/b"));
        assert_eq!(long_date(f64::NAN), "Invalid Date");
    }

    #[test]
    fn a_codex_run_is_isolated() {
        let args = codex_exec_args("/t/run", "/t/run/last.txt", Some("/t/run/schema.json"));
        let after = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].as_str());
        assert_eq!(
            (after("-C"), after("-o"), after("--output-schema"), args.last().map(String::as_str)),
            (Some("/t/run"), Some("/t/run/last.txt"), Some("/t/run/schema.json"), Some("-"))
        );
        assert!(args.iter().any(|a| a == "--ephemeral"));
        let set: Vec<&str> = args.windows(2).filter(|w| w[0] == "-c").map(|w| w[1].as_str()).collect();
        for kv in [
            "features.plugins=false",
            "features.hooks=false",
            "features.shell_tool=false",
            "features.unified_exec=false",
            "web_search=\"disabled\"",
            "cli_auth_credentials_store=\"file\"",
        ] {
            assert!(set.contains(&kv), "{kv}");
        }
        for flag in ["--disable", "--ignore-user-config", "--ignore-rules", "--strict-config"] {
            assert!(!args.iter().any(|a| a == flag), "{flag}: a Codex that doesn't know it, or a feature, fails the run");
        }
        assert!(!codex_exec_args("/t", "/t/o", None).iter().any(|a| a == "--output-schema"));
    }

    #[cfg(unix)]
    #[test]
    fn a_codex_runs_home_links_your_sign_in_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("xenonet-run-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let yours = dir.join("yours");
        std::fs::create_dir_all(&yours).unwrap();
        std::fs::write(yours.join("auth.json"), "{}").unwrap();
        std::fs::write(yours.join("hooks.json"), "{}").unwrap();
        std::fs::write(yours.join("AGENTS.md"), "Always answer in French.").unwrap();
        let run = dir.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let home = codex_run_home(&run.to_string_lossy(), &yours.to_string_lossy()).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&home).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["auth.json"]);
        let auth = std::path::Path::new(&home).join("auth.json");
        assert!(std::fs::symlink_metadata(&auth).unwrap().file_type().is_symlink(), "linked, never copied");
        assert_eq!(std::fs::canonicalize(&auth).unwrap(), std::fs::canonicalize(yours.join("auth.json")).unwrap());
        // Someone signed out of Codex: the run's home has no sign-in.
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        let bare = codex_run_home(&other.to_string_lossy(), &dir.join("nobody").to_string_lossy()).unwrap();
        assert_eq!(std::fs::read_dir(&bare).unwrap().count(), 0);
        std::fs::remove_dir_all(&run).unwrap();
        assert!(yours.join("auth.json").is_file(), "taking the run's folder away leaves yours");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
