//! What each actor's work costs today, at API prices (core/actors/spend.js):
//! Token use's spend by actor, GET /api/spend and xenonet spend. Each
//! session's calls today go to at most one actor, so nothing is counted
//! twice and the parts add up to the day's estimate. A session is an
//! actor's when XenoNet knows the actor started it:
//!
//!   1. A run an actor reported through the actor gateway is that actor's.
//!   2. Else a session an actor started, as its spawned events say. When
//!      more than one actor says so, what XenoNet saw itself (observed,
//!      attributed) comes before what an actor said of itself
//!      (registered), then the earliest, then the actor's id.
//!   3. Else a subagent's, or a session another session started, goes with
//!      the session it came from, by the same rules, up to 8 sessions up.
//!      A fork doesn't.
//!   4. Else no actor's: unattributed. That includes the sessions you start
//!      yourself.
//!
//! A run reported from elsewhere has no transcript here, so XenoNet can't
//! price it: it's counted apart, as one of the actor's runs elsewhere.

use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use xenonet_engine::js::{locale_compare, template, utf16_cmp};

pub const UNATTRIBUTED: &str = "unattributed";
const LINEAGE_MAX: usize = 8;

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|f| f.is_finite())
}

/// A session's actor and how: "reported" (1), "started" (2) or "parent" (3).
#[derive(Clone, Debug, PartialEq)]
pub struct Who {
    pub actor: String,
    pub how: &'static str,
}

// A key a JS Map would tell apart: a session's id as it's given.
fn id_key(v: &Value) -> String {
    match v {
        Value::String(s) => format!("s{s}"),
        other => format!("v{other}"),
    }
}

/// Each session's actor by the rule, for the sessions that have one.
/// `sessions`: the map's sessions ({ id, reportedBy, parentId,
/// parentKind }); `starts`: who started what ({ actor, session, at, via });
/// `known`: the actors that count, or every one when None. `ids`: the
/// sessions to answer for, the map's when None. In the order asked.
pub fn attribute(sessions: &[Value], starts: &[Value], known: Option<&HashSet<String>>, ids: Option<&[Value]>) -> Vec<(Value, Who)> {
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    let mut order: Vec<Value> = Vec::new();
    for s in sessions {
        if let Some(id) = s.get("id").filter(|v| xenonet_engine::js::truthy(Some(v))) {
            if by_id.insert(id_key(id), s).is_none() {
                order.push(id.clone());
            }
        }
    }
    fn counted<'a>(actor: Option<&'a Value>, known: Option<&HashSet<String>>) -> Option<&'a str> {
        actor.and_then(Value::as_str).filter(|a| known.is_none_or(|k| k.contains(*a)))
    }
    let counts = |actor| counted(actor, known);
    let mut best: HashMap<String, (String, u8, f64)> = HashMap::new();
    for st in starts {
        let (Some(actor), Some(session)) = (counts(st.get("actor")), st.get("session").and_then(Value::as_str)) else { continue };
        let rank = if matches!(st.get("via").and_then(Value::as_str), Some("observed" | "attributed")) { 2 } else { 1 };
        let at = finite(st.get("at")).unwrap_or(f64::INFINITY);
        let better = match best.get(session) {
            None => true,
            Some((b_actor, b_rank, b_at)) => rank > *b_rank || (rank == *b_rank && (at < *b_at || (at == *b_at && utf16_cmp(actor, b_actor) == Ordering::Less))),
        };
        if better {
            best.insert(session.to_string(), (actor.to_string(), rank, at));
        }
    }
    let own = |id: &Value| -> Option<Who> {
        let s = by_id.get(&id_key(id));
        if let Some(r) = s.and_then(|s| counts(s.get("reportedBy"))) {
            return Some(Who { actor: r.to_string(), how: "reported" });
        }
        id.as_str().and_then(|i| best.get(i)).map(|(a, _, _)| Who { actor: a.clone(), how: "started" })
    };
    let mut out: Vec<(Value, Who)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for id in ids.map(<[Value]>::to_vec).unwrap_or(order) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut cur = Some(id.clone());
        let mut up = 0;
        while let Some(c) = cur.filter(|c| xenonet_engine::js::truthy(Some(c)) && up <= LINEAGE_MAX && !seen.contains(&id_key(c))) {
            seen.insert(id_key(&c));
            if let Some(o) = own(&c) {
                let who = if up > 0 { Who { actor: o.actor, how: "parent" } } else { o };
                // A Map set again keeps its first place.
                match at.get(&id_key(&id)) {
                    Some(&i) => out[i].1 = who,
                    None => {
                        at.insert(id_key(&id), out.len());
                        out.push((id.clone(), who));
                    }
                }
                break;
            }
            let s = by_id.get(&id_key(&c));
            cur = s.filter(|s| matches!(s.get("parentKind").and_then(Value::as_str), Some("subagent" | "spawn"))).and_then(|s| s.get("parentId")).filter(|p| p.is_string()).cloned();
            up += 1;
        }
    }
    out
}

#[derive(Default)]
struct Part {
    cost: f64,
    tokens: f64,
    sessions: f64,
    unpriced: BTreeSet<Utf16>,
    elsewhere: f64,
}

// A text ordered as JS's sort() orders text.
#[derive(Clone, PartialEq, Eq)]
struct Utf16(String);

impl PartialOrd for Utf16 {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Utf16 {
    fn cmp(&self, o: &Self) -> Ordering {
        utf16_cmp(&self.0, &o.0)
    }
}

fn plain(id: Option<(&str, &Value)>, p: &Part) -> Value {
    let mut o = Map::new();
    if let Some((id, name)) = id {
        o.insert("id".into(), json!(id));
        o.insert("name".into(), name.clone());
    }
    o.insert("cost".into(), xenonet_base::json::number(p.cost));
    o.insert("tokens".into(), xenonet_base::json::number(p.tokens));
    o.insert("sessions".into(), xenonet_base::json::number(p.sessions));
    if id.is_some() {
        o.insert("elsewhere".into(), xenonet_base::json::number(p.elsewhere));
    }
    o.insert("unpriced".into(), Value::Array(p.unpriced.iter().map(|u| json!(u.0)).collect()));
    Value::Object(o)
}

/// Today's spend by actor: { total, actors, unattributed }. `spend`: each
/// session's calls today ({ id, cost, tokens, unpriced }); `names`: the
/// actors that count, each one's name by its id; `since`: when today began,
/// for the runs reported from elsewhere today. Each actor: { id, name,
/// cost, tokens, sessions, unpriced, elsewhere }, the most spent first.
pub fn spend_by_actor(spend: &[Value], sessions: &[Value], starts: &[Value], names: &[(String, Value)], since: Option<f64>) -> Value {
    let known: HashSet<String> = names.iter().map(|(k, _)| k.clone()).collect();
    let name_of = |id: &str| names.iter().rev().find(|(k, _)| k == id).map(|(_, v)| v.clone()).filter(|v| !v.is_null()).unwrap_or_else(|| json!(id));
    let list: Vec<&Value> = spend.iter().filter(|s| s.get("id").is_some_and(Value::is_string) && finite(s.get("cost")).is_some()).collect();
    let runs: Vec<&Value> = sessions.iter().filter(|s| s.get("observed").and_then(Value::as_str) == Some("gateway") && s.get("id").is_some_and(Value::is_string)).collect();
    let ids: Vec<Value> = list.iter().map(|s| s["id"].clone()).chain(runs.iter().map(|s| s["id"].clone())).collect();
    let who: HashMap<String, Who> = attribute(sessions, starts, Some(&known), Some(&ids)).into_iter().map(|(k, w)| (template(Some(&k)), w)).collect();
    let mut rows: Vec<(String, Part)> = Vec::new();
    let row_index = |rows: &mut Vec<(String, Part)>, id: &str| -> usize {
        match rows.iter().position(|(k, _)| k == id) {
            Some(i) => i,
            None => {
                rows.push((id.to_string(), Part::default()));
                rows.len() - 1
            }
        }
    };
    let mut none = Part::default();
    let mut counted: HashSet<String> = HashSet::new();
    let mut total = 0.0;
    for s in &list {
        let id = s["id"].as_str().unwrap();
        if !counted.insert(id.to_string()) {
            continue;
        }
        let r = match who.get(id) {
            Some(w) => {
                let i = row_index(&mut rows, &w.actor);
                &mut rows[i].1
            }
            None => &mut none,
        };
        let cost = finite(s.get("cost")).unwrap();
        r.cost += cost;
        r.tokens += finite(s.get("tokens")).unwrap_or(0.0);
        r.sessions += 1.0;
        if let Some(Value::Array(u)) = s.get("unpriced") {
            for m in u {
                r.unpriced.insert(Utf16(template(Some(m))));
            }
        }
        total += cost;
    }
    for s in &runs {
        let id = s["id"].as_str().unwrap();
        let today = match since {
            None => true,
            Some(since) => s.get("updatedAt").and_then(Value::as_f64).is_some_and(|u| u >= since),
        };
        if counted.contains(id) || !today {
            continue;
        }
        if let Some(w) = who.get(id) {
            let i = row_index(&mut rows, &w.actor);
            rows[i].1.elsewhere += 1.0;
        }
    }
    let mut actors: Vec<(String, Value, &Part)> = rows.iter().map(|(id, p)| (id.clone(), name_of(id), p)).collect();
    actors.sort_by(|(ai, an, a), (bi, bn, b)| {
        let by = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(Ordering::Equal);
        by(a.cost, b.cost)
            .then_with(|| by(a.sessions, b.sessions))
            .then_with(|| by(a.elsewhere, b.elsewhere))
            .then_with(|| locale_compare(&template(Some(an)), &template(Some(bn))))
            .then_with(|| locale_compare(ai, bi))
    });
    json!({
        "total": xenonet_base::json::number(total),
        "actors": actors.iter().map(|(id, name, p)| plain(Some((id, name)), p)).collect::<Vec<_>>(),
        "unattributed": plain(None, &none),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_go_to_their_actor_or_parent() {
        let sessions = vec![json!({ "id": "a" }), json!({ "id": "b", "parentId": "a", "parentKind": "subagent" }), json!({ "id": "c", "parentId": "a", "parentKind": "fork" })];
        let starts = vec![json!({ "actor": "actor:wren", "session": "a", "at": 1, "via": "registered" }), json!({ "actor": "actor:hermes", "session": "a", "at": 5, "via": "observed" })];
        let w = attribute(&sessions, &starts, None, None);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].1, Who { actor: "actor:hermes".into(), how: "started" });
        assert_eq!(w[1].1, Who { actor: "actor:hermes".into(), how: "parent" });
    }
}
