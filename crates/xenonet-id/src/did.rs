//! A key as a did:key (W3C CCG's did:key method): `did:key:z` and the
//! base58btc of the key's multicodec prefix and its bytes. It maps a
//! XenoNet key onto whichever agent-identity registry other projects use,
//! and costs a few lines.

use crate::codec::base58;
use crate::sign::VerifyingKey;

/// The did:key of `key`. A P-256 key is written compressed, as the method
/// says.
pub fn did_key(key: &VerifyingKey) -> String {
    let mut b = Vec::with_capacity(36);
    match key {
        VerifyingKey::Ed25519(k) => {
            b.extend_from_slice(&[0xed, 0x01]);
            b.extend_from_slice(k);
        }
        VerifyingKey::P256(k) => {
            b.extend_from_slice(&[0x80, 0x24]);
            // SEC1 compressed: 02 or 03 by the parity of y, then x.
            b.push(if k[64] & 1 == 0 { 2 } else { 3 });
            b.extend_from_slice(&k[1..33]);
        }
    }
    format!("did:key:z{}", base58(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ed25519_key_as_the_method_writes_it() {
        // Any Ed25519 key reads `z6Mk…` once prefixed with 0xed01.
        let key: [u8; 32] = [
            0x2e, 0x6f, 0xcc, 0xe3, 0x67, 0x01, 0xdc, 0x91, 0x48, 0x8e, 0x95, 0x0d, 0x0c, 0x4a, 0x0d, 0x23, 0x2b, 0x2a, 0xf3, 0x79, 0xc8, 0x3f, 0x3d, 0x54, 0x75, 0x2f, 0x32, 0x49,
            0xd7, 0x33, 0xbd, 0x42,
        ];
        let did = did_key(&VerifyingKey::Ed25519(key));
        assert!(did.starts_with("did:key:z6Mk"), "{did}");
        assert_eq!(did.len(), "did:key:z".len() + 47, "34 bytes in base58btc");
    }
}
