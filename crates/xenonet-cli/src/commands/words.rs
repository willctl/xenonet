//! What the help says of the apps and of the doctor's checks, read from the
//! app registry (xenonet_base::apps) and from the checks themselves
//! (xenonet_doctor), so an app or a group of checks added there shows in
//! the help without anyone writing it in by hand.
//!
//! A help text, or a command's line in the list, names them with a token:
//!
//! ```text
//! {apps connect}      Claude Code, Codex and Grok Build
//! {apps connect or}   Claude Code, Codex or Grok Build
//! {apps connect's}    Claude Code's, Codex's and Grok Build's
//! {ids connect}       [claude] [codex] [grok]
//! {ids connect|}      claude|codex|grok
//! {doctor groups}     the groups doctor --only takes
//! {doctor apps}       the apps the doctor checks
//! ```
//!
//! `connect` is a capability in the registry; `lines` and `sessions` work
//! the same way.

use crate::out::wrap_text;
use xenonet_base::apps;

/// How wide a paragraph the help fills in runs, as the help's text does.
const WIDTH: usize = 76;

/// The capabilities a token can name.
const CAPABILITIES: [&str; 3] = ["connect", "lines", "sessions"];

/// Names as a sentence lists them: "A, B and C", or with `last` "or".
pub fn listed(names: &[String], last: &str) -> String {
    match names.split_last() {
        Some((end, rest)) if !rest.is_empty() => format!("{} {last} {end}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// The names of the apps that have a capability, in the registry's order.
pub fn names(capability: &str) -> Vec<String> {
    apps::with(capability).map(|a| a.name.clone()).collect()
}

/// The doctor's groups of checks, as --only takes them: the first part of
/// each check's id, in the order the checks run.
pub fn groups() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (id, _, _) in xenonet_doctor::checks::all() {
        let group = id.split('.').next().unwrap_or(id);
        if !out.contains(&group) {
            out.push(group);
        }
    }
    out
}

/// The apps the doctor checks: each with a group of checks of its own.
fn checked() -> Vec<String> {
    let groups = groups();
    apps::apps().iter().filter(|a| groups.contains(&a.id.as_str())).map(|a| a.name.clone()).collect()
}

/// Holds an app's name together on a line while a paragraph is wrapped,
/// and is a plain space again after. The help's own text has none.
const KEEP: char = '\u{a0}';

/// Each token, and what it stands for; with `keep`, each app's name holds
/// together (KEEP between its words).
fn tokens(keep: bool) -> Vec<(String, String)> {
    let held = |names: Vec<String>| -> Vec<String> {
        if keep {
            names.into_iter().map(|n| n.replace(' ', &KEEP.to_string())).collect()
        } else {
            names
        }
    };
    let mut out = Vec::new();
    for cap in CAPABILITIES {
        let names = held(names(cap));
        let ids: Vec<&str> = apps::with(cap).map(|a| a.id.as_str()).collect();
        let owned: Vec<String> = names.iter().map(|n| format!("{n}'s")).collect();
        out.push((format!("{{apps {cap}}}"), listed(&names, "and")));
        out.push((format!("{{apps {cap} or}}"), listed(&names, "or")));
        out.push((format!("{{apps {cap}'s}}"), listed(&owned, "and")));
        out.push((format!("{{ids {cap}}}"), ids.iter().map(|i| format!("[{i}]")).collect::<Vec<_>>().join(" ")));
        out.push((format!("{{ids {cap}|}}"), ids.join("|")));
    }
    out.push(("{doctor groups}".into(), groups().join(", ")));
    out.push(("{doctor apps}".into(), listed(&held(checked()), "and")));
    out
}

fn has(tokens: &[(String, String)], text: &str) -> bool {
    tokens.iter().any(|(k, _)| text.contains(k.as_str()))
}

fn fill_with(tokens: &[(String, String)], text: &str) -> String {
    tokens.iter().fold(text.to_string(), |s, (k, v)| s.replace(k.as_str(), v))
}

/// One line with its tokens filled in, and nothing else changed: a
/// command's line in the list, or its usage.
pub fn line(text: &str) -> String {
    fill_with(&tokens(false), text)
}

/// A help text with its tokens filled in. A paragraph of prose that had
/// one is wrapped again; an indented line that had one, such as an
/// option, wraps under the column its description starts at. The rest
/// stays as it was written. A line never breaks inside an app's name.
pub fn help(text: &str) -> String {
    let t = tokens(true);
    if !has(&t, text) {
        return text.to_string();
    }
    let prose = |l: &str| !l.is_empty() && !l.starts_with(' ');
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if prose(lines[i]) {
            let end = lines[i..].iter().position(|l| !prose(l)).map_or(lines.len(), |n| i + n);
            let para = &lines[i..end];
            if para.iter().any(|l| has(&t, l)) {
                out.extend(wrap_text(&fill_with(&t, &para.join(" ")), WIDTH));
            } else {
                out.extend(para.iter().map(|l| (*l).to_string()));
            }
            i = end;
        } else {
            let l = lines[i];
            if has(&t, l) {
                out.extend(hanging(&fill_with(&t, l)));
            } else {
                out.push(l.to_string());
            }
            i += 1;
        }
    }
    out.join("\n").replace(KEEP, " ")
}

/// An indented line wrapped under the column its description starts at,
/// after the first two spaces past its indent; as it is when it fits, or
/// has no description.
fn hanging(l: &str) -> Vec<String> {
    if l.chars().count() <= WIDTH {
        return vec![l.to_string()];
    }
    let lead = l.len() - l.trim_start().len();
    let Some(gap) = l[lead..].find("  ") else { return vec![l.to_string()] };
    let after = &l[lead + gap..];
    let at = lead + gap + after.len() - after.trim_start().len();
    let (head, desc) = l.split_at(at);
    let col = head.chars().count();
    let mut wrapped = wrap_text(desc, WIDTH.saturating_sub(col).max(16)).into_iter();
    let first = format!("{head}{}", wrapped.next().unwrap_or_default());
    std::iter::once(first).chain(wrapped.map(|w| format!("{}{w}", " ".repeat(col)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apps_come_from_the_registry() {
        let connect: Vec<&str> = apps::with("connect").map(|a| a.id.as_str()).collect();
        assert_eq!(connect, ["claude", "codex", "grok"], "if this changes, the help follows it");
        assert_eq!(line("{apps connect}"), "Claude Code, Codex and Grok Build");
        assert_eq!(line("{apps connect or}"), "Claude Code, Codex or Grok Build");
        assert_eq!(line("Take XenoNet out of {apps connect's} settings"), "Take XenoNet out of Claude Code's, Codex's and Grok Build's settings");
        assert_eq!(line("connect {ids connect}"), "connect [claude] [codex] [grok]");
        assert_eq!(line("<{ids connect|}>"), "<claude|codex|grok>");
        assert_eq!(line("{apps lines}"), line("{apps sessions}"));
        assert_eq!(listed(&["A".into()], "and"), "A");
        assert_eq!(listed(&["A".into(), "B".into()], "or"), "A or B");
    }

    #[test]
    fn the_doctors_groups_come_from_its_checks() {
        let groups = groups();
        assert_eq!(groups[..2], ["xenonet", "contracts"], "in the order the checks run");
        for g in ["claude", "codex", "grok", "system", "agents", "actors", "service"] {
            assert!(groups.contains(&g), "{g}");
        }
        for (id, _, _) in xenonet_doctor::checks::all() {
            assert!(groups.iter().any(|g| id.starts_with(&format!("{g}."))), "{id} is in a group --only takes");
        }
        assert_eq!(groups.iter().filter(|g| **g == "xenonet").count(), 1, "each group once");
        assert_eq!(line("{doctor apps}"), "Claude Code, Codex and Grok Build");
    }

    #[test]
    fn a_paragraph_that_names_them_is_wrapped_again_and_the_rest_kept() {
        let text = "Plain words that stay\nas they were written.\n\nSessions of {apps connect} see\neach other, and nothing else here changes.\n\n  --json  as JSON";
        assert_eq!(
            help(text),
            "Plain words that stay\nas they were written.\n\nSessions of Claude Code, Codex and Grok Build see each other, and nothing\nelse here changes.\n\n  --json  as JSON"
        );
        let plain = "  xenonet x   y\nz {not a token}";
        assert_eq!(help(plain), plain, "a text without a token comes back as it was");
        assert_eq!(help("  xenonet connect {ids connect}"), "  xenonet connect [claude] [codex] [grok]");
    }

    #[test]
    fn an_option_wraps_under_its_description() {
        let long = "  --only <groups>  only some checks: xenonet, contracts, claude, codex, grok, system, agents, actors, service (comma-separated)";
        assert_eq!(
            hanging(long),
            ["  --only <groups>  only some checks: xenonet, contracts, claude, codex,", "                   grok, system, agents, actors, service (comma-separated)"]
        );
        assert_eq!(hanging("  --json  short"), ["  --json  short"]);
        let usage = format!("  xenonet connect {}", "[app] ".repeat(14));
        assert_eq!(hanging(&usage), std::slice::from_ref(&usage), "no description to wrap");
        for l in help(include_str!("help/doctor.txt")).lines() {
            assert!(l.chars().count() <= WIDTH, "{l}");
        }
    }
}
