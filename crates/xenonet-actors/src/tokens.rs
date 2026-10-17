//! Actors' tokens (docs/design/actors/interface.md, section 2, "Tokens";
//! core/actors/tokens.js): 32 random bytes written as
//! xenonet_actor_<64 hex digits>. The prefix makes a leaked token easy to
//! recognize, by people and by secret scanners, and hex never holds an
//! underscore, so a double-click still selects the whole token. XenoNet
//! keeps only the SHA-256: a token has 256 bits of randomness, so a fast
//! hash is enough, and XenoNet finds an actor by the hash, so no comparison
//! leaks timing. A token is shown once and travels only as Authorization:
//! Bearer, never in a URL.
//!
//! A gateway token (the actor gateway's) is xenonet_gateway_<64 hex
//! digits>: a credential apart from the actor's own token, which works only
//! at the gateway, so a token that leaks from this machine can't be used
//! from the internet, and one made for the gateway can't stand in for the
//! process checks here.

use sha2::{Digest, Sha256};
use xenonet_base::text::is_js_space;

pub const TOKEN_PREFIX: &str = "xenonet_actor_";
pub const GATEWAY_PREFIX: &str = "xenonet_gateway_";
/// Either kind anywhere in a file, as `git grep -E` reads it.
pub const TOKEN_GREP: &str = "xenonet_(actor|gateway)_[0-9a-f]{64}";
pub const TOKEN_PATTERN: &str = "^xenonet_actor_[0-9a-f]{64}$";
pub const GATEWAY_PATTERN: &str = "^xenonet_gateway_[0-9a-f]{64}$";

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn is_token(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX).is_some_and(hex64)
}

pub fn is_gateway_token(s: &str) -> bool {
    s.strip_prefix(GATEWAY_PREFIX).is_some_and(hex64)
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [DIGITS[usize::from(b >> 4)] as char, DIGITS[usize::from(b & 15)] as char]).collect()
}

/// The SHA-256 XenoNet keeps of a token, in hex.
pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// A token with this prefix and 32 random bytes, and its hash.
pub fn new_with(prefix: &str) -> Result<(String, String), getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    let token = format!("{prefix}{}", hex(&bytes));
    let hash = hash_token(&token);
    Ok((token, hash))
}

pub fn new_token() -> Result<(String, String), getrandom::Error> {
    new_with(TOKEN_PREFIX)
}

pub fn new_gateway_token() -> Result<(String, String), getrandom::Error> {
    new_with(GATEWAY_PREFIX)
}

/// What a request's Authorization header says (RFC 6750): None for a header
/// that isn't a Bearer, and otherwise the token as sent, which may still be
/// one XenoNet doesn't know. (No header at all is the caller's to tell.)
pub fn bearer_of(raw: &str) -> Option<&str> {
    // /^Bearer[ \t]+(\S+)[ \t]*$/i
    let scheme = raw.get(..6).filter(|s| s.eq_ignore_ascii_case("bearer"))?;
    let rest = &raw[scheme.len()..];
    let after = rest.trim_start_matches([' ', '\t']);
    if after.len() == rest.len() {
        return None;
    }
    let end = after.find(is_js_space).unwrap_or(after.len());
    let (token, tail) = after.split_at(end);
    (!token.is_empty() && tail.chars().all(|c| c == ' ' || c == '\t')).then_some(token)
}

fn holds_prefix(s: &str) -> bool {
    s.contains(TOKEN_PREFIX) || s.contains(GATEWAY_PREFIX)
}

/// Whether a URL carries a token, which XenoNet refuses: tokens "SHOULD NOT
/// be passed in page URLs" (RFC 6750, section 5.3), where logs and history
/// keep them. Any parameter named token or access_token, or any value
/// holding a token's prefix; the parameters as the URL's search params
/// decode them.
pub fn token_in_url<'a>(params: impl IntoIterator<Item = (&'a str, &'a str)>, path: &str) -> bool {
    for (k, v) in params {
        let k = k.to_ascii_lowercase();
        if k == "token" || k == "access_token" || holds_prefix(v) {
            return true;
        }
    }
    holds_prefix(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_their_hashes() {
        let (t, h) = new_token().unwrap();
        assert!(is_token(&t) && !is_gateway_token(&t) && h == hash_token(&t) && h.len() == 64);
        assert!(!is_token("xenonet_actor_ABC") && !is_token(&format!("{t}0")));
        assert_eq!(hash_token("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn bearers() {
        assert_eq!(bearer_of("Bearer abc"), Some("abc"));
        assert_eq!(bearer_of("bearer \t abc \t"), Some("abc"));
        assert_eq!(bearer_of("Bearer a b"), None);
        assert_eq!(bearer_of("Bearerabc"), None);
        assert_eq!(bearer_of("Basic abc"), None);
        assert_eq!(bearer_of("Bearer "), None);
        assert_eq!(bearer_of("Bearer abc\u{a0}"), None, "a JS space ends the token, and isn't a space or tab");
    }

    #[test]
    fn tokens_in_urls() {
        assert!(token_in_url([("Access_Token", "x")], "/"));
        assert!(token_in_url([("q", "xenonet_gateway_1")], "/"));
        assert!(token_in_url([], "/api/xenonet_actor_x"));
        assert!(!token_in_url([("tokens", "x")], "/api/actors"));
    }
}
