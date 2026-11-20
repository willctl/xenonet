//! xenonet folders: the folders each app is read from, and naming another
//! (through a running XenoNet's API, which is yours to change).

use crate::args::Flags;
use crate::js::{list, str, t};
use crate::out::{c, json, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Value};

pub const HELP: &str = include_str!("help/folders.txt");

fn shown(home: &str, p: &str) -> String {
    xenonet_os::paths::tilde(p, home)
}

fn show(found: &Value, home: &str) {
    for app in list(found, "apps") {
        print(c::bold(t(app, "name")));
        for r in list(app, "roots") {
            let named = r.get("named") == Some(&Value::Bool(true));
            let what = if named { "named" } else { "its own" };
            print(format!("  {} {} {}", c::green("●"), shown(home, &t(r, "folder")), c::dim(what)));
        }
        for cand in list(app, "candidates") {
            let path = cand.as_str().unwrap_or_default();
            print(format!("  {} {} {}", c::dim("○"), shown(home, path), c::dim(format!("looks like one; xenonet folders add {} {} reads it", t(app, "app"), shown(home, path)))));
        }
    }
    for h in list(found, "hooks") {
        let root = shown(home, &t(h, "root"));
        if str(h, "hooks") == Some("removed") {
            print(format!("{} XenoNet's hooks are out of {root}.", c::green("✓")));
        } else {
            print(c::yellow(format!("! XenoNet's hooks are still in {root}: {}", t(h, "why"))));
        }
    }
    if found.get("restart") == Some(&Value::Bool(true)) {
        print(c::yellow("! The folders you named change when XenoNet's service starts again: xenonet service stop, then xenonet service start."));
    }
}

/// The folder as given here; ~ is left for XenoNet.
fn folder(arg: &str) -> String {
    crate::commands::root::folder(arg)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let sub = args.first().map_or("list", String::as_str);
    let s = need(flags)?;
    let found = s.get("/api/workspace/app-roots")?;
    let home = std::env::var("HOME").unwrap_or_default();
    match sub {
        "list" => {
            if flags.on("json") {
                json(&found);
            } else {
                show(&found, &home);
            }
            Ok(0)
        }
        "add" | "remove" => {
            let (Some(app), Some(dir)) = (args.get(1), args.get(2)) else {
                return Err(Fail::new(format!("say which app and folder: xenonet folders {sub} <app> <folder>"), 2));
            };
            let entry = list(&found, "apps")
                .iter()
                .find(|a| str(a, "app") == Some(app.as_str()))
                .ok_or_else(|| Fail::new(format!("{app} has no folders to name. xenonet folders lists the apps that do."), 2))?;
            let dir = folder(dir);
            let mut named: Vec<String> =
                list(entry, "roots").iter().filter(|r| r.get("named") == Some(&Value::Bool(true))).filter_map(|r| str(r, "root").map(str::to_string)).collect();
            if sub == "add" {
                if !named.contains(&dir) {
                    named.push(dir);
                }
            } else {
                // A folder as it was named, or as this machine spells it.
                named.retain(|n| *n != dir && n.trim_end_matches('/') != dir.trim_end_matches('/'));
            }
            let after = s.request("PUT", "/api/workspace/app-roots", Some(&json!({ "app": app, "roots": named })), &[]).map_err(|r| Fail::new(r.message, 1))?;
            if flags.on("json") {
                json(&after);
            } else {
                show(&after, &home);
            }
            Ok(0)
        }
        other => Err(Fail::new(format!("xenonet folders {other}? It takes list, add or remove."), 2)),
    }
}
