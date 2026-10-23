//! The owner key the service wrote (xenolith_core::owner), as the tray uses it: only when the
//! service answering /status on this tray's port has the pid the key file names, as keyhandoff.rs
//! checks the pipe's server. The window gets it in the URL's fragment (never sent to the server, so
//! never in a request log) and sends it back as a header when it decides something.

use crate::config::Config;
use crate::status::Status;
use xenolith_core::owner::{self, OwnerKey};
use xenolith_core::paths::AppPaths;

/// What the tray knows about the key, for --selftest and the log.
pub enum State {
    Ok(OwnerKey),
    /// No key file this tray may read: the service isn't running, or isn't this user's.
    Missing,
    /// A key from another service: (its pid, the pid /status names, or 0 with the service down).
    Stale(u32, i64),
}

pub fn state(cfg: &Config, status: &Status) -> State {
    let Some(k) = owner::read(&AppPaths::current()) else { return State::Missing };
    let pid = status.int(&["pid"]);
    if k.port != cfg.port || !status.reachable() || i64::from(k.pid) != pid {
        return State::Stale(k.pid, pid);
    }
    State::Ok(k)
}

pub fn key(cfg: &Config, status: &Status) -> Option<String> {
    match state(cfg, status) {
        State::Ok(k) => Some(k.key),
        _ => None,
    }
}

pub fn describe(cfg: &Config, status: &Status) -> String {
    match state(cfg, status) {
        State::Ok(k) => format!("ok (pid {})", k.pid),
        State::Missing => "missing".into(),
        State::Stale(of, now) => format!("stale (pid {of}, the service is pid {now})"),
    }
}

/// The window's address: with the owner key in the fragment when the service checks out.
pub fn ui_url(cfg: &Config, key: Option<&str>) -> String {
    match key {
        Some(k) => format!("{}#owner={k}", cfg.ui_url()),
        None => cfg.ui_url(),
    }
}
