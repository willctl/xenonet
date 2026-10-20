//! core/assistant/cursors.js: page tokens for the assistant's lists,
//! opaque to the caller, signed so they can't be changed or made, and good
//! for five minutes. Each is bound to the grant, its revision, the tool and
//! its filters, and holds where the last page ended. The key is made when
//! the service starts and never written down, so a restart ends every
//! token.

use crate::crypto::{b64url, b64url_decode, hmac_sha256, random_bytes, same, sha256};
use serde_json::{Map, Value};
use xenonet_app::jsv::strict_eq;

pub const CURSOR_MS: f64 = 5.0 * 60e3;

// A short digest of the filters, as JSON writes them.
fn digest(filters: Option<&Value>) -> String {
    let text = xenonet_base::json::stringify(filters.unwrap_or(&Value::Null), 0);
    b64url(&sha256(text.as_bytes()))[..16].to_string()
}

pub struct Cursors {
    key: Vec<u8>,
}

/// What a token is for. Each part as the JS had it (undefined as None).
pub struct For<'a> {
    pub grant: Option<&'a Value>,
    pub revision: Option<&'a Value>,
    pub op: Option<&'a Value>,
    pub filters: Option<&'a Value>,
}

impl Cursors {
    /// With the key given, or 32 random bytes.
    pub fn new(key: Option<Vec<u8>>) -> Self {
        Cursors { key: key.unwrap_or_else(|| random_bytes(32)) }
    }

    fn sign(&self, payload: &str) -> String {
        b64url(&hmac_sha256(&self.key, payload.as_bytes()))
    }

    /// A token for the page after `after`.
    pub fn make(&self, f: &For<'_>, after: Option<&Value>, now: f64) -> String {
        let mut p = Map::new();
        p.insert("v".into(), Value::from(1));
        for (k, v) in [("g", f.grant), ("r", f.revision), ("o", f.op)] {
            if let Some(v) = v {
                p.insert(k.into(), v.clone());
            }
        }
        p.insert("f".into(), Value::String(digest(f.filters)));
        if let Some(a) = after {
            p.insert("a".into(), a.clone());
        }
        p.insert("t".into(), xenonet_base::json::number(now));
        let payload = b64url(xenonet_base::json::stringify(&Value::Object(p), 0).as_bytes());
        let mac = self.sign(&payload);
        format!("{payload}.{mac}")
    }

    /// Where a token says to go on from (None for an after it didn't
    /// have), or the code it's refused with: INVALID_ARGUMENT for one that
    /// was changed, made up, or is for another grant, tool or filters;
    /// CURSOR_EXPIRED for one too old or from before the grant changed.
    pub fn read(&self, cursor: &str, f: &For<'_>, now: f64) -> Result<Option<Value>, &'static str> {
        let bad = Err("INVALID_ARGUMENT");
        let mut parts = cursor.split('.');
        let (payload, mac) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if payload.is_empty() || mac.is_empty() {
            return bad;
        }
        if !same(self.sign(payload).as_bytes(), mac.as_bytes()) {
            return bad;
        }
        let text = String::from_utf8_lossy(&b64url_decode(payload)).into_owned();
        let Ok(p) = xenonet_base::json::parse(&text) else { return bad };
        let field = |k: &str| match &p {
            Value::Object(m) => m.get(k),
            _ => None,
        };
        let one = Value::from(1);
        let filters = Value::String(digest(f.filters));
        if !strict_eq(field("v"), Some(&one)) || !strict_eq(field("g"), f.grant) || !strict_eq(field("o"), f.op) || !strict_eq(field("f"), Some(&filters)) {
            return bad;
        }
        let t = field("t").and_then(Value::as_f64).filter(|t| t.is_finite());
        match t {
            Some(t) if now - t <= CURSOR_MS && t <= now + 1000.0 && strict_eq(field("r"), f.revision) => Ok(field("a").cloned()),
            _ => Err("CURSOR_EXPIRED"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_token_reads_back_only_for_what_it_was_made_for() {
        let c = Cursors::new(Some(vec![7; 32]));
        let (g, r, o, fl) = (json!("gr_1"), json!(1), json!("sessions"), json!({ "project": null }));
        let f = For { grant: Some(&g), revision: Some(&r), op: Some(&o), filters: Some(&fl) };
        let after = json!([-5, "claude:x"]);
        let tok = c.make(&f, Some(&after), 1000.0);
        assert_eq!(c.read(&tok, &f, 2000.0), Ok(Some(after)));
        assert_eq!(c.read(&tok, &f, 1000.0 + CURSOR_MS + 1.0), Err("CURSOR_EXPIRED"));
        let r2 = json!(2);
        assert_eq!(c.read(&tok, &For { revision: Some(&r2), ..f }, 2000.0), Err("CURSOR_EXPIRED"));
        let o2 = json!("actors");
        let f = For { grant: Some(&g), revision: Some(&r), op: Some(&o), filters: Some(&fl) };
        assert_eq!(c.read(&tok, &For { op: Some(&o2), ..f }, 2000.0), Err("INVALID_ARGUMENT"));
        let f = For { grant: Some(&g), revision: Some(&r), op: Some(&o), filters: Some(&fl) };
        assert_eq!(c.read(&format!("{tok}x"), &f, 2000.0), Err("INVALID_ARGUMENT"));
        assert_eq!(Cursors::new(Some(vec![8; 32])).read(&tok, &f, 2000.0), Err("INVALID_ARGUMENT"));
    }
}
