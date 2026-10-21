//! core/engine/toml.js: just enough TOML to add and take out one MCP server
//! in Grok Build's config.toml and leave the rest of the file as it is.
//! `grok mcp add` rewrites the whole file and drops its comments, so
//! XenoNet edits the text itself (connect.rs's GrokHooks).
//!
//! XenoNet's server is two tables, [mcp_servers.<name>] and
//! [mcp_servers.<name>.env], each line of them a string or a list of
//! strings, written as JSON writes them (which TOML reads the same). It
//! finds its tables by their headers, takes out the lines from each header
//! to the next, and writes its own at the end. A server of that name
//! written another way is one XenoNet doesn't edit.

use serde_json::{Map, Value};
use xenonet_base::json::stringify;
use xenonet_base::text::{is_js_space, js_trim};

// A key as a header or a key line has it: bare, or quoted.
fn toml_key(k: &str) -> String {
    if !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        k.to_string()
    } else {
        stringify(&Value::String(k.into()), 0)
    }
}

/// The dotted parts of a header or a key (mcp_servers."xenonet-dev".env),
/// unquoted, or None when they don't read as one.
pub fn key_parts(text: &str) -> Option<Vec<String>> {
    let s = js_trim(text);
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i] == b' ' || b[*i] == b'\t') {
            *i += 1;
        }
    };
    while i < b.len() {
        skip(&mut i);
        match b.get(i) {
            Some(q @ (b'"' | b'\'')) => {
                let end = s[i + 1..].find(*q as char)? + i + 1;
                parts.push(s[i + 1..end].to_string());
                i = end + 1;
            }
            _ => {
                let n = s[i..].bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-').count();
                if n == 0 {
                    return None;
                }
                parts.push(s[i..i + n].to_string());
                i += n;
            }
        }
        skip(&mut i);
        if i < b.len() {
            if b[i] != b'.' {
                return None;
            }
            i += 1;
            if i >= b.len() {
                return None;
            }
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// A line of the file: the table it's in, whether it's a header, and its
/// key and value when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub header: bool,
    pub path: Vec<String>,
    pub key: Option<Vec<String>>,
    pub value: Option<String>,
}

/// The file's lines. A line inside a multi-line string (""" or ''') is one
/// of its value's.
pub fn toml_lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut open: Option<&str> = None;
    for line in text.split('\n') {
        let bare = line.strip_suffix('\r').unwrap_or(line);
        if let Some(q) = open {
            out.push(Line { text: line.into(), header: false, path: path.clone(), key: None, value: None });
            if bare.matches(q).count() % 2 == 1 {
                open = None;
            }
            continue;
        }
        let parts = crate::re!(r"^\s*\[\[?([^\[\]#]+)\]\]?\s*(#.*)?$").captures(bare).and_then(|h| key_parts(&h[1]));
        if let Some(parts) = parts {
            path = parts;
            out.push(Line { text: line.into(), header: true, path: path.clone(), key: None, value: None });
            continue;
        }
        let m = crate::re!(r"^\s*([^=#\s][^=#]*?)\s*=\s*(.*)$").captures(bare);
        let key = m.as_ref().and_then(|m| key_parts(&m[1]));
        let value = key.as_ref().and_then(|_| m.as_ref().map(|m| m[2].to_string()));
        if let Some(v) = &value {
            for q in ["\"\"\"", "'''"] {
                if v.matches(q).count() % 2 == 1 {
                    open = Some(q);
                }
            }
        }
        out.push(Line { text: line.into(), header: false, path: path.clone(), key, value });
    }
    out
}

fn is_server(path: &[String], name: &str) -> bool {
    path.first().map(String::as_str) == Some("mcp_servers") && path.get(1).map(String::as_str) == Some(name)
}

/// Whether a file names server `name` in a form XenoNet doesn't edit: a
/// key for it outside its own tables.
pub fn foreign_server(text: &str, name: &str) -> bool {
    toml_lines(text).iter().any(|l| !l.header && !is_server(&l.path, name) && l.key.as_ref().is_some_and(|k| is_server(&[l.path.clone(), k.clone()].concat(), name)))
}

/// The file without server `name`'s tables (its own and its sub-tables),
/// each from its header to the next table's; the rest as it was.
pub fn without_server(text: &str, name: &str) -> String {
    let mut kept = Vec::new();
    let mut dropping = false;
    for l in toml_lines(text) {
        if l.header {
            dropping = is_server(&l.path, name);
        }
        if !dropping {
            kept.push(l.text);
        }
    }
    kept.join("\n")
}

fn json_text(s: &str) -> String {
    stringify(&Value::String(s.into()), 0)
}

/// Server `name` as XenoNet writes it: its command and arguments, then its
/// environment.
pub fn server_toml(name: &str, command: &str, args: &[String], env: &[(String, String)]) -> String {
    let head = format!("mcp_servers.{}", toml_key(name));
    let args: Vec<String> = args.iter().map(|a| json_text(a)).collect();
    let mut lines = vec![format!("[{head}]"), format!("command = {}", json_text(command)), format!("args = [{}]", args.join(", ")), String::new(), format!("[{head}.env]")];
    lines.extend(env.iter().map(|(k, v)| format!("{} = {}", toml_key(k), json_text(v))));
    lines.join("\n")
}

/// The file with server `name` written as given, in place of the one there
/// was, at the end after a blank line. Err when the file names it in a
/// form XenoNet doesn't edit.
pub fn with_server(text: &str, name: &str, command: &str, args: &[String], env: &[(String, String)]) -> Result<String, String> {
    if foreign_server(text, name) {
        return Err(format!("it names mcp_servers.{name} in a form XenoNet doesn't edit"));
    }
    let without = without_server(text, name);
    let rest = without.trim_end_matches(is_js_space);
    let gap = if rest.is_empty() { String::new() } else { format!("{rest}\n\n") };
    Ok(format!("{gap}{}\n", server_toml(name, command, args, env)))
}

// A value as XenoNet writes them: a string, or a list of strings, as JSON
// reads them; None for any other.
fn value(text: &str) -> Option<Value> {
    match xenonet_base::json::parse(js_trim(text)).ok()? {
        v @ Value::String(_) => Some(v),
        Value::Array(a) if a.iter().all(Value::is_string) => Some(Value::Array(a)),
        _ => None,
    }
}

/// Server `name` as the file has it: { command, args, env } from its
/// tables (a part it doesn't have is null; one it has in a form XenoNet
/// doesn't write is left out, as JS's undefined), or None when it has none.
pub fn read_server(text: &str, name: &str) -> Option<Value> {
    let lines: Vec<Line> = toml_lines(text).into_iter().filter(|l| is_server(&l.path, name)).collect();
    if !lines.iter().any(|l| l.header && l.path.len() == 2) {
        return None;
    }
    let mut server = Map::new();
    for k in ["command", "args", "env"] {
        server.insert(k.into(), Value::Null);
    }
    // undefined: the key goes.
    let set = |m: &mut Map<String, Value>, k: &str, v: Option<Value>| match v {
        Some(v) => {
            m.insert(k.into(), v);
        }
        None => {
            m.remove(k);
        }
    };
    for l in &lines {
        let (false, Some(key), Some(v)) = (l.header, &l.key, &l.value) else { continue };
        let key: Vec<&str> = l.path[2..].iter().chain(key.iter()).map(String::as_str).collect();
        match key.as_slice() {
            [k @ ("command" | "args")] => set(&mut server, k, value(v)),
            ["env", k] => {
                let mut env = match server.get("env") {
                    Some(Value::Object(e)) => e.clone(),
                    _ => Map::new(),
                };
                set(&mut env, k, value(v));
                server.insert("env".into(), Value::Object(env));
            }
            ["env"] => set(&mut server, "env", None),
            _ => {}
        }
    }
    // The keys in the order JS gave them.
    let mut ordered = Map::new();
    for k in ["command", "args", "env"] {
        if let Some(v) = server.remove(k) {
            ordered.insert(k.into(), v);
        }
    }
    Some(Value::Object(ordered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINE: &str = "[mcp_servers.xenonet]\ncommand = \"/bin/xenonet\"\nargs = [\"mcp\"]\n\n[mcp_servers.xenonet.env]\nXENONET_PORT = \"47840\"";

    #[test]
    fn keys_and_lines() {
        assert_eq!(key_parts(r#"mcp_servers."xenonet-dev" . env"#), Some(vec!["mcp_servers".into(), "xenonet-dev".into(), "env".into()]));
        assert_eq!(key_parts("a.'b c'"), Some(vec!["a".into(), "b c".into()]));
        assert_eq!((key_parts("a."), key_parts("a b"), key_parts("1, 2")), (None, None, None));
        let lines = toml_lines("x = \"\"\"\n[not.a.header]\n\"\"\"\n[cli]\ninstaller = \"internal\"");
        assert_eq!(lines.iter().map(|l| l.header).collect::<Vec<_>>(), [false, false, false, true, false]);
        assert_eq!(lines[4].path, ["cli"]);
    }

    #[test]
    fn a_server_in_and_out_with_the_rest_as_it_was() {
        let text = "# my settings\n[cli]\ninstaller = \"internal\" # kept\n\n[mcp_servers.other]\ncommand = \"x\"\n";
        let env = [("XENONET_PORT".to_string(), "47840".to_string())];
        let with = with_server(text, "xenonet", "/bin/xenonet", &["mcp".into()], &env).unwrap();
        assert_eq!(with, format!("{}\n\n{MINE}\n", text.trim_end()));
        assert_eq!(read_server(&with, "xenonet"), Some(json!({ "command": "/bin/xenonet", "args": ["mcp"], "env": { "XENONET_PORT": "47840" } })));
        assert_eq!(read_server(&with, "other"), Some(json!({ "command": "x", "args": null, "env": null })));
        assert_eq!(with_server(&with, "xenonet", "/bin/xenonet", &["mcp".into()], &env).unwrap(), with, "again, the same");
        assert_eq!(without_server(&with, "xenonet").trim_end(), text.trim_end());
        assert_eq!(read_server(text, "xenonet"), None);
        assert_eq!(
            with_server("", "xenonet-dev", "/x", &["mcp".into(), "console".into()], &[]).unwrap(),
            "[mcp_servers.xenonet-dev]\ncommand = \"/x\"\nargs = [\"mcp\", \"console\"]\n\n[mcp_servers.xenonet-dev.env]\n"
        );
        assert_eq!(read_server("[mcp_servers.xenonet]\ncommand = 'literal'\nenv = { A = \"b\" }", "xenonet"), Some(json!({ "args": null })));
    }

    #[test]
    fn a_server_written_another_way_is_left_alone() {
        for text in ["[mcp_servers]\nxenonet = { command = \"x\" }", "mcp_servers.xenonet.command = \"x\""] {
            assert!(foreign_server(text, "xenonet"), "{text}");
            assert!(with_server(text, "xenonet", "/x", &[], &[]).is_err());
        }
        assert!(!foreign_server(MINE, "xenonet") && !foreign_server("[mcp_servers]\nother = {}", "xenonet"));
    }
}
