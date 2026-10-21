//! The xenonet command, in Rust (cli/xenonet.js; docs/design/rust-core.md).
//! Each command lives in commands/. Commands share the core with the
//! service and the MCP servers: they call it directly, or a running
//! XenoNet over its HTTP API. The core's functions they call are in
//! cores/, which drives each slice's Rust natively. `xenonet serve` is the
//! service itself.

mod args;
mod clock;
mod commands;
mod config;
mod cores;
mod js;
mod out;
mod prompt;
mod reach;
mod report;
mod sys;

use args::{parse, Flags, Parsed};
use commands::{alias, find, COMMANDS};
use out::{c, print, Done, Fail};
use serde_json::{json, Value};
use xenonet_base::log::{log_dir, ErrInfo, Log, MAX_BYTES};

const VERSION: &str = env!("XENONET_VERSION");

fn help() {
    let shown: Vec<_> = COMMANDS.iter().filter(|c| !c.hidden).collect();
    let usage = |c: &commands::Command| format!("{} {}", c.name, c.args().unwrap_or_default()).trim().to_string();
    let w = shown.iter().map(|c| usage(c).chars().count()).max().unwrap_or(0);
    let list: Vec<String> = shown.iter().map(|cmd| format!("  {:<width$}{}", usage(cmd), c::dim(cmd.about()), width = w + 2)).collect();
    print(format!(
        "{} {}

{}
  xenonet <command> [options]

{}
{}

{}
  --json         JSON, for scripts
  --dev, --demo  XenoNet Dev (port 47842) or its demo (47841), rather than XenoNet
  --port <n>     The XenoNet on this port
  --no-color     Plain text
  -h, --help     Help, for xenonet or a command
  -v, --version  The version

{}
  xenonet doctor
  xenonet agents --repo
  xenonet who src/scene.js
  xenonet connect",
        c::bold("xenonet"),
        c::dim("· the network for SI on this computer"),
        c::bold("Usage"),
        c::bold("Commands"),
        list.join("\n"),
        c::bold("Options"),
        c::bold("Examples"),
    ));
}

fn run(name: &str, args: &[String], flags: &Flags) -> Done {
    if flags.on("help") {
        let Some(cmd) = find(name) else { return Err(Fail::error(format!("no command \"{name}\". xenonet --help lists them."))) };
        print(format!("{}{}\n\n{}", c::bold(format!("xenonet {name}")), cmd.args().map_or(String::new(), |a| format!(" {a}")), commands::help(name)));
        return Ok(0);
    }
    commands::run(name, args, flags)
}

fn main_of(argv: &[String]) -> Done {
    let Parsed { mut flags, rest } = parse(argv);
    out::set_color(!(flags.is_false("color") || flags.on("json")));
    let mut words = rest.into_iter();
    let name = words.next().filter(|n| !n.is_empty()).map(|n| alias(&n).to_string());
    let args: Vec<String> = words.collect();
    if flags.on("version") {
        let version = sys::version(VERSION);
        print(if flags.on("json") { xenonet_base::json::stringify(&json!({ "version": version }), 0) } else { format!("xenonet {version}") });
        return Ok(0);
    }
    // What this binary is as installed XenoNet's launcher, a copy of the
    // Tools' (docs/design/contracts.md, section 12): launcher 3, with no
    // node of its own.
    if flags.on("launcher") && name.is_none() {
        print(xenonet_base::json::stringify(&json!({ "launcher": 3, "protocol": 1, "tools": VERSION, "node": null }), 0));
        return Ok(0);
    }
    let Some(name) = name else {
        if flags.on("help") {
            help();
            return Ok(0);
        }
        return run("status", &[], &flags);
    };
    if name == "help" {
        let Some(first) = args.first().filter(|a| !a.is_empty()) else {
            help();
            return Ok(0);
        };
        flags.set("help", Value::Bool(true));
        return run(alias(first), &[], &flags);
    }
    if find(&name).is_none() {
        return Err(Fail::new(format!("no command \"{name}\". xenonet --help lists them."), 2));
    }
    run(&name, &args, &flags)
}

// What XenoNet didn't expect (not a Fail, which says how it was used
// wrong) goes in its error log too, which xenonet logs reads.
fn log_error(fail: &Fail, argv: &[String]) {
    let dir = log_dir(&sys::state_dir(false));
    let first = argv.iter().find(|a| !a.starts_with('-'));
    let code = match first {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase()) => format!("cli.{n}"),
        _ => "cli.command".into(),
    };
    let err = ErrInfo { message: Some(fail.message.clone()), stack: None, code: None };
    Log::new(Some(&dir), "cli", None, None, &sys::home(), MAX_BYTES).write("error", &code, Some(&fail.message), Some(&err), None, None, out::now());
}

/// `xenonet serve`: the service, with each core the binary's own driver
/// for it (crates/xenonet-wire-*): the map first, whose demo world moves the
/// folders the rest use, then the engine and actors, then the app's parts.
fn serve(args: &[String]) -> i32 {
    xenonet_service::serve::run_with(
        args,
        VERSION,
        Box::new(|config, env| {
            let mut cores = xenonet_service::cores::Cores::pending(config.actors, config.assistant, config.hosts);
            xenonet_wire_map::wire(&mut cores, config, env);
            xenonet_wire_engine::wire(&mut cores, config, env);
            xenonet_wire_app::wire(&mut cores, config, env);
            // The terminal drawer's shells live in the service.
            cores.terminals = std::sync::Arc::new(xenonet_service::terminals::Shells::new());
            cores
        }),
    )
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("serve") {
        std::process::exit(serve(&argv[1..]));
    }
    let code = match main_of(&argv) {
        Ok(code) => code,
        // A message; there's no stack to show with XENONET_DEBUG=1 here.
        Err(fail) => {
            out::eprint(format!("xenonet: {}", fail.message));
            if !fail.expected {
                log_error(&fail, &argv);
            }
            fail.code
        }
    };
    std::process::exit(if code == 0 && out::wrote_badly() { 1 } else { code });
}
