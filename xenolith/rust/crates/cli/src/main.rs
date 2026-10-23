//! xenolith: the Xenolith MCP server, session hook, curator and Windows service (and, as the port
//! continues, the installer), in one small program.
//!
//!   xenolith serve                     stdio (launched per client by the host app)
//!   xenolith serve --http [--port N]   Streamable HTTP on 127.0.0.1 (the service)
//!       --parent-stdin                   exit gracefully when stdin closes (the service wrapper's stop signal)
//!       --exit-on-upgrade                exit (code 75) when an install replaces this program (launchd, systemd)
//!   xenolith serve --read-only         stdio with context_search and context_read only, writing nothing
//!   xenolith serve --init              create the wiki skeleton, print {"ok":true,...}, exit
//!   xenolith hook [stop]               the SessionStart hook (with "stop": the opt-in Stop hook)
//!   xenolith curator [...]             the curator and the Ask worker (see curator.rs)
//!   xenolith service --config <ini>    the Windows service (see service.rs)
//!   xenolith install | uninstall       set Xenolith up for the apps on this computer, or take it out (see install/)
//!   xenolith migrate agent-wiki [...]  move an agent-wiki install to Xenolith, journaled (see migrate/)
//!   xenolith forget "<text>" [--yes]   find every copy of a text in the wiki and logs; --yes redacts them
//!   xenolith spaces | grant | revoke | access   spaces and who may use them (see access_cli.rs)
//!   xenolith sync [...]                 copies of spaces another machine curates (see sync_cli.rs)
//!   xenolith note "<title>" [...]       a note from you to the curator (through the service, as yours)
//!   xenolith held [...]                 the changes waiting for your OK, in any space; deciding one
//!                                       uses the running service and its owner key (see held_cli.rs)
//!   xenolith status [--port N] [--json]  the service's health and whether the audit chain is written
//!   xenolith --version

mod access_cli;
mod curator;
mod embeddings;
mod forget;
mod held_cli;
mod hook;
mod http;
mod httpd;
mod install;
mod keypipe;
mod mcp;
mod migrate;
mod models;
mod note_cli;
mod owner_client;
mod resources;
mod service;
mod status_cli;
mod stdio;
mod sync_cli;
mod uiapi;

use serde_json::{Map, Value, json};

fn set_if_unset(k: &str, v: &str) {
    if std::env::var(k).map(|s| s.is_empty()).unwrap_or(true) {
        // SAFETY: called at startup, before any other thread exists.
        unsafe { std::env::set_var(k, v) };
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith serve [--http [--port N] [--parent-stdin] | --read-only | --init] | xenolith hook | xenolith curator [--once | --parent-stdin | --asks-only | --retry-dead | --file-raw-dead | --login-status] | xenolith forget \"<text>\" [--ignore-case] [--yes] | xenolith embeddings status|sync | xenolith models [set curator|ask <model> [--effort <level>] | reset curator|ask] | xenolith spaces [add <space>] | xenolith grant <subject> <scope> | xenolith revoke <subject> [<scope>] | xenolith access | xenolith token create|revoke ... | xenolith audit [verify | tail] | xenolith sync [...] | xenolith held [...] | xenolith note \"<title>\" [--body <text>] | xenolith migrate agent-wiki [--dry-run | --run | --resume | --rollback | --finish [--move-wiki] | --update-protocol] [--json] | xenolith status [--port N] [--json] | xenolith --version"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let opt = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    if has("--version") {
        println!("{}", xenolith_core::VERSION);
        return;
    }
    // A flag serve doesn't know (--help, a typo) must not start a server on the real wiki.
    if has("--help") || has("-h") {
        usage();
    }
    if args.first().map(String::as_str) == Some("serve") {
        const SERVE_FLAGS: [&str; 6] = ["--http", "--port", "--parent-stdin", "--exit-on-upgrade", "--read-only", "--init"];
        let mut it = args.iter().skip(1);
        while let Some(a) = it.next() {
            if !SERVE_FLAGS.contains(&a.as_str()) {
                eprintln!("xenolith serve: unknown argument {a}");
                usage();
            }
            if a == "--port" && it.next().and_then(|p| p.parse::<u16>().ok()).is_none() {
                eprintln!("xenolith serve: --port takes a port number");
                usage();
            }
        }
    }
    match args.first().map(String::as_str) {
        Some("hook") if args.get(1).map(String::as_str) == Some("stop") => hook::run_stop(),
        Some("hook") => hook::run(),
        Some("service") => service::run(&args[1..]),
        Some(cmd @ ("install" | "uninstall" | "install-service" | "uninstall-service")) => install::main(cmd, &args[1..]),
        Some("curator") => {
            set_if_unset("XENOLITH_PROCESS", "curator");
            curator::run(&args[1..])
        }
        Some("forget") => forget::run(&args[1..]),
        Some("migrate") => migrate::main(&args[1..]),
        Some("sync") => sync_cli::run(&args[1..]),
        Some("held") => held_cli::run(&args[1..]),
        Some("note") => note_cli::run(&args[1..]),
        Some("status") => status_cli::run(&args[1..]),
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
    if has("--http") {
        set_if_unset("XENOLITH_PROCESS", "service");
        let cfg = xenolith_core::wiki::read_config(&mcp::env()).unwrap_or_else(|_| json!({}));
        let port = opt("--port")
            .and_then(|p| p.parse::<u16>().ok())
            .or_else(|| cfg.get("httpPort").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()))
            .unwrap_or(xenolith_core::wiki::DEFAULT_HTTP_PORT);
        http::run(port, has("--parent-stdin"), has("--exit-on-upgrade"));
    }
    let read_only = has("--read-only");
    set_if_unset("XENOLITH_PROCESS", if read_only { "read-only" } else { "stdio" });
    stdio::run(read_only);
}
