//! A page as the agent reads it: a compact tree of what a person can see
//! and use, each element named by a short uid.
//!
//! The page is read by one script (`snapshot.js`) that returns the elements
//! as DOM nodes, so the browser names each one with a `sharedId`, and a tree
//! that points at them by index. Nothing is written into the page: no
//! attribute, no class. The uid-to-`sharedId` map lives here.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

/// The script that reads the page.
pub const SNAPSHOT_JS: &str = include_str!("snapshot.js");
/// The script that returns the page's visible text.
pub const PAGE_TEXT_JS: &str = include_str!("pagetext.js");
/// The script that readies a field for `fill_by_uid`.
pub const PREPARE_FILL_JS: &str = include_str!("prepare_fill.js");

#[derive(Debug, Default, Deserialize)]
struct Tree {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    items: Vec<Item>,
}

#[derive(Debug, Default, Deserialize)]
struct Item {
    /// Index into the returned nodes.
    n: Option<usize>,
    #[serde(default)]
    role: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    level: Option<u8>,
    #[serde(default)]
    attrs: BTreeMap<String, String>,
    /// Plain text.
    t: Option<String>,
    /// A group such as a form or a list.
    c: Option<String>,
    #[serde(default)]
    children: Vec<Item>,
}

/// One snapshot: its text and the uids it handed out.
#[derive(Debug)]
pub struct Snapshot {
    /// What the agent reads.
    pub text: String,
    /// uid to the browser's `sharedId` of the element.
    pub uids: HashMap<String, String>,
}

/// The `string` inside a BiDi remote value, or an error.
fn remote_string(v: &Value) -> Option<&str> {
    if v.get("type")?.as_str()? == "string" {
        v.get("value")?.as_str()
    } else {
        None
    }
}

/// Reads the remote value the snapshot script returned. `first_uid` is the
/// number the first uid gets, so uids of an older snapshot never name a
/// different element in a newer one.
pub fn parse(remote: &Value, first_uid: u64) -> Result<(Snapshot, u64)> {
    let entries = remote.get("value").and_then(Value::as_array).ok_or_else(|| Error::BadSnapshot("the script did not return an object".to_owned()))?;
    let field = |name: &str| entries.iter().find(|e| e.get(0).and_then(Value::as_str) == Some(name)).and_then(|e| e.get(1));
    let tree_text = field("tree").and_then(remote_string).ok_or_else(|| Error::BadSnapshot("there is no tree".to_owned()))?;
    let tree: Tree = serde_json::from_str(tree_text).map_err(|e| Error::BadSnapshot(format!("the tree is not valid: {e}")))?;
    let shared: Vec<Option<String>> = field("nodes")
        .and_then(|n| n.get("value"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::BadSnapshot("there is no list of nodes".to_owned()))?
        .iter()
        .map(|n| n.get("sharedId").and_then(Value::as_str).map(str::to_owned))
        .collect();

    let mut out = String::new();
    let title = if tree.title.is_empty() { String::new() } else { format!(" ({})", tree.title) };
    let _ = writeln!(out, "page: {}{title}", tree.url);
    let mut uids = HashMap::new();
    let mut next = first_uid;
    render(&tree.items, 0, &shared, &mut next, &mut uids, &mut out);
    if tree.truncated {
        out.push_str("(the page is long; the rest is left out. Scroll or navigate, then take another snapshot.)\n");
    }
    Ok((Snapshot { text: out, uids }, next))
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn render(items: &[Item], depth: usize, shared: &[Option<String>], next: &mut u64, uids: &mut HashMap<String, String>, out: &mut String) {
    let pad = "  ".repeat(depth);
    for item in items {
        if let Some(text) = &item.t {
            let _ = writeln!(out, "{pad}{}", quoted(text));
        } else if let Some(group) = &item.c {
            let name = if item.name.is_empty() { String::new() } else { format!(" {}", quoted(&item.name)) };
            let _ = writeln!(out, "{pad}{group}{name}:");
            render(&item.children, depth + 1, shared, next, uids, out);
        } else {
            // An element the browser gave no id (it left the page) gets no uid.
            let id = item.n.and_then(|n| shared.get(n)).and_then(Clone::clone);
            let uid = id.map(|id| {
                let uid = format!("e{next}");
                *next += 1;
                uids.insert(uid.clone(), id);
                uid
            });
            let mut line = format!("{pad}[{}] {}", uid.as_deref().unwrap_or("-"), item.role);
            if let Some(level) = item.level {
                let _ = write!(line, " level {level}");
            }
            if !item.name.is_empty() {
                let _ = write!(line, " {}", quoted(&item.name));
            }
            if !item.value.is_empty() {
                let _ = write!(line, " value={}", quoted(&item.value));
            }
            for (k, v) in &item.attrs {
                if k == "checked" || k == "disabled" || k == "required" || k == "multiple" {
                    let _ = write!(line, " {k}");
                } else {
                    let _ = write!(line, " {k}={}", quoted(v));
                }
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
}
