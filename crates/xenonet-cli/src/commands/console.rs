//! xenonet console (cli/commands/console.js): drives XenoNet Dev one step
//! after another, through its control channel (core/control.js), the way
//! the xenonet-console MCP tools do. It's how an agent without those tools,
//! or a person at a terminal, checks a change to XenoNet. npm run drive is
//! the same command. Its help lists core/script.js's steps.

use crate::args::Flags;
use crate::cores;
use crate::out::{print, Done, Fail};
use crate::sys;
use serde_json::Value;

pub const HELP: &str = include_str!("help/console.txt");

/// Whether this is the Tools package's command, which drives installed
/// XenoNet unless told otherwise (package.json's xenonetComponent).
pub const INSTALLED: bool = matches!(env!("XENONET_COMPONENT").as_bytes(), b"tools");

/// A step's word, as the command prints it: text as it is, anything else
/// as JSON.
pub fn said(v: &Value) -> String {
    v.as_str().map_or_else(|| xenonet_base::json::stringify(v, 0), str::to_string)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    if args.is_empty() {
        return Err(Fail::new("say what to do: xenonet console <steps…>; xenonet console --help lists the steps", 2));
    }
    let instance = flags.text("instance").or_else(|| sys::var("XENONET_INSTANCE")).unwrap_or_else(|| (if INSTALLED { "installed" } else { "demo" }).into());
    let steps: Vec<Value> = args.iter().map(|a| Value::String(a.clone())).collect();
    cores::run_script(&instance, &steps, &mut |v| print(said(v)))?;
    Ok(0)
}
