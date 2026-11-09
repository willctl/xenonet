//! An account work happens under, as XenoNet found it (`identity/v1`).

use crate::num::{Ms, Uint};
use crate::schema::{Document, Opt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Which kind of account it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Claude,
    Chatgpt,
    Openai,
    Github,
    Git,
}

// What else is known about the account, by kind.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Detail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// A Claude team's or enterprise's name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    /// A GitHub account's host
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// The name git commits carry
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// An account work happens under: a Claude account in an organization, a ChatGPT workspace, an OpenAI API key, a GitHub account, or the email git commits with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// claude:<account>:<organization>, chatgpt:<workspace>, openai:api-key, github:<host>/<login> or git:<hash of the email>. Identities have no host: the ids are already global
    pub id: String,
    pub kind: Kind,
    /// The email or login it shows as
    #[schemars(with = "Opt<Option<String>>")]
    pub handle: Option<String>,
    pub detail: Detail,
    pub signed_in: bool,
    /// gh's active account for its host
    pub active: bool,
    /// Where XenoNet found it, with the home folder as ~
    pub sources: Vec<String>,
    pub first_seen: Ms,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub seen_at: Opt<Ms>,
    /// The name you gave it; identities with the same name show as one
    #[schemars(with = "Opt<Option<String>>")]
    pub name: Option<String>,
    pub hidden: bool,
    pub sessions: Uint,
    pub projects: Uint,
    /// With hosts on: the machines it was seen on, null for this one
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub hosts: Opt<Vec<Option<String>>>,
    /// With hosts on: the machines it's signed in on, null for this one
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub signed_in_on: Opt<Vec<Option<String>>>,
}

impl Document for Identity {
    const ID: &'static str = "identity/v1";
    const TITLE: &'static str = "Identity";
    const DESCRIPTION: &'static str =
        "An account work happens under: a Claude account in an organization, a ChatGPT workspace, an OpenAI API key, a GitHub account, or the email git commits with.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Identity>];
