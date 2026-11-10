//! Text that informs and never authorizes. A message, a handoff's note, an
//! ask's question and a prompt are `Information<T>`: whoever holds one can
//! read it and show it, and that's all. It converts into nothing this crate
//! takes (no `From`, `Into`, `Deref`, `AsRef` or `Clone`), so no path from a
//! body to a [`Request`](crate::Request), a [`Token`](crate::Token) or an
//! [`Authorized`](crate::Authorized) compiles; tests/ui holds the proof.

use std::fmt;

pub struct Information<T>(T);

impl<T> Information<T> {
    pub fn new(content: T) -> Information<T> {
        Information(content)
    }

    pub fn read(&self) -> &T {
        &self.0
    }
}

// Logged, it says what it is, so a log line can't be read as an approval.
impl<T: fmt::Debug> fmt::Debug for Information<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Information({:?})", self.0)
    }
}
