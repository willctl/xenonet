//! What each network sees of this machine (phase 5, revision 8): a network's
//! presence carries only the sessions in confirmed checkouts of the
//! projects the person shares into that network.
//!
//! The first network a person founds or joins on a machine shares all of
//! it, as the one network did before. Every network after that starts with
//! none: the person shares projects into it from the window (phase 5's
//! commit 13). Sessions carry no network of their own, so the project a
//! session's folder is a confirmed checkout of (presence's `projectKey`,
//! a `prj_…`) is what decides.
//!
//! A machine in several networks with a peer shows that peer the union of
//! what it shares into them: it is one connection, and the peer is in each.
//! A session the peer no longer sees goes out as removed, so a delta never
//! leaves a session showing that has moved out of what is shared.

use serde_json::{json, Value};
use std::collections::BTreeSet;

/// What one network is shown of this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sharing {
    /// Everything: the first network.
    All,
    /// The sessions in confirmed checkouts of these projects (`prj_…`).
    Projects(BTreeSet<String>),
}

impl Sharing {
    /// What a network shares until the person says otherwise: everything
    /// for the first network this machine is in, nothing for the others.
    pub fn default_for(first: bool) -> Sharing {
        if first {
            Sharing::All
        } else {
            Sharing::Projects(BTreeSet::new())
        }
    }

    /// Whether an agent of this machine's presence is shown.
    pub fn shows(&self, agent: &Value) -> bool {
        match self {
            Sharing::All => true,
            Sharing::Projects(p) => agent.get("projectKey").and_then(Value::as_str).is_some_and(|k| p.contains(k)),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Sharing::All => json!("all"),
            Sharing::Projects(p) => json!(p),
        }
    }

    pub fn parse(v: &Value) -> Option<Sharing> {
        match v {
            Value::String(s) if s == "all" => Some(Sharing::All),
            Value::Array(a) => a.iter().map(|p| p.as_str().filter(|p| p.starts_with("prj_")).map(str::to_string)).collect::<Option<BTreeSet<_>>>().map(Sharing::Projects),
            _ => None,
        }
    }
}

/// Whether any of `sharings` (the networks a peer shares with this
/// machine) shows `agent`.
pub fn shown(sharings: &[Sharing], agent: &Value) -> bool {
    sharings.iter().any(|s| s.shows(agent))
}

/// This machine's presence as a peer in `sharings` sees it: the agents none
/// of them shows are left out, and a delta lists those as removed, so the
/// peer drops one it showed before. With everything shared, the body is as
/// it was.
pub fn presence_for(body: &Value, sharings: &[Sharing]) -> Value {
    if sharings.contains(&Sharing::All) {
        return body.clone();
    }
    let mut out = body.clone();
    let agents = body.get("agents").and_then(Value::as_array).cloned().unwrap_or_default();
    let (shown_now, hidden): (Vec<Value>, Vec<Value>) = agents.into_iter().partition(|a| shown(sharings, a));
    out["agents"] = Value::Array(shown_now);
    let full = body.get("full").and_then(Value::as_bool).unwrap_or(false);
    if !full {
        let mut removed = body.get("removed").and_then(Value::as_array).cloned().unwrap_or_default();
        removed.extend(hidden.iter().filter_map(|a| a.get("id").cloned()));
        out["removed"] = Value::Array(removed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presence(full: bool) -> Value {
        json!({
            "schema": "network/presence/v1",
            "counter": 7,
            "full": full,
            "agents": [
                { "id": "claude:aaaaaaaa", "projectKey": "prj_web" },
                { "id": "claude:bbbbbbbb", "projectKey": "prj_api" },
                { "id": "claude:cccccccc" },
            ],
            "removed": ["claude:dddddddd"],
        })
    }

    #[test]
    fn the_first_network_sees_everything_and_a_later_one_nothing_until_shared() {
        assert_eq!(Sharing::default_for(true), Sharing::All);
        assert_eq!(Sharing::default_for(false), Sharing::Projects(BTreeSet::new()));
        assert_eq!(presence_for(&presence(false), &[Sharing::All]), presence(false));
        let none = presence_for(&presence(true), &[Sharing::default_for(false)]);
        assert_eq!(none["agents"], json!([]));
        assert_eq!(none["removed"], json!(["claude:dddddddd"]), "a full presence lists nothing removed it didn't");
    }

    #[test]
    fn a_network_sees_only_the_sessions_in_projects_shared_into_it() {
        let web = Sharing::Projects(["prj_web".to_string()].into());
        let api = Sharing::Projects(["prj_api".to_string()].into());
        let only_web = presence_for(&presence(false), std::slice::from_ref(&web));
        assert_eq!(only_web["agents"], json!([{ "id": "claude:aaaaaaaa", "projectKey": "prj_web" }]));
        // A delta drops what the peer can't see, so one it showed goes.
        assert_eq!(only_web["removed"], json!(["claude:dddddddd", "claude:bbbbbbbb", "claude:cccccccc"]));
        // A session in no confirmed checkout is shown only where all is.
        let both = presence_for(&presence(true), &[web, api]);
        assert_eq!(both["agents"].as_array().map(Vec::len), Some(2), "a peer in two networks sees the union");
    }

    #[test]
    fn sharing_reads_back() {
        for s in [Sharing::All, Sharing::Projects(BTreeSet::new()), Sharing::Projects(["prj_a".to_string(), "prj_b".to_string()].into())] {
            assert_eq!(Sharing::parse(&s.to_json()), Some(s));
        }
        assert_eq!(Sharing::parse(&json!(["web"])), None, "a project is a prj_ id");
        assert_eq!(Sharing::parse(&json!("some")), None);
    }
}
