//! core/mcp-names.js: a name for a shared system whose id says nothing.
//! Claude desktop reaches its connectors (Spotify, PostHog...) by uuid, and
//! nothing on disk maps the uuid to the service. What does say it is the
//! server itself, in the transcripts: the instructions it hands the session,
//! and its own results. Reading the lines it is given is all this does.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use xenonet_base::jsstr;
use xenonet_base::text::{is_js_space, js_trim};

const MIN_PROSE: usize = 3;
const MAX_LABELS: usize = 24;
const MAX_FORMS: usize = 4;
const MAX_CALLS: usize = 200;
const MAX_RESULT: usize = 40000;

// Hosts that name a platform, not the service on it.
const INFRA: [&str; 14] = [
    "amazonaws",
    "cloudfront",
    "cloudflare",
    "googleusercontent",
    "googleapis",
    "gstatic",
    "githubusercontent",
    "azureedge",
    "example",
    "w3",
    "schema",
    "claude",
    "anthropic",
    "modelcontextprotocol",
];
const SECOND_LEVEL: [&str; 7] = ["co", "com", "org", "net", "gov", "edu", "ac"];
const GENERIC_LEAD: [&str; 15] = ["Note", "Notes", "Important", "Usage", "Tool", "Tools", "Use", "Warning", "Instructions", "Rules", "Overview", "Server", "You", "This", "The"];
const GENERIC_TAIL: [&str; 8] = ["Tool", "Tools", "MCP", "Server", "Connector", "Integration", "API", "Plugin"];

/// An id that is a random token rather than a name: only these get
/// detected, so a server someone named keeps the name they gave it.
pub fn is_opaque(id: &str) -> bool {
    is_uuid(id) || (id.len() >= 24 && id.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// 8-4-4-4-12 hexadecimal digits.
pub fn is_uuid(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

/// The resources with the name Claude desktop declares for each connector
/// (desktop_connectors.rs) in place of what the lines suggested.
pub fn with_declared(resources: &mut Value, declared: &[(String, String)]) {
    for r in resources.as_array_mut().into_iter().flatten().filter_map(Value::as_object_mut) {
        let found = r.get("server").and_then(Value::as_str).and_then(|s| declared.iter().find(|(u, _)| u == s)).map(|(_, n)| n.clone());
        if let Some(name) = found {
            r.insert("name".into(), Value::String(name));
            r.insert("nameSource".into(), Value::String("declared".into()));
        }
    }
}

/// What the map shows for an opaque id nothing named, in place of 36
/// characters.
pub fn short_name(id: &str) -> String {
    format!("Connector {}", id.chars().take(8).collect::<String>())
}

/// The part of a host that names its owner: open.spotify.com is "spotify",
/// news.bbc.co.uk is "bbc". None for an address or a name that can't be one.
pub fn label_of(host: &str) -> Option<String> {
    let lower = host.to_lowercase();
    let parts: Vec<&str> = lower.split('.').filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 || parts.iter().all(|p| p.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut i = parts.len() - 2;
    if parts.len() > 2 && parts.get(i + 1).is_some_and(|p| p.chars().count() == 2) && parts.get(i).is_some_and(|p| SECOND_LEVEL.contains(p)) {
        i -= 1;
    }
    let label = *parts.get(i)?;
    let b = label.as_bytes();
    let fits = b.first().is_some_and(u8::is_ascii_lowercase) && (3..=24).contains(&b.len()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    (fits && !INFRA.contains(&label)).then(|| label.to_string())
}

// /^[A-Z][A-Za-z0-9]*$/
fn is_word(w: &str) -> bool {
    w.as_bytes().first().is_some_and(u8::is_ascii_uppercase) && w.bytes().all(|c| c.is_ascii_alphanumeric())
}

// /^(W(?: W){0,2})\s*[:—–-]\s/ with W a word: up to three words, then a
// colon or a dash and a space.
fn words_before_colon(line: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut rest = line;
    loop {
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric()).unwrap_or(rest.len());
        let (w, after) = rest.split_at(end);
        if !is_word(w) {
            return Vec::new();
        }
        words.push(w);
        match after.strip_prefix(' ') {
            Some(next) if words.len() < 3 && next.starts_with(|c: char| c.is_ascii_uppercase()) => rest = next,
            _ => {
                rest = after;
                break;
            }
        }
    }
    let mut chars = rest.trim_start_matches(is_js_space).chars();
    match (chars.next(), chars.next()) {
        (Some(':' | '\u{2014}' | '\u{2013}' | '-'), Some(c)) if is_js_space(c) => words,
        _ => Vec::new(),
    }
}

/// The name an instructions text opens with: its heading ("# PostHog `exec`
/// covers"), or the words before a colon ("Claude Docs: living docs...").
/// None when it opens with a sentence, which says what to do, not who it is.
pub fn lead_of(text: &str) -> Option<String> {
    let line = text.split('\n').find(|l| !js_trim(l).is_empty()).map_or("", js_trim);
    let mut words: Vec<&str> = if line.starts_with('#') {
        let heading = line.trim_start_matches('#').trim_start_matches(is_js_space);
        let mut words = Vec::new();
        for w in heading.split(is_js_space).filter(|w| !w.is_empty()) {
            if !is_word(w) || words.len() == 3 {
                break;
            }
            words.push(w);
        }
        words
    } else {
        words_before_colon(line)
    };
    while words.last().is_some_and(|w| GENERIC_TAIL.contains(w)) {
        words.pop();
    }
    if words.first().is_none_or(|w| GENERIC_LEAD.contains(w)) {
        return None;
    }
    let name = words.join(" ");
    (3..=28).contains(&name.len()).then_some(name)
}

/// What a domain's label came to in a server's results: its links, its
/// mentions in the words around them, and the spellings those had.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub urls: usize,
    pub prose: usize,
    pub forms: Vec<(String, usize)>,
}

/// What the lines showed of one server: the names its instructions opened
/// with, and the domains its results linked to, in the order first seen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Evidence {
    pub lead: Vec<(String, usize)>,
    pub labels: Vec<(String, Counts)>,
}

/// A name and where it was read from ("instructions" or "results").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub name: String,
    pub source: &'static str,
}

/// The name the evidence comes to, or None. Instructions that open with a
/// name are the strongest. Failing that, a service's own name shows up in its
/// own results, in prose and in the links to it; its CDN's name shows up
/// only in links, so a domain counts only when its label is also a word in
/// the text around the links.
pub fn detect(ev: &Evidence) -> Option<Name> {
    let lead = ev.lead.iter().reduce(|best, e| if e.1 > best.1 || (e.1 == best.1 && jsstr::cmp_units(&e.0, &best.0).is_lt()) { e } else { best });
    if let Some((name, _)) = lead {
        return Some(Name { name: name.clone(), source: "instructions" });
    }
    let mut best: Option<(&str, &Counts, usize)> = None;
    for (label, c) in &ev.labels {
        if c.prose < MIN_PROSE || c.urls < 1 {
            continue;
        }
        let score = c.prose + c.urls;
        if best.is_none_or(|(l, _, s)| score > s || (score == s && jsstr::cmp_units(label, l).is_lt())) {
            best = Some((label, c, score));
        }
    }
    let (label, c, _) = best?;
    // The spelling it was written with, when it has capitals inside ("PostHog").
    let form = c.forms.iter().filter(|(f, _)| *f != f.to_lowercase() && *f != f.to_uppercase()).reduce(|best, e| {
        if e.1 > best.1 || (e.1 == best.1 && jsstr::cmp_units(&e.0, &best.0).is_lt()) {
            e
        } else {
            best
        }
    });
    let name = form.map_or_else(
        || {
            let mut cs = label.chars();
            cs.next().map(|f| f.to_uppercase().chain(cs).collect()).unwrap_or_default()
        },
        |(f, _)| f.clone(),
    );
    Some(Name { name, source: "results" })
}

// A link, as /https?:\/\/([A-Za-z0-9.-]+)[^\s"'<>)\]\\]*/g finds it: where it
// starts and ends in the text, and its host.
struct Link<'a> {
    start: usize,
    end: usize,
    host: &'a str,
}

fn links_in(text: &str) -> Vec<Link<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(off) = text.get(at..).and_then(|t| t.find("http")) {
        let start = at + off;
        let after = &text[start + 4..];
        let after = after.strip_prefix('s').unwrap_or(after);
        let Some(after) = after.strip_prefix("://") else {
            at = start + 1;
            continue;
        };
        let host_len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-')).unwrap_or(after.len());
        if host_len == 0 {
            at = start + 1;
            continue;
        }
        let tail = &after[host_len..];
        let tail_len = tail.find(|c: char| is_js_space(c) || matches!(c, '"' | '\'' | '<' | '>' | ')' | ']' | '\\')).unwrap_or(tail.len());
        let end = text.len() - tail.len() + tail_len;
        out.push(Link { start, end, host: &after[..host_len] });
        at = end;
    }
    out
}

// The words in `prose` that spell `label` in any case, with no letter or
// digit beside them: the spellings as written.
fn spellings<'a>(prose: &'a str, label: &str) -> Vec<&'a str> {
    let (bytes, want) = (prose.as_bytes(), label.as_bytes());
    let mut out = Vec::new();
    let mut i = 0;
    while i + want.len() <= bytes.len() {
        let end = i + want.len();
        let hit = bytes.get(i..end).is_some_and(|w| w.eq_ignore_ascii_case(want))
            && (i == 0 || !bytes.get(i - 1).is_some_and(u8::is_ascii_alphanumeric))
            && !bytes.get(end).is_some_and(u8::is_ascii_alphanumeric);
        if let (true, Some(s)) = (hit, prose.get(i..end)) {
            out.push(s);
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// What the transcripts' lines show of each opaque server, line by line.
#[derive(Debug, Default)]
pub struct ServerNames {
    servers: HashMap<String, Evidence>,
    calls: Vec<(String, String)>,
}

impl ServerNames {
    pub fn new() -> Self {
        Self::default()
    }

    /// A Claude Code transcript's line, parsed. `server_of` says which
    /// server a tool's name is of, as live.js does.
    pub fn observe(&mut self, raw: &Value, server_of: impl Fn(&str) -> Option<String>) {
        let attachment = raw.get("attachment");
        if raw.get("type").and_then(Value::as_str) == Some("attachment") && attachment.and_then(|a| a.get("type")).and_then(Value::as_str) == Some("mcp_instructions_delta") {
            for block in attachment.and_then(|a| a.get("addedBlocks")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                // /^## (.+)\n([\s\S]*)$/: a header without line breaks, then the text.
                let Some((header, body)) = block.strip_prefix("## ").and_then(|b| b.split_once('\n')) else { continue };
                let server = js_trim(header);
                if header.is_empty() || header.contains(['\r', '\u{2028}', '\u{2029}']) || !is_opaque(server) {
                    continue;
                }
                if let Some(lead) = lead_of(body) {
                    let ev = self.servers.entry(server.to_string()).or_default();
                    match ev.lead.iter_mut().find(|(n, _)| *n == lead) {
                        Some(e) => e.1 += 1,
                        None => ev.lead.push((lead, 1)),
                    }
                }
            }
        }
        let Some(content) = raw.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) else { return };
        for b in content {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let server = b.get("name").and_then(Value::as_str).and_then(&server_of).filter(|s| is_opaque(s));
                    let (Some(server), Some(id)) = (server, b.get("id").and_then(Value::as_str)) else { continue };
                    match self.calls.iter_mut().find(|(i, _)| i == id) {
                        Some(call) => call.1 = server,
                        None => self.calls.push((id.to_string(), server)),
                    }
                    if self.calls.len() > MAX_CALLS {
                        self.calls.remove(0);
                    }
                }
                Some("tool_result") => {
                    let Some(at) = b.get("tool_use_id").and_then(Value::as_str).and_then(|id| self.calls.iter().position(|(i, _)| i == id)) else { continue };
                    let (_, server) = self.calls.remove(at);
                    let text = match b.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(items)) => items.iter().map(|x| x.get("text").and_then(Value::as_str).unwrap_or("")).collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    self.results(&server, &jsstr::head(&text, MAX_RESULT));
                }
                _ => {}
            }
        }
    }

    fn results(&mut self, server: &str, text: &str) {
        let links = links_in(text);
        let mut by_label: Vec<(String, HashSet<&str>)> = Vec::new();
        for l in &links {
            let Some(label) = label_of(l.host) else { continue };
            let full = &text[l.start..l.end];
            match by_label.iter_mut().find(|(k, _)| *k == label) {
                Some((_, set)) => {
                    set.insert(full);
                }
                None => by_label.push((label, HashSet::from([full]))),
            }
        }
        if by_label.is_empty() {
            return;
        }
        let mut prose = String::with_capacity(text.len());
        let mut at = 0;
        for l in &links {
            prose.push_str(&text[at..l.start]);
            prose.push(' ');
            at = l.end;
        }
        prose.push_str(&text[at..]);
        let ev = self.servers.entry(server.to_string()).or_default();
        for (label, set) in by_label.iter().take(16) {
            let at = match ev.labels.iter().position(|(k, _)| k == label) {
                Some(at) => at,
                None => {
                    ev.labels.push((label.clone(), Counts::default()));
                    ev.labels.len() - 1
                }
            };
            let Some((_, c)) = ev.labels.get_mut(at) else { continue };
            c.urls += set.len();
            for m in spellings(&prose, label) {
                c.prose += 1;
                if let Some(i) = c.forms.iter().position(|(f, _)| f == m) {
                    if let Some(f) = c.forms.get_mut(i) {
                        f.1 += 1;
                    }
                } else if c.forms.len() < MAX_FORMS {
                    c.forms.push((m.to_string(), 1));
                }
            }
        }
        if ev.labels.len() > MAX_LABELS {
            let weakest = ev.labels.iter().enumerate().fold(None, |w: Option<(usize, usize)>, (i, (_, c))| {
                let weight = c.prose + c.urls;
                if w.is_none_or(|(_, least)| weight < least) {
                    Some((i, weight))
                } else {
                    w
                }
            });
            if let Some((i, _)) = weakest {
                ev.labels.remove(i);
            }
        }
    }

    /// The server's name and where it came from, or None while the lines
    /// haven't said.
    pub fn name(&self, server: &str) -> Option<Name> {
        self.servers.get(server).and_then(detect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::mcp_server_of;
    use serde_json::json;

    const ID: &str = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";
    const OTHER: &str = "a9d3f0b2-6e41-4c57-8b12-77c0e5a49d36";

    fn tool_use(id: &str, server: &str) -> Value {
        json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "id": id, "name": format!("mcp__{server}__search"), "input": {} }] } })
    }
    fn tool_result(id: &str, content: Value) -> Value {
        json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "tool_use_id": id, "content": content }] } })
    }
    fn instructions(server: &str, body: &str) -> Value {
        json!({ "type": "attachment", "attachment": { "type": "mcp_instructions_delta", "addedNames": [server], "addedBlocks": [format!("## {server}\n{body}")], "removedNames": [] } })
    }
    fn names(lines: &[Value]) -> ServerNames {
        let mut n = ServerNames::new();
        for l in lines {
            n.observe(l, mcp_server_of);
        }
        n
    }
    fn tracks() -> String {
        (0..6).map(|i| format!("{{\"name\":\"Song {i}\",\"uri\":\"tunebox:track:{i}\",\"url\":\"https://open.tunebox.com/track/{i}\",\"cover\":\"https://i.tbcdn.co/image/{i}a\",\"thumb\":\"https://i.tbcdn.co/image/{i}b\",\"note\":\"Tunebox track {i}\"}}")).collect::<Vec<_>>().join("\n")
    }
    fn found(n: &ServerNames, id: &str) -> Option<(String, &'static str)> {
        n.name(id).map(|f| (f.name, f.source))
    }

    #[test]
    fn an_id_that_is_a_uuid_or_a_long_hex_token_is_opaque_and_a_name_someone_gave_is_not() {
        assert!(is_opaque(ID));
        assert!(is_opaque("0123456789abcdef0123456789abcdef"));
        for named in ["xenonet", "Claude_Browser", "plugin:data:amplitude", "firefox", "a1b2c3d4", ""] {
            assert!(!is_opaque(named), "{named}");
        }
        assert_eq!(short_name(ID), "Connector 5c1e4a90");
    }

    #[test]
    fn a_hosts_label_is_the_part_that_names_its_owner() {
        assert_eq!(label_of("open.tunebox.com").as_deref(), Some("tunebox"));
        assert_eq!(label_of("news.bbc.co.uk").as_deref(), Some("bbc"));
        assert_eq!(label_of("TuneBox.COM").as_deref(), Some("tunebox"));
        for not in ["localhost", "127.0.0.1", "10.0.0.8", "co.uk", "a.b", "bucket.s3.amazonaws.com", "d111.cloudfront.net", "my-site.com", ""] {
            assert_eq!(label_of(not), None, "{not}");
        }
    }

    #[test]
    fn instructions_name_a_server_by_their_heading_or_the_words_before_a_colon_and_not_by_a_sentence() {
        assert_eq!(lead_of("# Brightdesk `exec` covers\n\naction|alert").as_deref(), Some("Brightdesk"));
        assert_eq!(lead_of("Docs Studio: living docs you create and edit here.").as_deref(), Some("Docs Studio"));
        assert_eq!(lead_of("Docs Studio \u{2014} living docs").as_deref(), Some("Docs Studio"));
        assert_eq!(lead_of("# Notebook tools").as_deref(), Some("Notebook"));
        assert_eq!(lead_of("\n\n# Xenolith protocol").as_deref(), Some("Xenolith"));
        assert_eq!(lead_of("Foo Bar Baz Qux: four words"), None);
        for sentence in [
            "You have a computer-use MCP available.",
            "Use this server to fetch documentation.",
            "Note: prefer the dedicated tool.",
            "# Tools",
            "lowercase heading",
            "",
            "A: b",
            "Docs:",
        ] {
            assert_eq!(lead_of(sentence), None, "{sentence}");
        }
    }

    #[test]
    fn a_service_is_named_by_the_domain_its_own_results_also_say_in_words_not_by_its_cdn() {
        let n = names(&[tool_use("t1", ID), tool_result("t1", json!(tracks()))]);
        assert_eq!(found(&n, ID), Some(("Tunebox".into(), "results")));
    }

    #[test]
    fn the_spelling_the_service_is_written_with_wins_over_the_hosts_lowercase() {
        let body = (0..4).map(|i| format!("BrightDesk ticket {i}: https://app.brightdesk.com/t/{i}")).collect::<Vec<_>>().join("\n");
        let n = names(&[tool_use("t1", ID), tool_result("t1", json!(body))]);
        assert_eq!(found(&n, ID), Some(("BrightDesk".into(), "results")));
    }

    #[test]
    fn evidence_adds_up_over_calls_and_a_name_needs_the_domain_in_prose_not_just_in_links() {
        let mut n = names(&[
            tool_use("t1", ID),
            tool_result("t1", json!("see https://open.tunebox.com/track/1")),
            tool_use("t2", ID),
            tool_result("t2", json!("Tunebox says so, https://open.tunebox.com/track/2")),
        ]);
        assert_eq!(n.name(ID), None, "one mention in words is not enough");
        n.observe(&tool_use("t3", ID), mcp_server_of);
        n.observe(&tool_result("t3", json!("Tunebox, Tunebox again: https://open.tunebox.com/track/3")), mcp_server_of);
        assert_eq!(n.name(ID).map(|f| f.name).as_deref(), Some("Tunebox"));
    }

    #[test]
    fn only_the_results_of_calls_to_an_opaque_server_count_matched_by_the_calls_id() {
        let n = names(&[
            tool_use("t1", "xenonet"),
            tool_result("t1", json!(tracks())),
            tool_result("never-called", json!(tracks())),
            tool_use("t2", ID),
            tool_result("t2", json!("no links, no name")),
        ]);
        assert_eq!(n.name("xenonet"), None);
        assert_eq!(n.name(ID), None);
        let blocks = names(&[tool_use("t1", ID), tool_result("t1", json!([{ "type": "text", "text": tracks() }]))]);
        assert_eq!(blocks.name(ID).map(|f| f.name).as_deref(), Some("Tunebox"), "a result's text blocks are read too");
    }

    #[test]
    fn instructions_that_open_with_a_name_outrank_what_the_results_say() {
        let n = names(&[instructions(ID, "Docs Studio: living docs you create and edit here."), tool_use("t1", ID), tool_result("t1", json!(tracks()))]);
        assert_eq!(found(&n, ID), Some(("Docs Studio".into(), "instructions")));
        let sentence = names(&[instructions(ID, "Use this server to fetch documentation."), tool_use("t1", ID), tool_result("t1", json!(tracks()))]);
        assert_eq!(sentence.name(ID).map(|f| f.source), Some("results"), "a sentence names nobody, so the results decide");
    }

    #[test]
    fn a_servers_name_stays_its_own_others_evidence_and_readable_ids_are_not_mixed_in() {
        let n = names(&[instructions("firefox", "Browser: drives Firefox."), instructions(OTHER, "# Mailroom"), tool_use("t1", ID), tool_result("t1", json!(tracks()))]);
        assert_eq!(n.name("firefox"), None);
        assert_eq!(n.name(OTHER).map(|f| f.name).as_deref(), Some("Mailroom"));
        assert_eq!(n.name(ID).map(|f| f.name).as_deref(), Some("Tunebox"));
    }

    #[test]
    fn the_evidence_kept_is_bounded_and_odd_lines_are_skipped_without_panicking() {
        let mut n = ServerNames::new();
        let odd = [
            Value::Null,
            json!(7),
            json!("x"),
            json!({}),
            json!({ "message": {} }),
            json!({ "message": { "content": "text" } }),
            json!({ "type": "attachment", "attachment": { "type": "mcp_instructions_delta", "addedBlocks": [1, null, "no header", "## \nx", format!("## {ID}\r\nnope")] } }),
            json!({ "message": { "content": [null, 3, { "type": "tool_use" }, { "type": "tool_result", "tool_use_id": {} }] } }),
        ];
        for bad in &odd {
            n.observe(bad, mcp_server_of);
        }
        assert_eq!(n.name(ID), None);
        let many = (0..40).map(|i| format!("https://h{i}.site{}{i}.com/x Site{i}", (b'a' + (i % 26) as u8) as char)).collect::<Vec<_>>().join("\n");
        n.observe(&tool_use("t1", ID), mcp_server_of);
        n.observe(&tool_result("t1", json!(many)), mcp_server_of);
        assert_eq!(n.name(ID), None);
        assert!(n.servers.get(ID).is_some_and(|e| e.labels.len() <= MAX_LABELS));
    }

    #[test]
    fn detect_no_evidence_is_no_name_and_ties_break_by_name_so_the_answer_doesnt_flip() {
        assert_eq!(detect(&Evidence::default()), None);
        let c = || Counts { urls: 2, prose: 4, forms: Vec::new() };
        let ev = Evidence { lead: Vec::new(), labels: vec![("zeta".into(), c()), ("alpha".into(), c())] };
        assert_eq!(detect(&ev).map(|f| f.name).as_deref(), Some("Alpha"));
        let ev = Evidence { lead: vec![("Zed".into(), 1), ("Abe".into(), 1)], labels: Vec::new() };
        assert_eq!(detect(&ev).map(|f| f.name).as_deref(), Some("Abe"));
    }

    #[test]
    fn a_declared_name_replaces_what_the_lines_suggested_and_leaves_the_other_resources_as_they_were() {
        let mut resources = json!([{ "server": ID, "ts": 3, "name": "Guess", "nameSource": "results" }, { "server": OTHER, "ts": 2 }, { "server": "firefox", "ts": 1 }]);
        let declared = [(ID.to_string(), "Tunebox".to_string()), (OTHER.to_string(), "Brightdesk".to_string())];
        with_declared(&mut resources, &declared);
        assert_eq!(
            resources,
            json!([{ "server": ID, "ts": 3, "name": "Tunebox", "nameSource": "declared" }, { "server": OTHER, "ts": 2, "name": "Brightdesk", "nameSource": "declared" }, { "server": "firefox", "ts": 1 }])
        );
        let keys: Vec<&String> = resources[0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["server", "ts", "name", "nameSource"], "in the keys' order");
        let mut none = json!([{ "server": ID, "ts": 3 }]);
        with_declared(&mut none, &[]);
        assert_eq!(none, json!([{ "server": ID, "ts": 3 }]));
        with_declared(&mut Value::Null, &declared);
    }

    #[test]
    fn links_end_where_the_regex_ends_them() {
        let text = "a http://x.io/a?b=1) b \"https://y.dev/p\" c https://z.org\\n http:// d httpsx://n.com";
        let found: Vec<(&str, &str)> = links_in(text).iter().map(|l| (&text[l.start..l.end], l.host)).collect();
        assert_eq!(found, [("http://x.io/a?b=1", "x.io"), ("https://y.dev/p", "y.dev"), ("https://z.org", "z.org")]);
        assert_eq!(spellings("Tunebox, tunebox! tuneboxes xtunebox TUNEBOX", "tunebox"), ["Tunebox", "tunebox", "TUNEBOX"]);
    }
}
