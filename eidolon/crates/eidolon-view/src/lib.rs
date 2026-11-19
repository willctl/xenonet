//! The person's window for signing in to a browser facet.
//!
//! While a window is open the browser runs as the plain view unit with no
//! automation socket, which is what a site sees. The person signs in through
//! a VNC viewer, closes the window, and automation starts again on the same
//! profile.
//!
//! - [`View::open`] switches the cell to the view units and listens on
//!   127.0.0.1 only. Each connection gets its own `Driver::stream` to the
//!   guest's VNC port; Lima forwards nothing.
//! - [`View::secret`] is the screen share's password for this session, given
//!   out once. VNC's own authentication is weak, so the protection is that
//!   both ends are loopback-only and the secret lives for one session.
//! - [`View::close`] stops listening, drops connections and starts the
//!   automation browser again.
//! - [`ViewGuard`] is the marker the CLI and the MCP tools check, so
//!   automation is refused while a window is open.

mod error;
mod guard;
mod secret;
mod view;

pub use error::{Result, ViewError};
pub use guard::ViewGuard;
pub use view::{OpenOptions, View};
