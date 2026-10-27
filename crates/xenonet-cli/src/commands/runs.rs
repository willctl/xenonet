//! xenonet runs (docs/guide/live-delivery.md, "Signing in for XenoNet's
//! runs"): the Claude Code sign-in the sessions XenoNet starts and resumes
//! use, a long-lived token from `claude setup-token`, kept in your
//! computer's credential store through the local service's
//! /api/runs/sign-in. It's yours: the service refuses it from under an
//! agent. The token is read from standard input, not echoed at a terminal,
//! so it never shows on screen, in your shell's history or in a process
//! list, and nothing ever shows it again.

use crate::args::Flags;
use crate::js::{on, t};
use crate::out::{c, json, print, write, Done, Fail};
use crate::reach::need;
use serde_json::{json, Value};
use std::io::{IsTerminal, Read};

pub const HELP: &str = include_str!("help/runs.txt");

/// Whether a terminal echoes what's typed: `stty` on macOS and Linux, so
/// the token never shows. Windows' console has no `stty`; there the token
/// shows as you paste it, as other keys do in `xenonet decide key`.
#[cfg(unix)]
fn echo(on: bool) {
    let _ = std::process::Command::new("stty").arg(if on { "echo" } else { "-echo" }).stdin(std::process::Stdio::inherit()).status();
}

#[cfg(not(unix))]
fn echo(_on: bool) {}

// The token, pasted at a terminal (up to Enter) or piped in.
fn read_token() -> String {
    let tty = std::io::stdin().is_terminal();
    let mut text = String::new();
    if tty {
        write("Paste the token claude setup-token printed, then Enter (it doesn't show): ");
        echo(false);
        let _ = std::io::stdin().read_line(&mut text);
        echo(true);
        write("\n");
    } else {
        let mut bytes = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    xenonet_base::text::js_trim(&text).to_string()
}

fn show(v: &Value) {
    if on(v, "signedIn") {
        print(format!("{} XenoNet's runs sign in to Claude Code with the token kept in {}", c::green("✓"), t(v, "store")));
    } else {
        print("XenoNet's runs use Claude Code's own sign-in. If they say it isn't signed in, run claude setup-token, then xenonet runs sign-in.");
    }
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let call = |method: &str, body: Option<&Value>| s.request(method, "/api/runs/sign-in", body, &[]).map_err(|r| Fail::new(r.message, 1));
    let v = match args.first().map_or("status", String::as_str) {
        "status" => call("GET", None)?,
        "sign-in" => {
            let token = read_token();
            call("POST", Some(&json!({ "token": token })))?
        }
        "forget" => call("DELETE", None)?,
        sub => return Err(Fail::new(format!("no such: xenonet runs {sub}. xenonet runs --help says more."), 2)),
    };
    if flags.on("json") {
        json(&v);
    } else {
        show(&v);
    }
    Ok(0)
}
