//! What a call to a model cost at API prices on the day it was made: the
//! prices Anthropic and OpenAI published, with the date each took effect, in
//! US dollars per million tokens at the standard tier. Sessions run on a
//! plan (Claude Max, ChatGPT Pro…) cost the plan, not this; it's what the
//! same work would cost on the API. The table and the rules are
//! `core/pricing.js`'s, which lists the sources they were checked against.
//!
//! Not counted: batch (half price), data residency (1.1x), and Codex models
//! OpenAI doesn't price (codex-auto-review, gpt-5.3-codex-spark).

use crate::sync::Locked;
use crate::time::parse_iso;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const PRICES_CHECKED: &str = "2026-10-03";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Anthropic => "anthropic",
            Provider::OpenAi => "openai",
        }
    }
}

/// The rates for a whole call above `over` input tokens: from the day
/// `from` (or always) until the end of the day `until` (or for good).
/// `scale_cache` scales the cache rates; without it they scale as input.
#[derive(Clone, Debug, PartialEq)]
pub struct LongContext {
    pub over: f64,
    pub from: Option<&'static str>,
    pub until: Option<&'static str>,
    pub input: f64,
    pub output: f64,
    pub scale_cache: Option<f64>,
}

/// Anthropic's fast mode rates; cache rates scale with input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fast {
    pub input: f64,
    pub output: f64,
}

/// A price row: the models it's for (the first names the row), the day it
/// took effect, and its rates. OpenAI rows keep their one cache-write price
/// (GPT-5.6 and later) in `write5m`.
#[derive(Clone, Debug, PartialEq)]
pub struct Price {
    pub provider: Provider,
    pub ids: Vec<&'static str>,
    pub from: f64,
    pub input: f64,
    pub output: f64,
    pub write5m: Option<f64>,
    pub write1h: Option<f64>,
    pub read: f64,
    pub long_context: Option<LongContext>,
    pub fast: Option<Fast>,
}

type Row = (&'static str, &'static str, f64, f64, Option<f64>, Option<f64>, f64, Option<LongContext>, Option<Fast>);

const fn lc_until(from: Option<&'static str>, until: &'static str, input: f64, output: f64) -> LongContext {
    LongContext { over: 200_000.0, from, until: Some(until), input, output, scale_cache: None }
}

// OpenAI's long context: above 272K input tokens the whole call costs 2x for
// input, cached input and cache writes, and 1.5x for output.
fn lc(input: f64, output: f64) -> LongContext {
    LongContext { over: 272_000.0, from: None, until: None, input: input * 2.0, output: output * 1.5, scale_cache: Some(2.0) }
}

const fn fast(input: f64, output: f64) -> Fast {
    Fast { input, output }
}

fn anthropic() -> Vec<Row> {
    vec![
        ("claude-3-5-sonnet-20240620|claude-3-5-sonnet-20241022|claude-3-5-sonnet", "2024-06-20", 3.0, 15.0, Some(3.75), Some(6.0), 0.3, None, None),
        ("claude-3-5-haiku-20241022|claude-3-5-haiku", "2024-11-04", 1.0, 5.0, Some(1.25), None, 0.1, None, None),
        ("claude-3-5-haiku-20241022|claude-3-5-haiku", "2024-12-03", 0.8, 4.0, Some(1.0), Some(1.6), 0.08, None, None),
        ("claude-3-7-sonnet-20250219|claude-3-7-sonnet", "2025-02-24", 3.0, 15.0, Some(3.75), Some(6.0), 0.3, None, None),
        ("claude-opus-4-20250514|claude-opus-4-0|claude-opus-4", "2025-05-22", 15.0, 75.0, Some(18.75), Some(30.0), 1.5, None, None),
        (
            "claude-sonnet-4-20250514|claude-sonnet-4-0|claude-sonnet-4",
            "2025-05-22",
            3.0,
            15.0,
            Some(3.75),
            Some(6.0),
            0.3,
            Some(lc_until(Some("2025-08-12"), "2026-04-29", 6.0, 22.5)),
            None,
        ),
        ("claude-opus-4-1-20250805|claude-opus-4-1", "2025-08-05", 15.0, 75.0, Some(18.75), Some(30.0), 1.5, None, None),
        ("claude-sonnet-4-5-20250929|claude-sonnet-4-5", "2025-09-29", 3.0, 15.0, Some(3.75), Some(6.0), 0.3, Some(lc_until(None, "2026-04-29", 6.0, 22.5)), None),
        ("claude-haiku-4-5-20251001|claude-haiku-4-5", "2025-10-15", 1.0, 5.0, Some(1.25), Some(2.0), 0.1, None, None),
        ("claude-opus-4-5-20251101|claude-opus-4-5", "2025-11-24", 5.0, 25.0, Some(6.25), Some(10.0), 0.5, None, None),
        ("claude-opus-4-6", "2026-02-05", 5.0, 25.0, Some(6.25), Some(10.0), 0.5, Some(lc_until(None, "2026-03-12", 10.0, 37.5)), Some(fast(30.0, 150.0))),
        ("claude-sonnet-4-6", "2026-02-17", 3.0, 15.0, Some(3.75), Some(6.0), 0.3, Some(lc_until(None, "2026-03-12", 6.0, 22.5)), None),
        ("claude-opus-4-7", "2026-04-16", 5.0, 25.0, Some(6.25), Some(10.0), 0.5, None, Some(fast(30.0, 150.0))),
        ("claude-opus-4-8", "2026-05-28", 5.0, 25.0, Some(6.25), Some(10.0), 0.5, None, Some(fast(10.0, 50.0))),
        ("claude-fable-5|claude-mythos-5", "2026-06-09", 10.0, 50.0, Some(12.5), Some(20.0), 1.0, None, None),
        ("claude-sonnet-5", "2026-06-30", 2.0, 10.0, Some(2.5), Some(4.0), 0.2, None, None),
        ("claude-opus-5", "2026-07-24", 5.0, 25.0, Some(6.25), Some(10.0), 0.5, None, Some(fast(10.0, 50.0))),
        ("claude-fable-5-1|claude-mythos-5-1", "2026-09-01", 10.0, 50.0, Some(12.5), Some(20.0), 0.25, None, None),
        ("claude-opus-5-5", "2026-09-22", 4.0, 20.0, Some(5.0), Some(8.0), 0.2, None, Some(fast(8.0, 40.0))),
        ("claude-sonnet-5-5", "2026-09-28", 2.0, 10.0, Some(2.5), Some(4.0), 0.2, None, None),
    ]
}

fn openai() -> Vec<Row> {
    vec![
        ("gpt-4.1", "2025-04-14", 2.0, 8.0, None, None, 0.5, None, None),
        ("o3", "2025-04-16", 10.0, 40.0, None, None, 2.5, None, None),
        ("o3", "2025-06-10", 2.0, 8.0, None, None, 0.5, None, None),
        ("o4-mini", "2025-04-16", 1.1, 4.4, None, None, 0.275, None, None),
        ("codex-mini-latest", "2025-05-15", 1.5, 6.0, None, None, 0.375, None, None),
        ("gpt-5", "2025-08-07", 1.25, 10.0, None, None, 0.125, None, None),
        ("gpt-5-mini", "2025-08-07", 0.25, 2.0, None, None, 0.025, None, None),
        ("gpt-5-codex", "2025-09-15", 1.25, 10.0, None, None, 0.125, None, None),
        ("gpt-5.1|gpt-5.1-codex", "2025-11-13", 1.25, 10.0, None, None, 0.125, None, None),
        ("gpt-5.1-codex-mini", "2025-11-13", 0.25, 2.0, None, None, 0.025, None, None),
        ("gpt-5.1-codex-max", "2025-11-19", 1.25, 10.0, None, None, 0.125, None, None),
        ("gpt-5.2", "2025-12-11", 1.75, 14.0, None, None, 0.175, None, None),
        ("gpt-5.2-codex", "2025-12-18", 1.75, 14.0, None, None, 0.175, None, None),
        ("gpt-5.3-codex", "2026-02-05", 1.75, 14.0, None, None, 0.175, None, None),
        ("gpt-5.4", "2026-03-05", 2.5, 15.0, None, None, 0.25, Some(lc(2.5, 15.0)), None),
        ("gpt-5.4-mini", "2026-03-17", 0.75, 4.5, None, None, 0.075, None, None),
        ("gpt-5.5", "2026-04-23", 5.0, 30.0, None, None, 0.5, Some(lc(5.0, 30.0)), None),
        ("gpt-5.6-sol|gpt-5.6", "2026-07-09", 5.0, 30.0, Some(6.25), None, 0.5, Some(lc(5.0, 30.0)), None),
        ("gpt-5.6-sol|gpt-5.6", "2026-08-21", 4.0, 20.0, Some(5.0), None, 0.4, Some(lc(4.0, 20.0)), None),
        ("gpt-5.6-terra", "2026-07-09", 2.5, 15.0, Some(3.125), None, 0.25, Some(lc(2.5, 15.0)), None),
        ("gpt-5.6-terra", "2026-07-30", 2.0, 12.0, Some(2.5), None, 0.2, Some(lc(2.0, 12.0)), None),
        ("gpt-5.6-luna", "2026-07-09", 1.0, 6.0, Some(1.25), None, 0.1, Some(lc(1.0, 6.0)), None),
        ("gpt-5.6-luna", "2026-07-30", 0.2, 1.2, Some(0.25), None, 0.02, Some(lc(0.2, 1.2)), None),
        ("gpt-6-astra", "2026-09-03", 10.0, 50.0, Some(12.5), None, 1.0, Some(lc(10.0, 50.0)), None),
        ("gpt-6-sol", "2026-09-22", 2.0, 10.0, Some(2.5), None, 0.2, Some(lc(2.0, 10.0)), None),
        ("gpt-6-luna", "2026-09-22", 0.1, 0.5, Some(0.125), None, 0.01, Some(lc(0.1, 0.5)), None),
        ("gpt-6.1-sol", "2026-09-29", 2.0, 10.0, Some(2.5), None, 0.1, Some(lc(2.0, 10.0)), None),
    ]
}

// OpenAI's priority tier (Fast in Codex): about twice standard.
fn priority_scale(id: &str) -> f64 {
    match id {
        "gpt-5.5" => 2.5,
        "gpt-5-mini" => 1.8,
        "o3" => 1.75,
        "gpt-4.1" => 1.75,
        _ => 2.0,
    }
}

fn day(d: &str) -> f64 {
    parse_iso(&format!("{d}T00:00:00Z")).expect("a price row's day")
}

/// Every price row, Anthropic's then OpenAI's, in the table's order.
pub fn rows() -> &'static [Price] {
    static ROWS: OnceLock<Vec<Price>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let tag = |p: Provider| move |r: Row| (p, r);
        anthropic()
            .into_iter()
            .map(tag(Provider::Anthropic))
            .chain(openai().into_iter().map(tag(Provider::OpenAi)))
            .map(|(provider, (ids, from, input, output, write5m, write1h, read, long_context, fast))| Price {
                provider,
                ids: ids.split('|').collect(),
                from: day(from),
                input,
                output,
                write5m,
                write1h,
                read,
                long_context,
                fast,
            })
            .collect()
    })
}

/// Whether a service tier is OpenAI's priority tier (Fast, in Codex).
pub fn priority_tier(tier: &str) -> bool {
    tier.eq_ignore_ascii_case("priority") || tier.eq_ignore_ascii_case("fast")
}

// [^\]]*\]$: a trailing [flag], such as [1m].
fn strip_flag(s: &str) -> &str {
    if !s.ends_with(']') {
        return s;
    }
    let body = &s[..s.len() - 1];
    match body.rfind(['[', ']']) {
        Some(i) if body.as_bytes()[i] == b'[' => &s[..i],
        _ => s,
    }
}

// ^(?:[a-z]{2,4}\.)?(?:anthropic|openai)[./]: a provider, and a region
// before it (Bedrock's us.anthropic.…).
fn strip_provider(s: &str) -> &str {
    let after_provider = |t: &str| -> Option<usize> {
        let p = ["anthropic", "openai"].into_iter().find(|p| t.starts_with(p))?;
        matches!(t.as_bytes().get(p.len()), Some(b'.' | b'/')).then_some(p.len() + 1)
    };
    let b = s.as_bytes();
    let region = (2..=4).filter(|&n| b.len() > n && b[..n].iter().all(u8::is_ascii_lowercase) && b[n] == b'.').find_map(|n| after_provider(&s[n + 1..]).map(|p| n + 1 + p));
    if let Some(start) = region {
        return &s[start..];
    }
    after_provider(s).map_or(s, |n| &s[n..])
}

// -v\d+(?::\d+)?$: Bedrock's version, -v1:0.
fn strip_version(s: &str) -> &str {
    let digits_before = |t: &str| t.len() - t.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let mut end = s.len();
    let n = digits_before(&s[..end]);
    if n == 0 {
        return s;
    }
    end -= n;
    if s[..end].ends_with(':') {
        let m = digits_before(&s[..end - 1]);
        if m > 0 && s[..end - 1 - m].ends_with("-v") {
            return &s[..end - 1 - m - 2];
        }
    }
    if s[..end].ends_with("-v") {
        return &s[..end - 2];
    }
    s
}

/// A model id as the transcripts write it, plain: no provider or region
/// prefix (Bedrock's us.anthropic.…-v1:0, Vertex's …@date), no `[1m]` flag.
pub fn plain_model(model: &str) -> String {
    let s = model.to_lowercase();
    let s = strip_flag(&s);
    let s = strip_provider(s);
    let s = strip_version(s);
    // @(\d{8})$: Vertex's date, written as the API writes it.
    let b = s.as_bytes();
    if b.len() >= 9 && b[b.len() - 9] == b'@' && b[b.len() - 8..].iter().all(u8::is_ascii_digit) {
        return format!("{}-{}", &s[..s.len() - 9], &s[s.len() - 8..]);
    }
    s.to_string()
}

// The rows of a plain id, oldest first: those of the longest id it is or
// starts with, followed by "-". Worked out once per id, since the tailer
// prices a few models over and over; 500 ids at most, then it starts over.
fn price_rows(id: &str) -> Option<&'static [usize]> {
    type Kept = Mutex<HashMap<String, Option<&'static [usize]>>>;
    static KEPT: OnceLock<Kept> = OnceLock::new();
    let kept = KEPT.get_or_init(Default::default);
    if let Some(v) = kept.locked().get(id) {
        return *v;
    }
    let mut best: Option<(&str, &'static str)> = None;
    for r in rows() {
        for alt in &r.ids {
            let fits = id == *alt || (id.starts_with(alt) && id.as_bytes().get(alt.len()) == Some(&b'-'));
            if fits && best.is_none_or(|(b, _)| alt.len() > b.len()) {
                best = Some((alt, r.ids[0]));
            }
        }
    }
    let found = best.map(|(_, key)| rows_of(key));
    let mut kept = kept.locked();
    if kept.len() >= 500 {
        kept.clear();
    }
    kept.insert(id.to_string(), found);
    found
}

// A model's rows, oldest first, by its first id. Worked out once for each
// model the table has and kept for good: the ids a transcript names are
// anyone's, but the models they price to are the table's few dozen.
fn rows_of(key: &'static str) -> &'static [usize] {
    type Kept = Mutex<HashMap<&'static str, &'static [usize]>>;
    static KEPT: OnceLock<Kept> = OnceLock::new();
    let mut kept = KEPT.get_or_init(Default::default).locked();
    if let Some(v) = kept.get(key) {
        return v;
    }
    let all = rows();
    let mut idx: Vec<usize> = (0..all.len()).filter(|&i| all[i].ids[0] == key).collect();
    idx.sort_by(|&a, &b| all[a].from.total_cmp(&all[b].from));
    let v: &'static [usize] = Box::leak(idx.into_boxed_slice());
    kept.insert(key, v);
    v
}

/// The price row for a model on a day, or None when XenoNet doesn't know
/// one. `at` is when, in ms; None is now.
pub fn price_of(model: &str, at: Option<f64>) -> Option<&'static Price> {
    let id = plain_model(model);
    if id.is_empty() || id == "<synthetic>" {
        return None;
    }
    let idx = price_rows(&id)?;
    let all = rows();
    let at = at.unwrap_or_else(now_ms);
    let i = idx.iter().rev().find(|&&i| all[i].from <= at).unwrap_or(&idx[0]);
    Some(&all[*i])
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// A call's tokens: input (not cached), cache reads, cache writes kept five
/// minutes and an hour, and output (reasoning included). None counts as 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tokens {
    pub input: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write5m: Option<f64>,
    pub cache_write1h: Option<f64>,
    pub output: Option<f64>,
}

/// What one call cost, in dollars, by kind of token.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cost {
    pub input: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub output: f64,
    pub total: f64,
}

/// What one call cost, or None for a model without a price. `fast` is
/// Anthropic's fast mode, `priority` OpenAI's priority tier.
pub fn cost_parts(model: &str, at: Option<f64>, u: &Tokens, fast: bool, priority: bool) -> Option<Cost> {
    if plain_model(model) == "<synthetic>" {
        return Some(Cost { input: 0.0, cache_read: 0.0, cache_write: 0.0, output: 0.0, total: 0.0 });
    }
    let p = price_of(model, at)?;
    let n = |v: Option<f64>| v.unwrap_or(0.0);
    let prompt = n(u.input) + n(u.cache_read) + n(u.cache_write5m) + n(u.cache_write1h);
    let l = p.long_context.as_ref();
    // As JS compares with a missing `at`: never inside a dated window.
    let after = |d: Option<&str>| d.is_none_or(|d| at.is_some_and(|at| at >= day(d)));
    let before = |d: Option<&str>| d.is_none_or(|d| at.is_some_and(|at| at < day(d) + 86_400e3));
    let long = l.filter(|l| prompt > l.over && after(l.from) && before(l.until));
    let (input, output) = long.map_or((p.input, p.output), |l| (l.input, l.output));
    let cache_scale = long.map_or(1.0, |l| l.scale_cache.unwrap_or(l.input / p.input));
    let mut scale = 1.0;
    if fast {
        if let Some(f) = p.fast {
            scale = f.input / p.input;
        }
    }
    if priority && p.provider == Provider::OpenAi {
        scale = priority_scale(p.ids[0]);
    }
    // Cache writes without a price of their own: Anthropic's are 1.25x input
    // (5 minutes) and 2x (an hour); OpenAI's, before GPT-5.6, cost nothing extra.
    let write5m = p.write5m.unwrap_or(if p.provider == Provider::OpenAi { p.input } else { p.input * 1.25 });
    let write1h = p.write1h.unwrap_or(p.input * 2.0);
    let d = |v: Option<f64>, rate: f64| (n(v) * rate * scale) / 1e6;
    let input = d(u.input, input);
    let cache_read = d(u.cache_read, p.read * cache_scale);
    let cache_write = d(u.cache_write5m, write5m * cache_scale) + d(u.cache_write1h, write1h * cache_scale);
    let output = d(u.output, output);
    Some(Cost { input, cache_read, cache_write, output, total: input + cache_read + cache_write + output })
}

pub fn cost_of(model: &str, at: Option<f64>, u: &Tokens, fast: bool, priority: bool) -> Option<f64> {
    cost_parts(model, at, u, fast, priority).map(|c| c.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_of_one_model_share_its_rows() {
        // Each id is looked up once, and those of one model share its rows:
        // a thousand made-up ids in a transcript keep one copy, not one each.
        let first = price_rows("claude-3-5-haiku-a").unwrap();
        for i in 0..1000 {
            assert!(std::ptr::eq(first, price_rows(&format!("claude-3-5-haiku-{i}")).unwrap()));
        }
    }

    #[test]
    fn plain_ids() {
        for (written, plain) in [
            ("us.anthropic.claude-opus-4-1-20250805-v1:0", "claude-opus-4-1-20250805"),
            ("anthropic/claude-sonnet-4-5", "claude-sonnet-4-5"),
            ("claude-opus-4-1@20250805", "claude-opus-4-1-20250805"),
            ("claude-opus-5-5[1m]", "claude-opus-5-5"),
            ("Claude-Opus-5-5", "claude-opus-5-5"),
            ("openai.gpt-5-v2", "gpt-5"),
            ("eu.openai/gpt-5", "gpt-5"),
            ("anthropicx.claude", "anthropicx.claude"),
            ("a]b]", "a]b]"),
            ("x[a]b]", "x[a]b]"),
        ] {
            assert_eq!(plain_model(written), plain, "{written}");
        }
    }

    #[test]
    fn rows_by_day_and_longest_id() {
        let p = price_of("claude-opus-4-1-20250805", Some(day("2026-01-01"))).unwrap();
        assert_eq!(p.ids[0], "claude-opus-4-1-20250805");
        let haiku = |d| price_of("claude-3-5-haiku", Some(day(d))).unwrap().input;
        assert_eq!((haiku("2024-11-10"), haiku("2025-01-01"), haiku("2020-01-01")), (1.0, 0.8, 1.0));
        assert!(price_of("<synthetic>", None).is_none());
        assert!(price_of("gpt-unknown", None).is_none());
    }
}
