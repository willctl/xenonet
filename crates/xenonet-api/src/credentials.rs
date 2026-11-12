//! What a client of XenoNet holds as its credential, by its shape: an
//! actor's token (`xenonet_actor_` and base64url), the pattern secret
//! scanners grep for, and a paired assistant's grant id. Whether a token
//! is one is xenonet-authz's to decide, in the service; a client only
//! tells its shape, to name a file's contents or withhold one from an
//! answer.

use crate::bearer::TOKEN_PREFIXES;

/// An actor's token's prefix (xenonet-authz's `ACTOR_PREFIX`).
pub const TOKEN_PREFIX: &str = TOKEN_PREFIXES[0];
/// Either kind of token anywhere in a text, as `git grep -E` reads it: a
/// prefix and the base64url of a Biscuit, which is never shorter than
/// this (xenonet-actors' `TOKEN_GREP`, which a test holds this to).
pub const TOKEN_GREP: &str = "xenonet_(actor|gateway)_[A-Za-z0-9_-]{64,}";
/// The shortest and longest base64url a token's text holds.
const BODY_MIN: usize = 64;
const BODY_MAX: usize = 16 * 1024;

/// Whether `s` is written as an actor's token: its prefix and base64url.
pub fn is_token(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX).is_some_and(|b| (BODY_MIN..=BODY_MAX).contains(&b.len()) && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
}

/// Whether `s` is a paired assistant's grant id: `gr_` and 24 hex digits.
pub fn is_grant(s: &str) -> bool {
    s.strip_prefix("gr_").is_some_and(|h| h.len() == 24 && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_are_the_actors() {
        assert_eq!(TOKEN_GREP, xenonet_actors::tokens::TOKEN_GREP);
        assert_eq!(TOKEN_PREFIX, xenonet_actors::tokens::TOKEN_PREFIX);
        let cases = [
            format!("{TOKEN_PREFIX}{}", "Ab-_".repeat(20)),
            "xenonet_actor_nope".into(),
            format!("{TOKEN_PREFIX}{}=", "a".repeat(64)),
            format!("{TOKEN_PREFIX}{}", "a".repeat(BODY_MAX + 1)),
        ];
        for t in cases {
            assert_eq!(is_token(&t), xenonet_actors::tokens::is_token(&t), "{t}");
        }
    }

    #[test]
    fn grants() {
        assert!(is_grant("gr_000000000000000000000000"));
        assert!(!is_grant("gr_00000000000000000000000") && !is_grant("gr_00000000000000000000000A") && !is_grant("xx_000000000000000000000000"));
    }
}
