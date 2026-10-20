//! core/patch.js: an apply_patch text (Codex's patch format) read for the
//! files it touches. Live, the engine and the native readers each keep
//! their own record of what they find; this is only the reading.
//!
//! Only the headers are read: `*** Add File: `, `*** Update File: ` and
//! `*** Delete File: `, and the `*** Move to: ` line under an update that
//! renames its file. Each is a whole line, as JS's
//! /^\*\*\* (Add|Update|Delete) File: (.+)$|^\*\*\* Move to: (.+)$/gm reads
//! it: a line ends at \n, \r, U+2028 or U+2029, a header with nothing after
//! it isn't one, and the path is trimmed as String.prototype.trim does.

use crate::text::js_trim;

/// What a header says is done to its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Add,
    Update,
    Delete,
    /// The new name of the file the update above it changes.
    Move,
}

impl Kind {
    /// Its name in core/patch.js.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Add => "add",
            Kind::Update => "update",
            Kind::Delete => "delete",
            Kind::Move => "move",
        }
    }
}

/// A header: what's done, and to which path (trimmed; it can be empty when
/// the header has only spaces after it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: Kind,
    pub path: String,
}

const HEADS: [(&str, Kind); 4] = [("*** Add File: ", Kind::Add), ("*** Update File: ", Kind::Update), ("*** Delete File: ", Kind::Delete), ("*** Move to: ", Kind::Move)];

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// The headers of an apply_patch text, in the order they come.
pub fn headers(text: &str) -> Vec<Header> {
    text.split(is_line_terminator)
        .filter_map(|line| {
            HEADS.iter().find_map(|(head, kind)| line.strip_prefix(head).filter(|rest| !rest.is_empty()).map(|rest| Header { kind: *kind, path: js_trim(rest).to_string() }))
        })
        .collect()
}

/// The paths of an apply_patch text's headers, in the order they come: the
/// files it adds, updates and deletes, and with `moves` the names it moves
/// files to.
pub fn paths(text: &str, moves: bool) -> Vec<String> {
    headers(text).into_iter().filter(|h| moves || h.kind != Kind::Move).map(|h| h.path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_in_order_with_their_kinds() {
        let text = "*** Begin Patch\n*** Add File: a.txt\n+x\n*** Update File: b.txt\r\n*** Move to:  c.txt \n*** Delete File: d.txt\n*** Add File: \n*** End Patch";
        let found: Vec<(Kind, String)> = headers(text).into_iter().map(|h| (h.kind, h.path)).collect();
        assert_eq!(found, [(Kind::Add, "a.txt".into()), (Kind::Update, "b.txt".into()), (Kind::Move, "c.txt".into()), (Kind::Delete, "d.txt".into())]);
        assert_eq!(paths(text, true), ["a.txt", "b.txt", "c.txt", "d.txt"]);
        assert_eq!(paths(text, false), ["a.txt", "b.txt", "d.txt"]);
    }

    #[test]
    fn lines_end_where_js_ends_them() {
        // \r, U+2028 and U+2029 end a line as \n does, so a header after one
        // counts and a path stops at one.
        assert_eq!(paths("x\r*** Add File: a\u{2028}b\u{2029}*** Delete File: c", true), ["a", "c"]);
        // A header must start its line.
        assert_eq!(paths(" *** Add File: a\nx*** Update File: b", true), Vec::<String>::new());
        // Spaces alone are a path once trimmed: empty, kept for the caller.
        assert_eq!(paths("*** Add File:  ", true), [""]);
        assert_eq!(paths("*** Add File: \u{a0}a\u{feff}", true), ["a"]);
        assert_eq!(paths("", true), Vec::<String>::new());
    }
}
