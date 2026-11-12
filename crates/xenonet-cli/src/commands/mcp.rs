//! xenonet mcp (cli/commands/mcp.js): the xenonet MCP server on stdio
//! (mcp/xenonet.js, crates/xenonet-mcp). Claude Code, Codex and Grok Build
//! start it once they're connected. xenonet mcp console is the console
//! operator (mcp/console.js), and xenonet mcp assistant the read-only
//! server for a remote actor you paired (mcp/assistant.js).
//!
//! The servers import xenonet-api alone: what they know of the rest is
//! given here, the registry's apps, the console's control and the
//! assistant's made-up data.

use crate::args::Flags;
use crate::out::{Done, Fail};
use xenonet_text::text::{display, to_number};

pub const HELP: &str = include_str!("help/mcp.txt");

fn serve<F: std::future::Future<Output = Result<(), String>>>(f: impl FnOnce() -> F) -> Done {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Fail::error(e.to_string()))?;
    rt.block_on(f()).map_err(Fail::error)?;
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    match args.first().map(String::as_str) {
        Some("console") => serve(|| xenonet_mcp::serve_console(std::sync::Arc::new(crate::cores::ConsoleControl), crate::sys::installed_tools())),
        Some("assistant") => {
            if args.len() > 1 {
                return Err(Fail::error("xenonet mcp assistant takes --grant <id> (and --port <n>), or --fixture."));
            }
            // --port, or --dev's or --demo's; the server's own choice otherwise.
            let port = match flags.get("port") {
                Some(p) => {
                    let n = if *p == serde_json::Value::Bool(true) { 1.0 } else { to_number(Some(&serde_json::Value::String(display(p)))) };
                    if n.fract() != 0.0 || !(1.0..=65535.0).contains(&n) {
                        return Err(Fail::new("--port is a number from 1 to 65535", 2));
                    }
                    Some(n as u16)
                }
                None if flags.on("dev") => Some(47842),
                None if flags.on("demo") => Some(47841),
                None => None,
            };
            let grant = flags.str("grant").map(str::to_string);
            let fixture = flags
                .is_true("fixture")
                .then(|| std::sync::Arc::new(crate::cores::assistant_fixture::Fixture::new(crate::VERSION)) as std::sync::Arc<dyn xenonet_mcp::fixture::Fixture>);
            serve(move || xenonet_mcp::serve_assistant(grant, fixture, port))
        }
        Some(_) => Err(Fail::error("Use xenonet mcp, xenonet mcp console or xenonet mcp assistant.")),
        None => serve(|| async {
            xenonet_mcp::serve(registry()).await;
            Ok(())
        }),
    }
}

/// What the sessions' server knows of the registry: the apps whose
/// sessions say they're calling, and the apps XenoNet starts sessions of
/// (headless, or over ACP where the registry gives a command), in its
/// order, each with whether a start of it takes a mode.
pub fn registry() -> xenonet_mcp::Registry {
    use xenonet_engine::delivery::{apps, starts};
    let modes = |a: &xenonet_adapter::apps::App| apps::of(&a.id).and_then(|w| w.no_mode).is_none() && !starts::acp_only(a);
    xenonet_mcp::Registry {
        callers: xenonet_adapter::apps::with("caller").map(|a| a.id.clone()).collect(),
        starts: starts::apps().into_iter().map(|a| xenonet_mcp::StartApp { id: a.id.clone(), name: a.name.clone(), modes: modes(a) }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registrys_apps() {
        xenonet_platforms::install();
        let r = registry();
        assert_eq!(r.callers, ["claude", "codex", "grok"]);
        let starts: Vec<(&str, bool)> = r.starts.iter().map(|a| (a.id.as_str(), a.modes)).collect();
        assert_eq!(starts, [("claude", true), ("codex", false), ("grok", true), ("hermes-agent", false)], "Hermes Agent over ACP, under its own settings");
    }
}
