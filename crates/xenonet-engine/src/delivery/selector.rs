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

use super::template;
use crate::sql::Arg;
use crate::store::js_text;
use serde_json::{json, Value};
use xenonet_base::text::is_js_space;

pub const KEYS: [&str; 6] = ["project", "identity", "app", "machine", "status", "by"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selector {
    pub terms: Vec<Term>,
}

/// A term, `key:value`, as the JS's /^(keys):([^\s+]+)$/i takes it: the key
/// in any case of ASCII's letters, the value anything without white space
/// or "+".
fn term(p: &str) -> Option<Term> {
    let (key, value) = p.split_once(':')?;
    let key = KEYS.iter().find(|k| k.eq_ignore_ascii_case(key))?;
    if value.is_empty() || value.chars().any(|c| is_js_space(c) || c == '+') {
        return None;
    }
    Some(Term { key: (*key).to_string(), value: value.to_lowercase() })
}

fn like(value: Option<&Value>, want: &str) -> bool {
    let Some(value) = value.filter(|v| !v.is_null()) else { return false };
    let v = template(value).to_lowercase();
    match want.strip_suffix('*') {
        Some(prefix) => v.starts_with(prefix),
        None => v == want,
    }
}

impl Selector {
    /// The selector `text` spells, or None for what isn't one.
    pub fn parse(text: &str) -> Option<Selector> {
        let s = xenonet_base::text::js_trim(text);
        if s.is_empty() {
            return None;
        }
        if s == "all" {
            return Some(Selector { terms: Vec::new() });
        }
        let mut terms = Vec::new();
        for p in s.split(|c: char| is_js_space(c) || c == '+').filter(|p| !p.is_empty()) {
            terms.push(term(p)?);
        }
        Some(Selector { terms })
    }

    /// Whether `text` is a selector with terms: "all" and what isn't a
    /// selector are not.
    pub fn is_selector(text: &str) -> bool {
        Selector::parse(text).is_some() && xenonet_base::text::js_trim(text) != "all"
    }

    /// The selector's text: its terms, or "all" for none.
    pub fn format(sel: Option<&Selector>) -> String {
        match sel {
            Some(s) if !s.terms.is_empty() => s.terms.iter().map(|t| format!("{}:{}", t.key, t.value)).collect::<Vec<_>>().join(" "),
            _ => "all".into(),
        }
    }

    /// Whether the facts of one session or actor match: { app, project,
    /// projectNames[], identities[], machine, status, actors[] }.
    pub fn matches(&self, f: &Value) -> bool {
        for t in &self.terms {
            let value = t.value.as_str();
            let any = |k: &str| f.get(k).and_then(Value::as_array).is_some_and(|l| l.iter().any(|n| like(Some(n), value)));
            let ok = match t.key.as_str() {
                "app" => like(f.get("app"), value),
                "project" => any("projectNames"),
                "identity" => any("identities"),
                "machine" => like(f.get("machine").filter(|m| !m.is_null()).or(Some(&Value::String("local".into()))), value),
                "status" => {
                    if value == "live" {
                        f.get("status").and_then(Value::as_str) != Some("ended")
                    } else {
                        like(f.get("status"), value)
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

    /// Whether everything this selector can select lies inside `outer`:
    /// every term of outer appears here.
    pub fn within(&self, outer: Option<&Selector>) -> bool {
        let Some(outer) = outer else { return false };
        outer.terms.iter().all(|o| self.terms.iter().any(|i| i.key == o.key && (i.value == o.value || o.value.strip_suffix('*').is_some_and(|p| i.value.starts_with(p)))))
    }

    /// { terms: [{ key, value }] }.
    pub fn to_json(&self) -> Value {
        json!({ "terms": self.terms.iter().map(|t| json!({ "key": t.key, "value": t.value })).collect::<Vec<_>>() })
    }
}

pub fn format_selector(sel: Option<&Selector>) -> String {
    Selector::format(sel)
}

pub fn within(inner: &Selector, outer: Option<&Selector>) -> bool {
    inner.within(outer)
}

// The engine (engine.rs) and the service's routes still hand selectors
// and facts over as Arg: these go when the engine is typed.

pub fn parse_selector(text: &Arg) -> Option<Selector> {
    Selector::parse(&if text.is_nullish() { String::new() } else { js_text(text) })
}

pub fn is_selector(text: &Arg) -> bool {
    Selector::is_selector(&js_text(text))
}

/// The facts a selector is matched against, for one session or actor:
/// { app, project, projectNames[], identities[], machine, status, actors[] }.
pub fn matches(sel: Option<&Selector>, f: &Arg) -> bool {
    sel.is_some_and(|s| s.matches(&f.to_json()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_are_terms_or_nothing() {
        let s = Selector::parse(" Project:XenoNet+app:open* ").unwrap();
        assert_eq!(Selector::format(Some(&s)), "project:xenonet app:open*");
        assert_eq!(Selector::parse("all").unwrap().terms.len(), 0);
        assert!(Selector::parse("claude:abc").is_none());
        assert!(Selector::parse("ſtatus:idle").is_none(), "only ASCII's cases of a key");
        assert!(!Selector::is_selector("all"));
    }

    #[test]
    fn a_selector_matches_its_facts() {
        let f = json!({ "app": "opencode", "projectNames": ["XenoNet"], "status": "idle" });
        assert!(Selector::parse("project:xeno* app:opencode status:live machine:local").unwrap().matches(&f));
        assert!(!Selector::parse("identity:adal").unwrap().matches(&f));
        let outer = Selector::parse("project:xeno*");
        assert!(Selector::parse("project:xenonet app:codex").unwrap().within(outer.as_ref()));
    }
}
