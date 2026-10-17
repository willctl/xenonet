//! `xenolith sync`: keeping copies of spaces another machine curates (xenolith_core::sync).
//!
//!   xenolith sync [--json]                       push and pull every followed space now
//!   xenolith sync status [--json]                this machine's name, its peers and what it follows
//!   xenolith sync name <machine>                 this machine's name, as curators' grants know it
//!   xenolith sync peer add <name> <url>          a peer's Xenolith service; its token comes on stdin
//!   xenolith sync peer remove <name>
//!   xenolith sync follow <space> <peer>          keep a copy of a space the peer curates
//!   xenolith sync unfollow <space>               curate it here again (its copy stays as it is)

use serde_json::{Value, json};
use std::io::Read;
use xenolith_core::paths::AppPaths;
use xenolith_core::spaces::SpaceId;
use xenolith_core::sync::{self, Config, valid_name, valid_url};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith sync: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!("usage: xenolith sync [status | name <machine> | peer add <name> <url> | peer remove <name> | follow <space> <peer> | unfollow <space>] [--json]");
    std::process::exit(2)
}

fn done(json_out: bool, v: Value, text: &str) -> ! {
    if json_out {
        println!("{v}");
    } else {
        println!("{text}");
    }
    std::process::exit(0)
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let pos: Vec<&str> = args.iter().filter(|a| !a.starts_with("--")).map(String::as_str).collect();
    let paths = AppPaths::current();
    let change = |f: &dyn Fn(&mut Config) -> xenolith_core::wiki::Result<()>| sync::update(&paths, |c| f(c)).unwrap_or_else(|e| fail(&e.to_string()));
    match pos.as_slice() {
        [] => {
            let root = crate::access_cli::root();
            let reports = sync::sync_all(&paths, &root);
            let failed = reports.iter().any(|r| r.get("error").is_some());
            if json_out {
                println!("{}", json!({ "spaces": reports }));
            } else if reports.is_empty() {
                println!("This machine follows no space. `xenolith sync follow <space> <peer>` starts one.");
            } else {
                for r in &reports {
                    match r["error"].as_str() {
                        Some(e) => println!("{}: {e}", r["space"].as_str().unwrap_or("?")),
                        None => println!(
                            "{} with {}: pushed {}, {} proposal(s), {} refused; pulled {} page(s).",
                            r["space"].as_str().unwrap_or("?"),
                            r["peer"].as_str().unwrap_or("?"),
                            r["pushed"],
                            r["conflicts"],
                            r["refused"],
                            r["pulled"]
                        ),
                    }
                }
            }
            std::process::exit(if failed { 1 } else { 0 })
        }
        ["status"] => {
            let c = Config::load(&paths);
            let peers: Vec<Value> = c.peers.iter().map(|(n, u)| json!({ "name": n, "url": u, "token": sync::token_file(&paths, n).exists() })).collect();
            let v = json!({ "machine": c.machine, "peers": peers, "follow": c.follow });
            let mut lines = vec![format!("This machine: {}.", c.machine.as_deref().unwrap_or("(no name yet: xenolith sync name <machine>)"))];
            for p in &peers {
                lines.push(format!("peer {} at {}{}", p["name"].as_str().unwrap_or(""), p["url"].as_str().unwrap_or(""), if p["token"] == true { "" } else { " (no token)" }));
            }
            for (s, p) in &c.follow {
                lines.push(format!("follows {s}, curated by {p}"));
            }
            done(json_out, v, &lines.join("\n"))
        }
        ["name", machine] => {
            if !valid_name(machine) {
                fail("a machine name is lowercase letters, digits, '-' and '.'");
            }
            change(&|c| {
                c.machine = Some(machine.to_string());
                Ok(())
            });
            done(json_out, json!({ "machine": machine }), &format!("This machine is {machine} to its peers: their grants name machine:{machine}."))
        }
        ["peer", "add", name, url] => {
            if !valid_name(name) {
                fail("a peer name is lowercase letters, digits, '-' and '.'");
            }
            if !valid_url(url) {
                fail("the URL must be https://, or http:// to 127.0.0.1 or localhost (a token never crosses a network in clear)");
            }
            // The token comes on stdin, so it never shows in a process list or a shell history.
            let mut token = String::new();
            let _ = std::io::stdin().read_to_string(&mut token);
            let token = token.trim().to_string();
            if !xenolith_core::access::is_token(&token) {
                fail("give the peer's token for this machine on stdin (xenolith_ and 64 hex digits), e.g. from a file: < token.txt");
            }
            sync::save_token(&paths, name, &token).unwrap_or_else(|e| fail(&e.to_string()));
            change(&|c| {
                c.peers.insert(name.to_string(), url.trim_end_matches('/').to_string());
                Ok(())
            });
            done(json_out, json!({ "peer": name, "url": url.trim_end_matches('/') }), &format!("Peer {name} at {url}, with its token."))
        }
        ["peer", "remove", name] => {
            change(&|c| {
                c.peers.remove(*name);
                c.follow.retain(|_, p| p != name);
                Ok(())
            });
            let _ = std::fs::remove_file(sync::token_file(&paths, name));
            done(json_out, json!({ "removed": name }), &format!("Peer {name} removed, with what this machine followed from it."))
        }
        ["follow", space, peer] => {
            let id = SpaceId::parse(space).unwrap_or_else(|e| fail(&e.to_string()));
            if id.is_local() {
                fail("machine:local is this machine's own space: it is always curated here");
            }
            if !Config::load(&paths).peers.contains_key(*peer) {
                fail(&format!("no peer named {peer}: xenolith sync peer add {peer} <url> first"));
            }
            change(&|c| {
                c.follow.insert(id.to_string(), peer.to_string());
                Ok(())
            });
            done(json_out, json!({ "space": id.to_string(), "peer": peer }), &format!("This machine follows {id}: {peer} curates it, and `xenolith sync` keeps the copy."))
        }
        ["unfollow", space] => {
            let id = SpaceId::parse(space).unwrap_or_else(|e| fail(&e.to_string()));
            change(&|c| {
                c.follow.remove(&id.to_string());
                Ok(())
            });
            done(json_out, json!({ "space": id.to_string() }), &format!("{id} is curated here again."))
        }
        _ => usage(),
    }
}
