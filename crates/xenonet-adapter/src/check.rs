//! A doctor check an adapter brings: its row (id, group and title, as the
//! doctor lists them) and the verdict a run of it gives.

use serde::{Deserialize, Serialize};
pub use xenonet_model::doctor::Status;

/// A check's place in the doctor's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRow {
    /// `<group>.<check>`: `claude.auth`, `agents.codex`.
    pub id: &'static str,
    /// The group it shows under: the app's name, "Sessions", "Actors".
    pub group: &'static str,
    pub title: &'static str,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub status: Status,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Verdict {
    fn new(status: Status, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Verdict { status, detail: detail.into(), fix: fix.filter(|f| !f.is_empty()).map(String::from) }
    }
    pub fn ok(detail: impl Into<String>) -> Self {
        Self::new(Status::Ok, detail, None)
    }
    /// A fact, not a problem.
    pub fn info(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Info, detail, fix)
    }
    pub fn warn(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Warn, detail, fix)
    }
    pub fn fail(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Fail, detail, fix)
    }
    /// The check couldn't run here.
    pub fn skip(detail: impl Into<String>) -> Self {
        Self::new(Status::Skip, detail, None)
    }
}
