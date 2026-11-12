//! xenolith: the Xenolith MCP server over stdio, the session hook, the commands and the installer,
//! on xenolith-core alone, so it stays small and starts fast (the hook runs it for every
//! conversation). The HTTP service, the curator and the Windows service are xenolith-service
//! (crates/service), installed beside it.
//!
//!   xenolith serve                     stdio (launched per client by the host app)
//!   xenolith serve --read-only         stdio with context_search and context_read only, writing nothing
//!   xenolith serve --init              create the wiki skeleton, print {"ok":true,...}, exit
//!   xenolith hook [stop]               the SessionStart hook (with "stop": the opt-in Stop hook)
//!   xenolith install | uninstall       set Xenolith up for the apps on this computer, or take it out (see install/)
//!   xenolith migrate agent-wiki [...]  move an agent-wiki install to Xenolith, journaled (see migrate/)
//!   xenolith forget "<text>" [--yes]   find every copy of a text in the wiki and logs; --yes redacts them
//!   xenolith spaces | grant | revoke | access   spaces and who may use them (see access_cli.rs)
//!   xenolith note "<title>" [...]       a note from you to the curator (through the service, as yours)
//!   xenolith held [...]                 the changes waiting for your OK, in any space; deciding one
//!                                       uses the running service and its owner key (see held_cli.rs)
//!   xenolith status [--port N] [--json]  the service's health and whether the audit chain is written
//!   xenolith upgrade [--run | --undo]   the upgrade from 1.x: a dry run, doing it, taking it back
//!                                       (see upgrade_cli.rs)
//!   xenolith doctor [--json]            where the upgrade stands, and what 1.x left
//!   xenolith sync [status]             sync between machines (see sync_cli.rs)
//!   xenolith invite <space>... | join <invite>   one Xenolith joins spaces another curates
//!   xenolith --version

mod access_cli;
mod doctor_cli;
mod embeddings;
mod forget;
mod held_cli;
mod hook;
mod install;
mod migrate;
mod models;
mod note_cli;
mod owner_client;
mod status_cli;
mod stdio;
mod sync_cli;
mod upgrade_cli;

// The MCP server, shared with xenolith-service.
use xenolith_mcp::{mcp, resources};

use serde_json::{Map, Value, json};

fn set_if_unset(k: &str, v: &str) {
    if std::env::var(k).map(|s| s.is_empty()).unwrap_or(true) {
        // SAFETY: called at startup, before any other thread exists.
        unsafe { std::env::set_var(k, v) };
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith serve [--read-only | --init] | xenolith hook | xenolith forget \"<text>\" [--ignore-case] [--yes] | xenolith embeddings status|sync | xenolith models [set curator|ask <model> [--effort <level>] | reset curator|ask] | xenolith spaces [add <space> | bind <space> [--project prj_…] [--run]] | xenolith grant <subject> <scope> | xenolith revoke <subject> [<scope>] | xenolith access | xenolith token create|revoke ... | xenolith audit [verify | tail] | xenolith held [...] | xenolith note \"<title>\" [--body <text>] | xenolith migrate agent-wiki [--dry-run | --run | --resume | --rollback | --finish [--move-wiki] | --update-protocol] [--json] | xenolith status [--port N] [--json] | xenolith upgrade [--run | --undo] [--json] | xenolith doctor [--json] | xenolith sync [status | grant] | xenolith invite <space>... [--level read|append|write] | xenolith join <invite> [--at <ip:port>]... | xenolith --version\n(the HTTP service, the curator and the Windows service: xenolith-service)"
    );
    std::process::exit(2)
}

/// 1.x's sync and pairing, which sync/2 and joining replace.
fn gone(what: &str) -> ! {
    eprintln!("xenolith {what}: 1.x's pairing is gone; one Xenolith joins another's spaces with an invite instead (xenolith invite, xenolith join)");
    std::process::exit(2)
}

/// What xenolith-service runs now: said, with where it is, rather than a usage line.
fn moved(what: &str) -> ! {
    eprintln!("xenolith {what}: that's xenolith-service now, installed beside xenolith (xenolith-service {what} ...)");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--version") {
        println!("{}", xenolith_core::VERSION);
        return;
    }
    // A flag serve doesn't know (--help, a typo) must not start a server on the real wiki.
    if has("--help") || has("-h") {
        usage();
    }
    if args.first().map(String::as_str) == Some("serve") {
        for a in &args[1..] {
            if a == "--http" {
                moved("serve --http");
            }
            if !["--read-only", "--init"].contains(&a.as_str()) {
                eprintln!("xenolith serve: unknown argument {a}");
                usage();
            }
        }
    }
    match args.first().map(String::as_str) {
        Some("hook") if args.get(1).map(String::as_str) == Some("stop") => hook::run_stop(),
        Some("hook") => hook::run(),
        Some(cmd @ ("service" | "curator")) => moved(cmd),
        Some(cmd @ ("install" | "uninstall" | "install-service" | "uninstall-service")) => install::main(cmd, &args[1..]),
        Some("forget") => forget::run(&args[1..]),
        Some("migrate") => migrate::main(&args[1..]),
        Some("sync") => sync_cli::run(&args[1..]),
        Some("invite") => sync_cli::invite(&args[1..]),
        Some("join") => sync_cli::join(&args[1..]),
        Some("pair") => gone("pair"),
        Some("held") => held_cli::run(&args[1..]),
        Some("note") => note_cli::run(&args[1..]),
        Some("status") => status_cli::run(&args[1..]),
        Some("upgrade") => upgrade_cli::run(&args[1..]),
        Some("doctor") => doctor_cli::run(&args[1..]),
        Some(cmd @ ("spaces" | "grant" | "revoke" | "access" | "token" | "audit")) => access_cli::run(cmd, &args[1..]),
        Some("embeddings") => embeddings::run(&args[1..]),
        Some("models") => models::run(&args[1..]),
        Some("serve") => {}
        _ => usage(),
    }
    if has("--init") {
        let st = mcp::setup();
        let mut o = Map::new();
        o.insert("ok".into(), json!(st.setup_error.is_none()));
        o.insert("wikiDir".into(), st.wiki_dir.as_ref().map(|d| json!(d.to_string_lossy())).unwrap_or(Value::Null));
        o.insert("version".into(), json!(xenolith_core::VERSION));
        o.insert("writeMode".into(), json!(if st.curated { "curated" } else { "direct" }));
        if let Some(e) = &st.setup_error {
            o.insert("error".into(), json!(e));
        }
        println!("{}", Value::Object(o));
        std::process::exit(if st.setup_error.is_none() { 0 } else { 1 });
    }
    let read_only = has("--read-only");
    set_if_unset("XENOLITH_PROCESS", if read_only { "read-only" } else { "stdio" });
    stdio::run(read_only);
}
