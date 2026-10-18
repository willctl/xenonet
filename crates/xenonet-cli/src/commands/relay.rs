//! xenonet relay (cli/commands/relay.js): this XenoNet's map, streamed to
//! a console on another machine over the SSH connection the console opened
//! (core/hosts/relay.js). Consoles run it, as ssh <host> xenonet relay;
//! nothing listens anywhere.

use crate::args::Flags;
use crate::cores;
use crate::out::{Done, Fail};
use crate::reach::candidates;
use serde_json::json;
use xenonet_base::json::number;

pub const HELP: &str = include_str!("help/relay.txt");

/// Sessions updated this recently, unless --since says otherwise: 7 days.
pub const DEFAULT_SINCE_MS: f64 = 7.0 * 86400e3;

/// parseSince (core/hosts/relay.js): "7d" is 604800000; None for anything
/// else.
pub fn parse_since(text: &str) -> Option<f64> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let unit = match &text[digits..] {
        "m" => 60e3,
        "h" => 3600e3,
        "d" => 86400e3,
        _ => return None,
    };
    (1..=4).contains(&digits).then(|| text[..digits].parse::<f64>().unwrap_or(0.0) * unit)
}

/// fromOriginalCommand (core/hosts/relay.js): a key restricted to the relay
/// in the host's authorized_keys (command="xenonet relay --port 47840")
/// runs that command whatever the console asked for, and ssh puts what it
/// asked for in SSH_ORIGINAL_COMMAND. Only --since is taken from there.
pub fn from_original_command(text: &str) -> Option<f64> {
    let words: Vec<&str> = text.split(xenonet_base::text::is_js_space).filter(|w| !w.is_empty()).collect();
    let at = words.iter().position(|w| *w == "--since")?;
    words.get(at + 1).and_then(|w| parse_since(w)).filter(|n| *n != 0.0)
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let forced = crate::sys::var("SSH_ORIGINAL_COMMAND").and_then(|c| from_original_command(&c));
    let since = match flags.get("since") {
        Some(v) => parse_since(&xenonet_base::text::js_string(v)),
        None => Some(forced.unwrap_or(DEFAULT_SINCE_MS)),
    };
    let Some(since) = since.filter(|n| *n != 0.0) else {
        return Err(Fail::new(format!("--since is a number and m, h or d, such as 7d, 12h or 90m, not {}", crate::js::text(flags.get("since"))), 2));
    };
    let port = candidates(flags)[0];
    if port.fract() != 0.0 || port.is_nan() || !(1.0..=65535.0).contains(&port) {
        return Err(Fail::new("--port is a number from 1 to 65535", 2));
    }
    let code = cores::run_relay(
        &json!({ "port": number(port), "sinceMs": number(since), "detail": flags.is_true("detail"), "start": flags.is_true("start"), "demo": flags.is_true("demo") }),
    )?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_of_time() {
        assert_eq!(parse_since("7d"), Some(DEFAULT_SINCE_MS));
        assert_eq!(parse_since("90m"), Some(90.0 * 60e3));
        assert_eq!(parse_since("12345h"), None);
        assert_eq!(parse_since("forever"), None);
        assert_eq!(from_original_command("xenonet relay --since 12h --detail"), Some(12.0 * 3600e3));
        assert_eq!(from_original_command("xenonet relay"), None);
    }
}
