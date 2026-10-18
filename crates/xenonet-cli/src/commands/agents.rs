//! xenonet agents (cli/commands/agents.js): the agents at work on this
//! machine, as the engine sees them (core/engine): what each is doing,
//! where, and what it has in hand. The engine counts actors among them;
//! they're listed apart, after the sessions, and never counted as sessions
//! connected to XenoNet.

use crate::args::Flags;
use crate::js::{list, on, or_empty, str, t};
use crate::out::{ago, c, clip, json, now, print, width, Done};
use crate::reach::need;
use crate::sys;
use serde_json::Value;

pub const HELP: &str = include_str!("help/agents.txt");

fn show(a: &Value, room: usize) {
    let working = str(a, "status") == Some("working");
    let dot = if working { c::green("●") } else { c::dim("○") };
    let mut meta = Vec::new();
    if str(a, "app") != Some("actor") && on(a, "app") {
        meta.push(t(a, "app"));
    }
    if on(a, "project") {
        meta.push(t(a, "project"));
    }
    if on(a, "branch") {
        meta.push(clip(&t(a, "branch"), 28));
    }
    if !working {
        meta.push(ago(crate::js::num(a, "lastActive"), now()));
    }
    let meta = meta.join(" · ");
    let title = clip(&or_empty(a, "title"), 20.max(room as i64 - meta.chars().count() as i64 - 2) as usize);
    print(if meta.is_empty() { format!("{dot} {}", c::bold(&title)) } else { format!("{dot} {}  {}", c::bold(&title), c::dim(&meta)) });
    let mut notes = Vec::new();
    if on(a, "needsYou") {
        notes.push(c::yellow(if str(a, "needsYou") == Some("question") { "has a question for you" } else { "waiting for your permission" }));
    }
    if on(a, "focus") {
        notes.push(format!("says: {}", t(a, "focus")));
    }
    let editing = list(a, "editing");
    if !editing.is_empty() {
        let files: Vec<String> = editing.iter().take(3).map(|e| format!("{}{}", sys::base_name(&t(e, "path")), if on(e, "observed") { " (observed)" } else { "" })).collect();
        let more = if editing.len() > 3 { format!(" and {} more", editing.len() - 3) } else { String::new() };
        notes.push(format!("editing {}{more}", files.join(", ")));
    }
    let claims = list(a, "claims");
    if !claims.is_empty() {
        let top = str(a, "top").filter(|t| !t.is_empty());
        let shown: Vec<String> = claims
            .iter()
            .map(|x| {
                let pattern = t(x, "pattern");
                top.and_then(|top| pattern.strip_prefix(&format!("{top}/")).map(str::to_string)).unwrap_or(pattern)
            })
            .collect();
        notes.push(format!("claimed {}", shown.join(", ")));
    }
    if !notes.is_empty() {
        print(format!("  {}", clip(&notes.join(" · "), room)));
    }
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let answer = s.get("/api/engine/agents")?;
    let mut agents = answer.as_array().cloned().unwrap_or_default();
    if flags.on("repo") {
        let here = sys::cwd();
        agents.retain(|a| ["top", "cwd"].iter().any(|k| str(a, k).filter(|d| !d.is_empty()).is_some_and(|d| here == d || here.starts_with(&format!("{d}{}", sys::SEP)))));
    }
    if flags.on("json") {
        json(&if flags.on("repo") { Value::Array(agents) } else { answer });
        return Ok(0);
    }
    if agents.is_empty() {
        print(c::dim(if flags.on("repo") {
            "No session or actor has worked in this repo in the last two hours."
        } else {
            "No session or actor has worked in the last two hours."
        }));
        return Ok(0);
    }
    let (actors, sessions): (Vec<&Value>, Vec<&Value>) = agents.iter().partition(|a| str(a, "app") == Some("actor"));

    // A line for each agent, and a second for what it has in hand.
    let room = width() - 2;
    for a in &sessions {
        show(a, room);
    }
    if !actors.is_empty() {
        if !sessions.is_empty() {
            print("");
        }
        print(c::bold("Actors"));
        for a in &actors {
            show(a, room);
        }
    }
    // Only sessions connect to XenoNet; an actor reaches it with its token.
    let connected = sessions.iter().filter(|a| on(a, "reachable")).count();
    if !sessions.is_empty() && connected == 0 {
        print(c::dim("\nNone of these sessions is connected to XenoNet, so they can't see or message each other. xenonet connect sets that up."));
    }
    Ok(0)
}
