//! Tickets (docs/design/network.md, section 4): the one-time code **Add a
//! machine** shows, in the line to run on the new machine:
//! `xenonet net join gw.example.net KX7Q-49MZ-T2`.
//!
//! A ticket is 10 characters of Crockford's base32 (50 bits), shown in
//! groups of four. The gateway keeps only its SHA-256, and it works once,
//! for ten minutes. The ticket itself never crosses the network: the join
//! request proves it with an HMAC keyed by its hash, and the gateway proves
//! it knew it the same way, so the new machine can pin the gateway's key.

use crate::codec::{b64, random, sha256, unb64};
use crate::R;
use ring::hmac;

pub const LENGTH: usize = 10;
pub const TICKET_MS: f64 = 10.0 * 60e3;
/// At most this many wait at once.
pub const MAX_WAITING: usize = 5;
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

/// What the gateway keeps: the SHA-256 of the code.
pub fn hash(code: &str) -> [u8; 32] {
    sha256(code.as_bytes())
}

pub fn hash_b64(code: &str) -> String {
    b64(&hash(code))
}

fn mac(hash: &[u8], msg: &str) -> String {
    b64(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, hash), msg.as_bytes()).as_ref())
}

fn check(hash: &[u8], msg: &str, proof: &str) -> bool {
    unb64(proof).is_some_and(|p| hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, hash), msg.as_bytes(), &p).is_ok())
}

fn join_message(nonce: &str, key_id: &str) -> String {
    format!("xenonet-join/1\n{nonce}\n{key_id}")
}

fn gateway_message(gateway_key_id: &str, nonce: &str) -> String {
    format!("xenonet-gateway/1\n{gateway_key_id}\n{nonce}")
}

/// The join request's proof that it holds the ticket: an HMAC keyed by
/// the ticket's hash over the request's nonce and the new host's key id.
pub fn join_proof(hash: &[u8], nonce: &str, key_id: &str) -> String {
    mac(hash, &join_message(nonce, key_id))
}

pub fn check_join(hash: &[u8], nonce: &str, key_id: &str, proof: &str) -> bool {
    check(hash, &join_message(nonce, key_id), proof)
}

/// The gateway's answer that it knew the ticket: an HMAC keyed by its hash
/// over the gateway's key id and the request's nonce.
pub fn gateway_proof(hash: &[u8], gateway_key_id: &str, nonce: &str) -> String {
    mac(hash, &gateway_message(gateway_key_id, nonce))
}

pub fn check_gateway(hash: &[u8], gateway_key_id: &str, nonce: &str, proof: &str) -> bool {
    check(hash, &gateway_message(gateway_key_id, nonce), proof)
}

fn maker_message(nonce: &str, key_id: &str) -> String {
    format!("xenonet-join-maker/1\n{nonce}\n{key_id}")
}

/// The join request's proof for a ticket's maker when that isn't the
/// gateway's host: an HMAC keyed by the ticket itself, which only the new
/// machine and the maker know. The gateway, which keeps only the hash,
/// can't make one, so it can't have the maker sign keys of its own choosing
/// into the roster.
pub fn maker_proof(code: &str, nonce: &str, key_id: &str) -> String {
    mac(code.as_bytes(), &maker_message(nonce, key_id))
}

pub fn check_maker(code: &str, nonce: &str, key_id: &str, proof: &str) -> bool {
    check(code.as_bytes(), &maker_message(nonce, key_id), proof)
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
    fn proofs_check_only_with_the_ticket() {
        let h = hash("KX7Q49MZT2");
        let other = hash("KX7Q49MZT3");
        let p = join_proof(&h, "n1", "xnk1_a");
        assert!(check_join(&h, "n1", "xnk1_a", &p));
        assert!(!check_join(&other, "n1", "xnk1_a", &p));
        assert!(!check_join(&h, "n2", "xnk1_a", &p));
        assert!(!check_join(&h, "n1", "xnk1_b", &p));
        assert!(!check_join(&h, "n1", "xnk1_a", "not base64!"));
        let g = gateway_proof(&h, "xnk1_g", "n1");
        assert!(check_gateway(&h, "xnk1_g", "n1", &g));
        assert!(!check_gateway(&h, "xnk1_x", "n1", &g));
        assert_ne!(g, p, "the two proofs differ");
        // The maker's proof is keyed by the ticket, not its hash.
        let m = maker_proof("KX7Q49MZT2", "n1", "xnk1_a");
        assert!(check_maker("KX7Q49MZT2", "n1", "xnk1_a", &m));
        assert!(!check_maker("KX7Q49MZT3", "n1", "xnk1_a", &m));
        assert!(!check_maker("KX7Q49MZT2", "n1", "xnk1_b", &m));
        assert_ne!(m, mac(&h, &maker_message("n1", "xnk1_a")), "the hash alone can't make it");
    }
}
