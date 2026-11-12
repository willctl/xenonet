//! The upgrade from 1.x, from a terminal (xenolith_core::upgrade):
//!
//!   xenolith upgrade [--json]          a dry run: what --run would found, move aside, mark and bind
//!   xenolith upgrade --run [--json]    do it, through the running service (the owner key), each
//!                                      change recorded in <state>/upgrade.json
//!   xenolith upgrade --undo [--json]   take back exactly what the upgrade recorded, and nothing else
//!
//! The undo works on the files directly, with the service running or stopped, so a rollback to 1.x
//! can stop 2.0 first. It goes in the audit chain as made by "you"; the run, by "cli".

use crate::access_cli::root;
use serde_json::{Value, json};
use xenolith_core::audit;
use xenolith_core::paths::AppPaths;
use xenolith_core::upgrade;
use xenolith_core::wiki;

fn fail(msg: &str) -> ! {
    eprintln!("xenolith upgrade: {msg}");
    std::process::exit(1)
}

fn names(v: &Value) -> String {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

fn count(v: &Value) -> usize {
    v.as_array().map_or(0, Vec::len)
}

pub fn run(args: &[String]) -> ! {
    let has = |f: &str| args.iter().any(|a| a == f);
    if let Some(a) = args.iter().find(|a| !["--run", "--undo", "--json"].contains(&a.as_str())) {
        eprintln!("xenolith upgrade: unknown argument {a}\nusage: xenolith upgrade [--run | --undo] [--json]");
        std::process::exit(2)
    }
    let json_out = has("--json");
    let paths = AppPaths::current();
    if has("--run") && has("--undo") {
        fail("--run or --undo, not both");
    }
    if has("--undo") {
        undo(&paths, json_out)
    }
    if has("--run") {
        ran(json_out)
    }
    dry_run(&paths, json_out)
}

fn dry_run(paths: &AppPaths, json_out: bool) -> ! {
    let v = upgrade::plan_json(paths, &root()).unwrap_or_else(|e| fail(&e));
    if json_out {
        println!("{}", json!({ "dryRun": true, "upgrade": v }));
        std::process::exit(0)
    }
    let p = &v["plan"];
    match v["stage"].as_str().unwrap_or("") {
        "done" | "fresh" => {
            if v["stage"] == "fresh" {
                println!("This folder was made by Xenolith 2.0 itself, so it counted as upgraded from the start.");
            } else {
                println!("The upgrade from 1.x has run.");
            }
            println!("{} change(s) are recorded in {}; `xenolith upgrade --undo` takes them back.", v["recorded"], v["manifest"].as_str().unwrap_or(""));
            std::process::exit(0)
        }
        "running" => println!("The upgrade stopped part way. `xenolith upgrade --run` finishes it; until then:"),
        "undoing" => {
            println!("An undo stopped part way: `xenolith upgrade --undo` finishes it.");
            std::process::exit(0)
        }
        _ => println!("The upgrade from 1.x is waiting. `xenolith upgrade --run` would:"),
    }
    let moves = p["moveAside"].as_array().cloned().unwrap_or_default();
    if moves.is_empty() {
        println!("- move nothing aside: this machine followed no space from another under 1.x");
    } else {
        println!("- move {} copy(ies) of spaces this machine followed under 1.x out of the folder, kept, never deleted:", moves.len());
        for m in &moves {
            println!("    {} -> {}", m["space"].as_str().unwrap_or(""), m["to"].as_str().unwrap_or(""));
        }
    }
    match count(&p["found"]) {
        0 => println!("- found no chain"),
        n => println!("- found a chain for {n} space(s) this machine curates, owned by this Xenolith's owner key: {}", names(&p["found"])),
    }
    match count(&p["mark"]) {
        0 => println!("- write no xenolith-space.json"),
        n => println!("- write xenolith-space.json into {n} space folder(s): {}", names(&p["mark"])),
    }
    println!("- bind nothing: {}", p["bindWhy"].as_str().unwrap_or(""));
    println!("It records each change in {} first, so `xenolith upgrade --undo` can take back exactly those. It deletes no 1.x file.", v["manifest"].as_str().unwrap_or(""));
    println!("Nothing was changed: this was a dry run.");
    std::process::exit(0)
}

fn ran(json_out: bool) -> ! {
    match crate::owner_client::post("/api/upgrade/run", &json!({})) {
        Ok((200, v)) => {
            if json_out {
                println!("{}", json!({ "ran": true, "upgrade": v }));
            } else if v["already"].is_string() {
                println!("The upgrade had run already: nothing was changed.");
            } else {
                println!("Upgraded. Moved aside: {}.", Some(names(&v["moved"])).filter(|s| !s.is_empty()).unwrap_or_else(|| "nothing".into()));
                println!("Founded: {}.", Some(names(&v["founded"])).filter(|s| !s.is_empty()).unwrap_or_else(|| "nothing".into()));
                println!("Marked: {}.", Some(names(&v["marked"])).filter(|s| !s.is_empty()).unwrap_or_else(|| "nothing".into()));
                println!("`xenolith upgrade --undo` takes it back; `xenolith doctor` names what 1.x left.");
            }
            std::process::exit(0)
        }
        Ok((_, v)) => fail(&format!("not run: {}", v["error"].as_str().unwrap_or("the service refused"))),
        Err(e) => fail(&e),
    }
}

fn undo(paths: &AppPaths, json_out: bool) -> ! {
    let r = upgrade::undo(paths, &root()).unwrap_or_else(|e| fail(&e));
    if r.stage.is_some() {
        audit::best_effort(paths, "upgrade", "you", vec![("event", json!("undo")), ("undone", json!(r.undone)), ("left", json!(r.left.len()))]);
    }
    if json_out {
        println!("{}", json!({ "undo": r.to_json() }));
    } else if r.stage.is_none() {
        println!("Nothing to undo: the upgrade from 1.x hasn't run here.");
    } else {
        println!("Took back {} change(s).", r.undone);
        for (p, why) in &r.left {
            println!("  ! left {}: {why}", wiki::disp(p));
        }
        if r.left.is_empty() {
            println!("The upgrade from 1.x is waiting again.");
        } else {
            println!("What was left stays recorded in {}; `xenolith upgrade --undo` tries it again.", wiki::disp(&upgrade::file(paths)));
        }
    }
    std::process::exit(if r.left.is_empty() { 0 } else { 1 })
}
