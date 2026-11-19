//! Fakes for Eidolon's tests.
//!
//! - [`FakeBidi`]: the server side of WebDriver BiDi over any byte stream
//!   (a `tokio::io::duplex` end in tests), with a small model of one page.

mod bidi;

pub use bidi::{screenshot_bytes, FakeBidi, NodeKind, Page, SCREENSHOT_BASE64};
