//! xenonet uninstall (cli/commands/uninstall.js): takes the xenonet command
//! off your PATH (core/command.js).

use crate::args::Flags;
use crate::js::{on, t};
use crate::out::{c, json, print, Done, Fail};
use crate::pending;
use crate::sys;

pub const HELP: &str = include_str!("help/uninstall.txt");

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let dir = match flags.str("dir") {
        Some(d) => sys::resolve(d),
        None => pending::command_dir()?,
    };
    let before = pending::command_state(&dir)?;
    let st = pending::uninstall_command(&dir).map_err(|e| Fail::new(e.message, e.code))?;
    if flags.on("json") {
        json(&st);
        return Ok(0);
    }
    print(if on(&before, "installed") {
        format!("{} Took out {}", c::green("✓"), sys::tilde(&t(&before, "file")))
    } else {
        c::dim(format!("There's no xenonet command in {}.", sys::tilde(&dir)))
    });
    Ok(0)
}
