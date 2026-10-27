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

/// The terminal's echo, off while the token is read, so it never shows,
/// and back as it was when this is dropped, however the reading ends:
/// `stty` on macOS and Linux, the console's mode on Windows.
struct NoEcho {
    #[cfg(windows)]
    was: Option<(windows_sys::Win32::Foundation::HANDLE, windows_sys::Win32::System::Console::CONSOLE_MODE)>,
}

#[cfg(unix)]
impl NoEcho {
    fn new() -> NoEcho {
        stty("-echo");
        NoEcho {}
    }
}

#[cfg(unix)]
impl Drop for NoEcho {
    fn drop(&mut self) {
        stty("echo");
    }
}

#[cfg(unix)]
fn stty(arg: &str) {
    let _ = std::process::Command::new("stty").arg(arg).stdin(std::process::Stdio::inherit()).status();
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "GetConsoleMode and SetConsoleMode, which std doesn't wrap")]
impl NoEcho {
    fn new() -> NoEcho {
        use windows_sys::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE};
        // SAFETY: GetStdHandle takes a constant and returns the process's
        // standard input handle, or null or INVALID_HANDLE_VALUE, which
        // GetConsoleMode then fails on; GetConsoleMode writes one u32 to a
        // local that outlives the call; SetConsoleMode only reads its
        // arguments.
        unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode = 0;
            if GetConsoleMode(h, &raw mut mode) == 0 || SetConsoleMode(h, mode & !ENABLE_ECHO_INPUT) == 0 {
                return NoEcho { was: None };
            }
            NoEcho { was: Some((h, mode)) }
        }
    }
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "SetConsoleMode, which std doesn't wrap")]
impl Drop for NoEcho {
    fn drop(&mut self) {
        if let Some((h, mode)) = self.was {
            // SAFETY: the handle and mode GetConsoleMode gave in new(); the
            // standard input handle stays open for the process's life.
            unsafe {
                windows_sys::Win32::System::Console::SetConsoleMode(h, mode);
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
impl NoEcho {
    fn new() -> NoEcho {
        NoEcho {}
    }
}

// The token, pasted at a terminal (up to Enter) or piped in.
fn read_token() -> String {
    let tty = std::io::stdin().is_terminal();
    let mut text = String::new();
    if tty {
        write("Paste the token claude setup-token printed, then Enter (it doesn't show): ");
        let quiet = NoEcho::new();
        let _ = std::io::stdin().read_line(&mut text);
        drop(quiet);
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
