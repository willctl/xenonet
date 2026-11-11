//! Other machines' maps over the network: the relay's hello
//! (`host/hello/v1`).

use crate::ids;
use crate::num::{Ranged, Uint};
use crate::schema::{Document, Nullable, Text};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A name someone chose, ^[a-z][a-z0-9-]{1,31}$: never the machine's own name, which often holds a person's
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HostName(pub ids::HostId);

crate::pattern! {
    /// The host's key in the network: it recognizes the host across renames. It names the host and proves nothing; the network's seal does that
    pub struct HostKey(r"^[0-9a-f]{32}$");
}

// ---- host/hello/v1

crate::tag!(pub HelloT = "hello");
crate::tag!(pub HelloTag = "host/hello/v1");

// The host that says hello.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HelloHost {
    /// What the host calls itself, or null when it hasn't been named
    pub id: Nullable<HostName>,
    pub key: HostKey,
}

/// A relay's hello.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub t: HelloT,
    pub schema: HelloTag,
    /// The relay's protocol: a console refuses one it doesn't speak, naming the host's version
    pub protocol: Ranged<1>,
    /// The host's XenoNet
    pub version: Text<1, 64>,
    pub host: HelloHost,
    /// process.platform on the host: darwin, linux, win32
    pub platform: Text<1, 32>,
    /// The host shows made-up data
    pub demo: bool,
    /// The host's clock, as the hello leaves it
    pub now: f64,
    /// How far back the relay sends sessions, or null for all
    pub since_ms: Nullable<Uint>,
    /// Whether live detail crosses, or only titles, states and counts
    pub detail: bool,
}

impl Document for Hello {
    const ID: &'static str = "host/hello/v1";
    const TITLE: &'static str = "A relay's hello";
    const DESCRIPTION: &'static str = "The first frame a relay sends a console that subscribes to its map: which XenoNet is talking, in which protocol, and what it will send.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Hello>];
