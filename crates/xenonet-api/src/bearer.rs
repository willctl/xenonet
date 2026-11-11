//! How a request carries an actor's token (docs/design/actors/interface.md,
//! section 2, "Tokens"): as `Authorization: Bearer`, and never in a URL,
//! where logs and history keep it. The routes and the actor gateway read
//! it so; xenonet-authz says whether it's a token.

use xenonet_text::text::is_space;

/// The prefixes a token's text starts with: an actor's, and a gateway
/// token's (xenonet-authz's `ACTOR_PREFIX` and `GATEWAY_PREFIX`, which a
/// test holds these to).
pub const TOKEN_PREFIXES: [&str; 2] = ["xenonet_actor_", "xenonet_gateway_"];

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
    TOKEN_PREFIXES.iter().any(|p| s.contains(p))
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
    fn the_prefixes_are_authzs() {
        assert_eq!(TOKEN_PREFIXES, [xenonet_authz::ACTOR_PREFIX, xenonet_authz::GATEWAY_PREFIX]);
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
