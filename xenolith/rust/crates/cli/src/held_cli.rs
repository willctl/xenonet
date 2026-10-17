//! `xenolith held`: the changes that wait for your OK, in any space, from a terminal. The window
//! shows this machine's space; proposals that came in by sync wait in the space they are for.
//!
//!   xenolith held [--space <id>] [--json]                                  what waits
//!   xenolith held approve|reject|refile <batch> <index> [--space <id>]     decide one

use serde_json::{Value, json};
use xenolith_core::held;
use xenolith_core::spaces::{self, SpaceId};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith held: {msg}");
    std::process::exit(1)
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let space = args.windows(2).find(|w| w[0] == "--space").map(|w| w[1].clone());
    let mut pos = vec![];
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a == "--space" {
            skip = true;
        } else if !a.starts_with("--") {
            pos.push(a.as_str());
        }
    }
    let root = crate::access_cli::root();
    let dir = match &space {
        None => root.clone(),
        Some(s) => {
            let id = SpaceId::parse(s).unwrap_or_else(|e| fail(&e.to_string()));
            if !spaces::exists(&root, &id) {
                fail(&format!("no space {id}"));
            }
            spaces::space_dir(&root, &id)
        }
    };
    match pos.as_slice() {
        [] => {
            let changes = held::list(&dir);
            if json_out {
                println!("{}", json!({ "changes": changes }));
            } else if changes.is_empty() {
                println!("Nothing waits for your OK{}.", space.as_ref().map(|s| format!(" in {s}")).unwrap_or_default());
            } else {
                for c in &changes {
                    let kind = match c["kind"].as_str() {
                        Some("sync") => "proposal from another machine",
                        Some("lint") => "cleanup",
                        _ => "change",
                    };
                    println!("{} {}  {kind}: {} [[{}]]", c["batch"].as_str().unwrap_or(""), c["index"], c["title"].as_str().unwrap_or(""), c["slug"].as_str().unwrap_or(""));
                    for r in c["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                        println!("    {r}");
                    }
                }
            }
            std::process::exit(0)
        }
        [action @ ("approve" | "reject" | "refile"), batch, index] => {
            let index: usize = index.parse().unwrap_or_else(|_| fail("the index is a number"));
            match held::decide(&dir, batch, index, action) {
                Ok(v) => {
                    if json_out {
                        println!("{v}");
                    } else {
                        println!("{}", v["outcome"].as_str().unwrap_or("done"));
                    }
                    std::process::exit(0)
                }
                Err(held::Error::NotFound(m) | held::Error::Conflict(m)) => fail(&m),
                Err(held::Error::Wiki(e)) => fail(&e.to_string()),
            }
        }
        _ => {
            eprintln!("usage: xenolith held [--space <id>] [--json] | xenolith held approve|reject|refile <batch> <index> [--space <id>]");
            std::process::exit(2)
        }
    }
}
