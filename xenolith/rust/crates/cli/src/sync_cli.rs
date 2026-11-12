//! Sync between machines, from a terminal (xenolith-net's `xenolith/sync/2`):
//!
//!   xenolith sync [status] [--json]                     this Xenolith's key and endpoint, the spaces
//!                                                       it syncs, their members, who's connected
//!   xenolith sync share <space> <key> <endpoint> [--level read|append|write]
//!                                                       share a space this Xenolith curates with
//!                                                       another Xenolith (its context key and its
//!                                                       endpoint id, from its `xenolith sync`)
//!   xenolith sync follow <spc_…> <curator key> <endpoint> <ip:port>... [--all]
//!                                                       follow a space another Xenolith curates
//!                                                       (--all: and every other space it shares
//!                                                       with this one)
//!
//! Share and follow go through the running service with the owner key. Joining with an invite
//! (`xenolith join`) will write the same records without the ids being typed.

use crate::owner_client;
use serde_json::{Value, json};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith sync: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith sync [status] [--json]\n       xenolith sync share <space> <key> <endpoint> [--level read|append|write] [--json]\n       xenolith sync follow <spc_…> <curator key> <endpoint> <ip:port>... [--all] [--json]"
    );
    std::process::exit(2)
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json").collect();
    match words.first().copied() {
        None | Some("status") => status(json_out),
        Some("share") => share(&words[1..], json_out),
        Some("follow") => follow(&words[1..], json_out),
        _ => usage(),
    }
}

fn status(json_out: bool) -> ! {
    let (code, v) = owner_client::get("/status").unwrap_or_else(|e| fail(&e));
    if code != 200 {
        fail(v["error"].as_str().unwrap_or("the service didn't answer"));
    }
    let s = &v["sync"];
    if json_out {
        println!("{}", json!({ "sync": s }));
        std::process::exit(0)
    }
    if s.is_null() {
        println!("Sync is off until the upgrade from 1.x has run (xenolith upgrade).");
        std::process::exit(0)
    }
    println!("This Xenolith: key {}, endpoint {}", s["key"].as_str().unwrap_or("?"), s["endpointId"].as_str().unwrap_or("?"));
    match s["endpoint"]["addrs"].as_array() {
        Some(a) => println!("  open at {}", a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
        None => println!("  the endpoint opens once a space is shared or followed"),
    }
    for sp in s["spaces"].as_array().into_iter().flatten() {
        println!("{} ({}), {}", sp["address"].as_str().unwrap_or(""), sp["id"].as_str().unwrap_or(""), sp["role"].as_str().unwrap_or(""));
        for m in sp["members"].as_array().into_iter().flatten() {
            let on = if m["connected"] == true { "connected" } else { "not connected" };
            println!("  {} {} at {}, {on}", m["key"].as_str().unwrap_or(""), m["level"].as_str().unwrap_or(""), m["endpoint"].as_str().unwrap_or(""));
        }
        if sp["role"] == "member" {
            println!("  curator {}, {}", sp["curator"].as_str().unwrap_or(""), if sp["curatorConnected"] == true { "connected" } else { "not connected" });
        }
    }
    for f in s["follows"].as_array().into_iter().flatten().filter(|f| f["held"] != true) {
        println!("{} followed from {}, waiting for its chain", f["space"].as_str().unwrap_or(""), f["curator"].as_str().unwrap_or(""));
    }
    std::process::exit(0)
}

fn answer(r: Result<(u16, Value), String>, json_out: bool, done: &str) -> ! {
    let (code, v) = r.unwrap_or_else(|e| fail(&e));
    if code != 200 {
        fail(v["error"].as_str().unwrap_or("refused"));
    }
    if json_out {
        println!("{v}");
    } else {
        println!("{done}");
    }
    std::process::exit(0)
}

fn share(w: &[&str], json_out: bool) -> ! {
    let level = w.iter().position(|a| *a == "--level").and_then(|i| w.get(i + 1)).copied().unwrap_or("append");
    let pos: Vec<&str> = w.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && (*i == 0 || w[i - 1] != "--level")).map(|(_, a)| *a).collect();
    let [space, key, endpoint] = pos[..] else { usage() };
    let body = json!({ "space": space, "key": key, "endpoint": endpoint, "level": level });
    answer(owner_client::post("/api/sync/share", &body), json_out, &format!("Shared {space} with {key} ({level})."))
}

fn follow(w: &[&str], json_out: bool) -> ! {
    let all = w.contains(&"--all");
    let pos: Vec<&str> = w.iter().filter(|a| !a.starts_with("--")).copied().collect();
    if pos.len() < 4 {
        usage();
    }
    let body = json!({ "space": pos[0], "curator": pos[1], "endpoint": pos[2], "addrs": pos[3..], "all": all });
    answer(owner_client::post("/api/sync/follow", &body), json_out, &format!("Following {} from {}.", pos[0], pos[1]))
}
