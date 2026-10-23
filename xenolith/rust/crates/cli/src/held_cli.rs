//! `xenolith held`: the changes that wait for your OK, in any space, from a terminal. The window
//! shows this machine's space; proposals that came in by sync wait in the space they are for.
//!
//!   xenolith held [--space <id>] [--json]                       what waits, each as <batch>:<index>
//!   xenolith held approve|deny|refile <id> [--space <id>]       decide one (<id>: <batch>:<index>)
//!
//! Listing reads the files. Deciding goes through the running service with the owner key
//! (owner_client.rs), like the window, so it needs an explicit read of that owner-only file and the
//! audit chain records it as made by "cli". `reject` is another word for `deny`, and
//! `<batch> <index>` works as well as `<batch>:<index>`.

use serde_json::{Value, json};
use xenolith_core::held;
use xenolith_core::spaces::{self, SpaceId};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith held: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!("usage: xenolith held [--space <id>] [--json] | xenolith held approve|deny|refile <batch>:<index> [--space <id>] [--json]");
    std::process::exit(2)
}

/// `<batch>:<index>` or `<batch> <index>`.
fn change_id<'a>(rest: &[&'a str]) -> (&'a str, usize) {
    let (batch, index) = match rest {
        [id] => id.rsplit_once(':').unwrap_or_else(|| usage()),
        [batch, index] => (*batch, *index),
        _ => usage(),
    };
    (batch, index.parse().unwrap_or_else(|_| fail("the index after the batch is a number")))
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
    let id = space.as_ref().map(|s| {
        let id = SpaceId::parse(s).unwrap_or_else(|e| fail(&e.to_string()));
        if !spaces::exists(&root, &id) {
            fail(&format!("no space {id}"));
        }
        id
    });
    match pos.as_slice() {
        [] => {
            let dir = id.as_ref().map(|i| spaces::space_dir(&root, i)).unwrap_or(root);
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
                    println!("{}:{}  {kind}: {} [[{}]]", c["batch"].as_str().unwrap_or(""), c["index"], c["title"].as_str().unwrap_or(""), c["slug"].as_str().unwrap_or(""));
                    for r in c["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                        println!("    {r}");
                    }
                }
            }
            std::process::exit(0)
        }
        [action @ ("approve" | "deny" | "reject" | "refile"), rest @ ..] => {
            let (batch, index) = change_id(rest);
            let action = if *action == "deny" { "reject" } else { action };
            let mut body = json!({ "batch": batch, "index": index, "action": action });
            if let Some(id) = &id {
                body["space"] = json!(id.to_string());
            }
            match crate::owner_client::post("/api/held", &body) {
                Ok((200, v)) => {
                    if json_out {
                        println!("{v}");
                    } else {
                        println!("{}", v["outcome"].as_str().unwrap_or("done"));
                    }
                    std::process::exit(0)
                }
                Ok((_, v)) => fail(v["error"].as_str().unwrap_or("the service refused")),
                Err(e) => fail(&e),
            }
        }
        _ => usage(),
    }
}
