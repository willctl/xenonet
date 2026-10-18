//! A normalizer turns a vendor's record (a line of a transcript, an item an
//! app's API returns) into the record XenoNet works with, so a vendor's
//! names and quirks stay in one place, and every reader of the same source
//! goes through the same door (core/blocks/normalizer.js).
//!
//! Between parsing and normalizing, the record passes through `inspect`,
//! the hook for checking it against the vendor's schema leniently and
//! recording drift: it returns the record to use (the same, or with what
//! doesn't fit taken out), or None to drop it. A line that can't be parsed
//! goes to `unparsable` instead.
//!
//! It never fails for a record: what's wrong with one is the reader's to
//! find, where it was found before. A hook or `normalize` that fails on a
//! record is counted as an error (the last error says what), and the
//! reader goes on to the next record; one record isn't the normalizer's
//! health, so its state stays as it is.
//!
//! The hooks are whoever runs it: Rust in the binary, JS callbacks in the
//! twin. `R` is a record before normalizing, `T` after, and `W` where it
//! came from (whatever the caller keeps per file or request).

use crate::lifecycle::{Counters, Lifecycle};

pub trait Hooks<R, T, W: ?Sized> {
    /// A record from a line's text; Err with why when it isn't one.
    fn parse(&mut self, text: &str) -> Result<R, String>;
    /// The record to use, None to drop it, Err when the check failed.
    fn inspect(&mut self, raw: R, _where_: &W) -> Result<Option<R>, String> {
        Ok(Some(raw))
    }
    /// Told of a line that couldn't be parsed.
    fn unparsable(&mut self, _text: &str, _where_: &W, _why: &str) {}
    /// The record XenoNet works with.
    fn normalize(&mut self, raw: R, where_: &W) -> Result<T, String>;
    /// Whether there's an inspect hook at all.
    fn inspects(&self) -> bool {
        true
    }
}

pub struct Normalizer {
    pub name: String,
    /// The vendor schema's path under vendor/ (for drift records).
    pub source: Option<String>,
    pub life: Lifecycle,
    seen: f64,
}

impl Normalizer {
    pub fn new(name: &str, source: Option<&str>, now: f64) -> Self {
        let mut life = Lifecycle::new(now);
        life.counters = Counters::of(&[("errors", 0.0), ("records", 0.0), ("unparsable", 0.0), ("dropped", 0.0)]);
        Normalizer { name: name.to_string(), source: source.map(Into::into), life, seen: 0.0 }
    }

    /// A line of text, parsed and normalized; None when it isn't one.
    pub fn line<R, T, W: ?Sized>(&mut self, text: &str, where_: &W, hooks: &mut dyn Hooks<R, T, W>, now: impl Fn() -> f64) -> Option<T> {
        match hooks.parse(text) {
            Ok(raw) => self.record(raw, where_, hooks, now),
            Err(why) => {
                self.life.count("unparsable", 1.0);
                hooks.unparsable(text, where_, &why);
                None
            }
        }
    }

    /// A record already parsed, normalized; None when inspect drops it, or
    /// when a hook or normalize fails on it.
    pub fn record<R, T, W: ?Sized>(&mut self, raw: R, where_: &W, hooks: &mut dyn Hooks<R, T, W>, now: impl Fn() -> f64) -> Option<T> {
        self.life.count("records", 1.0);
        let raw = if hooks.inspects() {
            match hooks.inspect(raw, where_) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    self.life.count("dropped", 1.0);
                    return None;
                }
                Err(why) => return self.failed(&why, now()),
            }
        } else {
            raw
        };
        match hooks.normalize(raw, where_) {
            Ok(v) => Some(v),
            Err(why) => self.failed(&why, now()),
        }
    }

    fn failed<T>(&mut self, why: &str, at: f64) -> Option<T> {
        self.life.error(why, at);
        None
    }

    /// Records go through by the hundred thousand, so the clock isn't read
    /// for each: lastOk is when records were last seen going through, as
    /// of the last look.
    pub fn look(&mut self, at: f64) {
        let records = self.life.counters.get("records").unwrap_or(0.0);
        if records != self.seen {
            self.seen = records;
            self.life.ok(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl Hooks<String, String, ()> for Plain {
        fn parse(&mut self, text: &str) -> Result<String, String> {
            text.strip_prefix('{').map(str::to_string).ok_or_else(|| "not a record".to_string())
        }
        fn inspect(&mut self, raw: String, (): &()) -> Result<Option<String>, String> {
            match raw.as_str() {
                "drop" => Ok(None),
                "bad" => Err("can't check it".into()),
                _ => Ok(Some(raw)),
            }
        }
        fn normalize(&mut self, raw: String, (): &()) -> Result<String, String> {
            Ok(raw.to_uppercase())
        }
    }

    #[test]
    fn counts_what_goes_through() {
        let mut n = Normalizer::new("t", None, 0.0);
        let now = || 5.0;
        assert_eq!(n.line("{a", &(), &mut Plain, now), Some("A".to_string()));
        assert_eq!(n.line("x", &(), &mut Plain, now), None);
        assert_eq!(n.line("{drop", &(), &mut Plain, now), None);
        assert_eq!(n.line("{bad", &(), &mut Plain, now), None);
        let c: Vec<_> = n.life.counters.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(c, ["errors=1", "records=3", "unparsable=1", "dropped=1"]);
        assert_eq!(n.life.last_error.as_ref().map(|e| (e.at, e.message.as_str())), Some((5.0, "can't check it")));
        n.look(9.0);
        assert_eq!(n.life.last_ok, Some(9.0));
    }
}
