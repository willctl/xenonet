//! xenonet doctor (cli/commands/doctor.js): XenoNet's checks (core/doctor),
//! run by the running XenoNet or, with none running, here
//! (core/doctor/checkup.js).

use crate::args::Flags;
use crate::cores;
use crate::js::{list, num, on, str, t};
use crate::out::{c, json, plural, print, table, Done, Fail, Layout};
use crate::reach::find;
use serde_json::{json, Map, Value};
use xenonet_base::client::name_of;
use xenonet_base::units::to_fixed;

pub const HELP: &str = include_str!("help/doctor.txt");

const STATUSES: [&str; 5] = ["ok", "info", "warn", "fail", "skip"];

// The first Claude Code with channels, and how long a session listens
// after its turns (core/delivery/setup.js's CHANNELS_FROM and
// LISTEN_SECONDS), for the words here.
const CHANNELS_FROM: [u32; 3] = [2, 1, 80];
const LISTEN_SECONDS: u32 = 300;

fn mark(status: &str) -> &'static str {
    match status {
        "ok" => "✓",
        "info" => "·",
        "warn" => "!",
        "fail" => "✗",
        "skip" => "–",
        _ => "undefined",
    }
}

fn paint(status: &str) -> &'static str {
    match status {
        "ok" => "green",
        "info" => "blue",
        "warn" => "yellow",
        "fail" => "red",
        _ => "gray",
    }
}

/// How many results have each status (core/doctor/index.js's summarize).
pub fn summarize(results: &[Value]) -> Value {
    let mut m = Map::new();
    for s in STATUSES {
        m.insert(s.into(), json!(results.iter().filter(|r| str(r, "status") == Some(s)).count()));
    }
    Value::Object(m)
}

/// The doctor, wherever it's asked for (core/doctor/checkup.js): a running
/// XenoNet runs it, since it knows its own sources and connections; with
/// none running, it runs here, and the checks that need XenoNet are
/// skipped. `only` keeps the checks whose id starts with one of its
/// entries.
pub fn checkup(flags: &Flags, only: Option<&[String]>) -> Result<Value, Fail> {
    let Some(s) = find(flags) else { return cores::checkup_here(flags, only) };
    let result = s.post("/api/doctor", &json!({}), &[])?;
    let all = list(&result, "results");
    let results: Vec<Value> = match only {
        Some(only) => all.iter().filter(|r| only.iter().any(|o| t(r, "id") == *o || t(r, "id").starts_with(&format!("{o}.")))).cloned().collect(),
        None => all.to_vec(),
    };
    // As {...result, results, summary, by}: a key the answer has keeps its place.
    let summary = summarize(&results);
    let mut out = result.as_object().cloned().unwrap_or_default();
    out.insert("results".into(), Value::Array(results));
    out.insert("summary".into(), summary);
    out.insert("by".into(), json!(name_of(&s)));
    Ok(Value::Object(out))
}

fn live(flags: &Flags) -> Done {
    let port = flags.get("port").cloned().unwrap_or_else(|| {
        json!(if flags.on("dev") {
            47842
        } else if flags.on("demo") {
            47841
        } else {
            47840
        })
    });
    let listen_file = crate::config::config().ok().and_then(|cfg| cores::write_listen_settings(&cfg.state_dir, &port).ok());
    let r = cores::live_setup(&port, listen_file.as_deref())?;
    if flags.on("json") {
        json(&r);
        return Ok(0);
    }
    let ok = |b: bool| if b { c::green("✓") } else { c::yellow("!") };
    let claude = r.get("claude").cloned().unwrap_or(Value::Null);
    let channels = on(&claude, "channels");
    print(c::bold("Live delivery"));
    let version = crate::js::have(&claude, "version").map_or_else(|| "not found".into(), |v| crate::js::text(Some(v)));
    let needs = if channels { " has channels".to_string() } else { format!(" needs {} or later for channels", CHANNELS_FROM.map(|n| n.to_string()).join(".")) };
    print(format!("  {} Claude Code {version}{needs}", ok(channels)));
    let tmux = crate::js::have(&r, "tmux")
        .map_or_else(|| "not found: XenoNet can't type into terminals, which only matters for sessions you allow it to".into(), |v| crate::js::text(Some(v)));
    print(format!("  {} tmux {tmux}", ok(on(&r, "tmux"))));
    let opencode = list(&r, "opencode");
    print(format!("  {} OpenCode servers running: {}", c::blue("·"), if opencode.is_empty() { "none".into() } else { xenonet_base::text::join(opencode.iter().map(Some), ", ") }));
    print("");
    print("Start Claude Code with XenoNet as a channel, so messages reach the session as they're sent:");
    print(format!("  {}", t(&r, "command")));
    print(c::dim("Claude Code asks you to confirm development channels each time it starts, since XenoNet's isn't on Anthropic's list yet. To make it the default, add to your shell's profile:"));
    print(format!("  {}", t(&r, "alias")));
    print(c::dim("Team and Enterprise organizations have to turn channels on first (channelsEnabled). If a message doesn't arrive, claude --debug logs why, and the session gets it on its next turn."));
    print("");
    if on(&r, "listen") {
        print(format!("Or have a session listen for {} minutes after each turn, which works in headless runs (claude -p) too:", LISTEN_SECONDS / 60));
        print(format!("  {}", t(&r, "listen")));
        print(c::dim(
            "While it listens, the session waits at the end of its turn, and a message that comes keeps it going. It goes with the channel flag above, if you want both.",
        ));
        print("");
    }
    print("For OpenCode, its TUI serves what XenoNet posts to only when told where to listen:");
    print(format!("  {}", t(&r, "opencodeAlias")));
    print("");
    print(c::dim("OpenCode sessions and tmux panes take messages only once you allow it, per session: xenonet live allow <session>. So does a Claude Code or Codex session that has ended, which XenoNet then resumes for a turn to take a message (claude -p --resume, codex exec resume). xenonet live pause stops everything."));
    Ok(if channels { 0 } else { 1 })
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    if flags.on("live") {
        return live(flags);
    }
    let only: Option<Vec<String>> = flags.str("only").map(|o| o.split(',').map(|x| xenonet_base::text::js_trim(x).to_string()).filter(|x| !x.is_empty()).collect());
    let result = checkup(flags, only.as_deref())?;
    let results = list(&result, "results");
    let count = |status: &str| results.iter().filter(|r| str(r, "status") == Some(status)).count();
    let fails = count("fail");
    if flags.on("json") {
        json(&result);
        return Ok(if fails > 0 { 1 } else { 0 });
    }

    // One width for every group's titles, so the details line up.
    let titles = results.iter().map(|r| crate::js::length(r.get("title"))).max().unwrap_or(0);
    let mut groups: Vec<String> = Vec::new();
    for r in results {
        let g = t(r, "group");
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    for group in &groups {
        print(c::bold(group));
        let mut rows = Vec::new();
        for r in results.iter().filter(|x| t(x, "group") == *group) {
            let status = t(r, "status");
            let detail = if status == "skip" { c::dim(t(r, "detail")) } else { t(r, "detail") };
            rows.push(vec![c::named(paint(&status), mark(&status)), format!("{:<titles$}", t(r, "title")), detail]);
            if on(r, "fix") && status != "ok" && status != "skip" {
                rows.push(vec![String::new(), String::new(), c::dim(format!("↳ {}", t(r, "fix")))]);
            }
        }
        print(table(&rows, Layout::wrapped(2)));
        print("");
    }
    let warns = count("warn");
    let verdict = if fails > 0 || warns > 0 {
        let mut said = Vec::new();
        if fails > 0 {
            said.push(c::red(plural(fails as f64, "problem")));
        }
        if warns > 0 {
            said.push(c::yellow(plural(warns as f64, "warning")));
        }
        said.join(" and ")
    } else {
        c::green("Everything XenoNet needs is in place")
    };
    let by = if on(&result, "by") { format!(", run by {}", t(&result, "by")) } else { String::new() };
    print(format!("{verdict} {}", c::dim(format!("· {} in {} s{by}", plural(results.len() as f64, "check"), to_fixed(num(&result, "ms") / 1000.0, 1)))));
    Ok(if fails > 0 { 1 } else { 0 })
}
