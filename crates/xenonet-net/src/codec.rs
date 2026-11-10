//! The encodings the network uses: the ones keys and signatures use
//! (`xenonet_id::codec`: base64url, base32, SHA-256, random bytes), and the
//! ids of networks and frames.

use xenonet_id::codec::hex;
pub use xenonet_id::codec::{b64, base32, hash_b64, is_b64, is_key_id, nonce, random as random_id, sha256, unb64, unb64_n, uuid as uuid_id};

use crate::R;

/// `N` random bytes from the system.
pub fn random<const N: usize>() -> R<[u8; N]> {
    Ok(random_id::<N>()?)
}

/// A random (version 4) UUID, in lowercase.
pub fn uuid() -> R<String> {
    Ok(uuid_id()?)
}

/// `frm_<uuid>`.
pub fn frame_id() -> R<String> {
    Ok(format!("frm_{}", uuid()?))
}

/// `net_<32 hex>`, made once by the founder.
pub fn network_id() -> R<String> {
    Ok(format!("net_{}", hex(&random::<16>()?)))
}

pub fn is_frame_id(id: &str) -> bool {
    id.strip_prefix("frm_").is_some_and(|u| u.len() == 36 && u.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-'))
}

pub fn is_network_id(id: &str) -> bool {
    id.strip_prefix("net_").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_their_shapes() {
        let id = frame_id().unwrap();
        assert!(is_frame_id(&id), "{id}");
        assert!(!is_frame_id("frm_x"));
        assert!(is_network_id(&network_id().unwrap()));
        assert!(!is_network_id("net_ABC"));
    }
}
