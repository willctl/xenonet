//! Tickets (docs/design/network.md, section 4): the one-time code an
//! invite's join proves, which the person gives the new machine apart from
//! the invite.
//!
//! A ticket is 10 characters of Crockford's base32 (50 bits), shown in
//! groups of four. The inviter keeps only its SHA-256, and it works once,
//! for ten minutes. The code itself never crosses the network: join/2's
//! proofs are HMACs keyed by its hash (`join`).

use crate::codec::{b64, random, sha256, unb64};
use crate::R;
use ring::hmac;

pub const LENGTH: usize = 10;
pub const TICKET_MS: f64 = 10.0 * 60e3;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A new ticket's code, 10 characters.
pub fn make() -> R<String> {
    let bytes = random::<LENGTH>()?;
    // 256 is a multiple of 32, so each character is uniform.
    Ok(bytes.iter().map(|b| char::from(ALPHABET[usize::from(b & 31)])).collect())
}

/// As people see and type it: KX7Q-49MZ-T2.
pub fn display(code: &str) -> String {
    let parts: Vec<String> = code.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    parts.join("-")
}

/// A ticket as it was typed, to its 10 characters: any case, with or
/// without dashes and spaces, O for 0 and I or L for 1 forgiven. None when
/// it can't be one.
pub fn normalize(typed: &str) -> Option<String> {
    let mut out = String::with_capacity(LENGTH);
    for c in typed.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !ALPHABET.contains(&(c as u8)) {
            return None;
        }
        out.push(c);
    }
    (out.len() == LENGTH).then_some(out)
}

/// What the inviter keeps: the SHA-256 of the code.
pub fn hash(code: &str) -> [u8; 32] {
    sha256(code.as_bytes())
}

pub(crate) fn mac(hash: &[u8], msg: &str) -> String {
    b64(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, hash), msg.as_bytes()).as_ref())
}

pub(crate) fn check(hash: &[u8], msg: &str, proof: &str) -> bool {
    unb64(proof).is_some_and(|p| hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, hash), msg.as_bytes(), &p).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_is_ten_characters_shown_in_groups() {
        let code = make().unwrap();
        assert_eq!(code.len(), LENGTH);
        assert_eq!(normalize(&code).as_deref(), Some(code.as_str()));
        let shown = display(&code);
        assert_eq!(shown.len(), 12);
        assert_eq!(normalize(&shown).as_deref(), Some(code.as_str()));
        assert_eq!(normalize("kx7q-49mz-t2").as_deref(), Some("KX7Q49MZT2"));
        assert_eq!(normalize("KX7Q 49MZ TO").as_deref(), Some("KX7Q49MZT0"));
        assert_eq!(normalize("KX7Q-49MZ-TU"), None, "U isn't in it");
        assert_eq!(normalize("KX7Q-49MZ"), None);
        assert_eq!(normalize("KX7Q-49MZ-T2X"), None);
        assert_ne!(make().unwrap(), make().unwrap());
    }

    #[test]
    fn a_proof_checks_only_with_the_tickets_hash() {
        let h = hash("KX7Q49MZT2");
        let p = mac(&h, "joiner");
        assert!(check(&h, "joiner", &p));
        assert!(!check(&hash("KX7Q49MZT3"), "joiner", &p));
        assert!(!check(&h, "inviter", &p));
        assert!(!check(&h, "joiner", "not base64!"));
    }
}
