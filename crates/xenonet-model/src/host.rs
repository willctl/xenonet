//! Hosts shown over SSH: the relay's hello (`host/hello/v1`) and the hosts
//! file (`host/list/v1`).

use crate::ids;
use crate::num::{Ms, Uint};
use crate::schema::{Document, Nullable, Opt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A name someone chose, ^[a-z][a-z0-9-]{1,31}$: never the machine's own name, which often holds a person's
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HostName(pub ids::HostId);

/// Made once, at random, by the host: it recognizes the host across renames and SSH aliases. It names the host and proves nothing; SSH does that
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HostKey(#[schemars(regex(pattern = r"^[0-9a-f]{32}$"))] pub String);

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
    #[schemars(range(min = 1))]
    pub protocol: Uint,
    /// The host's XenoNet
    #[schemars(length(min = 1, max = 64))]
    pub version: String,
    pub host: HelloHost,
    /// process.platform on the host: darwin, linux, win32
    #[schemars(length(min = 1, max = 32))]
    pub platform: String,
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
    const DESCRIPTION: &'static str = "The first frame xenonet relay writes on its standard output: which XenoNet is talking, in which protocol, and what it will send.";
}

// ---- host/list/v1

crate::tag!(pub ListTag = "host/list/v1");

// This XenoNet as a host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SelfHost {
    /// The name this XenoNet gave itself (xenonet hosts self), or null
    pub id: Nullable<HostName>,
    pub key: HostKey,
    pub created_at: Ms,
}

/// ssh: ssh -F <XenoNet's ssh config> <id> xenonet relay; local: the relay of another XenoNet on this machine
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    Ssh,
    Local,
}

// A machine's name or address, as ssh takes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SshHostname(#[schemars(regex(pattern = r"^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$"))] pub String);

// A user name, as ssh takes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SshUser(#[schemars(regex(pattern = r"^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$"))] pub String);

// An absolute path, or one under the home folder, as ssh's config takes it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SshPath(#[schemars(regex(pattern = r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"#))] pub String);

// A port.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Port(#[schemars(range(min = 1, max = 65535))] pub Uint);

/// How ssh reaches the host, as XenoNet writes it into its own ssh config
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Ssh {
    /// ssh's HostName: the machine's name or address
    pub hostname: SshHostname,
    pub user: Nullable<SshUser>,
    /// ssh's port, when it isn't 22
    pub port: Nullable<Port>,
    /// A key of its own for this host (IdentityFile, with IdentitiesOnly)
    pub identity: Nullable<SshPath>,
    /// A known_hosts file of its own (UserKnownHostsFile)
    pub known_hosts: Nullable<SshPath>,
    /// The name its host key is kept under (HostKeyAlias)
    pub host_key_alias: Nullable<SshHostname>,
}

// The xenonet command on a host: by name, or by absolute path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Command(#[schemars(regex(pattern = r"^(?:xenonet|\/[A-Za-z0-9._+/-]{1,1023})$"))] pub String);

// A span back in time: 7d, 12h, 90m.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Since(#[schemars(regex(pattern = r"^\d{1,4}[mhd]$"))] pub String);

// Another machine's XenoNet this one shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    /// This console's name for the host, which its ids carry: @<id>:…
    pub id: ids::HostId,
    pub transport: Transport,
    /// Null for local
    pub ssh: Nullable<Ssh>,
    /// The port of the XenoNet it reads on that machine
    pub port: Port,
    /// The xenonet command on the host: xenonet, or its absolute path where commands run over SSH don't find it; null for local
    pub command: Nullable<Command>,
    /// How far back the relay sends sessions: 7d, 12h, 90m
    pub since: Since,
    /// Whether live detail crosses too (a waiting question's text, the last message, tool details, files touched), or only titles, states and counts
    pub detail: bool,
    /// The relay starts a XenoNet there, for as long as it runs, when none is running: for a machine with no service set up
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub start: Opt<bool>,
    /// The host's key, learned the first time it answered
    pub key: Nullable<HostKey>,
    /// What the host calls itself, when that isn't the id here
    pub name: Nullable<HostName>,
    pub added_at: Ms,
}

/// Hosts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct List {
    pub schema: ListTag,
    /// This XenoNet as a host; null until hosts are first used
    #[serde(rename = "self")]
    pub this_host: Nullable<SelfHost>,
    /// The hosts this XenoNet shows
    pub hosts: Vec<Host>,
}

impl Document for List {
    const ID: &'static str = "host/list/v1";
    const TITLE: &'static str = "Hosts";
    const DESCRIPTION: &'static str =
        "hosts.json in XenoNet's state folder: this XenoNet's own host id and key, and the other machines' XenoNets it shows, with how to reach each.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Hello>, crate::schema::file::<List>];
