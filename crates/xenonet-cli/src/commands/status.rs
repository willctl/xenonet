//! xenonet status, and xenonet on its own (cli/commands/status.js): which
//! XenoNet is running, what it reads, the agents and actors at work, what
//! needs you, and the doctor's last word.

use crate::args::Flags;
use crate::js::{have, list, num, on, str, t};
use crate::out::{ago, c, json, now, plural, print, table, Done, Layout};
use crate::reach::{find, where_};
use serde_json::{json, Map, Value};
use xenonet_base::client::name_of;
use xenonet_state::needs_you::{queue_of_summary, waits_words};

pub const HELP: &str = include_str!("help/status.txt");

const PRESENCES: [&str; 5] = ["working", "idle", "away", "offline", "unknown"];

fn need_words(kind: &str) -> String {
    match kind {
        "question" => "has a question".into(),
        "permission" => "is waiting for a permission".into(),
        k => k.into(),
    }
}

// The fields of `from` named, those it has.
fn pick(from: &Value, keys: &[&str]) -> Value {
    let mut m = Map::new();
    for k in keys {
        if let Some(v) = from.get(*k) {
            m.insert((*k).into(), v.clone());
        }
    }
    Value::Object(m)
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let Some(s) = find(flags) else {
        if flags.on("json") {
            json(&json!({ "running": false }));
        } else {
            print(format!(
                "{} XenoNet isn't running {}\n  Open XenoNet, or start XenoNet Dev from its checkout with npm run app.",
                c::dim("○"),
                c::dim(format!("(nothing answers on {})", where_(flags)))
            ));
        }
        return Ok(1);
    };
    let summary = s.get("/api/summary")?;
    let all = s.get("/api/engine/agents").unwrap_or_else(|_| json!([]));
    let actor_list = s.get("/api/actors").ok();
    let doctor = s.get("/api/doctor").ok().filter(|d| !d.is_null());
    // The engine counts actors among the agents; here they're apart, as the
    // actors' list has them (you, outside XenoNet's window, only a placeholder).
    let agents: Vec<Value> = all.as_array().into_iter().flatten().filter(|a| str(a, "app") != Some("actor")).cloned().collect();
    let actors: Option<Vec<Value>> = actor_list.as_ref().filter(|l| !l.is_null()).map(|l| {
        list(l, "actors")
            .iter()
            .filter(|a| str(a, "id") != Some("actor:you") || str(a, "presence") != Some("unknown"))
            .map(|a| pick(a, &["id", "name", "presence", "status"]))
            .collect()
    });
    // What waits on you, in the order the Needs you panel lists it.
    let queue = queue_of_summary(Some(&summary));
    let health = &s.health;
    if flags.on("json") {
        let mut out = Map::new();
        out.insert("running".into(), json!(true));
        out.insert("name".into(), json!(name_of(&s)));
        out.insert("version".into(), have(health, "version").cloned().unwrap_or(Value::Null));
        out.insert("port".into(), json!(s.port));
        out.insert("demo".into(), json!(on(health, "demo")));
        if let Some(sources) = health.get("sources") {
            out.insert("sources".into(), sources.clone());
        }
        out.insert("summary".into(), summary.clone());
        out.insert("queue".into(), Value::Array(queue));
        out.insert("agents".into(), Value::Array(agents));
        out.insert("actors".into(), actors.map_or(Value::Null, Value::Array));
        out.insert("doctor".into(), doctor.as_ref().map_or(Value::Null, |d| pick(d, &["at", "summary"])));
        json(&Value::Object(out));
        return Ok(0);
    }

    let count = |name: &str| {
        list(health, "sources").iter().find(|x| str(x, "name") == Some(name) && on(x, "ok")).and_then(|x| have(x, "count")).map(|n| xenonet_base::text::js_number(Some(n)))
    };
    let reads: Vec<String> =
        [("projects", "project"), ("claude", "Claude session"), ("codex", "Codex thread")].iter().filter_map(|(k, word)| count(k).map(|n| plural(n, word))).collect();
    let connected = agents.iter().filter(|a| on(a, "reachable")).count();
    let working = agents.iter().filter(|a| str(a, "status") == Some("working")).count();
    let mut rows = vec![
        vec![c::dim("Reads"), if reads.is_empty() { "nothing yet".into() } else { reads.join(", ") }],
        vec![
            c::dim("Sessions"),
            if agents.is_empty() {
                "none at work in the last two hours".into()
            } else {
                format!("{working} working, {} idle{}", agents.len() - working, if connected > 0 { format!(", {connected} connected to XenoNet") } else { String::new() })
            },
        ],
    ];
    // Actors only where there are some, or the line would be noise.
    if let Some(actors) = actors.as_ref().filter(|a| !a.is_empty()) {
        let by: Vec<String> =
            PRESENCES.iter().map(|p| (p, actors.iter().filter(|a| str(a, "presence") == Some(p)).count())).filter(|(_, n)| *n > 0).map(|(p, n)| format!("{n} {p}")).collect();
        rows.push(vec![c::dim("Actors"), by.join(", ")]);
    }
    // A session on another machine (XENONET_HOSTS=1) says which.
    let named = |n: &Value| format!("{}{}", if on(n, "host") { format!("{} · ", t(n, "host")) } else { String::new() }, t(n, "title"));
    // A program asking to be an actor is answered only in XenoNet's window.
    let waits = |it: &Value| match str(it, "kind") {
        Some("requests") => format!("{} {} {}", c::yellow("◇"), t(it, "title"), c::dim(format!("{}; answer in XenoNet's window", waits_words(it)))),
        Some("actor") => format!("{} {} {}", c::yellow("◇"), t(it, "title"), c::dim(format!("(actor) {}", waits_words(it)))),
        _ => format!("{} {} {}", c::yellow("●"), named(it), c::dim(format!("({}) {}", t(it, "app"), need_words(&t(it, "kind"))))),
    };
    for (i, it) in queue.iter().enumerate() {
        rows.push(vec![if i > 0 { String::new() } else { c::dim("Needs you") }, waits(it)]);
    }
    let stuck = list(&summary, "stuck");
    if !stuck.is_empty() {
        rows.push(vec![c::dim("Stuck"), stuck.iter().map(named).collect::<Vec<_>>().join(", ")]);
    }
    if let Some(d) = &doctor {
        let sum = d.get("summary").cloned().unwrap_or(Value::Null);
        let of = |k: &str| if sum.get(k).is_none() { 0.0 } else { num(&sum, k) };
        let (fail, warn) = (of("fail"), of("warn"));
        let is = |n: f64| n != 0.0 && !n.is_nan();
        let verdict = if is(fail) || is(warn) {
            let mut said = Vec::new();
            if is(fail) {
                said.push(plural(fail, "problem"));
            }
            if is(warn) {
                said.push(plural(warn, "warning"));
            }
            c::named(if is(fail) { "red" } else { "yellow" }, format!("{}, which xenonet doctor explains", said.join(" and ")))
        } else {
            "everything XenoNet needs is in place".into()
        };
        rows.push(vec![c::dim("Doctor"), format!("{verdict} {}", c::dim(format!("(checked {})", ago(num(d, "at"), now()))))]);
    }
    let version = if on(health, "version") { format!(" {}", t(health, "version")) } else { String::new() };
    print(format!("{} {}{version} {}", c::green("●"), c::bold(name_of(&s)), c::dim(format!("on 127.0.0.1:{}{}", s.port, if on(health, "demo") { ", made-up data" } else { "" }))));
    print(table(&rows, Layout::wrapped(2)));
    Ok(0)
}
