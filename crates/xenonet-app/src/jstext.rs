//! Text as JS's built-ins make it, where XenoNet relies on their exact
//! output: encodeURIComponent, URLSearchParams' form encoding, a string's
//! slice in UTF-16 units, and a URL's host.

/// encodeURIComponent: everything but A-Z a-z 0-9 - _ . ! ~ * ' ( ) as
/// %XX of its UTF-8 bytes.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// encodeURI: as encodeURIComponent, but leaving a URL's own punctuation.
pub fn encode_uri(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'();/?:@&=+$,#".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// pathToFileURL(p).href for a POSIX path: %, \\, line breaks and tabs
/// encoded first, then what a URL's path encodes (spaces, quotes, #, ?,
/// <, >, `, {, } and everything past ASCII).
pub fn path_to_file_url(p: &str) -> String {
    let mut out = String::from("file://");
    for &b in p.as_bytes() {
        match b {
            b'%' | b'\\' | b'\n' | b'\r' | b'\t' | b' ' | b'"' | b'#' | b'<' | b'>' | b'?' | b'`' | b'{' | b'}' => out.push_str(&format!("%{b:02X}")),
            0..=0x1f | 0x7f..=0xff => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    out
}

/// One name or value as URLSearchParams writes it: space as +, and
/// everything but A-Z a-z 0-9 * - . _ as %XX.
pub fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"*-._".contains(&b) {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// new URLSearchParams(pairs).toString().
pub fn search_params(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v))).collect::<Vec<_>>().join("&")
}

/// How many UTF-16 units a string is, as its length in JS.
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// s.slice(0, n) in UTF-16 units. A cut through a character's two units
/// leaves U+FFFD where JS keeps the half, which is how that half prints.
pub fn slice16(s: &str, n: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.len_utf16();
        if used + w > n {
            if used < n {
                out.push('\u{fffd}');
            }
            break;
        }
        out.push(c);
        used += w;
    }
    out
}

/// new URL(u).host: the host, lowercased, with its port unless it's the
/// scheme's own. None when it isn't a URL with a host.
pub fn url_host(u: &str) -> Option<String> {
    let (scheme, rest) = u.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let (host, port) = if authority.starts_with('[') {
        let end = authority.find(']')?;
        (&authority[..=end], authority[end + 1..].strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let default = match scheme.as_str() {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        "ftp" => Some("21"),
        _ => None,
    };
    Some(match port {
        Some(p) if !p.is_empty() => {
            let p = match p.trim_start_matches('0') {
                "" => "0",
                t => t,
            };
            if Some(p) == default {
                host
            } else {
                format!("{host}:{p}")
            }
        }
        _ => host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_js_writes_it() {
        assert_eq!(encode_uri_component("C:\\Users\\me a#é"), "C%3A%5CUsers%5Cme%20a%23%C3%A9");
        assert_eq!(encode_uri_component("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(search_params(&[("folder", "C:\\a b"), ("q", "fix & ship\n*~")]), "folder=C%3A%5Ca+b&q=fix+%26+ship%0A*%7E");
        assert_eq!(slice16("a😀b", 2), "a\u{fffd}");
        assert_eq!(slice16("a😀b", 3), "a😀");
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(url_host("https://OpenRouter.ai/api/x").as_deref(), Some("openrouter.ai"));
        assert_eq!(url_host("http://127.0.0.1:47840/x").as_deref(), Some("127.0.0.1:47840"));
        assert_eq!(url_host("https://h:443/").as_deref(), Some("h"));
        assert_eq!(url_host("not a url"), None);
    }
}

// Where an ASCII character falls in the root collation (CLDR's, which
// ICU and so localeCompare use): white space, punctuation and symbols in
// their own order, then digits, then letters without case; its case
// second.
// Latin letters with a mark, as the letter they're made on (NFD's first).
const MARKED: &[(char, char)] = &[
    ('À', 'A'),
    ('Á', 'A'),
    ('Â', 'A'),
    ('Ã', 'A'),
    ('Ä', 'A'),
    ('Å', 'A'),
    ('Ç', 'C'),
    ('È', 'E'),
    ('É', 'E'),
    ('Ê', 'E'),
    ('Ë', 'E'),
    ('Ì', 'I'),
    ('Í', 'I'),
    ('Î', 'I'),
    ('Ï', 'I'),
    ('Ñ', 'N'),
    ('Ò', 'O'),
    ('Ó', 'O'),
    ('Ô', 'O'),
    ('Õ', 'O'),
    ('Ö', 'O'),
    ('Ù', 'U'),
    ('Ú', 'U'),
    ('Û', 'U'),
    ('Ü', 'U'),
    ('Ý', 'Y'),
    ('à', 'a'),
    ('á', 'a'),
    ('â', 'a'),
    ('ã', 'a'),
    ('ä', 'a'),
    ('å', 'a'),
    ('ç', 'c'),
    ('è', 'e'),
    ('é', 'e'),
    ('ê', 'e'),
    ('ë', 'e'),
    ('ì', 'i'),
    ('í', 'i'),
    ('î', 'i'),
    ('ï', 'i'),
    ('ñ', 'n'),
    ('ò', 'o'),
    ('ó', 'o'),
    ('ô', 'o'),
    ('õ', 'o'),
    ('ö', 'o'),
    ('ù', 'u'),
    ('ú', 'u'),
    ('û', 'u'),
    ('ü', 'u'),
    ('ý', 'y'),
    ('ÿ', 'y'),
    ('Ā', 'A'),
    ('ā', 'a'),
    ('Ă', 'A'),
    ('ă', 'a'),
    ('Ą', 'A'),
    ('ą', 'a'),
    ('Ć', 'C'),
    ('ć', 'c'),
    ('Ĉ', 'C'),
    ('ĉ', 'c'),
    ('Ċ', 'C'),
    ('ċ', 'c'),
    ('Č', 'C'),
    ('č', 'c'),
    ('Ď', 'D'),
    ('ď', 'd'),
    ('Ē', 'E'),
    ('ē', 'e'),
    ('Ĕ', 'E'),
    ('ĕ', 'e'),
    ('Ė', 'E'),
    ('ė', 'e'),
    ('Ę', 'E'),
    ('ę', 'e'),
    ('Ě', 'E'),
    ('ě', 'e'),
    ('Ĝ', 'G'),
    ('ĝ', 'g'),
    ('Ğ', 'G'),
    ('ğ', 'g'),
    ('Ġ', 'G'),
    ('ġ', 'g'),
    ('Ģ', 'G'),
    ('ģ', 'g'),
    ('Ĥ', 'H'),
    ('ĥ', 'h'),
    ('Ĩ', 'I'),
    ('ĩ', 'i'),
    ('Ī', 'I'),
    ('ī', 'i'),
    ('Ĭ', 'I'),
    ('ĭ', 'i'),
    ('Į', 'I'),
    ('į', 'i'),
    ('İ', 'I'),
    ('Ĵ', 'J'),
    ('ĵ', 'j'),
    ('Ķ', 'K'),
    ('ķ', 'k'),
    ('Ĺ', 'L'),
    ('ĺ', 'l'),
    ('Ļ', 'L'),
    ('ļ', 'l'),
    ('Ľ', 'L'),
    ('ľ', 'l'),
    ('Ń', 'N'),
    ('ń', 'n'),
    ('Ņ', 'N'),
    ('ņ', 'n'),
    ('Ň', 'N'),
    ('ň', 'n'),
    ('Ō', 'O'),
    ('ō', 'o'),
    ('Ŏ', 'O'),
    ('ŏ', 'o'),
    ('Ő', 'O'),
    ('ő', 'o'),
    ('Ŕ', 'R'),
    ('ŕ', 'r'),
    ('Ŗ', 'R'),
    ('ŗ', 'r'),
    ('Ř', 'R'),
    ('ř', 'r'),
    ('Ś', 'S'),
    ('ś', 's'),
    ('Ŝ', 'S'),
    ('ŝ', 's'),
    ('Ş', 'S'),
    ('ş', 's'),
    ('Š', 'S'),
    ('š', 's'),
    ('Ţ', 'T'),
    ('ţ', 't'),
    ('Ť', 'T'),
    ('ť', 't'),
    ('Ũ', 'U'),
    ('ũ', 'u'),
    ('Ū', 'U'),
    ('ū', 'u'),
    ('Ŭ', 'U'),
    ('ŭ', 'u'),
    ('Ů', 'U'),
    ('ů', 'u'),
    ('Ű', 'U'),
    ('ű', 'u'),
    ('Ų', 'U'),
    ('ų', 'u'),
    ('Ŵ', 'W'),
    ('ŵ', 'w'),
    ('Ŷ', 'Y'),
    ('ŷ', 'y'),
    ('Ÿ', 'Y'),
    ('Ź', 'Z'),
    ('ź', 'z'),
    ('Ż', 'Z'),
    ('ż', 'z'),
    ('Ž', 'Z'),
    ('ž', 'z'),
];

fn collation(c: char) -> (u32, u32, u32, u32) {
    const ORDER: &str = "\t\n\x0b\x0c\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789";
    let (base, marked) = match MARKED.iter().find(|(m, _)| *m == c) {
        Some((_, b)) => (*b, c as u32),
        None => (c, 0),
    };
    if base.is_ascii_alphabetic() {
        let upper = base.is_ascii_uppercase() || (marked != 0 && c.is_uppercase());
        return (1, 100 + (base.to_ascii_lowercase() as u32 - 'a' as u32), marked, u32::from(upper));
    }
    match ORDER.find(c) {
        Some(i) => (1, i as u32, 0, 0),
        None => (2, c as u32, 0, 0),
    }
}

/// a.localeCompare(b), as ICU's root collation orders what XenoNet
/// compares this way (ids, names): letters without case first, then case,
/// lowercase first; punctuation before digits before letters. Past ASCII,
/// characters go by their code points.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let ka: Vec<(u32, u32, u32, u32)> = a.chars().map(collation).collect();
    let kb: Vec<(u32, u32, u32, u32)> = b.chars().map(collation).collect();
    let primary = |k: &[(u32, u32, u32, u32)]| k.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>();
    primary(&ka).cmp(&primary(&kb)).then_with(|| ka.iter().map(|x| x.2).cmp(kb.iter().map(|x| x.2))).then_with(|| ka.iter().map(|x| x.3).cmp(kb.iter().map(|x| x.3)))
}
