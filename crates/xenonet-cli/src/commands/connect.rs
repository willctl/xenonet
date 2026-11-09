//! xenonet connect (cli/commands/connect.js): adds XenoNet to Claude Code's,
//! Codex's and Grok Build's settings, so their sessions can see and message
//! each other
//! (core/engine/connect.js, through the running XenoNet, which knows how
//! its agents' tools should run). It shows what will change and asks
//! first.

use crate::args::Flags;
use crate::cores;
use crate::js::{list, str, t};
use crate::out::{c, json, print, Done, Fail};
use crate::prompt::confirm;
use crate::reach::{candidates, need};
use serde_json::{json, Value};
use xenonet_base::client::name_of;

pub const HELP: &str = include_str!("help/connect.txt");

/// How each change shows: its mark, color and what happens to it.
fn look(state: &str) -> (&'static str, &'static str, String) {
    match state {
        "present" => ("✓", "green", "already in place".into()),
        "missing" => ("+", "yellow", "to add".into()),
        "different" => ("~", "yellow", "to update".into()),
        "remove" => ("-", "red", "to take out".into()),
        s => ("?", "gray", s.into()),
    }
}

/// The changes a preview shows, for the apps named.
pub fn render(preview: &Value, apps: &[&str]) {
    for a in list(preview, "apps").iter().filter(|x| str(x, "app").is_some_and(|app| apps.contains(&app))) {
        print(c::bold(t(a, "name")));
        for ch in list(a, "changes") {
            let (mark, color, say) = look(&t(ch, "state"));
            print(format!("  {} {} {}", c::named(color, mark), t(ch, "what"), c::dim(format!("({say})"))));
            print(c::dim(format!("      {}", t(ch, "where"))));
            print(c::dim(format!("      {}", t(ch, "detail"))));
        }
        for b in list(a, "broken") {
            print(format!("  {} {}", c::red("✗"), crate::js::text(Some(b))));
        }
        for n in list(a, "notes") {
            print(c::dim(format!("  {}", crate::js::text(Some(n)))));
        }
        print("");
    }
}

/// Names as a sentence lists them: "A, B and C".
fn and_list(names: &[String]) -> String {
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// The apps to connect: the ones named, or every one that's here (an app
/// that may not be, Grok Build, says in the preview whether it is).
fn chosen<'a>(words: &'a [String], preview: &Value) -> Vec<&'a str> {
    if !words.is_empty() {
        return words.iter().map(|n| if n == "claude-code" { "claude" } else { n.as_str() }).collect();
    }
    connected().into_iter().filter(|id| list(preview, "apps").iter().find(|a| str(a, "app") == Some(id)).is_none_or(|a| a.get("installed") != Some(&Value::Bool(false)))).collect()
}

/// The apps of a preview that something's to be done for.
pub fn todo<'a>(preview: &'a Value, apps: &[&str]) -> Vec<&'a Value> {
    list(preview, "apps")
        .iter()
        .filter(|a| {
            str(a, "app").is_some_and(|app| apps.contains(&app)) && (list(a, "changes").iter().any(|ch| str(ch, "state") != Some("present")) || !list(a, "broken").is_empty())
        })
        .collect()
}

fn names(apps: &[&Value]) -> String {
    and_list(&apps.iter().map(|a| t(a, "name")).collect::<Vec<_>>())
}

/// What's left undone after connecting: `"<app>: <what>"` for each change
/// that isn't in place.
fn left(after: &Value, apps: &[&str]) -> Vec<String> {
    list(after, "apps")
        .iter()
        .filter(|a| str(a, "app").is_some_and(|app| apps.contains(&app)))
        .flat_map(|a| list(a, "changes").iter().filter(|ch| str(ch, "state") != Some("present")).map(move |ch| format!("{}: {}", t(a, "name"), t(ch, "what"))))
        .collect()
}

/// What createConnect is given for this computer's own settings: the
/// port, and the xenonet binary XenoNet's hooks and MCP servers run:
/// installed XenoNet's launcher, which never moves, else this binary
/// (cores::xenonet_bin).
pub fn local_options(port: f64) -> Value {
    json!({ "port": xenonet_base::json::number(port), "bin": cores::xenonet_bin() })
}

// The XenoNet on that port runs elsewhere, so it can't write this
// computer's settings: they're written here, as disconnect writes them
// with no XenoNet running, with this computer's launcher and that
// XenoNet's port.
fn here(words: &[String], flags: &Flags) -> Done {
    if !flags.on("port") {
        return Err(Fail::new("--here needs --port: the port the other XenoNet answers on here", 2));
    }
    let port = candidates(flags)[0];
    let opts = local_options(port);
    let preview = cores::connect_preview(&opts)?;
    if flags.on("json") {
        json(&preview);
        return Ok(0);
    }
    let apps = &chosen(words, &preview);
    let todo = todo(&preview, apps);
    let who = format!("the XenoNet on port {}", xenonet_base::json::number_string(port));
    render(&preview, apps);
    if todo.is_empty() {
        print(c::green(format!("Already connected to {who}.")));
        return Ok(0);
    }
    if !confirm(&format!("Connect {} here to {who}?", names(&todo)), flags)? {
        print(c::dim("Nothing changed."));
        return Ok(0);
    }
    let chosen: Vec<String> = todo.iter().map(|a| t(a, "app")).collect();
    let after = cores::connect_apply(&opts, &chosen)?;
    let left = left(&after, apps);
    if !left.is_empty() {
        return Err(Fail::new(format!("some changes didn't take: {}. xenonet doctor says more.", left.join("; ")), 1));
    }
    print(format!("{} Connected {} to {who}. While nothing answers there, its hooks stay out of the way.", c::green("✓"), names(&todo)));
    Ok(0)
}

/// The apps connect goes through, as the app registry has them.
pub fn connected() -> Vec<&'static str> {
    xenonet_adapter::apps::with("connect").map(|a| a.id.as_str()).collect()
}

/// XenoNet's plugin for Hermes Agent: copied into Hermes's plugins folder,
/// and off until the owner turns it on and gives it a token
/// (xenonet_hermes::plugin).
fn hermes(flags: &Flags) -> Done {
    use xenonet_hermes::plugin as hp;
    let home = crate::sys::home();
    let hh = std::env::var("HERMES_HOME").ok();
    let before = hp::inspect(&home, hh.as_deref(), crate::out::now());
    let shown = |p: &std::path::Path| xenonet_base::paths::tilde(&p.display().to_string(), &home);
    if flags.on("json") {
        json(&json!({ "dir": shown(&before.dir), "installed": before.installed, "current": before.current, "enabled": before.enabled, "token": before.token }));
        return Ok(0);
    }
    print(c::bold("Hermes Agent"));
    let (mark, say) = if before.current {
        (c::green("✓"), "already in place")
    } else if before.installed {
        (c::yellow("+"), "to update")
    } else {
        (c::yellow("+"), "to add")
    };
    print(format!("  {mark} the XenoNet plugin {}", c::dim(format!("({say})"))));
    print(c::dim(format!("      {}", shown(&before.dir))));
    print(c::dim("      Two files. It sends Hermes's session starts and ends, tool names and edited file paths to XenoNet, never arguments or text, and hands Hermes the messages other agents send it. It changes nothing else of Hermes's."));
    print("");
    if before.current {
        print(c::green("The plugin is in place."));
    } else {
        if !confirm("Put the plugin in Hermes's plugins folder?", flags)? {
            print(c::dim("Nothing changed."));
            return Ok(0);
        }
        hp::install(&home, hh.as_deref()).map_err(|e| Fail::new(format!("couldn't write {}: {e}", shown(&before.dir)), 1))?;
        print(format!("{} The plugin is in {}. It does nothing yet.", c::green("✓"), shown(&before.dir)));
    }
    let steps = hp::owner_steps(&hp::inspect(&home, hh.as_deref(), crate::out::now()), &shown);
    for (i, step) in steps.iter().enumerate() {
        print(format!("  {}. {step}", i + 1));
    }
    if steps.is_empty() {
        print(c::dim("It's on and has its token; xenonet doctor says whether it reports."));
    }
    Ok(0)
}

pub fn run(words: &[String], flags: &Flags) -> Done {
    if words.iter().any(|w| w == "hermes") {
        hermes(flags)?;
        let rest: Vec<String> = words.iter().filter(|w| *w != "hermes").cloned().collect();
        if rest.is_empty() {
            return Ok(0);
        }
        return run(&rest, flags);
    }
    let all = connected();
    let named: Vec<&str> = words.iter().map(|n| if n == "claude-code" { "claude" } else { n.as_str() }).collect();
    let bad: Vec<&str> = named.iter().copied().filter(|a| !all.contains(a)).collect();
    if !bad.is_empty() {
        let all: Vec<String> = all.iter().map(|a| (*a).to_string()).collect();
        return Err(Fail::new(format!("connect works with {}, not {}", and_list(&all), bad.join(", ")), 2));
    }
    if flags.on("here") {
        return here(words, flags);
    }
    let s = need(flags)?;
    let preview = s.get("/api/engine/connect")?;
    if flags.on("json") {
        json(&preview);
        return Ok(0);
    }
    let apps = chosen(words, &preview);
    let name = name_of(&s);
    if crate::js::on(&preview, "demo") {
        let apps = super::words::listed(&super::words::names("connect"), "and");
        return Err(Fail::new(format!("{name} shows made-up data, so it can't connect {apps}. Use XenoNet or XenoNet Dev."), 1));
    }
    let todo = todo(&preview, &apps);
    if todo.is_empty() {
        render(&preview, &apps);
        print(c::green(format!("Already connected to {name}.")));
        return Ok(0);
    }
    render(&preview, &apps);
    let who = names(&todo);
    if !confirm(&format!("Connect {who} to {name}?"), flags)? {
        print(c::dim("Nothing changed."));
        return Ok(0);
    }
    let chosen: Vec<String> = todo.iter().map(|a| t(a, "app")).collect();
    let after = s.post("/api/engine/connect", &json!({ "apps": chosen }), &[])?;
    let left = left(&after, &apps);
    if !left.is_empty() {
        return Err(Fail::new(format!("some changes didn't take: {}. xenonet doctor says more.", left.join("; ")), 1));
    }
    print(format!("{} Connected {who} to {name}. New sessions get XenoNet's tools and hooks; sessions already open pick them up when restarted.", c::green("✓")));
    if todo.iter().any(|a| str(a, "app") == Some("codex")) {
        print(c::dim("  Codex asks you to trust new hooks once: run /hooks in Codex and trust XenoNet's."));
    }
    Ok(0)
}
