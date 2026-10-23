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

/// The tray's key: 32 random bytes, made at each start of the service, for
/// the tray app's routes (routes/tray.rs). It goes only to the tray app the
/// component socket can tell is XenoNet's (verify.tray), in its hello's
/// answer, and nowhere else: never written to a file, logged or put in a
/// URL. A file would be read by anything running as you, agents included.
pub struct TrayKey {
    value: String,
}

impl TrayKey {
    pub fn new() -> TrayKey {
        TrayKey { value: random_hex(32) }
    }

    /// For the tray's hello; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-tray is the key, in constant time.
    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }
}

impl Default for TrayKey {
    fn default() -> Self {
        TrayKey::new()
    }
}

impl fmt::Debug for TrayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the tray's key]")
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
    xenonet_base::jsstr::hex(&b)
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
        let t = TrayKey::new();
        assert!(t.matches(Some(t.reveal())) && !t.matches(Some(k.reveal())) && !t.matches(None));
        assert_eq!(format!("{t:?}"), "[the tray's key]");
    }
}
