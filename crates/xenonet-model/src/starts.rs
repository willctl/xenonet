//! What a start says to the one that asked for it (the engine's starts,
//! and the made-up network a development XenoNet shows): the words of an
//! answer, and the Allow card's warning about the run's environment.

use serde_json::Value;

/// What the requester is told of an answer: started, refused or ended, on
/// `host` ("this machine" for this one). `a` is the receipt's body. An end
/// says what the session did (`outcome`: finished, reported, asked,
/// denied, failed or silent) and quotes its last words (`last`), never
/// only that a process exited.
pub fn answer_words(state: &str, host: &str, a: &Value) -> String {
    let session = a.get("session").and_then(Value::as_str);
    let reason = a.get("reason").and_then(Value::as_str).unwrap_or("");
    let on = if host == "this machine" { String::new() } else { format!(" on {host}") };
    match state {
        "started" => {
            let folder = a.get("folder").and_then(Value::as_str).map(|f| format!(", in {f}")).unwrap_or_default();
            match session {
                Some(s) => format!("[XenoNet] Started {s}{on}{folder}. Message it as you would any session."),
                None => format!("[XenoNet] Started a session{on}{folder}."),
            }
        }
        "ended" => {
            let who = format!("{}{on}", session.unwrap_or("The session"));
            let minutes = a.get("minutes").and_then(Value::as_f64).unwrap_or(0.0);
            let after = format!(" after {minutes} min");
            let last = a.get("last").and_then(Value::as_str).unwrap_or("").trim();
            let quote = if last.is_empty() { String::new() } else { format!("\nIts last words: «{last}»") };
            let tools = a.get("denied").and_then(Value::as_array).map(|d| d.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            let because = if reason.is_empty() { String::new() } else { format!(": {reason}") };
            let exit = a.get("exit").and_then(Value::as_f64).map_or_else(|| "stopped".to_string(), |e| format!("exit {e}"));
            let head = match a.get("outcome").and_then(Value::as_str) {
                Some("asked") => format!("{who} stopped to ask something, and has done nothing more{after}. Answer it by messaging it: it takes up from there.{quote}"),
                Some("denied") => format!("{who} stopped{after} for want of permission to use {tools}, which it was denied.{quote}"),
                Some("failed") => format!("{who} failed{after} ({exit}){because}.{quote}"),
                Some("reported") => format!("{who} finished{after} and reported back itself.{quote}"),
                Some("finished") => format!("{who} finished{after}, but sent no report of its own.{quote}"),
                Some("silent") => format!("{who} ended{after} without a word: it sent no report and left no last message ({exit}){because}."),
                _ => format!("{who} ended{after} ({exit}){because}. That machine's XenoNet is older and can't say whether it finished or stopped to ask something, so don't take it for done: update XenoNet there (xenonet update tools, then xenonet update service)."),
            };
            format!("[XenoNet] {head}")
        }
        _ => format!("[XenoNet] No session was started{on}: {reason}."),
    }
}

/// What the Allow card says of a call that may read the run's environment:
/// Claude Code passes its own environment to its tools, so a token given
/// to XenoNet's runs (`xenonet runs sign-in`) is readable there.
pub const ENV_WARNING: &str =
    "This may read the run's environment, which holds the Claude Code sign-in token you gave XenoNet's runs, if you gave one. Deny it unless the session may see that token.";
