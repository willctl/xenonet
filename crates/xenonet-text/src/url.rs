//! Text as URLs carry it: a component, a form's pairs and a file's path
//! percent-encoded byte for byte, and a URL's host, as the URLs XenoNet
//! builds and compares have them.

use std::fmt::Write as _;

/// A URL's component: everything but A-Z a-z 0-9 - _ . ! ~ * ' ( ) as
/// %XX of its UTF-8 bytes.
pub fn component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A POSIX path's file: URL: %, \\, line breaks and tabs encoded first,
/// then what a URL's path encodes (spaces, quotes, #, ?, <, >, `, {, }
/// and everything past ASCII).
pub fn file_url(p: &str) -> String {
    let mut out = String::from("file://");
    for &b in p.as_bytes() {
        match b {
            b'%' | b'\\' | b'\n' | b'\r' | b'\t' | b' ' | b'"' | b'#' | b'<' | b'>' | b'?' | b'`' | b'{' | b'}' | 0..=0x1f | 0x7f..=0xff => {
                let _ = write!(out, "%{b:02X}");
            }
            _ => out.push(b as char),
        }
    }
    out
}

/// One name or value of a form's query: space as +, and everything but
/// A-Z a-z 0-9 * - . _ as %XX.
pub fn form_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"*-._".contains(&b) {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A form's query: each name=value, joined by &.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", form_value(k), form_value(v))).collect::<Vec<_>>().join("&")
}

/// A URL's host, lowercased, with its port unless it's the scheme's own.
/// None when it isn't a URL with a host.
pub fn host(u: &str) -> Option<String> {
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
    fn encoded_as_urls_carry_it() {
        assert_eq!(component("C:\\Users\\me a#é"), "C%3A%5CUsers%5Cme%20a%23%C3%A9");
        assert_eq!(component("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(form(&[("folder", "C:\\a b"), ("q", "fix & ship\n*~")]), "folder=C%3A%5Ca+b&q=fix+%26+ship%0A*%7E");
        assert_eq!(host("https://OpenRouter.ai/api/x").as_deref(), Some("openrouter.ai"));
        assert_eq!(host("http://127.0.0.1:47840/x").as_deref(), Some("127.0.0.1:47840"));
        assert_eq!(host("https://h:443/").as_deref(), Some("h"));
        assert_eq!(host("not a url"), None);
    }
}
