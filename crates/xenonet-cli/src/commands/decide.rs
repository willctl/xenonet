//! xenonet decide (cli/commands/decide.js): the decision model XenoNet can
//! ask instead of Codex (core/decide.js), and its OpenRouter key, which
//! goes straight into your computer's credential store. The key is read
//! from standard input, so it never shows in your shell's history or a
//! process list.

use crate::args::Flags;
use crate::cores;
use crate::js::t;
use crate::out::{c, print, write, Done, Fail};
use std::io::{IsTerminal, Read};

pub const HELP: &str = include_str!("help/decide.txt");

// The key, pasted at a terminal (up to Enter) or piped in.
fn read_key() -> String {
    let tty = std::io::stdin().is_terminal();
    if tty {
        write("Paste the OpenRouter key, then Enter: ");
    }
    let mut text = String::new();
    if tty {
        let _ = std::io::stdin().read_line(&mut text);
        write("\n");
    } else {
        let mut bytes = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    xenonet_base::text::js_trim(&text).to_string()
}

pub fn run(args: &[String], _flags: &Flags) -> Done {
    match args.first().map_or("status", String::as_str) {
        "status" => {
            let s = cores::decider_status()?;
            let key = if crate::js::str(&s, "key") == Some("stored") { c::green(format!("a key is kept in {}", t(&s, "store"))) } else { "no key yet: xenonet decide key".into() };
            print(format!("{} · {key}", t(&s, "model")));
            Ok(0)
        }
        "key" => {
            let r = cores::decider_store_key(&read_key())?;
            print(format!("{} The key is kept in {}. XenoNet uses it only when you send something to the decision model.", c::green("✓"), t(&r, "store")));
            Ok(0)
        }
        "forget" => {
            cores::decider_forget_key()?;
            print(format!("{} XenoNet forgot the key.", c::green("✓")));
            Ok(0)
        }
        sub => Err(Fail::error(format!("no such: xenonet decide {sub}. xenonet decide --help says more."))),
    }
}
