//! Actors' tokens (docs/design/actors/interface.md, section 2, "Tokens";
//! docs/design/direction.md, "Authorization"): Biscuits rooted at this
//! machine, minted and decided by xenonet-authz, written as
//! `xenonet_actor_<base64url>`. The prefix makes a leaked token easy to
//! recognize, by people and by secret scanners. XenoNet keeps only a
//! token's revocation id, never the token. A token is shown once and
//! travels only as Authorization: Bearer, never in a URL.
//!
//! A gateway token (the actor gateway's) is `xenonet_gateway_<base64url>`:
//! a token minted for the actor with the gateway's right, narrowed to the
//! gateway door, so a token that leaks from this machine can't be used
//! from the internet, and one made for the gateway can't stand in for the
//! process checks here.
//!
//! What an actor may do in XenoNet is its token's `right(...)` facts: the
//! capability words it was allowed map onto authz's actions (`rights`),
//! with reading for every actor. The other words (browsing, running
//! commands…) say what it does elsewhere; they allow nothing here but
//! reporting it. Which sessions and identities it may use are its grant's
//! scopes, rows kept beside the token by its revocation id.

use xenonet_authz::Action;
use xenonet_text::text::is_space;

pub use xenonet_authz::{ACTOR_PREFIX as TOKEN_PREFIX, GATEWAY_PREFIX};
/// Either kind anywhere in a file, as `git grep -E` reads it: a prefix and
/// the base64url of a Biscuit, which is never shorter than this.
pub const TOKEN_GREP: &str = "xenonet_(actor|gateway)_[A-Za-z0-9_-]{64,}";
/// The shortest and longest base64url a token's text holds.
const BODY_MIN: usize = 64;
const BODY_MAX: usize = 16 * 1024;

fn b64url(s: &str) -> bool {
    (BODY_MIN..=BODY_MAX).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `s` is written as an actor's token: its prefix and base64url.
/// Only xenonet-authz says whether it is one.
pub fn is_token(s: &str) -> bool {
    s.strip_prefix(TOKEN_PREFIX).is_some_and(b64url)
}

pub fn is_gateway_token(s: &str) -> bool {
    s.strip_prefix(GATEWAY_PREFIX).is_some_and(b64url)
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [DIGITS[usize::from(b >> 4)] as char, DIGITS[usize::from(b & 15)] as char]).collect()
}

/// The actions a token minted for these capability words has the right
/// to: reading what its scopes let it see, which every actor does, and
/// sessions it may start, messages and handoffs it may send, files it may
/// claim.
pub fn rights(capabilities: &[String]) -> Vec<Action> {
    let mut out = vec![Action::Read];
    for c in capabilities {
        out.extend_from_slice(match c.as_str() {
            "spawn-sessions" => &[Action::Start][..],
            "message-agents" => &[Action::Message, Action::Handoff],
            "edit-repos" => &[Action::Claim],
            _ => &[],
        });
    }
    out.sort();
    out.dedup();
    out
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
    let end = after.find(is_space).unwrap_or(after.len());
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
    fn tokens_are_written_as_their_prefix_and_base64url() {
        let t = format!("{TOKEN_PREFIX}{}", "Ab-_".repeat(20));
        assert!(is_token(&t) && !is_gateway_token(&t));
        assert!(is_gateway_token(&format!("{GATEWAY_PREFIX}{}", "x".repeat(64))));
        assert!(!is_token("xenonet_actor_nope") && !is_token(&format!("{t}=")) && !is_token(&format!("{TOKEN_PREFIX}{}", "a".repeat(BODY_MAX + 1))));
    }

    #[test]
    fn capabilities_map_onto_rights() {
        let words = |w: &[&str]| w.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(rights(&words(&["browse", "message-agents", "spawn-sessions"])), [Action::Message, Action::Start, Action::Handoff, Action::Read]);
        assert_eq!(rights(&words(&["edit-repos", "run-commands"])), [Action::Claim, Action::Read]);
        assert_eq!(rights(&words(&["use-computer"])), [Action::Read], "every actor reads");
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
