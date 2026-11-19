use crate::{EidolonId, HolderKey, ModelError};
use serde::{Deserialize, Serialize};
use xenonet_id::codec::{b64, unb64};
use xenonet_id::sign::{Purpose, Signer};

/// Where an eidolon is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Its state is sealed in the vault and open nowhere.
    Sealed,
    /// Open in one cell.
    Open { cell: String },
    /// Done with: it never opens again.
    Retired,
}

/// Who may open an eidolon now. A move writes the next generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatRecord {
    pub eidolon: EidolonId,
    /// 1 for the first seat; each move adds one.
    pub generation: u64,
    /// The machine that may open it now.
    pub holder: HolderKey,
    /// The machine that held it before; none for generation 1.
    pub previous: Option<HolderKey>,
    /// RFC 3339, UTC.
    pub at: String,
}

impl SeatRecord {
    /// The bytes a seat's signature covers: JSON with the keys in the
    /// order `eidolon`, `generation`, `holder`, `previous`, `at`, no
    /// whitespace, `previous` as `null` when none, keys as base64url
    /// without padding. The same record is always the same bytes.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let previous = self.previous.map_or_else(|| "null".to_owned(), |p| format!("\"{}\"", p.to_b64()));
        // `at` is free text, so it is escaped the way JSON does; the rest are
        // made of characters JSON never escapes.
        let at = serde_json::to_string(&self.at).unwrap_or_else(|_| "\"\"".to_owned());
        format!("{{\"eidolon\":\"{}\",\"generation\":{},\"holder\":\"{}\",\"previous\":{previous},\"at\":{at}}}", self.eidolon, self.generation, self.holder.to_b64()).into_bytes()
    }
}

/// A seat record and its signature (base64url), made by the machine that
/// held the previous seat, or by the first holder itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedSeat {
    pub record: SeatRecord,
    pub signature: String,
}

impl SignedSeat {
    /// Sign `record` with the machine's key.
    pub fn sign(record: SeatRecord, signer: &dyn Signer) -> Result<SignedSeat, ModelError> {
        let sig = signer.sign(Purpose::EIDOLON_SEAT, &record.canonical_bytes()).map_err(|e| ModelError::Sign(e.message))?;
        Ok(SignedSeat { record, signature: b64(&sig) })
    }
}

/// Whether `seat` was signed by `signer_key`.
pub fn verify(seat: &SignedSeat, signer_key: &HolderKey) -> bool {
    unb64(&seat.signature).is_some_and(|sig| signer_key.verifying_key().verify(Purpose::EIDOLON_SEAT, &seat.record.canonical_bytes(), &sig))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    fn signer(n: u8) -> (SeedSigner, HolderKey) {
        let s = SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap();
        let k = HolderKey::from_verifying_key(&s.public()).unwrap();
        (s, k)
    }

    fn record(holder: HolderKey) -> SeatRecord {
        SeatRecord { eidolon: EidolonId::from_bytes([1; 16]), generation: 1, holder, previous: None, at: "2026-01-02T03:04:05Z".into() }
    }

    #[test]
    fn a_seat_checks_only_for_its_signer_and_its_own_bytes() {
        let (desk, desk_key) = signer(1);
        let (_, tower_key) = signer(2);
        let seat = SignedSeat::sign(record(desk_key), &desk).unwrap();
        assert!(verify(&seat, &desk_key));
        assert!(!verify(&seat, &tower_key), "another machine");
        let mut moved = seat.clone();
        moved.record.generation = 2;
        assert!(!verify(&moved, &desk_key), "another record");
        let mut junk = seat;
        junk.signature = "not a signature".into();
        assert!(!verify(&junk, &desk_key));
    }

    #[test]
    fn the_signed_bytes_are_fixed_json() {
        let (_, k) = signer(1);
        let mut r = record(k);
        let first = String::from_utf8(r.canonical_bytes()).unwrap_or_default();
        assert_eq!(
            first,
            format!("{{\"eidolon\":\"eid_{}\",\"generation\":1,\"holder\":\"{}\",\"previous\":null,\"at\":\"2026-01-02T03:04:05Z\"}}", "aeaqcaibaeaqcaibaeaqcaibae", k.to_b64())
        );
        r.previous = Some(k);
        assert!(String::from_utf8(r.canonical_bytes()).unwrap_or_default().contains(&format!("\"previous\":\"{}\"", k.to_b64())));
    }

    #[test]
    fn states_are_snake_case_json() {
        assert_eq!(serde_json::to_string(&State::Sealed).unwrap_or_default(), "\"sealed\"");
        assert_eq!(serde_json::to_string(&State::Open { cell: "adytum-a".into() }).unwrap_or_default(), "{\"open\":{\"cell\":\"adytum-a\"}}");
    }
}
