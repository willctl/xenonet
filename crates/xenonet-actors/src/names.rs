//! Actors' names and slugs (core/actors/names.js;
//! docs/design/actors/interface.md, section 2, "Registering"). A name is
//! what agents and the card see, so it can't read as you or as XenoNet, nor
//! as an app or product the actor isn't; names are compared without case,
//! spaces, punctuation or accents, after folding look-alike forms
//! (full-width letters, ligatures) and look-alike letters (a Cyrillic letter
//! shaped like C, a Greek omicron, a Latin alpha, the digit 0) into the
//! Latin ones they pass for. A name keeps to one script, as Unicode's highly
//! restrictive profile has it (UTS #39), and holds nothing that draws as
//! nothing. A slug is the actor's id (actor:<slug>): some are XenoNet's own
//! words or its routes', and a product's slug is that product's alone.

use crate::detail::looks_like_key;
use crate::tokens::hex;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;
use xenonet_base::ids::is_actor_slug;
use xenonet_base::log::js_slice;
use xenonet_base::text::js_trim;
use xenonet_engine::js::{template, truthy};
use xenonet_engine::products::{product_of, product_of_slug};

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

fn letters_or_digits() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"[^\p{L}\p{N}]"))
}

/// A name as names are compared: look-alike forms folded (NFKC), without
/// case, and only its letters and digits.
pub fn name_key(s: &str) -> String {
    let folded: String = s.nfkc().collect::<String>().to_lowercase();
    letters_or_digits().replace_all(&folded, "").into_owned()
}

// Letters that pass for Latin ones, by the letter they pass for, as code
// points: from Cyrillic, Greek and Armenian, the Latin alphabet's own small
// capitals and IPA letters, Cherokee (whose small letters are drawn as
// small capitals of the same shapes), and the digit 0. "I" stands for what
// reads as a capital I or a small l alike, which is read both ways.
const PASSES_FOR: &[(&str, &[u32], &[u32])] = &[
    ("a", &[0x0410, 0x0430, 0x0391, 0x03b1, 0x0251, 0x1d00], &[0x13aa]),
    ("b", &[0x0412, 0x0432, 0x042c, 0x044c, 0x0392, 0x0299], &[0x13f4]),
    ("c", &[0x0421, 0x0441, 0x03f9, 0x03f2, 0x1d04], &[0x13df]),
    ("d", &[0x0501, 0x1d05], &[0x13a0]),
    ("e", &[0x0415, 0x0435, 0x0395, 0x1d07], &[0x13ac]),
    ("g", &[0x0581, 0x0261, 0x0262], &[0x13c0]),
    ("h", &[0x041d, 0x043d, 0x04ba, 0x04bb, 0x0397, 0x0570, 0x029c], &[0x13bb]),
    ("i", &[0x0456, 0xa647, 0x03b9, 0x0131, 0x0269], &[0x13a5]),
    ("j", &[0x0408, 0x0458, 0x03f3, 0x0237, 0x1d0a], &[0x13ab]),
    ("k", &[0x041a, 0x043a, 0x039a, 0x03ba, 0x0138, 0x1d0b], &[0x13e6]),
    ("l", &[0x053c, 0x029f], &[0x13de]),
    ("m", &[0x041c, 0x043c, 0x039c, 0x1d0d], &[0x13b7]),
    ("n", &[0x043f, 0x039d, 0x03b7, 0x0578, 0x0274], &[]),
    ("o", &[0x041e, 0x043e, 0x039f, 0x03bf, 0x0555, 0x0585, 0x1d0f, 0x30], &[]),
    ("p", &[0x0420, 0x0440, 0x03a1, 0x03c1, 0x1d18], &[0x13e2]),
    ("q", &[0x051a, 0x051b, 0x0566], &[]),
    ("r", &[0x0433, 0x0280], &[0x13a1]),
    ("s", &[0x0405, 0x0455, 0x054f, 0xa731], &[0x13da]),
    ("t", &[0x0422, 0x0442, 0x03a4, 0x03c4, 0x1d1b], &[0x13a2]),
    ("u", &[0x03c5, 0x054d, 0x057d, 0x028b, 0x1d1c], &[]),
    ("v", &[0x0474, 0x0475, 0x03bd, 0x1d20], &[0x13d9]),
    ("w", &[0x051c, 0x051d, 0x0461, 0x03c9, 0x0561, 0x1d21], &[0x13b3, 0x13d4]),
    ("x", &[0x0425, 0x0445, 0x03a7, 0x03c7], &[]),
    ("y", &[0x0423, 0x0443, 0x04ae, 0x04af, 0x03a5, 0x03b3, 0x03d2, 0x028f], &[0x13a9]),
    ("z", &[0x0396, 0x1d22], &[0x13c3]),
    ("I", &[0x49, 0x31, 0x7c, 0x0406, 0x04c0, 0x04cf, 0x0399, 0x026a, 0x01c0], &[]),
];

fn lookalikes() -> &'static HashMap<char, &'static str> {
    static M: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    M.get_or_init(|| {
        let mut m = HashMap::new();
        for (to, from, cherokee) in PASSES_FOR {
            let mut cps: Vec<u32> = from.to_vec();
            // A Cherokee capital, and the small letter drawn as its small
            // capital.
            for &u in *cherokee {
                cps.push(u);
                let c = char::from_u32(u).unwrap();
                cps.push(c.to_lowercase().next().map_or(u, u32::from));
            }
            for cp in cps {
                m.insert(char::from_u32(cp).unwrap(), *to);
            }
        }
        m
    })
}

fn marks() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"\p{M}"))
}

/// What a name reads as, as keys like `name_key`'s: look-alike forms and
/// accents folded, look-alike letters made the Latin ones they pass for,
/// and what reads as I or l taken first as an i, then as an l ("CIaude").
pub fn reads_as(s: &str) -> Vec<String> {
    let decomposed: String = s.nfkc().collect::<String>().nfd().collect();
    let bare = marks().replace_all(&decomposed, "");
    let map = lookalikes();
    let letters: Vec<String> = bare.chars().map(|c| map.get(&c).map_or_else(|| c.to_string(), |s| s.to_string())).collect();
    let key = |ell: bool| {
        name_key(
            &letters
                .iter()
                .map(|c| {
                    if c == "I" {
                        if ell {
                            "l"
                        } else {
                            "i"
                        }
                    } else {
                        c.as_str()
                    }
                })
                .collect::<String>(),
        )
    };
    let (a, b) = (key(false), key(true));
    if a == b {
        vec![a]
    } else {
        vec![a, b]
    }
}

// The scripts whose letters a name may hold, one at a time; Han with the
// scripts Japanese, Chinese and Korean write with it, and Latin with any
// of those, as UTS #39's highly restrictive profile allows. A letter of a
// script not listed counts as a script of its own.
const SCRIPTS: [&str; 48] = [
    "Latin",
    "Greek",
    "Cyrillic",
    "Armenian",
    "Georgian",
    "Cherokee",
    "Coptic",
    "Glagolitic",
    "Lisu",
    "Hebrew",
    "Arabic",
    "Syriac",
    "Thaana",
    "Nko",
    "Devanagari",
    "Bengali",
    "Gurmukhi",
    "Gujarati",
    "Oriya",
    "Tamil",
    "Telugu",
    "Kannada",
    "Malayalam",
    "Sinhala",
    "Thai",
    "Lao",
    "Tibetan",
    "Myanmar",
    "Khmer",
    "Mongolian",
    "Ethiopic",
    "Canadian_Aboriginal",
    "Tifinagh",
    "Vai",
    "Yi",
    "Javanese",
    "Balinese",
    "Sundanese",
    "Adlam",
    "Osage",
    "Deseret",
    "Runic",
    "Ogham",
    "Hangul",
    "Hiragana",
    "Katakana",
    "Bopomofo",
    "Han",
];
const TOGETHER: [&[&str]; 3] = [&["Latin", "Han", "Hiragana", "Katakana"], &["Latin", "Han", "Bopomofo"], &["Latin", "Han", "Hangul"]];

fn scripts() -> &'static Vec<(&'static str, Regex)> {
    static S: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    S.get_or_init(|| SCRIPTS.iter().map(|n| (*n, re(&format!(r"^\p{{Script={n}}}$")))).collect())
}

fn letter() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"^\p{L}$"))
}

fn one_script(name: &str) -> bool {
    let mut found: Vec<&str> = Vec::new();
    for c in name.chars() {
        let s = c.to_string();
        if !letter().is_match(&s) {
            continue;
        }
        let script = scripts().iter().find(|(_, r)| r.is_match(&s)).map_or("another", |(n, _)| *n);
        if !found.contains(&script) {
            found.push(script);
        }
    }
    found.len() <= 1 || TOGETHER.iter().any(|set| found.iter().all(|s| set.contains(s)))
}

// Characters that draw as nothing: those Unicode says to ignore when they
// can't be shown (Hangul fillers among them, which count as letters), and
// the blank Braille pattern.
fn invisible() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| re(r"[\p{Default_Ignorable_Code_Point}\x{2800}]"))
}

// Names that read as you or as XenoNet.
const YOURS: [&str; 8] = ["you", "me", "user", "youruser", "owner", "xenonet", "system", "admin"];
// Apps no actor is, by their keys.
const APPS: [(&str, &str); 4] = [("claude", "Claude"), ("claudecode", "Claude Code"), ("codex", "Codex"), ("chatgpt", "ChatGPT")];
// Names a product goes by, which only its own actor may take.
const PRODUCT_NAMES: [(&str, &str); 5] = [("hermes", "hermes-agent"), ("hermesagent", "hermes-agent"), ("grokbot", "grok-bot"), ("muse", "muse"), ("dots", "dots")];
// The makers of those apps and products, and XenoNet's: Anysphere (Cursor)
// builds and runs Grok Bot for SpaceXAI.
const MAKERS: [(&str, &str); 9] = [
    ("anthropic", "Anthropic"),
    ("openai", "OpenAI"),
    ("nousresearch", "Nous Research"),
    ("spacexai", "SpaceXAI"),
    ("xai", "xAI"),
    ("anysphere", "Anysphere"),
    ("cursor", "Cursor"),
    ("meta", "Meta"),
    ("willctl", "willctl"),
];

fn lookup<'a>(table: &'a [(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

// Characters that break a line or reorder text, which a name can't hold.
fn not_one_line(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

fn product_name(id: &str) -> &'static str {
    product_of(id).map_or("", |p| p.name)
}

/// A name as XenoNet takes it, for an actor that is `product` (an id, or
/// None for one of your own): Ok(name), or Err(reason). `raw` is None when
/// what was given isn't text.
pub fn check_name(raw: Option<&str>, product: Option<&str>) -> Result<String, String> {
    let Some(raw) = raw else { return Err("a name is text".into()) };
    let name = js_trim(raw);
    if name.chars().any(not_one_line) {
        return Err("a name is one line of plain text".into());
    }
    // Agents read a name quoted ("Wren" (actor, …)), so nothing in it may
    // close the quote or pass for one.
    if name.chars().any(|c| matches!(c, '"' | '`' | '\\' | '\u{201c}' | '\u{201d}')) {
        return Err("a name can't hold quotation marks or backslashes, since sessions read it quoted".into());
    }
    if name.is_empty() || name.chars().count() > 64 {
        return Err("a name is 1 to 64 characters".into());
    }
    if invisible().is_match(name) {
        return Err("a name can't hold characters that draw as nothing".into());
    }
    if name_key(name).is_empty() {
        return Err("a name has letters or digits in it".into());
    }
    let keys = reads_as(name);
    if keys.iter().any(|k| YOURS.contains(&k.as_str())) {
        return Err("that name reads as you or as XenoNet, which no actor may".into());
    }
    if let Some(app) = keys.iter().find_map(|k| lookup(&APPS, k)) {
        return Err(format!("that name reads as {app}, which an actor isn't"));
    }
    if let Some(owner) = keys.iter().filter_map(|k| lookup(&PRODUCT_NAMES, k)).find(|p| Some(*p) != product) {
        let n = product_name(owner);
        return Err(format!("that name is {n}'s, and this actor isn't {n}"));
    }
    if !one_script(name) {
        return Err("a name keeps to one script, so that no letter in it passes for another's".into());
    }
    if looks_like_key(name) {
        return Err("that name looks like it holds a key or a token".into());
    }
    Ok(name.to_string())
}

/// Why a product of an actor's own can't have this id or vendor (`what`),
/// or None when it can. They're the actor's own word, and show as such,
/// but none may read as you or XenoNet, as an app or product XenoNet knows,
/// or as one of their makers, compared as names are.
pub fn product_word_refusal(text: &str, what: &str) -> Option<String> {
    let keys = reads_as(text);
    if keys.iter().any(|k| YOURS.contains(&k.as_str())) {
        return Some(format!("the product's {what} reads as you or as XenoNet"));
    }
    if let Some(app) = keys.iter().find_map(|k| lookup(&APPS, k)) {
        return Some(format!("the product's {what} reads as {app}, which this actor isn't"));
    }
    if let Some(p) = keys.iter().find_map(|k| lookup(&PRODUCT_NAMES, k)) {
        return Some(format!("the product's {what} reads as {}, which this actor isn't", product_name(p)));
    }
    if let Some(maker) = keys.iter().find_map(|k| lookup(&MAKERS, k)) {
        return Some(format!("the product's {what} reads as {maker}, who doesn't make this actor"));
    }
    None
}

/// Slugs that are XenoNet's own words, or its routes' (/api/actors/me…): a
/// route is matched before /api/actors/<slug>, so an actor with its word
/// for a slug couldn't be read there.
pub const RESERVED_SLUGS: [&str; 21] = [
    "you",
    "me",
    "user",
    "owner",
    "xenonet",
    "system",
    "admin",
    "root",
    "claude",
    "codex",
    "chatgpt",
    "anthropic",
    "openai",
    "events",
    "requests",
    "register",
    "heartbeat",
    "targets",
    "observers",
    "tokens",
    "gateway",
];

/// A slug made from a name: lowercase, accents dropped, other characters
/// made dashes (one for a run of them, none at the ends), cut to 32.
pub fn slug_from(name: &str) -> String {
    let lower: String = name.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>().to_lowercase();
    let mut dashed = String::with_capacity(lower.len());
    let mut run = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            dashed.push(c);
            run = false;
        } else if !run {
            dashed.push('-');
            run = true;
        }
    }
    let cut = js_slice(dashed.trim_start_matches('-'), 32).trim_end_matches('-');
    if cut.is_empty() {
        "actor".into()
    } else {
        cut.to_string()
    }
}

/// Why an actor that is `product` can't have this slug, or None when it
/// can. `slug` is None when what was given isn't text.
pub fn slug_refusal(slug: Option<&str>, product: Option<&str>) -> Option<String> {
    let Some(slug) = slug.filter(|s| is_actor_slug(s)) else {
        return Some("a slug is 1 to 32 lowercase letters, digits and dashes, starting with a letter or a digit".into());
    };
    if RESERVED_SLUGS.contains(&slug) {
        return Some(format!("the slug {slug} is XenoNet's own"));
    }
    let owner = product_of_slug(slug);
    if let Some(o) = owner {
        if Some(o) != product {
            return Some(format!("the slug {slug} is {}'s", product_name(o)));
        }
    }
    if let Some(p) = product.and_then(product_of) {
        if owner != Some(p.id) {
            let profile = if p.id == "hermes-agent" { ", or hermes-<profile> for a profile" } else { "" };
            return Some(format!("{}'s actor has a slug of its own: {}{profile}", p.name, p.slug));
        }
    }
    None
}

/// A free slug like `base`, for a 409 taken: wren-2, wren-3…, cut to fit.
pub fn suggest_slug<E>(base: &str, product: Option<&str>, taken: &mut dyn FnMut(&str) -> Result<bool, E>) -> Result<Option<String>, E> {
    for n in 2..1000 {
        let tail = format!("-{n}");
        let s = format!("{}{tail}", js_slice(base, 32 - tail.len()).trim_end_matches('-'));
        if slug_refusal(Some(&s), product).is_none() && !taken(&s)? {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

/// The actor of a Hermes profile: hermes for the default one, else
/// hermes-<profile> when the profile's name is a slug as it stands (work),
/// and otherwise, or when it's cut to 32, with four hex digits of its name
/// added (Work, my_work, 工作, a long name). Names that differ only in
/// case or punctuation, or that have no letters of a-z, would otherwise
/// share one actor. Hermes's observer and a Hermes plugin registering later
/// both take it, so they name the same actor.
pub fn hermes_slug(profile: Option<&Value>) -> String {
    if !truthy(profile) || profile.and_then(Value::as_str) == Some("default") {
        return "hermes".into();
    }
    let text = template(profile);
    let slug = slug_from(&text);
    let full = format!("hermes-{slug}");
    if full.len() <= 32 && profile.and_then(Value::as_str) == Some(slug.as_str()) {
        return full;
    }
    let tag = &hex(&Sha256::digest(text.as_bytes()))[..4];
    format!("{}-{tag}", js_slice(&full, 27).trim_end_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(check_name(Some("  Wren  "), None), Ok("Wren".into()));
        assert!(check_name(Some("ｘｅｎｏｎｅｔ"), None).unwrap_err().contains("reads as you"));
        assert!(check_name(Some("\u{0421}laude"), None).unwrap_err().contains("reads as Claude,"));
        assert!(check_name(Some("CIaude"), None).unwrap_err().contains("reads as Claude,"));
        assert!(check_name(Some("Wren\u{3164}"), None).unwrap_err().contains("draw as nothing"));
        assert!(check_name(Some("Wr\u{0435}n"), None).unwrap_err().contains("one script"));
        assert_eq!(check_name(Some("\u{65e5}\u{672c}\u{30dc}\u{30c3}\u{30c8}"), None), Ok("\u{65e5}\u{672c}\u{30dc}\u{30c3}\u{30c8}".into()));
        assert_eq!(reads_as("Ivy"), vec!["ivy".to_string(), "lvy".to_string()]);
        assert_eq!(name_key("Your User!"), "youruser");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug_from("Café Bot!"), "cafe-bot");
        assert_eq!(slug_from("  --Docs   builder 2--  "), "docs-builder-2");
        assert_eq!(slug_from("日本語"), "actor");
        assert_eq!(hermes_slug(Some(&Value::from("work"))), "hermes-work");
        assert!(slug_refusal(Some("wren"), Some("hermes-agent")).unwrap().contains("hermes-<profile>"));
    }
}
