//! xenonet connect (cli/commands/connect.js): adds XenoNet to Claude Code's
//! and Codex's settings, so their sessions can see and message each other
//! (core/engine/connect.js, through the running XenoNet, which knows how
//! its agents' tools should run). It shows what will change and asks
//! first.

use crate::args::Flags;
use crate::cores;
use crate::js::{list, str, t};
use crate::out::{c, json, print, Done, Fail};
use crate::prompt::confirm;
use crate::reach::{candidates, need};
use crate::sys;
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
    apps.iter().map(|a| t(a, "name")).collect::<Vec<_>>().join(" and ")
}

/// What's left undone after connecting: "<app>: <what>" for each change
/// that isn't in place.
fn left(after: &Value, apps: &[&str]) -> Vec<String> {
    list(after, "apps")
        .iter()
        .filter(|a| str(a, "app").is_some_and(|app| apps.contains(&app)))
        .flat_map(|a| list(a, "changes").iter().filter(|ch| str(ch, "state") != Some("present")).map(move |ch| format!("{}: {}", t(a, "name"), t(ch, "what"))))
        .collect()
}

/// What createConnect is given for this computer's own settings: the
/// port, and what XenoNet's hooks and MCP servers run, a node and the
/// cli/xenonet.js in its root: the launcher's (XENONET_LAUNCHER_ROOT and
/// XENONET_LAUNCHER_EXEC), else the checkout this binary is in with the
/// Node on PATH, as node cli/xenonet.js gives its own. Without either,
/// this binary and its folder, which the preview shows and connecting
/// refuses (`runnable`).
pub fn local_options(port: f64) -> Value {
    let exe = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let checkout = Some(sys::xenonet_root()).filter(|r| std::path::Path::new(&sys::join(&sys::join(r, "cli"), "xenonet.js")).is_file());
    let root = sys::var("XENONET_LAUNCHER_ROOT").or_else(|| checkout.clone()).unwrap_or_else(|| sys::dir_name(&exe));
    // As process.execPath names it: its links followed.
    let node = checkout
        .and_then(|_| {
            xenonet_base::paths::on_path(
                "node",
                &std::env::var("PATH").unwrap_or_default(),
                std::env::var("PATHEXT").ok().as_deref(),
                cfg!(windows),
                &mut xenonet_base::paths::can_run,
            )
        })
        .map(|n| std::fs::canonicalize(&n).map_or(n, |p| p.to_string_lossy().into_owned()));
    let exec = sys::var("XENONET_LAUNCHER_EXEC").or(node).unwrap_or(exe);
    json!({ "port": xenonet_base::json::number(port), "root": root, "exec": exec })
}

/// Why these options can't be written into the apps' settings: the
/// entries would run a cli/xenonet.js that isn't there. The settings
/// XenoNet writes still name a node and its entry script (core/engine/
/// connect.js), not this binary alone.
fn runnable(opts: &Value) -> Result<(), Fail> {
    let root = t(opts, "root");
    if std::path::Path::new(&sys::join(&sys::join(&root, "cli"), "xenonet.js")).is_file() {
        return Ok(());
    }
    Err(Fail::new(
        "this xenonet can't connect the apps by itself yet: their hooks and MCP servers run XenoNet's cli/xenonet.js with Node, and there's none beside it. Run node cli/xenonet.js connect --here from XenoNet's checkout, or set XENONET_LAUNCHER_ROOT and XENONET_LAUNCHER_EXEC to XenoNet's launcher.",
        1,
    ))
}

// The XenoNet on that port runs elsewhere, so it can't write this
// computer's settings: they're written here, as disconnect writes them
// with no XenoNet running, with this computer's launcher and that
// XenoNet's port.
fn here(apps: &[&str], flags: &Flags) -> Done {
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
    let todo = todo(&preview, apps);
    let who = format!("the XenoNet on port {}", xenonet_base::json::number_string(port));
    render(&preview, apps);
    if todo.is_empty() {
        print(c::green(format!("Already connected to {who}.")));
        return Ok(0);
    }
    runnable(&opts)?;
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

pub fn run(words: &[String], flags: &Flags) -> Done {
    let apps: Vec<&str> = if words.is_empty() { vec!["claude", "codex"] } else { words.iter().map(|n| if n == "claude-code" { "claude" } else { n.as_str() }).collect() };
    let bad: Vec<&str> = apps.iter().copied().filter(|a| *a != "claude" && *a != "codex").collect();
    if !bad.is_empty() {
        return Err(Fail::new(format!("connect works with claude and codex, not {}", bad.join(", ")), 2));
    }
    if flags.on("here") {
        return here(&apps, flags);
    }
    let s = need(flags)?;
    let preview = s.get("/api/engine/connect")?;
    if flags.on("json") {
        json(&preview);
        return Ok(0);
    }
    let name = name_of(&s);
    if crate::js::on(&preview, "demo") {
        return Err(Fail::new(format!("{name} shows made-up data, so it can't connect Claude Code and Codex. Use XenoNet or XenoNet Dev."), 1));
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
