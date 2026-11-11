//! The remote agent's bearer token: what the person pasted into the
//! agent's secret card, kept by the service in their credential store and
//! handed here for one call. It never shows: no Display, no Serialize, no
//! Clone, a masked Debug, a header marked sensitive (so hyper's own Debug
//! masks it too), and [`Bearer::redact`] over every word that comes back.

use reqwest::header::HeaderValue;
use std::fmt;

pub struct Bearer(String);

impl Bearer {
    /// The token, when it's one line of visible characters; None
    /// otherwise, so a token with a newline can't add a header.
    pub fn new(token: String) -> Option<Bearer> {
        let t = token.trim();
        (!t.is_empty() && t.len() <= 4096 && t.bytes().all(|b| b.is_ascii_graphic())).then(|| Bearer(t.to_string()))
    }

    /// `Authorization: Bearer …`, marked sensitive.
    pub(crate) fn header(&self) -> Option<HeaderValue> {
        let mut v = HeaderValue::from_str(&format!("Bearer {}", self.0)).ok()?;
        v.set_sensitive(true);
        Some(v)
    }

    /// `text` with every copy of the token taken out: what a remote agent
    /// says may echo the request back.
    pub fn redact(&self, text: &str) -> String {
        if self.0.len() < 4 {
            text.to_string()
        } else {
            text.replace(&self.0, "[token]")
        }
    }
}

impl fmt::Debug for Bearer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bearer(…)")
    }
}

/// `text` with the token out, when there is one.
pub(crate) fn redact(bearer: Option<&Bearer>, text: &str) -> String {
    bearer.map_or_else(|| text.to_string(), |b| b.redact(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_never_shows() {
        let b = Bearer::new("sk-live-abcdef0123456789".into()).unwrap();
        assert_eq!(format!("{b:?}"), "Bearer(…)");
        let h = b.header().unwrap();
        assert!(h.is_sensitive());
        assert!(!format!("{h:?}").contains("abcdef"), "hyper masks a sensitive header");
        assert_eq!(b.redact("bad token sk-live-abcdef0123456789, try again"), "bad token [token], try again");
        assert!(Bearer::new("two\nlines".into()).is_none());
        assert!(Bearer::new("  ".into()).is_none());
    }
}
