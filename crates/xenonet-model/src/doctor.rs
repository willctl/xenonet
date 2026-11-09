//! The doctor's checks: one check's verdict (`doctor/result/v1`) and a
//! whole run (`doctor/run/v1`).

use crate::num::{Ms, Uint};
use crate::schema::{Document, Opt, Ref};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A check's verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Info,
    Warn,
    Fail,
    Skip,
}

/// One of the doctor's checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    /// <group>.<check>: xenonet.server, claude.auth, agents.checked…
    pub id: String,
    pub group: String,
    pub title: String,
    /// info is a fact, not a problem; skip means the check couldn't run here
    pub status: Status,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub fix: Opt<String>,
    pub ms: f64,
}

impl Document for Check {
    const ID: &'static str = "doctor/result/v1";
    const TITLE: &'static str = "A check";
    const DESCRIPTION: &'static str = "One of the doctor's checks: whether something XenoNet needs is in place, and how to fix it when it isn't.";
}

crate::tag!(pub RunTag = "doctor/run/v1");

// How many checks came out each way.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    pub ok: Uint,
    pub info: Uint,
    pub warn: Uint,
    pub fail: Uint,
    pub skip: Uint,
}

/// A run of the doctor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Run {
    /// In doctor.json
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub schema: Opt<RunTag>,
    pub at: Ms,
    pub ms: f64,
    pub results: Vec<Ref<Check>>,
    pub summary: Summary,
    /// Which XenoNet ran it, when the xenonet command asked one; null when it ran in the command itself
    pub by: Option<String>,
}

impl Document for Run {
    const ID: &'static str = "doctor/run/v1";
    const TITLE: &'static str = "A doctor run";
    const DESCRIPTION: &'static str = "Every check the doctor ran, and how many came out each way.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Check>, crate::schema::file::<Run>];
