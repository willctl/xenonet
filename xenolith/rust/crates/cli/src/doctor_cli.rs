//! `xenolith doctor [--json]`: where the upgrade from 1.x stands, and what 1.x left that 2.0
//! doesn't read and never deletes (xenolith_core::upgrade::leftovers), for the person to delete
//! once they no longer want a way back to 1.x. Reads only. Exits 1 when the upgrade can't be read
//! or stopped part way.

use crate::access_cli::root;
use serde_json::{Value, json};
use xenolith_core::paths::AppPaths;
use xenolith_core::upgrade;
use xenolith_core::wiki;

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let paths = AppPaths::current();
    let up = upgrade::status_json(&paths);
    let left = upgrade::leftovers(&paths, &root());
    let ok = matches!(up["stage"].as_str(), Some("waiting" | "fresh" | "done"));
    if json_out {
        let left: Vec<Value> = left.iter().map(|l| json!({ "what": l.what, "path": wiki::disp(&l.path) })).collect();
        println!("{}", json!({ "ok": ok, "upgrade": up, "leftovers": left }));
        std::process::exit(if ok { 0 } else { 1 })
    }
    let mark = if ok { " " } else { "!" };
    match up["stage"].as_str().unwrap_or("") {
        "fresh" => println!("{mark} upgrade: this folder was made by 2.0, so it counted as upgraded from the start"),
        "done" => println!("{mark} upgrade: ran; {} change(s) recorded, `xenolith upgrade --undo` takes them back", up["changes"]),
        _ => println!("{mark} upgrade: {}", up["next"].as_str().or(up["error"].as_str()).unwrap_or("unknown")),
    }
    if left.is_empty() {
        println!("  1.x leftovers: none");
    } else {
        println!("  1.x leftovers (2.0 doesn't read them and never deletes them; delete them once you no longer want a way back to 1.x):");
        for l in &left {
            println!("    {}: {}", l.what, wiki::disp(&l.path));
        }
    }
    std::process::exit(if ok { 0 } else { 1 })
}
