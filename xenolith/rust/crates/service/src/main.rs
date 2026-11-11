//! xenolith-service: the Xenolith service, its curator and the Windows service, apart from
//! `xenolith` so the hook's program stays small. It is the one program that will take
//! xenolith-net, and with it iroh and tokio.
//!
//!   xenolith-service serve --http [--port N]   Streamable HTTP on 127.0.0.1 (the service)
//!       --parent-stdin                           exit gracefully when stdin closes (the service wrapper's stop signal)
//!       --exit-on-upgrade                        exit (code 75) when an install replaces this program (launchd, systemd)
//!   xenolith-service curator [...]             the curator and the Ask worker (see curator.rs)
//!   xenolith-service service --config <ini>    the Windows service (see service.rs)
//!   xenolith-service --version

mod curator;
mod http;
mod httpd;
mod keypipe;
mod service;
mod uiapi;

// The MCP server, shared with xenolith.
use xenolith_mcp::mcp;

use serde_json::{Value, json};

fn set_if_unset(k: &str, v: &str) {
    if std::env::var(k).map(|s| s.is_empty()).unwrap_or(true) {
        // SAFETY: called at startup, before any other thread exists.
        unsafe { std::env::set_var(k, v) };
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: xenolith-service serve --http [--port N] [--parent-stdin] [--exit-on-upgrade] | xenolith-service curator [--once | --parent-stdin | --asks-only | --retry-dead | --file-raw-dead | --login-status] | xenolith-service service --config <ini> [--console] | xenolith-service --version\n(the stdio server, the hook and the commands: xenolith)"
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
    match args.first().map(String::as_str) {
        Some("service") => service::run(&args[1..]),
        Some("curator") => {
            set_if_unset("XENOLITH_PROCESS", "curator");
            curator::run(&args[1..])
        }
        Some("serve") => {}
        _ => usage(),
    }
    const SERVE_FLAGS: [&str; 4] = ["--http", "--port", "--parent-stdin", "--exit-on-upgrade"];
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if !SERVE_FLAGS.contains(&a.as_str()) {
            eprintln!("xenolith-service serve: unknown argument {a}");
            usage();
        }
        if a == "--port" && it.next().and_then(|p| p.parse::<u16>().ok()).is_none() {
            eprintln!("xenolith-service serve: --port takes a port number");
            usage();
        }
    }
    // The stdio server is xenolith's: here serve is always the HTTP service.
    if !has("--http") {
        eprintln!("xenolith-service serve: --http (the stdio server is `xenolith serve`)");
        usage();
    }
    set_if_unset("XENOLITH_PROCESS", "service");
    let cfg = xenolith_core::wiki::read_config(&mcp::env()).unwrap_or_else(|_| json!({}));
    let port =
        opt("--port").and_then(|p| p.parse::<u16>().ok()).or_else(|| cfg.get("httpPort").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok())).unwrap_or(xenolith_core::wiki::DEFAULT_HTTP_PORT);
    http::run(port, has("--parent-stdin"), has("--exit-on-upgrade"));
}
