//! The tray app's view of XenoNet: what GET /api/tray/v1/status answers
//! (`tray/status/v1`) and the alerts on GET /api/tray/v1/events
//! (`tray/event/v1`).

use crate::num::{Ms, Uint};
use crate::schema::{Document, Nullable, Opt, Text};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// A machine's id in a network.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Host(pub String);

impl JsonSchema for Host {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Host")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^[a-z][a-z0-9-]{1,31}$" })
    }
}

// What an item of the Needs you queue waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    Question,
    Permission,
    Actor,
    Request,
    Gateway,
    Requests,
    Ask,
    SignIn,
}

// An item of the Needs you queue, in the console's words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Item {
    /// The item's key in the Needs you queue (core/needs-you.js): session:<id>, actor:<id>, requests…
    #[schemars(length(min = 1, max = 300))]
    pub key: String,
    pub kind: ItemKind,
    /// The session or actor it's about, which open-console selects; null for programs asking to be actors
    pub id: Nullable<Text<0, 300>>,
    #[schemars(length(max = 200))]
    pub title: String,
    /// What it waits for, after its title, as the console says it: asks, needs permission, asks to join XenoNet…
    #[schemars(length(max = 60))]
    pub words: String,
    /// The session's app (claude, codex, grok, opencode), or null
    pub app: Nullable<Text<0, 32>>,
    /// The machine it's on, when it isn't this one
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<Host>,
    /// The question or what it asks, when XenoNet has it
    pub text: Nullable<Text<0, 500>>,
    pub since: Nullable<Ms>,
    /// A wait on another machine that isn't live, which may be over
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
    /// For requests: how many programs ask
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub count: Opt<Uint>,
}

crate::tag!(pub EventTag = "tray/event/v1");

// Whether the item started or stopped waiting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    NeedsYou,
    Resolved,
}

/// A tray alert.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub schema: EventTag,
    /// needs-you: it started waiting on you; resolved: it stopped
    pub kind: EventKind,
    pub item: Item,
    pub at: Ms,
}

impl Document for Event {
    const ID: &'static str = "tray/event/v1";
    const TITLE: &'static str = "A tray alert";
    const DESCRIPTION: &'static str = "An alert on GET /api/tray/v1/events: an item of the Needs you queue that started or stopped waiting on you, for the tray's notifications.";
}

crate::tag!(pub StatusTag = "tray/status/v1");

crate::constant!(
    /// A boolean that is always true: the service answered.
    pub Running = true
);

// What waits on you.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NeedsYou {
    /// How many wait on you, as the console's strip counts them
    pub count: Uint,
    /// The Needs you queue in its order, the longest wait first
    #[schemars(length(max = 20))]
    pub items: Vec<Item>,
}

// The gateway's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GatewayState {
    Off,
    Starting,
    Online,
    Problem,
}

// The actor gateway.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Gateway {
    pub on: bool,
    pub state: GatewayState,
    /// Its public address, when it has one
    pub url: Nullable<Text<0, 300>>,
    pub problem: Nullable<Text<0, 500>>,
}

// This machine's link to the network's gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    Off,
    Connecting,
    Live,
    Offline,
    Refused,
    Removed,
}

// This machine's network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Network {
    /// The network's name; null when this machine isn't in one
    pub name: Nullable<Text<0, 64>>,
    /// This machine's id in it
    pub host: Nullable<Host>,
    /// This machine's link to the network's gateway
    pub link: Link,
    /// The other machines in it
    pub hosts: Uint,
    /// How many of them are online
    pub online: Uint,
}

/// The tray's status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub schema: StatusTag,
    /// The service answered. A tray that gets no answer shows XenoNet as stopped
    pub running: Running,
    /// The service's version
    #[schemars(length(max = 40))]
    pub version: String,
    /// installed, demo, dev, or the instance XenoNet Dev names
    pub instance: Nullable<Text<0, 40>>,
    /// The console in a browser: http://127.0.0.1:<port>/
    #[schemars(length(max = 100))]
    pub url: String,
    /// Whether XenoNet Desktop is connected to the service now
    pub desktop: bool,
    /// Sessions that aren't idle
    pub working: Uint,
    pub needs_you: NeedsYou,
    /// The actor gateway, which actors reach this machine through
    pub gateway: Gateway,
    pub network: Network,
    pub at: Ms,
}

impl Document for Status {
    const ID: &'static str = "tray/status/v1";
    const TITLE: &'static str = "The tray's status";
    const DESCRIPTION: &'static str = "What GET /api/tray/v1/status answers XenoNet's tray app: that the service runs, what waits on you in the console's words, the actor gateway's state and this machine's link to its network.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Event>, crate::schema::file::<Status>];
