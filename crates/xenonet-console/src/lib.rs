//! What only the console has, apart from the core: how it looks and what
//! it plays, and the way scripts, tests and agents drive XenoNet Dev.
//!
//! - `themes`: the built-in themes and how one is checked and folded on
//!   another; `themefiles`: your own, JSON files in the state folder;
//!   `environments`: the place around the map, chosen apart from its theme;
//! - `spotify`: Spotify as a remote for the app you already use;
//! - `control`: the client of a XenoNet Dev instance's control channel
//!   (launch, call, quit); `script`: the steps `xenonet console` and
//!   scripts run through it;
//! - `cores`: the service's Themes and Spotify cores over them, which the
//!   composition root puts in place (`wire`).

// A core's error carries what the routes answer (a status, a code, extra
// fields), and is rare: boxing it would only make each core wordier.
#![expect(clippy::result_large_err, reason = "a core's error is the routes' own, and rare")]

pub mod control;
pub mod cores;
pub mod environments;
pub mod script;
pub mod spotify;
pub mod themefiles;
pub mod themes;

pub use cores::wire;
