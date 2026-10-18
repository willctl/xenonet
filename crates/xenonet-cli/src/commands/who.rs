//! `xenonet who <path>` (cli/commands/who.js): which agents have a hand in a
//! file or folder, as the engine sees it (the who_has tool, for people).

use crate::args::Flags;
use crate::clock;
use crate::js::{encode, num, on, str, t};
use crate::out::{ago, c, json, now, print, table, Done, Fail, Layout};
use crate::reach::need;
use serde_json::Value;
use xenonet_base::paths::{normalize_posix, path_style, PathStyle};

pub const HELP: &str = include_str!("help/who.txt");

fn how_words(how: &str) -> String {
    match how {
        "editing" => "is editing it".into(),
        "worktree" => "is editing it in another worktree".into(),
        "claimed" => "claimed it".into(),
        h => h.into(),
    }
}

/// pathStyle(p).resolve(p): a path in its own syntax made whole. A Windows
/// path read on another system keeps its drive and is only tidied.
pub fn resolve_in_style(p: &str) -> String {
    match path_style(p) {
        PathStyle::Posix if !cfg!(windows) => crate::sys::resolve(p),
        PathStyle::Posix => {
            let n = normalize_posix(p);
            if n.len() > 1 {
                n.trim_end_matches('/').to_string()
            } else {
                n
            }
        }
        PathStyle::Windows if !cfg!(windows) => win32_normalize(p),
        _ => crate::sys::resolve(p),
    }
}

// path.win32.normalize, for a path with a drive or a share: backslashes,
// "." and ".." worked out, no slash at the end.
fn win32_normalize(p: &str) -> String {
    let s = p.replace('/', "\\");
    let (root, rest) = if s.len() >= 2 && s.as_bytes()[1] == b':' {
        (format!("{}\\", &s[..2]), s[2..].to_string())
    } else if let Some(r) = s.strip_prefix("\\\\") {
        ("\\\\".to_string(), r.to_string())
    } else {
        (String::new(), s.clone())
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            x => parts.push(x),
        }
    }
    format!("{root}{}", parts.join("\\"))
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let Some(target) = args.first().filter(|t| !t.is_empty()) else { return Err(Fail::new("say which file: xenonet who <path>", 2)) };
    let file = resolve_in_style(target);
    let s = need(flags)?;
    let hands = s.get(&format!("/api/engine/who?path={}", encode(&file)))?;
    if flags.on("json") {
        json(&hands);
        return Ok(0);
    }
    let hands = hands.as_array().cloned().unwrap_or_default();
    if hands.is_empty() {
        print(c::dim(format!("No session or actor has a hand in {target} right now.")));
        return Ok(0);
    }
    let rows: Vec<Vec<String>> = hands
        .iter()
        .map(|h| {
            let agent = h.get("agent").cloned().unwrap_or(Value::Null);
            let claimed = str(h, "how") == Some("claimed");
            let when = if claimed {
                format!("{} until {}", if on(h, "note") { format!(" (\"{}\")", t(h, "note")) } else { String::new() }, clock::hour_minute(num(h, "until")))
            } else {
                format!(", {}", ago(num(h, "at"), now()))
            };
            vec![c::green("●"), t(&agent, "title"), c::dim(t(&agent, "app")), format!("{}{}{when}", how_words(&t(h, "how")), if on(h, "observed") { " (observed)" } else { "" })]
        })
        .collect();
    print(table(&rows, Layout::wrapped(0)));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_in_their_own_syntax() {
        assert_eq!(resolve_in_style("/w/atlas/../atlas/src/"), "/w/atlas/src");
        if !cfg!(windows) {
            assert_eq!(resolve_in_style("C:\\w\\x\\..\\y\\"), "C:\\w\\y");
        }
    }
}
