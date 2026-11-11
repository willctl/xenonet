//! A2A out of XenoNet: outbound actors, the agents XenoNet calls (by their
//! Agent Card, over A2A 1.0's JSON-RPC binding and its event stream), on
//! a2a-lf's types and the reqwest the service has (rustls on ring). Not
//! a2a-client-lf, which brings gRPC and a prebuilt protoc (revision 27).
//!
//! Revision 26's rules, each where it's kept:
//!
//! - **The card is pinned** (`card`): fetched once when the person
//!   registers the agent, its hash kept, and checked again before each
//!   call; a card that changed is refused until the person approves it
//!   again. Its JSON-RPC interface must be on the card's own host, or on
//!   the one other host the person approved.
//! - **https only, no redirect to another host, no private address**
//!   (`guard`): XenoNet resolves the name itself and connects to the
//!   addresses it checked; loopback, private and link-local ones are
//!   refused unless the person allowed that host at the window; no system
//!   proxy, which would resolve it instead.
//! - **The bearer never shows** (`bearer`): it has no Display, no
//!   Serialize and no Clone, its Debug is masked, its header is marked
//!   sensitive, and every word the remote agent sends back has it taken
//!   out before XenoNet passes it on.
//! - **States arrive by streaming** (`client`): SendStreamingMessage, and
//!   SubscribeToTask after a dropped connection, never GetTask in a loop.
//!   Push notifications stay unused.
//! - **States map to what XenoNet does next** (`outcome`): input-required
//!   is a question for the agent that delegated, auth-required one for the
//!   person; neither ever sends a credential. Everything the remote agent
//!   says is `Information`.
//!
//! Who may call an outbound actor at all is xenonet-authz's to decide
//! (`Act::Delegate` on `Resource::Actor`), before this crate is asked.

pub mod bearer;
pub mod card;
pub mod client;
pub mod guard;
pub mod outcome;

pub use bearer::Bearer;
pub use card::{Card, Interface, Pinned};
pub use client::{Agent, Delegated};
pub use guard::Private;
pub use outcome::{Ended, Update};

use std::fmt;

/// Why a call to an outside agent didn't go: a short code and words for a
/// person, with no bearer in them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A2aError {
    /// `http` (plain http), `address`, `private` (a non-public address the
    /// person didn't allow), `redirect`, `card` (no card, or one XenoNet
    /// can't use), `changed` (the card changed since it was pinned), `host`
    /// (the interface is on a host the person didn't approve), `transport`,
    /// `remote` (the agent answered with an error), `protocol` (it answered
    /// with something A2A doesn't say).
    pub code: &'static str,
    pub message: String,
}

impl A2aError {
    pub fn new(code: &'static str, message: impl Into<String>) -> A2aError {
        A2aError { code, message: message.into() }
    }
}

impl fmt::Display for A2aError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for A2aError {}

/// What reqwest said went wrong, in XenoNet's words: the guard's own
/// refusal when it was one (a redirect, a private address), else what
/// failed, never the request's headers.
pub(crate) fn transport(e: &reqwest::Error) -> A2aError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(s) = source {
        if let Some(ours) = s.downcast_ref::<A2aError>() {
            return ours.clone();
        }
        source = s.source();
    }
    let what = if e.is_timeout() {
        "it didn't answer in time"
    } else if e.is_connect() {
        "XenoNet couldn't connect"
    } else if e.is_redirect() {
        "it redirected where XenoNet doesn't follow"
    } else {
        "the connection failed"
    };
    let host = e.url().map(guard::host_of).unwrap_or_default();
    A2aError::new("transport", format!("{host}: {what}"))
}
