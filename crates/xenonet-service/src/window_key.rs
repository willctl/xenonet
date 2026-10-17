//! The window key (core/actors/key.js): 32 random bytes, made at each start
//! of the service, for the routes only XenoNet's own window may use. It goes
//! to a Desktop the service can tell is XenoNet's (the component service's
//! hello) and nowhere else: it's never logged, written down or put in a URL.

use std::fmt;

pub struct WindowKey {
    value: String,
}

impl WindowKey {
    pub fn new() -> WindowKey {
        WindowKey { value: random_hex(32) }
    }

    /// For the Desktop's hello; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-window is the key, compared in
    /// constant time.
    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }
}

impl Default for WindowKey {
    fn default() -> Self {
        WindowKey::new()
    }
}

// Logged by mistake, it says what it is and nothing more.
impl fmt::Debug for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the window key]")
    }
}

/// crypto.timingSafeEqual, with the lengths compared first as the JS does.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `crypto.randomBytes(n).toString("hex")`.
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_matches_only_itself() {
        let k = WindowKey::new();
        assert_eq!(k.reveal().len(), 64);
        assert!(k.matches(Some(k.reveal())));
        assert!(!k.matches(Some("nope")));
        assert!(!k.matches(None));
        assert_eq!(format!("{k:?}"), "[the window key]");
    }
}
