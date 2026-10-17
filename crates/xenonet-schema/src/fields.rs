//! Which keys read as a field's name and which as data (core/drift.js's
//! isField): a drift record keeps a field's name, but never a key that's
//! data (a login, a repo, a session id), which shows as `*`.

/// A key that reads as a field's name, not as data: no slashes, dots, @,
/// colons or spaces, no runs of digits, no hex id (all hex, or seven hex
/// characters in a row with a digit among them: a commit's short hash, an
/// id inside a name) and no token's prefix.
pub fn is_field(k: &str) -> bool {
    shaped_like_a_name(k) && !digits_run(k, 4) && !hex_id(k) && !token_prefix(k)
}

// /^[A-Za-z_$][\w$-]{0,47}$/
fn shaped_like_a_name(k: &str) -> bool {
    let b = k.as_bytes();
    let head = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
    let rest = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-';
    !b.is_empty() && b.len() <= 48 && head(b[0]) && b[1..].iter().all(|&c| rest(c))
}

// /\d{n,}/
fn digits_run(k: &str, n: usize) -> bool {
    let mut run = 0;
    for c in k.bytes() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= n {
            return true;
        }
    }
    false
}

// /^[0-9a-f-]{12,}$/i, or a run of 7 or more hex characters with a digit in it.
fn hex_id(k: &str) -> bool {
    let hexish = |c: u8| c.is_ascii_hexdigit() || c == b'-';
    if k.len() >= 12 && k.bytes().all(hexish) {
        return true;
    }
    k.as_bytes().split(|c| !c.is_ascii_hexdigit()).any(|run| run.len() >= 7 && run.iter().any(u8::is_ascii_digit))
}

// The prefixes of tokens and keys that secret scanners know.
fn token_prefix(k: &str) -> bool {
    const PLAIN: &[&str] =
        &["github_pat_", "sk-", "sk_live_", "sk_test_", "pk_live_", "pk_test_", "rk_live_", "rk_test_", "whsec_", "xapp-", "AIza", "eyJ", "npm_", "pypi-", "hf_", "dop_v1_"];
    if PLAIN.iter().any(|p| k.starts_with(p)) {
        return true;
    }
    let b = k.as_bytes();
    // gh[pousr]_
    if b.len() >= 4 && &b[..2] == b"gh" && b"pousr".contains(&b[2]) && b[3] == b'_' {
        return true;
    }
    // gl(pat|dt|rt|ptt|soat|ft)-
    if let Some(rest) = k.strip_prefix("gl") {
        if ["pat-", "dt-", "rt-", "ptt-", "soat-", "ft-"].iter().any(|p| rest.starts_with(p)) {
            return true;
        }
    }
    // xox[a-z]-
    if b.len() >= 5 && &b[..3] == b"xox" && b[3].is_ascii_lowercase() && b[4] == b'-' {
        return true;
    }
    // A[KS]IA[0-9A-Z]{12}
    if b.len() >= 16 && b[0] == b'A' && (b[1] == b'K' || b[1] == b'S') && &b[2..4] == b"IA" && b[4..16].iter().all(|c| c.is_ascii_digit() || c.is_ascii_uppercase()) {
        return true;
    }
    // shp(at|ss|ca|pa)_
    if let Some(rest) = k.strip_prefix("shp") {
        if ["at_", "ss_", "ca_", "pa_"].iter().any(|p| rest.starts_with(p)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::is_field;

    #[test]
    fn names_and_data() {
        for k in ["type", "tool_use_id", "$ref", "is-error", "cache_creation"] {
            assert!(is_field(k), "{k}");
        }
        for k in ["", "a.b", "user@x", "C:/x", "has space", "v2026", "0190d3e0-0000-7000", "deadbeef1", "abc1234def", "ghp_x", "sk-x", "AKIAABCDEFGHIJKL", "1abc", &"x".repeat(49)]
        {
            assert!(!is_field(k), "{k}");
        }
        assert!(is_field("abcdefg"), "seven hex letters with no digit is a word");
    }
}
