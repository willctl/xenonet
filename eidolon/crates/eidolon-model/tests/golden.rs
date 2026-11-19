//! The JSON the spec and the seat have on disk and on the wire, byte for byte.

use eidolon_model::{verify, EidolonSpec, HolderKey, SignedSeat};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

fn golden(name: &str) -> String {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_default().trim_end().to_owned()
}

#[test]
fn the_spec_round_trips_byte_for_byte() {
    let text = golden("eidolon-spec.json");
    let spec: EidolonSpec = serde_json::from_str(&text).unwrap_or_else(|e| panic_with(&e.to_string()));
    assert_eq!(serde_json::to_string(&spec).unwrap_or_default(), text);
    assert!(spec.validate().is_ok());
    assert_eq!(spec.holders.len(), 2);
}

#[test]
fn the_seat_round_trips_and_its_signature_checks() {
    let text = golden("seat.json");
    let seat: SignedSeat = serde_json::from_str(&text).unwrap_or_else(|e| panic_with(&e.to_string()));
    assert_eq!(serde_json::to_string(&seat).unwrap_or_default(), text);
    let desk = SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap_or_else(|e| panic_with(&e.message));
    let key = HolderKey::from_verifying_key(&desk.public()).unwrap_or_else(|| panic_with("not ed25519"));
    assert!(verify(&seat, &key), "the golden seat is signed by seed [1; 32]");
}

#[expect(clippy::panic, reason = "a test that cannot go on without its fixture")]
fn panic_with(why: &str) -> ! {
    panic!("{why}")
}
