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
