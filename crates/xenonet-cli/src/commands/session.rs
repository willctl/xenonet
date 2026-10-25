//! xenonet session (cli/commands/session.js): one session's models,
//! tokens, estimated cost, turns and tools (core/stats.js), the same as the
//! console's session details.

use crate::args::Flags;
use crate::clock::hour_minute as clock;
use crate::js::{amount, encode, have, list, num, on, slice_to, str, t};
use crate::out::{c, clip, json, plural, print, table, width, Done, Fail, Layout};
use crate::reach::need;
use crate::sys;
use serde_json::Value;
use xenonet_base::units::{duration, money, share, tokens};

pub const HELP: &str = include_str!("help/session.txt");

/// A tool's name as people read it: mcp__xenonet__send is xenonet.send.
pub fn tool_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("mcp__") {
        // (.+?)__: at least one character, then the first __ after it.
        if let Some(i) = rest.char_indices().nth(1).and_then(|(from, _)| rest[from..].find("__").map(|j| from + j)) {
            return format!("{}.{}", &rest[..i], &rest[i + 2..]);
        }
    }
    name.to_string()
}

pub fn run(words: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let q = words.join(" ");
    let q = xenonet_base::text::js_trim(&q).to_string();
    let by = if q.is_empty() { format!("cwd={}", encode(&sys::cwd())) } else { format!("q={}", encode(&q)) };
    let found = s.get(&format!("/api/sessions?limit=5&{by}"))?;
    let found = found.as_array().cloned().unwrap_or_default();
    let Some(first) = found.first() else {
        return Err(Fail::new(
            if q.is_empty() { "no session ran in this folder; name one: xenonet session <words or id>".to_string() } else { format!("no session matches \"{q}\"") },
            1,
        ));
    };
    let st = s.get(&format!("/api/session/{}/stats", encode(&t(first, "id"))))?;
    if flags.on("json") {
        json(&st);
        return Ok(0);
    }
    let home = sys::home();
    let tot = st.get("totals").cloned().unwrap_or(Value::Null);
    let turns = list(&st, "turns");

    print(c::bold(have(&st, "title").map_or_else(|| t(&st, "id"), |v| crate::js::text(Some(v)))));
    let mut meta = Vec::new();
    if on(&st, "app") {
        meta.push(t(&st, "app"));
    }
    if on(&st, "cwd") {
        meta.push(xenonet_base::paths::tilde(&t(&st, "cwd"), &home));
    }
    if on(&st, "branch") {
        meta.push(t(&st, "branch"));
    }
    if on(&st, "from") && on(&st, "to") {
        meta.push(format!("{}, {} at work", duration(Some(num(&st, "to") - num(&st, "from"))), duration(amount(&st, "activeMs"))));
    }
    meta.push(plural(turns.len() as f64, "turn"));
    print(c::dim(meta.join(" · ")));
    if found.len() > 1 && !q.is_empty() {
        print(c::dim(format!("({} also match; xenonet session <id> picks one)", plural((found.len() - 1) as f64, "other session"))));
    }
    print("");
    let unpriced = st.get("pricing").map(|p| list(p, "unpriced").iter().map(|v| crate::js::text(Some(v))).collect::<Vec<_>>()).unwrap_or_default();
    let total = amount(&tot, "cost").unwrap_or(0.0);
    // Grok Build's cost is what Grok recorded, which it does with an API key.
    let cost = if st.get("app").and_then(Value::as_str) == Some("grok") {
        match amount(&tot, "cost") {
            None => format!("{} {}", c::bold("unknown"), c::dim("Grok Build recorded no cost; it records one with an API key")),
            Some(n) => format!("{} {}", c::bold(money(Some(n))), c::dim("as Grok Build recorded it")),
        }
    } else {
        format!(
            "{} {}",
            c::bold(money(amount(&tot, "cost"))),
            c::dim(format!("at API prices on the day, standard tier{}", if unpriced.is_empty() { String::new() } else { format!("; no price for {}", unpriced.join(", ")) }))
        )
    };
    print(table(
        &[
            vec![c::dim("Estimated cost"), cost],
            vec![
                c::dim("Tokens"),
                format!(
                    "{} {}",
                    c::bold(tokens(amount(&tot, "tokens"))),
                    c::dim(format!("in {}; {} read from cache", plural(num(&tot, "calls"), "call"), share(num(&tot, "cacheRead"), num(&tot, "tokens"))))
                ),
            ],
        ],
        Layout::wrapped(2),
    ));

    print(format!("\n{}", c::bold("Models")));
    let models: Vec<Vec<String>> = list(&st, "models")
        .iter()
        .map(|m| {
            let efforts = list(m, "efforts");
            let effort = if efforts.is_empty() { String::new() } else { format!(" · {}", efforts.iter().map(|e| crate::js::text(Some(e))).collect::<Vec<_>>().join(", ")) };
            vec![
                t(m, "model"),
                c::dim(plural(num(m, "calls"), "call")),
                money(amount(m, "cost")),
                c::dim(share(amount(m, "cost").unwrap_or(0.0), total)),
                c::dim(format!("{} to {}{effort}", clock(num(m, "from")), clock(num(m, "to")))),
            ]
        })
        .collect();
    print(table(&models, Layout::indent(2)));

    print(format!("\n{}", c::bold("Tokens")));
    let kinds = st.get("kinds").cloned().unwrap_or(Value::Null);
    let rows: Vec<Vec<String>> = [("cacheRead", "read from cache"), ("cacheWrite", "written to cache"), ("input", "input"), ("output", "output")]
        .iter()
        .map(|(k, label)| {
            let kind = kinds.get(*k).cloned().unwrap_or(Value::Null);
            vec![label.to_string(), tokens(amount(&kind, "tokens")), money(amount(&kind, "cost")), c::dim(share(amount(&kind, "cost").unwrap_or(0.0), total))]
        })
        .collect();
    print(table(&rows, Layout::indent(2)));

    print(format!("\n{}", c::bold("Turns")));
    let room = (width() as i64 - 40).max(20) as usize;
    let shown = &turns[turns.len().saturating_sub(12)..];
    if turns.len() > shown.len() {
        print(c::dim(format!("  the last 12 of {}", turns.len())));
    }
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|x| {
            let prompt = have(x, "prompt").map_or_else(|| "(before the first prompt)".to_string(), |p| crate::js::text(Some(p)));
            vec![c::dim(t(x, "n")), clock(num(x, "start")), duration(Some(num(x, "end") - num(x, "start"))), money(amount(x, "cost")), c::dim(clip(&prompt, room))]
        })
        .collect();
    print(table(&rows, Layout::indent(2)));

    let tools = list(&st, "tools");
    if !tools.is_empty() {
        print(format!("\n{}", c::bold("Tools")));
        let rows: Vec<Vec<String>> = slice_to(tools, 10.0)
            .iter()
            .map(|x| {
                let errors = if on(x, "errors") { c::yellow(format!("{} failed", t(x, "errors"))) } else { String::new() };
                vec![
                    tool_name(str(x, "name").unwrap_or_default()),
                    c::dim(plural(num(x, "calls"), "call")),
                    errors,
                    c::dim(have(x, "ms").map_or(String::new(), |ms| duration(Some(xenonet_base::text::js_number(Some(ms)))))),
                ]
            })
            .collect();
        print(table(&rows, Layout::indent(2)));
    }
    let subagents = list(&st, "subagents");
    if !subagents.is_empty() {
        let cost: f64 = subagents.iter().map(|x| num(x, "cost")).sum();
        print(c::dim(format!("\n{}: {} of the cost", plural(subagents.len() as f64, "subagent"), money(Some(cost)))));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tool_names() {
        assert_eq!(super::tool_name("mcp__xenonet__send"), "xenonet.send");
        assert_eq!(super::tool_name("mcp____x"), "mcp____x");
        assert_eq!(super::tool_name("mcp___a__b"), "_a.b");
        assert_eq!(super::tool_name("Bash"), "Bash");
    }
}
