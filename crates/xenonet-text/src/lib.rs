//! The formats XenoNet writes and reads, which its files, answers and the
//! golden files pin character for character, with no IO:
//!
//! - `json`: JSON as XenoNet reads and writes it (numbers, key order,
//!   escapes, as the JS it was ported from did);
//! - `text`: what a value is as text or a number, and what `trim()` and
//!   `\s` take;
//! - `utf16`: strings counted, cut and compared in UTF-16 units, hex and
//!   base64, and local days;
//! - `time`: ISO 8601 times, read as `Date.parse` reads them;
//! - `units`: money, tokens, durations and shares as people read them;
//! - `ordered`: a map that keeps its keys in the order they were added;
//! - `sort`: a stable sort that takes whatever its comparator says.

pub mod json;
pub mod ordered;
pub mod sort;
pub mod text;
pub mod time;
pub mod units;
pub mod utf16;
