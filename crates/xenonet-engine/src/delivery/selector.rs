//! Selectors: which sessions a message goes to, or which ones a caller may
//! see at all: core/delivery/selector.js. A selector is one or more
//! key:value terms, all of which must hold (AND), separated by spaces or
//! "+":
//!
//!   project:xenonet            the project's name (or the repo's folder)
//!   identity:adal              an account the session ran as or belongs to
//!   app:opencode               the harness (claude, codex, opencode, …)
//!   machine:local              the machine (local for this one, or a host's name)
//!   status:idle                idle, working or live (working or idle, not ended)
//!   by:hermes                  sessions an actor started or works on, or (in
//!                              a grant) the actor itself
//!
//! A value may end in * for a prefix. "all" is the empty selector. Anything
//! that isn't made of these terms isn't a selector, so what `to` took
//! before still means what it meant.

use crate::peer::list;
use crate::sql::Arg;
use crate::store::js_text;

pub const KEYS: [&str; 6] = ["project", "identity", "app", "machine", "status", "by"];

#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub terms: Vec<Term>,
}

impl Selector {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![(
            "terms".into(),
            Arg::List(self.terms.iter().map(|t| Arg::Obj(vec![("key".into(), Arg::text(t.key.clone())), ("value".into(), Arg::text(t.value.clone()))])).collect()),
        )])
    }

    /// A selector as JS passed it ({ terms: [{ key, value }] }), or None
    /// for what isn't one.
    pub fn of(a: &Arg) -> Option<Selector> {
        if a.is_nullish() {
            return None;
        }
        let terms = list(a.field("terms")).iter().map(|t| Term { key: js_text(&t.field("key")), value: js_text(&t.field("value")) }).collect();
        Some(Selector { terms })
    }
}

/// The JS's white space, for `[\s+]` and `\s`.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// A term, `key:value`, as the JS's /^(keys):([^\s+]+)$/i takes it: the key
/// in any case of ASCII's letters, the value anything without white space
/// or "+".
fn term(p: &str) -> Option<Term> {
    let (key, value) = p.split_once(':')?;
    let key = KEYS.iter().find(|k| k.eq_ignore_ascii_case(key))?;
    if value.is_empty() || value.chars().any(|c| js_space(c) || c == '+') {
        return None;
    }
    Some(Term { key: key.to_string(), value: value.to_lowercase() })
}

pub fn parse_selector(text: &Arg) -> Option<Selector> {
    let s = if text.is_nullish() { String::new() } else { js_text(text) };
    let s = xenonet_base::text::js_trim(&s);
    if s.is_empty() {
        return None;
    }
    if s == "all" {
        return Some(Selector { terms: Vec::new() });
    }
    let mut terms = Vec::new();
    for p in s.split(|c: char| js_space(c) || c == '+').filter(|p| !p.is_empty()) {
        terms.push(term(p)?);
    }
    Some(Selector { terms })
}

pub fn is_selector(text: &Arg) -> bool {
    parse_selector(text).is_some() && xenonet_base::text::js_trim(&js_text(text)) != "all"
}

pub fn format_selector(sel: Option<&Selector>) -> String {
    match sel {
        Some(s) if !s.terms.is_empty() => s.terms.iter().map(|t| format!("{}:{}", t.key, t.value)).collect::<Vec<_>>().join(" "),
        _ => "all".into(),
    }
}

fn like(value: &Arg, want: &str) -> bool {
    if value.is_nullish() {
        return false;
    }
    let v = js_text(value).to_lowercase();
    match want.strip_suffix('*') {
        Some(prefix) => v.starts_with(prefix),
        None => v == want,
    }
}

/// The facts a selector is matched against, for one session or actor:
/// { app, project, projectNames[], identities[], machine, status, actors[] }.
pub fn matches(sel: Option<&Selector>, f: &Arg) -> bool {
    let Some(sel) = sel else { return false };
    for t in &sel.terms {
        let value = t.value.as_str();
        let any = |k: &str| list(f.field(k)).iter().any(|n| like(n, value));
        let ok = match t.key.as_str() {
            "app" => like(&f.field("app"), value),
            "project" => any("projectNames"),
            "identity" => any("identities"),
            "machine" => like(&f.field("machine").or(Arg::text("local")), value),
            "status" => {
                if value == "live" {
                    f.field("status").as_str() != Some("ended")
                } else {
                    like(&f.field("status"), value)
                }
            }
            "by" => any("actors"),
            _ => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Whether everything `inner` can select lies inside `outer`: every term
/// of outer appears in inner.
pub fn within(inner: &Selector, outer: Option<&Selector>) -> bool {
    let Some(outer) = outer else { return false };
    outer.terms.iter().all(|o| inner.terms.iter().any(|i| i.key == o.key && (i.value == o.value || o.value.strip_suffix('*').is_some_and(|p| i.value.starts_with(p)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_are_terms_or_nothing() {
        let s = parse_selector(&Arg::text(" Project:XenoNet+app:open* ")).unwrap();
        assert_eq!(format_selector(Some(&s)), "project:xenonet app:open*");
        assert_eq!(parse_selector(&Arg::text("all")).unwrap().terms.len(), 0);
        assert!(parse_selector(&Arg::text("claude:abc")).is_none());
        assert!(parse_selector(&Arg::text("ſtatus:idle")).is_none(), "only ASCII's cases of a key");
        assert!(!is_selector(&Arg::text("all")));
    }

    #[test]
    fn a_selector_matches_its_facts() {
        let f = Arg::Obj(vec![("app".into(), Arg::text("opencode")), ("projectNames".into(), Arg::List(vec![Arg::text("XenoNet")])), ("status".into(), Arg::text("idle"))]);
        assert!(matches(parse_selector(&Arg::text("project:xeno* app:opencode status:live machine:local")).as_ref(), &f));
        assert!(!matches(parse_selector(&Arg::text("identity:adal")).as_ref(), &f));
        let outer = parse_selector(&Arg::text("project:xeno*"));
        assert!(within(&parse_selector(&Arg::text("project:xenonet app:codex")).unwrap(), outer.as_ref()));
    }
}
