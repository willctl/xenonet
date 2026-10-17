//! The window key (core/actors/key.js; docs/design/actors/interface.md,
//! section 2): 32 random bytes, made at each start of the server, for the
//! routes only XenoNet's own window may use. The server hands it to the
//! desktop app, and the app gives it only to XenoNet's own page. It's never
//! written down, logged or put in a URL, and no other program has it.

use crate::tokens::hex;

/// What the key says of itself when it's logged or sent by mistake.
pub const HIDDEN: &str = "[the window key]";

pub struct WindowKey {
    value: String,
}

impl WindowKey {
    pub fn new() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)?;
        Ok(WindowKey { value: hex(&bytes) })
    }

    /// For the desktop app; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-window is the key, compared in
    /// constant time.
    pub fn matches(&self, given: &str) -> bool {
        let (a, b) = (given.as_bytes(), self.value.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_itself() {
        let k = WindowKey::new().unwrap();
        assert_eq!(k.reveal().len(), 64);
        assert!(k.matches(&k.reveal().to_string()));
        assert!(!k.matches(&k.reveal()[1..]) && !k.matches(""));
        assert_ne!(k.reveal(), WindowKey::new().unwrap().reveal());
    }
}
