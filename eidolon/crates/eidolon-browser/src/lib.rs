//! The browser facet: Firefox kept alive in a cell by systemd, with its
//! profile moved as a facet and its automation reached as a byte stream.
//!
//! - [`provision_plan`] and the other plans in [`plan`]: how a guest is set
//!   up, as data (argument lists, no shell strings).
//! - [`units`]: the systemd units and `user.js`, kept as files in `units/`.
//! - [`BrowserFacet`]: quiets the browser, copies the profile, leaves out
//!   what rebuilds itself.
//! - [`Browser`]: the live handle. It reaches the browser's WebDriver BiDi
//!   port through `Driver::stream` (the port is never forwarded), and
//!   reconnects when the browser restarts.
//!
//! No program of Eidolon's runs in the guest, and nothing here hides
//! automation from a site: no stealth preferences, no user-agent edits.

mod browser;
mod error;
mod facet;
pub mod plan;
pub mod units;

pub use browser::{Browser, ConnectOptions};
pub use error::{BrowserError, Result};
pub use facet::{BrowserFacet, PROFILE_LEAVE_OUT};
pub use plan::{provision_plan, start_browser_plan, start_view_plan, stream_argv, Plan, Step, MOZILLA_KEY_FINGERPRINT};
