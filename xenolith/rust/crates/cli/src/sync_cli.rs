//! Sync between machines, from a terminal (xenolith-net's `xenolith/sync/2` and `xenolith/join/1`):
//!
//!   xenolith sync [status] [--json]          this Xenolith's key and endpoint, the spaces it syncs,
//!                                            their members, who's connected, and shared mode
//!   xenolith sync grant                      keeps the grant XenoNet gave this Xenolith, read
//!                                            from stdin (never an argument): shared mode
//!                                            attaches to XenoNet's forward with it
//!   xenolith invite <space>... [--level read|append|write] [--json]
//!                                            an invite for one other Xenolith to join these spaces
//!                                            (this one curates them): the invite, its code and the
//!                                            line to run there; it works once, for ten minutes
//!   xenolith join <invite> [--at <ip:port>]... [--only <space>]... [--all] [--json]
//!                                            join with an invite; the code is read from a prompt
//!                                            or stdin, never an argument (--all: and every space
//!                                            the curator shares with this one later)
//!
//! Each goes through the running service with the owner key. There is no command that writes a
//! share or a follow record by hand: a join writes both, after both sides proved the code and the
//! joiner checked the chains it was sent.

use crate::owner_client;
use serde_json::{Value, json};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith sync: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith sync [status] [--json]\n       xenolith sync grant  (the grant on stdin)\n       xenolith invite <space>... [--level read|append|write] [--json]\n       xenolith join <invite> [--at <ip:port>]... [--only <space>]... [--all] [--json]"
    );
    std::process::exit(2)
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let words: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json").collect();
    match words.first().copied() {
        None | Some("status") => status(json_out),
        Some("grant") => grant(),
        _ => usage(),
    }
}

/// `xenolith sync grant`: the grant from stdin, to the service.
fn grant() -> ! {
    use std::io::Read as _;
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    if text.trim().is_empty() {
        fail("give the grant XenoNet made for this Xenolith on stdin");
    }
    match owner_client::post("/api/sync/grant", &json!({ "grant": text.trim() })) {
        Ok((200, v)) => {
            println!("Kept. Shared mode attaches to XenoNet's forward at {}.", v["socket"].as_str().unwrap_or("its socket"));
            std::process::exit(0)
        }
        Ok((_, v)) => fail(v["error"].as_str().unwrap_or("refused")),
        Err(e) => fail(&e),
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

/// The values after each `flag`, and the words that are no flag's.
fn split<'a>(w: &[&'a str], flags: &[&str]) -> (Vec<&'a str>, Vec<(&'a str, &'a str)>) {
    let mut pos = vec![];
    let mut named = vec![];
    let mut i = 0;
    while i < w.len() {
        if flags.contains(&w[i]) {
            let Some(v) = w.get(i + 1) else { usage() };
            named.push((w[i], *v));
            i += 2;
        } else {
            pos.push(w[i]);
            i += 1;
        }
    }
    (pos, named)
}

/// `xenolith invite`.
pub fn invite(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let w: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json").collect();
    let (spaces, named) = split(&w, &["--level"]);
    if spaces.is_empty() || spaces.iter().any(|s| s.starts_with("--")) {
        usage();
    }
    let level = named.iter().find(|(f, _)| *f == "--level").map_or("append", |(_, v)| *v);
    let (code, v) = owner_client::post("/api/sync/invite", &json!({ "spaces": spaces, "level": level })).unwrap_or_else(|e| fail(&e));
    if code != 200 {
        fail(v["error"].as_str().unwrap_or("refused"));
    }
    if json_out {
        println!("{v}");
        std::process::exit(0)
    }
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    println!("An invite for one Xenolith to join, at {}, owned by {}:", s("level"), v["offer"]["owner"].as_str().unwrap_or("?"));
    for sp in v["offer"]["spaces"].as_array().into_iter().flatten() {
        println!("  {} ({}, founded as {})", sp["address"].as_str().unwrap_or(""), sp["space"].as_str().unwrap_or(""), sp["genesis"].as_str().unwrap_or(""));
    }
    let at: Vec<String> = v["at"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|a| format!(" --at {a}")).collect();
    println!("\nOn the other machine, run:\n\n  xenolith join {}{}\n", s("invite"), at.join(""));
    println!("and type this code when it asks (tell it another way than the invite; it works once, for ten minutes):\n\n  {}\n", s("code"));
    std::process::exit(0)
}

/// The code, from a prompt when stdin is a terminal, else stdin's first line.
fn read_code() -> String {
    use std::io::{BufRead, IsTerminal, Write};
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        eprint!("Code: ");
        let _ = std::io::stderr().flush();
    }
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).is_err() || line.trim().is_empty() {
        fail("no code: type the code `xenolith invite` showed on the other machine, or give it on stdin");
    }
    line.trim().to_string()
}

/// `xenolith join`.
pub fn join(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let all = args.iter().any(|a| a == "--all");
    let w: Vec<&str> = args.iter().map(String::as_str).filter(|a| *a != "--json" && *a != "--all").collect();
    let (pos, named) = split(&w, &["--at", "--only"]);
    let [invite] = pos[..] else { usage() };
    if invite.starts_with("--") {
        usage();
    }
    let at: Vec<&str> = named.iter().filter(|(f, _)| *f == "--at").map(|(_, v)| *v).collect();
    let only: Vec<&str> = named.iter().filter(|(f, _)| *f == "--only").map(|(_, v)| *v).collect();
    let code = read_code();
    let (status, v) = owner_client::post("/api/sync/join", &json!({ "invite": invite, "code": code, "at": at, "only": only, "all": all })).unwrap_or_else(|e| fail(&e));
    if status != 200 {
        fail(v["error"].as_str().unwrap_or("refused"));
    }
    if json_out {
        println!("{v}");
        std::process::exit(0)
    }
    println!("Joined: spaces owned by {}, curated by {}, at {}.", v["owner"].as_str().unwrap_or("?"), v["curator"].as_str().unwrap_or("?"), v["level"].as_str().unwrap_or("?"));
    for f in v["followed"].as_array().into_iter().flatten() {
        println!("  following {} as {} (founded as {})", f["offeredAs"].as_str().unwrap_or(""), f["address"].as_str().unwrap_or(""), f["genesis"].as_str().unwrap_or(""));
    }
    for f in v["skipped"].as_array().into_iter().flatten() {
        println!("  not following {} (founded as {})", f["offeredAs"].as_str().unwrap_or(""), f["genesis"].as_str().unwrap_or(""));
    }
    std::process::exit(0)
}
