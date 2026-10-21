//! One call's tokens, as every reader counts them and pricing prices them:
//! the `{ input, cacheRead, cacheWrite5m, cacheWrite1h, output }` that
//! core/stats.js and core/pricing.js take, with how fast the call ran.
//!
//! A figure's type is the reader's. Pricing takes numbers (`Option<f64>`, the
//! default, where None counts as 0). The transcript readers keep each figure
//! as the line wrote it (`Figure`), since JS adds a count written as a string
//! by joining it on, and each reader turns figures into numbers its own way.

use crate::text::js_number;
use serde_json::Value;

/// How fast a call ran, as its price reads it: Anthropic's fast mode, OpenAI's
/// priority tier, or neither (the standard tier). The JS takes these as two
/// options, `{ fast, priority }`, and a caller may set both; each then counts
/// only for a model whose provider prices it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Speed {
    pub fast: bool,
    pub priority: bool,
}

impl Speed {
    pub const STANDARD: Speed = Speed { fast: false, priority: false };
    pub const FAST: Speed = Speed { fast: true, priority: false };
    pub const PRIORITY: Speed = Speed { fast: false, priority: true };
}

/// A token figure as a transcript wrote it (None where it says nothing), or
/// as a reader worked it out with JS's arithmetic, which can give NaN.
#[derive(Clone, Debug, PartialEq)]
pub enum Figure {
    Written(Option<Value>),
    Worked(f64),
}

impl Default for Figure {
    fn default() -> Self {
        Figure::Written(None)
    }
}

impl Figure {
    pub fn of(v: Option<&Value>) -> Figure {
        Figure::Written(v.cloned())
    }

    /// `v ?? 0`.
    pub fn or0(v: Option<&Value>) -> Figure {
        match v {
            None | Some(Value::Null) => Figure::Worked(0.0),
            v => Figure::of(v),
        }
    }

    /// `Number(v)`.
    pub fn number(&self) -> f64 {
        match self {
            Figure::Written(v) => js_number(v.as_ref()),
            Figure::Worked(n) => *n,
        }
    }

    /// As the pricing reads a figure: nothing for null or none, else
    /// `Number(v)`.
    pub fn priced(&self) -> Option<f64> {
        match self {
            Figure::Written(None | Some(Value::Null)) => None,
            f => Some(f.number()),
        }
    }
}

/// One call's tokens: input not read from the cache, cache reads, cache
/// writes kept five minutes and an hour, and output (reasoning included),
/// and how fast it ran.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TokenUsage<T = Option<f64>> {
    pub input: T,
    pub cache_read: T,
    pub cache_write_5m: T,
    pub cache_write_1h: T,
    pub output: T,
    pub speed: Speed,
}

impl<T> TokenUsage<T> {
    /// The same call, its figures borrowed.
    pub fn as_ref(&self) -> TokenUsage<&T> {
        TokenUsage {
            input: &self.input,
            cache_read: &self.cache_read,
            cache_write_5m: &self.cache_write_5m,
            cache_write_1h: &self.cache_write_1h,
            output: &self.output,
            speed: self.speed,
        }
    }

    /// The same call with each figure made another way, in the order
    /// input, cache reads, the two cache writes, output.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> TokenUsage<U> {
        TokenUsage {
            input: f(self.input),
            cache_read: f(self.cache_read),
            cache_write_5m: f(self.cache_write_5m),
            cache_write_1h: f(self.cache_write_1h),
            output: f(self.output),
            speed: self.speed,
        }
    }

    /// `map` with a conversion that can fail, stopping at the first figure
    /// that does.
    pub fn try_map<U, E>(self, mut f: impl FnMut(T) -> Result<U, E>) -> Result<TokenUsage<U>, E> {
        Ok(TokenUsage {
            input: f(self.input)?,
            cache_read: f(self.cache_read)?,
            cache_write_5m: f(self.cache_write_5m)?,
            cache_write_1h: f(self.cache_write_1h)?,
            output: f(self.output)?,
            speed: self.speed,
        })
    }
}

impl TokenUsage {
    /// Every token the call counted, None as 0.
    pub fn total(&self) -> f64 {
        let n = |v: Option<f64>| v.unwrap_or(0.0);
        n(self.input) + n(self.cache_read) + n(self.cache_write_5m) + n(self.cache_write_1h) + n(self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn figures_read_as_js_reads_them() {
        assert_eq!(Figure::or0(None), Figure::Worked(0.0));
        assert_eq!(Figure::or0(Some(&Value::Null)), Figure::Worked(0.0));
        assert_eq!(Figure::or0(Some(&json!("5"))), Figure::Written(Some(json!("5"))));
        assert_eq!(Figure::of(Some(&json!("5"))).priced(), Some(5.0));
        assert_eq!([Figure::of(None).priced(), Figure::of(Some(&Value::Null)).priced()], [None, None]);
        assert!(Figure::of(None).number().is_nan() && Figure::Worked(f64::NAN).priced().is_some_and(f64::is_nan));
    }

    #[test]
    fn a_call_mapped_keeps_its_speed() {
        let written = TokenUsage { input: Figure::of(Some(&json!(10))), output: Figure::Worked(2.0), speed: Speed::FAST, ..Default::default() };
        let priced = written.map(|f| f.priced());
        assert_eq!(priced, TokenUsage { input: Some(10.0), output: Some(2.0), speed: Speed::FAST, ..Default::default() });
        assert_eq!(priced.total(), 12.0);
        let failed: Result<TokenUsage<f64>, &str> = priced.try_map(|v| v.ok_or("none"));
        assert_eq!(failed, Err("none"));
    }
}
