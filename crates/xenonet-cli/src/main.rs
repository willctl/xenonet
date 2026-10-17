//! The xenonet command, in Rust (docs/design/rust-core.md). It runs the
//! hooks Claude Code and Codex call on every event, which gain the most
//! from a native start; the rest of the command is still `cli/xenonet.js`,
//! and this says so rather than guess.

mod hook;

const VERSION: &str = env!("XENONET_VERSION");

// `--port 47842` or `--port=47842`.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let long = format!("--{name}");
    let eq = format!("--{name}=");
    args.iter().enumerate().find_map(|(i, a)| if *a == long { args.get(i + 1).map(String::as_str) } else { a.strip_prefix(&eq) })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hook") => hook::run(args.get(1).map(String::as_str), flag(&args, "port")),
        Some("serve") => xenonet_service::run(&args[1..], VERSION),
        Some("mcp") => {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a runtime");
            rt.block_on(xenonet_mcp::serve());
            0
        }
        Some("-v" | "--version") => {
            println!("{VERSION}");
            0
        }
        _ => {
            eprintln!("This xenonet runs hooks only, for now: use cli/xenonet.js for {}.", args.first().map_or("the other commands", String::as_str));
            1
        }
    };
    std::process::exit(code);
}
