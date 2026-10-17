//! core/themes.js: how the console looks and sounds. The page keeps the
//! JS (it applies a theme); the service and the command list the themes,
//! fold your own on top of the built-ins and check what they say, here.
//!
//! The built-in themes' data is src/themes.json, made from core/themes.js
//! by `node crates/xenonet-app/themes.mjs` (and checked with `--check`).
//! `xenonet` is XenoNet's own look; every other theme names only what it
//! changes, and your own themes are JSON files with "extends" naming the
//! theme they start from. Every field is optional: a theme made before a
//! field existed takes the value of the theme underneath.

use crate::jsv::{truthy, type_name};
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub const DEFAULT_THEME: &str = "xenonet";

/// The built-in themes, as core/themes.js has them.
pub fn built_ins() -> &'static Map<String, Value> {
    static T: OnceLock<Map<String, Value>> = OnceLock::new();
    T.get_or_init(|| match xenonet_base::json::parse(include_str!("themes.json")) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    })
}

/// XenoNet's own theme, which every other is folded on top of.
pub fn xenonet() -> &'static Value {
    &built_ins()[DEFAULT_THEME]
}

/// The built-ins' ids, in order.
pub fn built_in_ids() -> Vec<String> {
    built_ins().keys().cloned().collect()
}

fn is_object(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(_)))
}

/// `over` on top of `base`, field by field; arrays and values replace.
pub fn merge(base: Option<&Value>, over: Option<&Value>) -> Value {
    let mut out = match base {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    if let Some(Value::Object(o)) = over {
        for (k, v) in o {
            let b = base.and_then(|b| b.get(k));
            let merged = if is_object(Some(v)) && is_object(b) { merge(b, Some(v)) } else { v.clone() };
            out.insert(k.clone(), merged);
        }
    }
    Value::Object(out)
}

const FORMS: [&str; 6] = ["orb", "cursor", "crystal", "flame", "swarm", "jelly"];
const VOICES: [&str; 4] = ["sine", "square", "sawtooth", "triangle"];

/// The values XenoNet knows for the fields that name a style. A value it
/// doesn't know isn't an error: it's drawn as the theme underneath draws it.
pub fn styles() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("motion.ease", vec!["smooth", "stepped"]),
        ("stars.shape", vec!["point", "flake", "mote"]),
        ("grid.style", vec!["polar", "hex", "cells", "none"]),
        ("sweep.shape", vec!["beam", "aurora", "flare", "hand", "ring", "none"]),
        ("sonar.style", vec!["ring", "ripple", "ping", "tick"]),
        ("lines.style", vec!["solid", "dashed", "arc", "filament", "seam", "hairline"]),
        ("actors.form", FORMS.to_vec()),
        ("actors.human.form", FORMS.to_vec()),
        ("actors.beam.style", vec!["pulse", "stream", "bolt"]),
        ("sound.pad", VOICES.to_vec()),
        ("sound.lead", VOICES.to_vec()),
    ]
}

fn style_of(path: &str) -> Option<Vec<&'static str>> {
    styles().into_iter().find(|(p, _)| *p == path).map(|(_, v)| v)
}

// Lists that keep their length: the sky's four colors, a drift's three
// axes and the five identity rings.
const TUPLES: [&str; 6] = ["sky.night", "sky.dawn", "sky.day", "sky.dusk", "stars.drift", "identities"];

fn hex_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^#(?:[0-9a-f]{3}|[0-9a-f]{6})$").unwrap())
}

fn is_hex(v: &Value) -> bool {
    v.as_str().is_some_and(|s| hex_re().is_match(s))
}

fn six_digits(v: &str) -> String {
    if v.len() == 4 {
        let mut out = String::from("#");
        for c in v[1..].chars() {
            out.push(c);
            out.push(c);
        }
        out.to_lowercase()
    } else {
        v.to_lowercase()
    }
}

fn is_number(v: &Value) -> bool {
    v.is_number()
}

// Whether every item of a list fits the kind of item base's list holds.
fn list_fits(base: &[Value], list: &Value, path: &str) -> bool {
    let Value::Array(list) = list else { return false };
    if list.is_empty() || (TUPLES.contains(&path) && list.len() != base.len()) {
        return false;
    }
    let Some(like) = base.first() else { return false };
    if like.is_array() {
        return list.iter().all(|x| matches!(x, Value::Array(a) if !a.is_empty() && a.iter().all(is_number)));
    }
    if is_hex(like) {
        return list.iter().all(is_hex);
    }
    if like.is_number() {
        return list.iter().all(is_number);
    }
    list.iter().all(|x| type_name(x) == type_name(like))
}

// `value` if it can stand in for `base` at `path`, else None.
fn fitting(base: &Value, value: &Value, path: &str) -> Option<Value> {
    match base {
        Value::Object(_) => is_object(Some(value)).then(|| conform(base, Some(value), path)),
        Value::Array(b) => list_fits(b, value, path).then(|| Value::Array(value.as_array().unwrap().iter().map(|x| if is_hex(x) { Value::String(six_digits(x.as_str().unwrap())) } else { x.clone() }).collect())),
        Value::Number(_) => is_number(value).then(|| value.clone()),
        Value::Bool(_) => value.is_boolean().then(|| value.clone()),
        _ => {
            let s = value.as_str()?;
            if is_hex(base) {
                return is_hex(value).then(|| Value::String(six_digits(s)));
            }
            if let Some(allowed) = style_of(path) {
                return allowed.contains(&s).then(|| value.clone());
            }
            Some(value.clone())
        }
    }
}

// The fonts: a list of font names, quoted or not, which the page puts in
// style attributes as well as its stylesheet. Nothing that could end the
// declaration it's in, or start another.
const FONTS: [&str; 3] = ["fonts.display", "fonts.mono", "fonts.body"];

fn is_font_list(v: &Value) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r#"^[\p{L}\p{M}\p{N} ,'"._+&-]+$"#).unwrap());
    v.as_str().is_some_and(|s| !xenonet_base::text::js_trim(s).is_empty() && re.is_match(s))
}

/// The names every JS object has, which are never a theme's field.
pub const OBJECT_PROTOTYPE: [&str; 12] = ["constructor", "__defineGetter__", "__defineSetter__", "hasOwnProperty", "__lookupGetter__", "__lookupSetter__", "isPrototypeOf", "propertyIsEnumerable", "toString", "valueOf", "__proto__", "toLocaleString"];

/// `over` on top of `base`, as merge does, but only where a value fits: a
/// color for a color, a number for a number, a list of the same kind for a
/// list, a style XenoNet knows for a style. Anything else keeps base's, so
/// a slip in a theme file costs that one value rather than the page.
/// Fields XenoNet doesn't use pass through, except that an actor form's
/// knobs are numbers and true or false, and the fonts are lists of names.
pub fn conform(base: &Value, over: Option<&Value>, at: &str) -> Value {
    let Some(Value::Object(o)) = over else { return base.clone() };
    let mut out = match base {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    for (k, v) in o {
        if OBJECT_PROTOTYPE.contains(&k.as_str()) {
            continue;
        }
        let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
        let b = base.get(k);
        if path == "actors.params" {
            let kept = match v {
                Value::Object(m) => Value::Object(m.iter().filter(|(_, x)| x.is_number() || x.is_boolean()).map(|(k, x)| (k.clone(), x.clone())).collect()),
                _ => b.cloned().unwrap_or(Value::Null),
            };
            if b.is_some() || v.is_object() {
                out.insert(k.clone(), kept);
            } else {
                out.shift_remove(k);
            }
        } else if FONTS.contains(&path.as_str()) {
            if is_font_list(v) {
                out.insert(k.clone(), v.clone());
            }
        } else {
            match b {
                None => {
                    out.insert(k.clone(), v.clone());
                }
                Some(b) => {
                    let fit = fitting(b, v, &path).unwrap_or_else(|| b.clone());
                    out.insert(k.clone(), fit);
                }
            }
        }
    }
    Value::Object(out)
}

/// A theme with every field XenoNet draws: what it lacks, or gets wrong,
/// taken from XenoNet's own.
pub fn complete(theme: Option<&Value>) -> Value {
    conform(xenonet(), theme, "")
}

/// How many seconds apart the core's sonar waves go, or None for none: 0
/// or less turns the sonar off, and they're never under a second apart.
pub fn sonar_gap(every: Option<&Value>) -> Option<f64> {
    let n = every.and_then(Value::as_f64)?;
    (n > 0.0).then(|| n.max(1.0))
}

/// The built-in themes, and your own ({ id: theme }), as complete themes:
/// each on top of what it extends, down to xenonet. A theme of yours with
/// a built-in's id takes its place, and starts from that built-in unless
/// it extends another. Built-ins only start from built-ins, so a file of
/// yours never changes another built-in. A loop of themes extending each
/// other ends at XenoNet's own.
pub fn themes(custom: &Map<String, Value>) -> Map<String, Value> {
    let built: Vec<(String, Value)> = built_ins().iter().map(|(id, t)| (id.clone(), if id == DEFAULT_THEME { t.clone() } else { conform(xenonet(), Some(t), "") })).collect();
    let built_in = |id: &str| built.iter().find(|(i, _)| i == id).map(|(_, t)| t);
    let mut yours: Map<String, Value> = Map::new();

    fn resolve<'a>(id: &str, seen: &mut Vec<String>, custom: &Map<String, Value>, built_in: &dyn Fn(&str) -> Option<&'a Value>, yours: &mut Map<String, Value>) -> Option<Value> {
        let Some(t) = custom.get(id) else { return built_in(id).cloned() };
        if let Some(y) = yours.get(id) {
            return Some(y.clone());
        }
        if seen.iter().any(|s| s == id) {
            return None;
        }
        seen.push(id.to_string());
        let from = match t.get("extends") {
            Some(Value::String(e)) => e.clone(),
            _ if built_in(id).is_some() => id.to_string(),
            _ => DEFAULT_THEME.to_string(),
        };
        let base = if from == id { built_in(id).cloned() } else { resolve(&from, seen, custom, built_in, yours) }.unwrap_or_else(|| xenonet().clone());
        // Named for its file, unless it says or takes the name of the one
        // it replaces.
        let over = if built_in(id).is_some() || truthy(t.get("name")) {
            t.clone()
        } else {
            let mut m = Map::new();
            m.insert("name".into(), Value::String(id.to_string()));
            if let Value::Object(tm) = t {
                for (k, v) in tm {
                    m.insert(k.clone(), v.clone());
                }
            }
            Value::Object(m)
        };
        let theme = conform(&base, Some(&over), "");
        yours.insert(id.to_string(), theme.clone());
        Some(theme)
    }

    let mut ids: Vec<String> = built.iter().map(|(i, _)| i.clone()).collect();
    for k in custom.keys() {
        if !ids.contains(k) {
            ids.push(k.clone());
        }
    }
    let mut out = Map::new();
    for id in ids {
        let mine = custom.contains_key(&id);
        let mut t = match resolve(&id, &mut Vec::new(), custom, &built_in, &mut yours) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        t.insert("id".into(), Value::String(id.clone()));
        t.insert("builtIn".into(), Value::Bool(!mine));
        if mine && built_in(&id).is_some() {
            t.insert("replacesBuiltIn".into(), Value::Bool(true));
        }
        out.insert(id, Value::Object(t));
    }
    out
}

// parseInt(s, 16), as far as the hex digits go.
fn parse_hex(s: &str) -> f64 {
    let t = xenonet_base::text::js_trim_start(s);
    let (neg, t) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let digits: String = t.chars().take_while(char::is_ascii_hexdigit).collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    let n = digits.chars().fold(0.0, |n, c| n * 16.0 + f64::from(c.to_digit(16).unwrap()));
    if neg {
        -n
    } else {
        n
    }
}

// A color (#rrggbb) as linear red, green and blue, 0 to 1.
fn linear(hex: &str) -> [f64; 3] {
    let units: Vec<u16> = hex.encode_utf16().collect();
    let part = |i: usize| String::from_utf16_lossy(units.get(i..(i + 2).min(units.len())).unwrap_or(&[]));
    [1, 3, 5].map(|i| {
        let c = parse_hex(&part(i)) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// How far apart two colors are in brightness, as WCAG 2 measures
/// contrast: from 1 (the same) to 21 (black on white).
pub fn contrast(a: &str, b: &str) -> f64 {
    let luminance = |hex: &str| {
        let [r, g, bl] = linear(hex);
        0.2126 * r + 0.7152 * g + 0.0722 * bl
    };
    let (x, y) = (luminance(a), luminance(b));
    // [x, y].sort((p, q) => q - p): a NaN compares as neither.
    let (hi, lo) = if y - x > 0.0 { (y, x) } else { (x, y) };
    (hi + 0.05) / (lo + 0.05)
}

/// Math.cbrt as V8 has it (FreeBSD's s_cbrt.c), to the last bit.
pub fn js_cbrt(x: f64) -> f64 {
    const B1: u32 = 715094163;
    const B2: u32 = 696219795;
    const P0: f64 = 1.87595182427177009643;
    const P1: f64 = -1.88497979543377169875;
    const P2: f64 = 1.621429720105354466140;
    const P3: f64 = -0.758397934778766047437;
    const P4: f64 = 0.145996192886612446982;
    let bits = x.to_bits();
    let mut hx = (bits >> 32) as u32;
    let low = bits as u32;
    let sign = hx & 0x8000_0000;
    hx ^= sign;
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    let mut t: f64;
    if hx < 0x0010_0000 {
        if (hx | low) == 0 {
            return x;
        }
        t = f64::from_bits(0x4350_0000u64 << 32);
        t *= x;
        let high = (t.to_bits() >> 32) as u32;
        t = f64::from_bits(u64::from(sign | ((high & 0x7fff_ffff) / 3 + B2)) << 32);
    } else {
        t = f64::from_bits(u64::from(sign | (hx / 3 + B1)) << 32);
    }
    let r = (t * t) * (t / x);
    t *= (P0 + r * (P1 + r * P2)) + ((r * r) * r) * (P3 + r * P4);
    t = f64::from_bits((t.to_bits().wrapping_add(0x8000_0000)) & 0xffff_ffff_c000_0000);
    let s = t * t;
    let mut r = x / s;
    let w = t + t;
    r = (r - t) / (w + r);
    t + t * r
}

/// Math.hypot as V8 has it: scaled by the largest, summed with Kahan's
/// compensation.
pub fn js_hypot(xs: &[f64]) -> f64 {
    let abs: Vec<f64> = xs.iter().map(|x| x.abs()).collect();
    let mut max = 0.0f64;
    let mut nan = false;
    for &v in &abs {
        if v.is_nan() {
            nan = true;
        } else if v > max {
            max = v;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for &v in &abs {
        let n = v / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// How different two colors look: the distance between them in CIELAB
/// (ΔE 1976). About 2 is just noticeable; past 20 they're plainly
/// different colors.
pub fn color_distance(a: &str, b: &str) -> f64 {
    let lab = |hex: &str| {
        let [r, g, bl] = linear(hex);
        let f = |t: f64| if t > 216.0 / 24389.0 { js_cbrt(t) } else { (841.0 / 108.0) * t + 4.0 / 29.0 };
        let x = f((0.4124 * r + 0.3576 * g + 0.1805 * bl) / 0.95047);
        let y = f(0.2126 * r + 0.7152 * g + 0.0722 * bl);
        let z = f((0.0193 * r + 0.1192 * g + 0.9505 * bl) / 1.08883);
        [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
    };
    let (p, q) = (lab(a), lab(b));
    js_hypot(&[p[0] - q[0], p[1] - q[1], p[2] - q[2]])
}

const RING_SPARES: [&str; 9] = ["#c3f584", "#f5a3ff", "#9fd0ff", "#ffc29e", "#d6c2ff", "#a8ffd1", "#ffe0f0", "#e2f5a8", "#ffffff"];
const MEANINGFUL: [&str; 11] = ["project", "satellite", "claude", "codex", "gold", "pressure", "attention", "danger", "collision", "stuck", "lineage"];
const CLEAR_OF_MEANING: f64 = 20.0;
const CLEAR_OF_RINGS: f64 = 15.0;

/// The rings named identities' projects stand on, in colors that mean
/// nothing else on the map: the theme's five, each kept unless the theme
/// has a meaningful color near it, when the first spare clear of them and
/// of the other rings takes its place, so a name keeps its place from
/// theme to theme.
pub fn ring_colors(theme: &Value) -> Vec<String> {
    let rings: Vec<String> = xenonet()["identities"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
    let colors = theme.get("colors");
    let meaning: Vec<String> = MEANINGFUL.iter().filter_map(|k| colors.and_then(|c| c.get(*k))).filter(|v| is_hex(v)).map(|v| six_digits(v.as_str().unwrap())).collect();
    let own: Vec<String> = match theme.get("identities") {
        Some(Value::Array(a)) if a.len() == rings.len() && a.iter().all(is_hex) => a.iter().map(|v| six_digits(v.as_str().unwrap())).collect(),
        _ => rings.clone(),
    };
    let mut out: Vec<String> = Vec::new();
    for ring in own {
        let clear = |c: &str, out: &[String]| meaning.iter().all(|m| color_distance(c, m) >= CLEAR_OF_MEANING) && out.iter().all(|r| color_distance(c, r) >= CLEAR_OF_RINGS);
        let pick = if clear(&ring, &out) { ring.clone() } else { RING_SPARES.iter().find(|c| !out.iter().any(|o| o == *c) && clear(c, &out)).map_or(ring.clone(), |c| c.to_string()) };
        out.push(pick);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_built_ins_and_your_own() {
        assert_eq!(built_in_ids()[0], "xenonet");
        let custom: Map<String, Value> = json!({ "ember": { "name": "Ember", "extends": "helios", "colors": { "accent": "#FF0000" } }, "ash": { "extends": "ember", "glow": { "bloom": 2 } }, "loop-a": { "extends": "loop-b" }, "loop-b": { "extends": "loop-a" } }).as_object().unwrap().clone();
        let all = themes(&custom);
        assert_eq!(all["ember"]["colors"]["accent"], "#ff0000");
        assert_eq!(all["ember"]["colors"]["claude"], all["helios"]["colors"]["claude"]);
        assert_eq!(all["ash"]["name"], "ash");
        assert_eq!(all["ash"]["glow"]["bloom"], 2);
        assert!(all["loop-a"]["colors"].is_object());
        assert_eq!(all["ember"]["builtIn"], false);
    }

    #[test]
    fn a_slip_costs_one_value() {
        let t = complete(Some(&json!({ "grid": { "opacity": "lots", "style": "spiral" }, "colors": { "accent": "#ABC" }, "constructor": 1 })));
        assert_eq!(t["grid"]["opacity"], xenonet()["grid"]["opacity"]);
        assert_eq!(t["grid"]["style"], "polar");
        assert_eq!(t["colors"]["accent"], "#aabbcc");
        assert!(t.get("constructor").is_none());
        assert_eq!(merge(Some(&json!({ "a": { "x": 1, "y": 2 }, "b": [1, 2], "c": 1 })), Some(&json!({ "a": { "y": 3 }, "b": [9] }))), json!({ "a": { "x": 1, "y": 3 }, "b": [9], "c": 1 }));
    }

    #[test]
    fn contrast_and_distance() {
        assert_eq!(contrast("#000000", "#ffffff"), 21.0);
        assert_eq!(contrast("#ffffff", "#000000"), 21.0);
        assert_eq!((contrast("#767676", "#ffffff") * 100.0).round() / 100.0, 4.54);
        assert_eq!(color_distance("#000000", "#ffffff").round(), 100.0);
        assert_eq!(js_cbrt(27.0), 3.0);
        assert_eq!(js_hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(sonar_gap(Some(&json!(0.2))), Some(1.0));
        assert_eq!(sonar_gap(Some(&json!(0))), None);
    }
}
