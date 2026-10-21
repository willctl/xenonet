//! `xenonet search <words>` (cli/commands/search.js): every Claude Code and
//! Codex transcript, through the running XenoNet's index (core/search.js).

use crate::args::Flags;
use crate::js::{encode, have, list, num, on, slice_to, str, t};
use crate::out::{ago, c, clip, json, now, plural, print, width, Done, Fail};
use crate::reach::need;
use serde_json::{Map, Value};
use xenonet_base::jsstr::collapse_space;

pub const HELP: &str = include_str!("help/search.txt");

// The words that matched, ⟦marked⟧ by the index, in yellow; a mark left
// open or closed alone goes.
fn marked(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('⟦') {
        out.push_str(&rest[..i]);
        let after = &rest[i + '⟦'.len_utf8()..];
        let Some(j) = after.find('⟧') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        out.push_str(&c::yellow(&after[..j]));
        rest = &after[j + '⟧'.len_utf8()..];
    }
    out.push_str(rest);
    out.replace(['⟦', '⟧'], "")
}

pub fn run(words: &[String], flags: &Flags) -> Done {
    let q = words.join(" ");
    let q = xenonet_base::text::js_trim(&q).to_string();
    if q.is_empty() {
        return Err(Fail::new("say what to look for: xenonet search <words>", 2));
    }
    let s = need(flags)?;
    let answer = s.get(&format!("/api/search?q={}", encode(&q)))?;
    let results = list(&answer, "results");
    let limit = flags.number("limit");
    let top = slice_to(results, if limit == 0.0 || limit.is_nan() { 10.0 } else { limit });
    if flags.on("json") {
        let mut out = Map::new();
        out.insert("results".into(), Value::Array(top.to_vec()));
        if let Some(st) = answer.get("status") {
            out.insert("status".into(), st.clone());
        }
        json(&Value::Object(out));
        return Ok(0);
    }
    if top.is_empty() {
        let docs = answer.get("status").and_then(|st| st.get("docs")).filter(|d| xenonet_base::text::truthy(Some(d)));
        print(c::dim(format!("Nothing in {} matches \"{q}\".", docs.map_or("the transcripts".into(), |d| plural(xenonet_base::text::js_number(Some(d)), "indexed message")))));
        return Ok(0);
    }
    let room = width() as i64 - 4;
    for r in top {
        let snippets = list(r, "snippets");
        let mut meta = Vec::new();
        if on(r, "app") {
            meta.push(t(r, "app"));
        }
        meta.push(plural(num(r, "hits"), "hit"));
        if let Some(first) = snippets.first().filter(|sn| on(sn, "ts")) {
            meta.push(ago(num(first, "ts"), now()));
        }
        let title = have(r, "title").map_or_else(|| t(r, "sessionId"), |v| crate::js::text(Some(v)));
        print(format!("{} {}", c::bold(title), c::dim(meta.join(" · "))));
        for sn in snippets.iter().take(2) {
            let role = str(sn, "role").unwrap_or("");
            let text = clip(&collapse_space(&t(sn, "text")), (room - role.encode_utf16().count() as i64 - 2).max(0) as usize);
            print(format!("  {} {}", c::dim(format!("{}:", t(sn, "role"))), marked(&text)));
        }
    }
    if results.len() > top.len() {
        print(c::dim(format!("\n{} more; --limit {} shows them.", results.len() - top.len(), results.len())));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_marked() {
        assert_eq!(marked("a ⟦refund⟧ b ⟦c"), "a refund b c");
        assert_eq!(marked("⟦a⟦b⟧"), "ab");
        assert_eq!(collapse_space("a \n\t b"), "a b");
    }
}
