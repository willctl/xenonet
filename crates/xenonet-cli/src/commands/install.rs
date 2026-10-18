//! xenonet install (cli/commands/install.js): puts the xenonet command on
//! your PATH (core/command.js): a small script that runs this binary.

use crate::args::Flags;
use crate::cores;
use crate::js::{on, t};
use crate::out::{c, json, print, Done, Fail};
use crate::sys;
use serde_json::json;

pub const HELP: &str = include_str!("help/install.txt");

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let dir = match flags.str("dir") {
        Some(d) => sys::resolve(d),
        None => cores::command_dir()?,
    };
    let st = cores::install_command(&json!({ "dir": dir, "bin": cores::xenonet_bin(), "force": flags.on("force") })).map_err(|e| {
        if e.expected {
            e
        } else {
            let more = if e.message.contains("isn't XenoNet's") { ". --force replaces it." } else { "" };
            Fail::new(format!("{}{more}", e.message), 1)
        }
    })?;
    if flags.on("json") {
        json(&st);
        return Ok(0);
    }
    let home = sys::home();
    let runs = xenonet_base::text::join(crate::js::list(&st, "runs").iter().map(Some), " ");
    print(format!("{} {} runs {}", c::green("✓"), sys::tilde(&t(&st, "file")), sys::tilde(&runs)));
    if !on(&st, "onPath") {
        let shown = sys::tilde(&dir);
        print(c::yellow(if cfg!(windows) {
            format!("  {shown} isn't on your PATH yet. Add it to your user Path in Windows' Environment Variables, then open a new terminal.")
        } else {
            let profile = dir.strip_prefix(&format!("{home}{}", sys::SEP)).map_or_else(|| dir.clone(), |rest| format!("$HOME{}{rest}", sys::SEP));
            format!("  {shown} isn't on your PATH yet. Add it in your shell's profile, for example:\n  export PATH=\"{profile}:$PATH\"")
        }));
    }
    Ok(0)
}
