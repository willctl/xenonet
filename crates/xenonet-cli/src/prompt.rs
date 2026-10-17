//! Asking a yes or no in the terminal, as cli/commands/connect.js's and
//! hosts.js's confirm do: --yes says yes, and only a terminal is asked.

use crate::args::Flags;
use crate::out::{c, write, Fail};
use std::io::{BufRead, IsTerminal};

// The answer to a question, y or yes in any case.
fn asked(question: &str) -> bool {
    write(&format!("{question} {} ", c::dim("[y/N]")));
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    let a = xenonet_base::text::js_trim(&line).to_ascii_lowercase();
    a == "y" || a == "yes"
}

/// connect.js's confirm: with no terminal to ask, it fails, saying to run
/// it again with --yes.
pub fn confirm(question: &str, flags: &Flags) -> Result<bool, Fail> {
    if flags.on("yes") {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(Fail::new(format!("{question} Run it again with --yes to go ahead."), 1));
    }
    Ok(asked(question))
}

/// hosts.js's confirm: with no terminal to ask, it's a no.
pub fn ask(question: &str, flags: &Flags) -> bool {
    if flags.on("yes") {
        return true;
    }
    std::io::stdin().is_terminal() && asked(question)
}
