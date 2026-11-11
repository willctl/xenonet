//! P-256 in the shapes Windows gives it and XenoNet takes it: CNG hands out
//! a public key as a BCRYPT_ECCPUBLIC_BLOB and a signature as r and s, 32
//! bytes each, while `VerifyingKey::P256` is SEC1 uncompressed and checks
//! DER. Plain byte work, so its tests run on every platform.

/// BCRYPT_ECDSA_PUBLIC_P256_MAGIC ("ECS1"), little-endian.
const P256_PUBLIC_MAGIC: u32 = 0x3153_4345;

/// SEC1 uncompressed (0x04, X, Y) from a P-256 BCRYPT_ECCPUBLIC_BLOB: the
/// magic and the key's length (u32 each, little-endian), then X and Y.
#[cfg_attr(all(not(windows), not(test)), expect(dead_code, reason = "only Windows' CNG gives keys this way"))]
pub fn sec1_from_ecc_blob(blob: &[u8]) -> Option<Vec<u8>> {
    let word = |at: usize| Some(u32::from_le_bytes(blob.get(at..at + 4)?.try_into().ok()?));
    if word(0)? != P256_PUBLIC_MAGIC || word(4)? != 32 || blob.len() != 8 + 64 {
        return None;
    }
    let mut out = Vec::with_capacity(65);
    out.push(4);
    out.extend_from_slice(&blob[8..]);
    Some(out)
}

/// The DER of an ECDSA signature (a SEQUENCE of the INTEGERs r and s) from
/// r and s, 32 bytes each, big-endian.
#[cfg_attr(all(not(windows), not(test)), expect(dead_code, reason = "only Windows' CNG signs this way"))]
pub fn der_from_raw(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() != 64 {
        return None;
    }
    let integer = |n: &[u8]| {
        // Shortest form, with a zero before a high bit so it stays positive.
        let n = &n[n.iter().position(|&b| b != 0).unwrap_or(n.len() - 1)..];
        let mut out = vec![0x02];
        if n[0] & 0x80 != 0 {
            out.extend_from_slice(&[n.len() as u8 + 1, 0]);
        } else {
            out.push(n.len() as u8);
        }
        out.extend_from_slice(n);
        out
    };
    let (r, s) = (integer(&raw[..32]), integer(&raw[32..]));
    // At most 2 × 35 bytes, so the length fits in one byte.
    let mut out = vec![0x30, (r.len() + s.len()) as u8];
    out.extend(r);
    out.extend(s);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    use xenonet_id::sign::{Purpose, VerifyingKey};

    #[test]
    fn a_fixed_signature_and_a_cng_blob_become_what_verifying_key_checks() {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        // The blob CNG would give for this key.
        let sec1 = pair.public_key().as_ref();
        let mut blob = Vec::new();
        blob.extend_from_slice(&P256_PUBLIC_MAGIC.to_le_bytes());
        blob.extend_from_slice(&32u32.to_le_bytes());
        blob.extend_from_slice(&sec1[1..]);
        let public = VerifyingKey::P256(sec1_from_ecc_blob(&blob).unwrap());
        assert_eq!(public.bytes(), sec1);
        // Many signatures, so r and s with a high bit and with leading
        // zeros both come up.
        for i in 0..64u8 {
            let msg = [i; 7];
            let raw = pair.sign(&rng, &Purpose::FEED.bytes(&msg)).unwrap();
            let der = der_from_raw(raw.as_ref()).unwrap();
            assert!(public.verify(Purpose::FEED, &msg, &der), "signature {i}");
            assert!(!public.verify(Purpose::FRAME, &msg, &der));
        }
        assert_eq!(sec1_from_ecc_blob(&blob[..70]), None);
        blob[0] ^= 1;
        assert_eq!(sec1_from_ecc_blob(&blob), None, "another curve's magic");
        assert_eq!(der_from_raw(&[1; 63]), None);
    }

    #[test]
    fn integers_keep_their_sign_and_shortest_form() {
        let mut raw = [0u8; 64];
        raw[31] = 1; // r = 1
        raw[32] = 0x80; // s has its high bit set
        let der = der_from_raw(&raw).unwrap();
        assert_eq!(&der[..5], &[0x30, 3 + 35, 0x02, 1, 1]);
        assert_eq!(&der[5..8], &[0x02, 33, 0]);
        assert_eq!(der.len(), 2 + 3 + 35);
    }
}
