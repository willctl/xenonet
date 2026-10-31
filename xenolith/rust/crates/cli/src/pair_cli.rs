//! `xenolith pair`: joining machines with a code (xenolith_core::pair).
//!
//!   xenolith pair offer [--json]         on home: print a pairing code, valid for 10 minutes and one
//!                                        join (needs the running service and its owner key)
//!   xenolith pair join --url <base> [--name <machine>] [--peer <name>] [--json]
//!                                        on a satellite: the code comes from a prompt or stdin, never
//!                                        an argument; this machine then follows home's spaces

use serde_json::json;
use std::io::{BufRead, IsTerminal, Read, Write};
use xenolith_core::paths::AppPaths;
use xenolith_core::sync::{self, Config};

fn fail(msg: &str) -> ! {
    eprintln!("xenolith pair: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!("usage: xenolith pair offer [--json] | xenolith pair join --url <base> [--name <machine>] [--peer <name>] [--json]");
    std::process::exit(2)
}

/// This machine's name for a pairing, as a valid machine name.
fn default_name() -> String {
    let raw = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")).unwrap_or_default();
    let name: String = raw.to_lowercase().split('.').next().unwrap_or("").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    let name = name.trim_matches('-').to_string();
    if sync::valid_name(&name) { name } else { "satellite".into() }
}

pub fn run(args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let paths = AppPaths::current();
    match args.first().map(String::as_str) {
        Some("offer") => match crate::owner_client::post("/api/pair/offer", &json!({})) {
            Ok((200, v)) => {
                if json_out {
                    println!("{v}");
                } else {
                    println!("{}", v["code"].as_str().unwrap_or(""));
                    eprintln!(
                        "Valid for {} minutes, for one join. On the other machine: xenolith pair join --url <this machine's address>, then paste the code.",
                        v["expiresInSeconds"].as_i64().unwrap_or(0) / 60
                    );
                }
                std::process::exit(0)
            }
            Ok((_, v)) => fail(v["error"].as_str().unwrap_or("the service refused")),
            Err(e) => fail(&e),
        },
        Some("join") => {
            let Some(url) = opt("--url") else { fail("--url <base> is required: the address of home's Xenolith service") };
            let code = if std::io::stdin().is_terminal() {
                eprint!("Pairing code: ");
                let _ = std::io::stderr().flush();
                let mut line = String::new();
                let _ = std::io::stdin().lock().read_line(&mut line);
                line
            } else {
                let mut all = String::new();
                let _ = std::io::stdin().read_to_string(&mut all);
                all
            };
            let machine = opt("--name").or_else(|| Config::load(&paths).machine).unwrap_or_else(default_name);
            let peer = opt("--peer").unwrap_or_else(|| "home".into());
            match xenolith_core::pair::join(&paths, &crate::access_cli::root(), &url, &code, &machine, &peer) {
                Ok(j) => {
                    if json_out {
                        println!("{}", json!({ "home": j.peer, "machine": j.machine, "token": j.token_id, "followedSpaces": j.spaces }));
                    } else {
                        println!(
                            "Paired: this machine is {} and a satellite of {}. It follows home's spaces ({} new) and keeps their copies in step; its own curator and Ask are off. Restart Xenolith (quit the tray and open it again) for that to take effect if it is running.",
                            j.machine, j.peer, j.spaces
                        );
                    }
                    std::process::exit(0)
                }
                Err(e) => fail(&e),
            }
        }
        _ => usage(),
    }
}
