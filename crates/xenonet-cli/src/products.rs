//! The actor products XenoNet knows, by id (core/actors/products.js's
//! PRODUCTS): what xenonet actor show needs to tell a product XenoNet knows
//! from one an actor names itself. The ids are the JS table's; the rest of
//! products.js (observers, slugs, pairing) is the actors core's, and this
//! goes when that's in Rust.

const PRODUCTS: [&str; 4] = ["hermes-agent", "grok-bot", "muse", "dots"];

/// Whether productOf(id) knows the product.
pub fn known(id: &str) -> bool {
    PRODUCTS.contains(&id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_by_id() {
        assert!(super::known("hermes-agent"));
        assert!(!super::known("acme-sync"));
        assert!(!super::known("__proto__"));
    }
}
