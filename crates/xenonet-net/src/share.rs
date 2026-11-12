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
//! leaves a session showing that has moved out of what is shared; a delta
//! names only sessions the peer was shown, so a session's id never reaches
//! a peer it isn't shared with.

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

    /// Whether what's of project `project` (a `prj_…`, as a claim names
    /// it) is shown.
    pub fn shows_project(&self, project: &str) -> bool {
        match self {
            Sharing::All => true,
            Sharing::Projects(p) => p.contains(project),
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

/// This machine's presence as a peer in `sharings` sees it, given `seen`,
/// the sessions that peer was shown so far, which this keeps up to date:
/// the agents none of the sharings shows are left out, and a delta lists
/// as removed only sessions the peer was shown, so one that moved out of
/// what is shared goes, and the id of one it never saw is never sent.
pub fn presence_for(body: &Value, sharings: &[Sharing], seen: &mut BTreeSet<String>) -> Value {
    let id = |a: &Value| a.get("id").and_then(Value::as_str).map(str::to_string);
    let full = body.get("full").and_then(Value::as_bool).unwrap_or(false);
    let agents = body.get("agents").and_then(Value::as_array).cloned().unwrap_or_default();
    let (visible, hidden): (Vec<Value>, Vec<Value>) = agents.into_iter().partition(|a| shown(sharings, a));
    let mut out = body.clone();
    if full {
        *seen = visible.iter().filter_map(id).collect();
        out["agents"] = Value::Array(visible);
        return out;
    }
    let mut removed: Vec<Value> = Vec::new();
    for r in body.get("removed").and_then(Value::as_array).into_iter().flatten() {
        if let Some(r) = r.as_str().filter(|r| seen.remove(*r)) {
            removed.push(json!(r));
        }
    }
    for a in &hidden {
        if let Some(i) = id(a).filter(|i| seen.remove(i)) {
            removed.push(json!(i));
        }
    }
    seen.extend(visible.iter().filter_map(id));
    out["agents"] = Value::Array(visible);
    out["removed"] = Value::Array(removed);
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
        let mut shown = BTreeSet::from(["claude:dddddddd".to_string()]);
        assert_eq!(presence_for(&presence(false), &[Sharing::All], &mut shown), presence(false));
        let mut shown = BTreeSet::new();
        let none = presence_for(&presence(true), &[Sharing::default_for(false)], &mut shown);
        assert_eq!(none["agents"], json!([]));
        assert!(shown.is_empty());
        let delta = presence_for(&presence(false), &[Sharing::default_for(false)], &mut shown);
        assert_eq!(delta["removed"], json!([]), "a session never shown is never named, not even as removed");
    }

    #[test]
    fn a_network_sees_only_the_sessions_in_projects_shared_into_it() {
        let web = Sharing::Projects(["prj_web".to_string()].into());
        let api = Sharing::Projects(["prj_api".to_string()].into());
        // Shown everything, then shared down to prj_web: the others go as
        // removed, since the peer saw them.
        let mut shown = BTreeSet::new();
        presence_for(&presence(true), &[Sharing::All], &mut shown);
        let only_web = presence_for(&presence(false), std::slice::from_ref(&web), &mut shown);
        assert_eq!(only_web["agents"], json!([{ "id": "claude:aaaaaaaa", "projectKey": "prj_web" }]));
        assert_eq!(only_web["removed"], json!(["claude:bbbbbbbb", "claude:cccccccc"]));
        assert_eq!(shown, BTreeSet::from(["claude:aaaaaaaa".to_string()]));
        // A session in no confirmed checkout is shown only where all is.
        let mut shown = BTreeSet::new();
        let both = presence_for(&presence(true), &[web, api], &mut shown);
        assert_eq!(both["agents"].as_array().map(Vec::len), Some(2), "a peer in two networks sees the union");
    }

    #[test]
    fn sharing_reads_back() {
        for s in [Sharing::All, Sharing::Projects(BTreeSet::new()), Sharing::Projects(["prj_a".to_string(), "prj_b".to_string()].into())] {
            assert_eq!(Sharing::parse(&s.to_json()), Some(s));
        }
        assert_eq!(Sharing::parse(&json!(["web"])), None, "a project is a prj_ id");
        assert_eq!(Sharing::parse(&json!("some")), None);
        assert!(Sharing::All.shows_project("prj_x") && !Sharing::Projects(BTreeSet::new()).shows_project("prj_x"));
    }
}
