//! The engine: which process is on the other end of a connection
//! (`procs`), whether a request made in an agent's or an actor's name
//! comes from it (`verify`), messages, claims and views (`engine`), the
//! agents it knows of, kept current as they change (`index`),
//! delivery (`delivery`), and the network's side of them (`network`).
//! What they keep goes through the engine's own `store::Store`, which the
//! composition implements over the one store.
//!
//! What a module does with the outside world (running a command, reading
//! the clock, the store) goes through a trait of its own, which the
//! runtime (the service's compose::runtime) and the tests implement.

pub mod attest;
pub mod delivery;
pub mod engine;
pub mod host;
pub mod index;
pub mod network;
pub mod processes;
pub mod procs;
pub mod procs_native;
pub mod store;
pub mod verify;

#[cfg(test)]
mod test_store;

/// A pattern, in the regex crate's syntax, compiled once, the first time
/// it's used. Where a pattern reads text as the engine's callers write it,
/// white space is `[\s\x{FEFF}--\x{85}]` (trim()'s white space, which has
/// U+FEFF and not U+0085), `.` stops at any line terminator
/// (`[^\n\r\x{2028}\x{2029}]`), and digits, word characters and word
/// boundaries are ASCII's.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| $crate::compile($p))
    }};
}
pub(crate) use re;

/// A pattern `re!` was given: one written in the source, so it compiles.
pub(crate) fn compile(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("a fixed pattern: callers pass them from the source")
}

#[cfg(test)]
mod tests {
    #[test]
    fn white_space_is_what_trim_takes_off() {
        let s = re!(r"^[\s\x{FEFF}--\x{85}]$");
        let ns = re!(r"^[^\s\x{FEFF}--\x{85}]$");
        for c in (0..=0x10ffff_u32).filter_map(char::from_u32) {
            let one = c.to_string();
            assert_eq!(s.is_match(&one), xenonet_text::text::is_space(c), "{c:?}");
            assert_eq!(ns.is_match(&one), !xenonet_text::text::is_space(c), "{c:?}");
        }
        let dot = re!(r"^[^\n\r\x{2028}\x{2029}]$");
        assert!(!dot.is_match("\r") && !dot.is_match("\u{2028}") && dot.is_match("\u{85}"));
    }
}
