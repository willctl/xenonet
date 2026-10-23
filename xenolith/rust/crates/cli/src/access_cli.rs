//! Spaces and who may use them, from a terminal (xenolith_core::spaces and access):
//!
//!   xenolith spaces [--json]                          the spaces, and each one's pages
//!   xenolith spaces add <space> [--identity <name>]   start a space; a project's identities let
//!                                                      identity:<name> grants reach it
//!   xenolith grant <subject> <scope> [--days N]       allow a subject (actor:<slug>, client:<id>,
//!                                                      machine:<name>) a scope, for N days (30)
//!   xenolith revoke <subject> [<scope>]               take its grants away: all, or one scope
//!   xenolith access [--json]                          the policy: grants, tokens (never secrets), spaces
//!   xenolith access local <read|append|write|admin|none>
//!                                                      what this machine's own processes may do
//!                                                      without a token (admin by default)
//!   xenolith token create <subject> [--scope "<scopes>"] [--label <text>] [--days N]
//!                                                      a token for the subject, shown once; its scopes
//!                                                      default to what the subject is granted now
//!   xenolith token revoke <tok_…|subject>             revoke one token, or every token of a subject
//!   xenolith audit [verify | tail [N]] [--json]       check the audit chain, or show its last entries
//!
//! Each change goes in the audit chain (xenolith_core::audit) as made by "you".

use serde_json::{Value, json};
use std::path::PathBuf;
use xenolith_core::access::{self, GRANT_MS, Policy, format_scope, format_selector, parse_scope};
use xenolith_core::audit;
use xenolith_core::paths::AppPaths;
use xenolith_core::spaces::{self, SpaceId};
use xenolith_core::text::{from_ms, local_iso, now_ms};
use xenolith_core::wiki;

fn fail(msg: &str) -> ! {
    eprintln!("xenolith: {msg}");
    std::process::exit(1)
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith spaces [add <space> [--identity <name>]] [--json] | xenolith grant <subject> <scope> [--days N] | xenolith revoke <subject> [<scope>] | xenolith access [--json] | xenolith access local <read|append|write|admin|none> | xenolith token create <subject> [--scope \"<scopes>\"] [--label <text>] [--days N] | xenolith token revoke <tok_…|subject> | xenolith audit [verify | tail [N]] [--json]"
    );
    std::process::exit(2)
}

pub fn root() -> PathBuf {
    match wiki::resolve_wiki_dir(&crate::mcp::env()) {
        Ok((d, _)) => d,
        Err(e) => fail(&e.to_string()),
    }
}

fn positional(args: &[String]) -> Vec<&str> {
    let mut out = vec![];
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if matches!(a.as_str(), "--days" | "--identity" | "--label" | "--scope" | "--hours") {
            skip = true;
        } else if !a.starts_with("--") {
            out.push(a.as_str());
        }
    }
    out
}

fn opts<'a>(args: &'a [String], name: &str) -> Vec<&'a str> {
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].as_str()).collect()
}

fn when(ms: i64) -> String {
    local_iso(&from_ms(ms))
}

/// The one space a selector names exactly (project:atlas, by:hermes → actor:hermes), if any:
/// a grant on it starts it, so a subject that may only append there can.
fn named_space(scope: &access::Scope) -> Option<SpaceId> {
    let [(k, v)] = scope.selector.terms.as_slice() else { return None };
    if v.ends_with('*') {
        return None;
    }
    let kind = if k == "by" { "actor" } else { k.as_str() };
    SpaceId::parse(&format!("{kind}:{v}")).ok()
}

pub fn run(cmd: &str, args: &[String]) -> ! {
    let json_out = args.iter().any(|a| a == "--json");
    let paths = AppPaths::current();
    let now = now_ms();
    let pos = positional(args);
    match (cmd, pos.as_slice()) {
        ("spaces", []) => {
            let root = root();
            let policy = access::load(&paths);
            let list: Vec<Value> = spaces::list(&root)
                .into_iter()
                .map(|id| {
                    let dir = spaces::space_dir(&root, &id);
                    let pages = wiki::list_pages(&dir).map(|p| p.len()).unwrap_or(0);
                    json!({ "space": id.to_string(), "pages": pages, "identities": policy.facts(&id).identities })
                })
                .collect();
            if json_out {
                println!("{}", json!({ "spaces": list }));
            } else {
                for s in &list {
                    let ids =
                        s["identities"].as_array().filter(|a| !a.is_empty()).map(|a| format!("  (identity {})", a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))).unwrap_or_default();
                    println!("{:<32} {:>4} page(s){ids}", s["space"].as_str().unwrap_or(""), s["pages"]);
                }
            }
            std::process::exit(0)
        }
        ("spaces", ["add", space]) => {
            let id = SpaceId::parse(space).unwrap_or_else(|e| fail(&e.to_string()));
            let identities: Vec<String> = opts(args, "--identity").iter().map(|i| i.trim().to_lowercase()).filter(|i| !i.is_empty()).collect();
            if !identities.is_empty() && id.kind != "project" {
                fail("--identity is for project spaces: an identity's own space is identity:<name>");
            }
            let root = root();
            let dir = spaces::create(&root, &id).unwrap_or_else(|e| fail(&e.to_string()));
            if !identities.is_empty() {
                access::update(&paths, |p| {
                    p.spaces.entry(id.to_string()).or_default().identities = identities.clone();
                    Ok(())
                })
                .unwrap_or_else(|e| fail(&e.to_string()));
            }
            audit::best_effort(
                &paths,
                "space",
                "you",
                vec![("space", json!(id.to_string())), ("event", json!("added")), ("identities", if identities.is_empty() { Value::Null } else { json!(identities) })],
            );
            if json_out {
                println!("{}", json!({ "space": id.to_string(), "identities": identities }));
            } else {
                println!("Space {id} is ready at {}.", wiki::disp(&dir));
            }
            std::process::exit(0)
        }
        ("grant", [subject, scope]) => {
            let scope = parse_scope(scope).unwrap_or_else(|| fail(&format!("\"{scope}\" is not a scope: use ctx:<read|append|write|admin>[:<selector>], e.g. ctx:read:project:atlas")));
            let days = opts(args, "--days").first().map(|d| d.parse::<f64>().unwrap_or_else(|_| fail("--days takes a number")));
            let ms = days.map_or(GRANT_MS, |d| ((d * 86_400_000.0) as i64).clamp(60_000, 3650 * 86_400_000));
            let g = access::update(&paths, |p| p.grant(subject, scope.clone(), ms, "you", now)).unwrap_or_else(|e| fail(&e.to_string()));
            audit::best_effort(&paths, "grant", "you", vec![("subject", json!(g.subject)), ("scope", json!(format_scope(&g.scope))), ("expiresAt", json!(g.expires_at))]);
            if let Some(id) = named_space(&scope).filter(|id| !id.is_local()) {
                spaces::create(&root(), &id).unwrap_or_else(|e| fail(&e.to_string()));
            }
            if json_out {
                println!("{}", json!({ "subject": g.subject, "scope": format_scope(&g.scope), "expiresAt": g.expires_at }));
            } else {
                println!("{} may now {} in {} until {}.", g.subject, g.scope.level.as_str(), format_selector(&g.scope.selector), when(g.expires_at));
            }
            std::process::exit(0)
        }
        ("revoke", [subject, rest @ ..]) if rest.len() <= 1 => {
            let scope = rest.first().map(|s| parse_scope(s).unwrap_or_else(|| fail(&format!("\"{s}\" is not a scope"))));
            let n = access::update(&paths, |p| Ok(p.revoke(subject, scope.as_ref()))).unwrap_or_else(|e| fail(&e.to_string()));
            audit::best_effort(&paths, "revoke", "you", vec![("subject", json!(subject)), ("scope", scope.as_ref().map(|s| json!(format_scope(s))).unwrap_or(Value::Null)), ("revoked", json!(n))]);
            if json_out {
                println!("{}", json!({ "revoked": n }));
            } else {
                println!("Revoked {n} grant(s) of {subject}.");
            }
            std::process::exit(0)
        }
        ("token", ["create", subject]) => {
            let scopes = match opts(args, "--scope").first() {
                Some(text) => access::parse_scope_list(text).unwrap_or_else(|e| fail(&e.to_string())),
                // What the subject is granted now: the token asks for no more, and no less.
                None => {
                    let p = access::load(&paths);
                    let granted: Vec<access::Scope> = p.grants.iter().filter(|g| g.subject == *subject && g.expires_at > now).map(|g| g.scope.clone()).collect();
                    if granted.is_empty() {
                        fail(&format!("{subject} has no grants, so a token would reach nothing. Grant it a scope first (xenolith grant), or pass --scope."));
                    }
                    granted
                }
            };
            let days = opts(args, "--days").first().map(|d| d.parse::<f64>().unwrap_or_else(|_| fail("--days takes a number")));
            let ttl = days.map(|d| ((d * 86_400_000.0) as i64).clamp(60_000, 3650 * 86_400_000));
            let label = opts(args, "--label").first().copied().unwrap_or("");
            let (token, rec) = access::update(&paths, |p| p.issue(subject, scopes.clone(), label, ttl, None, now)).unwrap_or_else(|e| fail(&e.to_string()));
            audit::best_effort(
                &paths,
                "token",
                "you",
                vec![
                    ("token", json!(rec.id)),
                    ("subject", json!(rec.subject)),
                    ("scopes", json!(rec.scopes.iter().map(format_scope).collect::<Vec<_>>())),
                    ("expiresAt", rec.expires_at.map(|e| json!(e)).unwrap_or(Value::Null)),
                ],
            );
            if json_out {
                println!("{}", json!({ "token": token, "id": rec.id, "subject": rec.subject, "scopes": rec.scopes.iter().map(format_scope).collect::<Vec<_>>(), "expiresAt": rec.expires_at }));
            } else {
                println!("{token}");
                eprintln!(
                    "Token {} for {} ({}), {}. It is shown once: give it to the agent as Authorization: Bearer, or as XENOLITH_TOKEN for a stdio server.",
                    rec.id,
                    rec.subject,
                    rec.scopes.iter().map(format_scope).collect::<Vec<_>>().join(" "),
                    rec.expires_at.map(|e| format!("until {}", when(e))).unwrap_or_else(|| "until revoked".into())
                );
            }
            std::process::exit(0)
        }
        ("token", ["revoke", which]) => {
            let n = access::update(&paths, |p| Ok(p.revoke_token(which))).unwrap_or_else(|e| fail(&e.to_string()));
            audit::best_effort(&paths, "token-revoked", "you", vec![("which", json!(which)), ("revoked", json!(n))]);
            if json_out {
                println!("{}", json!({ "revoked": n }));
            } else {
                println!("Revoked {n} token(s).");
            }
            std::process::exit(if n == 0 { 1 } else { 0 })
        }
        ("audit", rest) if rest.is_empty() || rest == ["verify"] => {
            let file = audit::chain_file(&paths);
            let v = audit::verify(&file).unwrap_or_else(|e| fail(&e.to_string()));
            if json_out {
                println!("{}", json!({ "lines": v.lines, "head": v.head, "broken": v.broken.as_ref().map(|(n, why)| json!({ "line": n, "why": why })) }));
            } else if let Some((line, why)) = &v.broken {
                println!("The audit chain breaks at line {line} of {}: {why}.", wiki::disp(&file));
            } else {
                println!("The audit chain holds: {} entries, last hash {}.", v.lines, v.head);
            }
            std::process::exit(if v.broken.is_some() { 1 } else { 0 })
        }
        ("audit", ["tail", rest @ ..]) if rest.len() <= 1 => {
            let n = rest.first().map(|n| n.parse::<usize>().unwrap_or_else(|_| fail("tail takes a number"))).unwrap_or(20);
            let lines = audit::tail(&audit::chain_file(&paths), n);
            if json_out {
                println!("{}", json!({ "entries": lines }));
            } else {
                for l in &lines {
                    let skip = ["seq", "at", "kind", "who", "prev", "hash"];
                    let rest: Vec<String> = l
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(k, _)| !skip.contains(&k.as_str()))
                        .map(|(k, v)| format!("{k}={}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                        .collect();
                    println!(
                        "{:>5} {} {:<13} {:<28} {}",
                        l["seq"].as_i64().unwrap_or(0),
                        l["at"].as_str().unwrap_or(""),
                        l["kind"].as_str().unwrap_or(""),
                        l["who"].as_str().unwrap_or(""),
                        rest.join(" ")
                    );
                }
            }
            std::process::exit(0)
        }
        ("access", ["local", level]) => {
            // What a process on this machine may do without a token. "none" when anyone reaches
            // the port through a tunnel that looks local: give each client its own token instead.
            let want = match level.to_ascii_lowercase().as_str() {
                "none" => None,
                l => Some(access::Level::parse(l).unwrap_or_else(|| fail(&format!("\"{level}\" is not a level: use read, append, write, admin or none")))),
            };
            access::update(&paths, |p| {
                p.local = want;
                Ok(())
            })
            .unwrap_or_else(|e| fail(&e.to_string()));
            let shown = want.map_or("none", access::Level::as_str);
            audit::best_effort(&paths, "local", "you", vec![("level", json!(shown))]);
            if json_out {
                println!("{}", json!({ "local": shown }));
            } else if want.is_none() {
                println!("This machine's processes now need a token too.");
            } else {
                println!("This machine's processes may now {shown} without a token.");
            }
            std::process::exit(0)
        }
        ("access", []) => {
            let p = access::load(&paths);
            if json_out {
                let mut v = p.to_json();
                for t in v["tokens"].as_array_mut().into_iter().flatten() {
                    let hash = t["hash"].as_str().unwrap_or("").to_string();
                    t["id"] = json!(format!("tok_{}", &hash[..hash.len().min(12)]));
                    t.as_object_mut().map(|o| o.remove("hash"));
                }
                if let Some(problem) = &p.problem {
                    v["problem"] = json!(problem);
                }
                println!("{v}");
                std::process::exit(0)
            }
            print_policy(&p, now);
            std::process::exit(0)
        }
        _ => usage(),
    }
}

fn print_policy(p: &Policy, now: i64) {
    if let Some(problem) = &p.problem {
        println!("! {problem}");
    }
    println!("Processes on this machine, without a token: {}.", p.local.map_or("nothing (\"local\": \"none\")", |l| l.as_str()));
    if p.grants.is_empty() {
        println!("No grants: no token can reach any space.");
    }
    for g in &p.grants {
        let state = if g.expires_at <= now { "expired" } else { "until" };
        println!("grant  {:<24} {:<40} {state} {}", g.subject, format_scope(&g.scope), when(g.expires_at));
    }
    for t in &p.tokens {
        let scopes: Vec<String> = t.scopes.iter().map(format_scope).collect();
        let exp = t.expires_at.map(|e| format!("{} {}", if e <= now { "expired" } else { "until" }, when(e))).unwrap_or_else(|| "until revoked".into());
        println!("token  {} {:<24} {} {exp}{}", t.id, t.subject, scopes.join(" "), if t.label.is_empty() { String::new() } else { format!(" ({})", t.label) });
    }
}
