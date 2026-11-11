//! A message's own words, as they're shown beneath XenoNet's lines. The
//! envelope that says how XenoNet knows who sent one is xenonet-api's
//! (`xenonet_api::attest`), which the routes and the gateway write.

/// A message's own words as they're shown beneath XenoNet's lines: each of
/// its lines quoted, with what could pass for a line of XenoNet's own
/// taken out (line breaks of other kinds, controls, and characters that
/// reorder text).
pub fn quoted(text: &str) -> String {
    let lines = crate::re!(r"\r\n?|[\u0085\u2028\u2029\u000b\u000c]").replace_all(text, "\n");
    let clean = crate::re!(r"[\u0000-\u0008\u000e-\u001f\u007f-\u0084\u0086-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]").replace_all(&lines, "");
    clean.split('\n').map(|l| format!("  > {l}")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_takes_out_what_could_pass_for_xenonets_own_lines() {
        assert_eq!(quoted("a\r\nb\u{2028}c\u{202e}d"), "  > a\n  > b\n  > cd");
    }
}
