//! OpenCode's ids as XenoNet's session ids take them, and back: localId and
//! openCodeId in core/native/opencode.js. Here, so its reader, numbat's runs
//! of OpenCode and delivery to its sessions share one reading.
//!
//! A session's id has letters, digits and dashes, and OpenCode makes ses_
//! and letters and digits. So an id of letters and digits in runs joined by
//! single underscores takes a dash for each underscore (ses_4f… is ses-4f…),
//! and never has two dashes together. Any other id is x-- and its UTF-8
//! bytes in hex, which always has.

use crate::jsstr::hex;

// Letters and digits in runs, joined by single underscores.
fn plain(id: &str) -> bool {
    id.split('_').all(|run| !run.is_empty() && run.bytes().all(|c| c.is_ascii_alphanumeric()))
}

/// OpenCode's id as a session's id takes it, after "opencode:".
pub fn local_id(id: &str) -> String {
    if plain(id) {
        id.replace('_', "-")
    } else {
        format!("x--{}", hex(id.as_bytes()))
    }
}

/// XenoNet's id for an OpenCode session (`opencode:…`, or what follows it)
/// as OpenCode's own. Hex that isn't UTF-8 reads as the plain form would.
pub fn open_code_id(id: &str) -> String {
    let s = id.strip_prefix("opencode:").unwrap_or(id);
    if let Some(text) = s.strip_prefix("x--").and_then(unhex).and_then(|b| String::from_utf8(b).ok()) {
        return text;
    }
    s.replace('-', "_")
}

// Lowercase hex, two digits a byte, as the JS's /^(?:[0-9a-f]{2})*$/ takes it.
fn unhex(s: &str) -> Option<Vec<u8>> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    b.chunks(2).map(|p| Some(digit(p[0])? << 4 | digit(p[1])?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencodes_ids_take_a_dash_for_each_underscore() {
        assert_eq!(local_id("ses_eb95b1836ffexD4vYDGuLeJDCy"), "ses-eb95b1836ffexD4vYDGuLeJDCy");
        assert_eq!(local_id("ses_ab_cd"), "ses-ab-cd");
        assert_eq!(local_id("abcdefg"), "abcdefg");
    }

    #[test]
    fn any_other_id_is_its_bytes_in_hex() {
        assert_eq!(local_id("ses_ab-cd"), "x--7365735f61622d6364");
        assert_eq!(local_id("ses__ab"), "x--7365735f5f6162");
        assert_eq!(local_id("_ses"), "x--5f736573");
        assert_eq!(local_id(""), "x--");
        assert_eq!(local_id("sés_1"), "x--73c3a9735f31");
    }

    #[test]
    fn every_id_comes_back_as_it_was() {
        for id in ["ses_eb95b1836ffexD4vYDGuLeJDCy", "ses_ab_cd", "ses_ab-cd", "ses-abc", "ses__ab", "_ses", "ses_", "", "x--61", "sés_1", "a b/c", "abcdefg"] {
            assert_eq!(open_code_id(&local_id(id)), id, "{id}");
            assert_eq!(open_code_id(&format!("opencode:{}", local_id(id))), id, "{id}");
        }
    }

    #[test]
    fn an_id_read_back_that_wasnt_written_so() {
        assert_eq!(open_code_id("opencode:ses-abc123"), "ses_abc123");
        assert_eq!(open_code_id("n0123456789abcdef"), "n0123456789abcdef");
        // Hex that isn't whole bytes, lowercase or UTF-8 is read as the
        // plain form.
        assert_eq!(open_code_id("x--6"), "x__6");
        assert_eq!(open_code_id("x--6A"), "x__6A");
        assert_eq!(open_code_id("x--ff"), "x__ff");
    }
}
