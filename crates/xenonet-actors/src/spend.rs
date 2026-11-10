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
//!
//! What Hermes and OpenCode say they spent is a separate figure, `reported`,
//! never added to the estimate: see `reported_today`.

use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use xenonet_text::sort::{locale_compare, utf16_cmp};
use xenonet_text::text::template;

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
        if let Some(id) = s.get("id").filter(|v| xenonet_text::text::truthy(Some(v))) {
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
        while let Some(c) = cur.filter(|c| xenonet_text::text::truthy(Some(c)) && up <= LINEAGE_MAX && !seen.contains(&id_key(c))) {
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
            cur =
                s.filter(|s| matches!(s.get("parentKind").and_then(Value::as_str), Some("subagent" | "spawn"))).and_then(|s| s.get("parentId")).filter(|p| p.is_string()).cloned();
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
    // What Hermes and OpenCode reported for today, apart from the estimate.
    reported: bool,
    rep_usd: f64,
    rep_tokens: f64,
    rep_sources: BTreeSet<Utf16>,
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
    o.insert("cost".into(), xenonet_text::json::number(p.cost));
    o.insert("tokens".into(), xenonet_text::json::number(p.tokens));
    o.insert("sessions".into(), xenonet_text::json::number(p.sessions));
    if id.is_some() {
        o.insert("elsewhere".into(), xenonet_text::json::number(p.elsewhere));
    }
    o.insert("unpriced".into(), Value::Array(p.unpriced.iter().map(|u| json!(u.0)).collect()));
    if p.reported {
        let usd = if p.rep_usd > 0.0 { xenonet_text::json::number(p.rep_usd) } else { Value::Null };
        let source = p.rep_sources.iter().map(|u| u.0.as_str()).collect::<Vec<_>>().join(", ");
        o.insert("reported".into(), json!({ "usd": usd, "tokens": xenonet_text::json::number(p.rep_tokens), "source": source }));
    }
    Value::Object(o)
}

/// Today's spend by actor: { total, actors, unattributed }. `spend`: each
/// session's calls today ({ id, cost, tokens, unpriced }); `names`: the
/// actors that count, each one's name by its id; `since`: when today began,
/// for the runs reported from elsewhere today. Each actor: { id, name,
/// cost, tokens, sessions, unpriced, elsewhere }, the most spent first.
/// `reported`: today's increases in what Hermes and OpenCode report
/// (`reported_today`'s items), each added to its actor's or session's
/// actor as a separate `reported: { usd, tokens, source }`, never to the
/// cost.
pub fn spend_by_actor(spend: &[Value], sessions: &[Value], starts: &[Value], names: &[(String, Value)], since: Option<f64>, reported: &[Value]) -> Value {
    let known: HashSet<String> = names.iter().map(|(k, _)| k.clone()).collect();
    let name_of = |id: &str| names.iter().rev().find(|(k, _)| k == id).map(|(_, v)| v.clone()).filter(|v| !v.is_null()).unwrap_or_else(|| json!(id));
    // Each session's calls with its id and cost, and each run reported
    // from elsewhere with its id.
    let list: Vec<(&str, f64, &Value)> = spend.iter().filter_map(|s| Some((s.get("id")?.as_str()?, finite(s.get("cost"))?, s))).collect();
    let runs: Vec<(&str, &Value)> =
        sessions.iter().filter(|s| s.get("observed").and_then(Value::as_str) == Some("gateway")).filter_map(|s| Some((s.get("id")?.as_str()?, s))).collect();
    let rep_sessions = reported.iter().filter_map(|r| r.get("session").filter(|s| s.is_string()).cloned());
    let ids: Vec<Value> = list.iter().map(|(id, _, _)| json!(id)).chain(runs.iter().map(|(id, _)| json!(id))).chain(rep_sessions).collect();
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
    for &(id, cost, s) in &list {
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
    for &(id, s) in &runs {
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
    for r in reported {
        let part = match (r.get("actor").and_then(Value::as_str), r.get("session").and_then(Value::as_str)) {
            // An actor missing from the list gets nothing, not the unattributed.
            (Some(a), _) if !known.contains(a) => continue,
            (Some(a), _) => {
                let i = row_index(&mut rows, a);
                &mut rows[i].1
            }
            (None, Some(s)) => match who.get(s) {
                Some(w) => {
                    let i = row_index(&mut rows, &w.actor);
                    &mut rows[i].1
                }
                None => &mut none,
            },
            (None, None) => &mut none,
        };
        part.reported = true;
        part.rep_usd += finite(r.get("usd")).unwrap_or(0.0);
        part.rep_tokens += finite(r.get("tokens")).unwrap_or(0.0);
        if let Some(src) = r.get("source").and_then(Value::as_str) {
            part.rep_sources.insert(Utf16(src.to_string()));
        }
    }
    let mut actors: Vec<(String, Value, &Part)> = rows.iter().map(|(id, p)| (id.clone(), name_of(id), p)).collect();
    xenonet_text::sort::sort_by(&mut actors, |(ai, an, a), (bi, bn, b)| {
        let by = |x: f64, y: f64| y.partial_cmp(&x).unwrap_or(Ordering::Equal);
        by(a.cost, b.cost)
            .then_with(|| by(a.sessions, b.sessions))
            .then_with(|| by(a.elsewhere, b.elsewhere))
            .then_with(|| locale_compare(&template(Some(an)), &template(Some(bn))))
            .then_with(|| locale_compare(ai, bi))
    });
    json!({
        "total": xenonet_text::json::number(total),
        "actors": actors.iter().map(|(id, name, p)| plain(Some((id, name)), p)).collect::<Vec<_>>(),
        "unattributed": plain(None, &none),
    })
}

const TOKEN_FIELDS: [&str; 5] = ["input", "output", "reasoning", "cacheRead", "cacheWrite"];

// The name the figure carries for an app or an actor's product: of the
// products and apps that report what they spent (Hermes, OpenCode), the
// first whose slug or id it holds, by its name; any other as it is.
fn source_label(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let product = xenonet_adapter::products::PRODUCTS.iter().filter(|p| p.reports_spend).find(|p| lower.contains(p.slug)).map(|p| p.short);
    let app = || xenonet_adapter::apps::apps().iter().filter(|a| a.reports_spend).find(|a| lower.contains(a.id.as_str())).map(|a| a.name.as_str());
    product.or_else(app).map_or_else(|| raw.to_string(), str::to_string)
}

// One cumulative reading: its key, where it came from, whose it is, and
// the dollars (None when unpriced) and tokens it totals.
struct Reading {
    key: String,
    source: String,
    actor: Option<String>,
    session: Option<String>,
    usd: Option<f64>,
    tokens: f64,
}

fn reading_of(usage: &Value) -> Option<(Option<f64>, f64)> {
    if !usage.is_object() {
        return None;
    }
    let tokens = TOKEN_FIELDS.iter().map(|f| finite(usage.get("tokens").and_then(|t| t.get(*f))).filter(|v| *v >= 0.0).unwrap_or(0.0)).sum();
    Some((finite(usage.get("costUsd")).filter(|v| *v >= 0.0), tokens))
}

/// What Hermes and OpenCode report as spent, for today only. Both keep
/// totals for the life of a session (Hermes's on the actor, over the
/// sessions it has seen), so each reading is a baseline at first sight and
/// only increases after it count, per day: `state` is the answer's own
/// state from the last call (null at first), `since` when today began.
/// `usages`: the actors' usage ({ id, product, usage }); `sessions`: the
/// map's, a session with `usage` being OpenCode's. A priced increase adds
/// dollars and tokens; an unpriced one (a subscription, an unknown price)
/// adds tokens and no dollars, never a zero price. Answers (state, items),
/// each item { key, source, actor | session, usd (null when no dollars),
/// tokens } for what grew today, by key.
pub fn reported_today(state: &Value, since: f64, usages: &[Value], sessions: &[Value]) -> (Value, Vec<Value>) {
    let mut readings: Vec<Reading> = Vec::new();
    for u in usages {
        let (Some(id), Some((usd, tokens))) = (u.get("id").and_then(Value::as_str), u.get("usage").and_then(reading_of)) else { continue };
        let product = u.get("product").and_then(Value::as_str).unwrap_or("");
        readings.push(Reading {
            key: id.to_string(),
            source: source_label(if product.is_empty() { id } else { product }),
            actor: Some(id.to_string()),
            session: None,
            usd,
            tokens,
        });
    }
    for s in sessions {
        let (Some(id), Some((usd, tokens))) = (s.get("id").and_then(Value::as_str), s.get("usage").and_then(reading_of)) else { continue };
        let app = s.get("app").and_then(Value::as_str).unwrap_or("");
        readings.push(Reading { key: id.to_string(), source: source_label(app), actor: None, session: Some(id.to_string()), usd, tokens });
    }
    let mut seen: Map<String, Value> = state.get("seen").and_then(Value::as_object).cloned().unwrap_or_default();
    let same_day = state.get("since").and_then(Value::as_f64) == Some(since);
    let mut today: Map<String, Value> = if same_day { state.get("today").and_then(Value::as_object).cloned().unwrap_or_default() } else { Map::new() };
    for r in readings {
        let usd = r.usd.unwrap_or(0.0);
        let before = seen.insert(r.key.clone(), json!({ "usd": xenonet_text::json::number(usd), "tokens": xenonet_text::json::number(r.tokens) }));
        // First sight is the baseline: what came before today isn't today's.
        let Some(before) = before else { continue };
        let d_usd = (usd - finite(before.get("usd")).unwrap_or(0.0)).max(0.0);
        let d_tokens = (r.tokens - finite(before.get("tokens")).unwrap_or(0.0)).max(0.0);
        if d_usd <= 0.0 && d_tokens <= 0.0 {
            continue;
        }
        let was = today.get(&r.key);
        let sum = |f: &str, d: f64| xenonet_text::json::number(was.and_then(|w| finite(w.get(f))).unwrap_or(0.0) + d);
        let mut item = Map::new();
        item.insert("key".into(), json!(r.key));
        item.insert("source".into(), json!(r.source));
        if let Some(a) = &r.actor {
            item.insert("actor".into(), json!(a));
        }
        if let Some(s) = &r.session {
            item.insert("session".into(), json!(s));
        }
        item.insert("usd".into(), sum("usd", d_usd));
        item.insert("tokens".into(), sum("tokens", d_tokens));
        today.insert(r.key, Value::Object(item));
    }
    let mut items: Vec<Value> = today.values().cloned().collect();
    xenonet_text::sort::sort_by(&mut items, |a, b| utf16_cmp(a["key"].as_str().unwrap_or(""), b["key"].as_str().unwrap_or("")));
    let out = items
        .iter()
        .map(|i| {
            let mut i = i.clone();
            if finite(i.get("usd")).unwrap_or(0.0) <= 0.0 {
                i["usd"] = Value::Null;
            }
            i
        })
        .collect();
    (json!({ "since": xenonet_text::json::number(since), "seen": seen, "today": today }), out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_go_to_their_actor_or_parent() {
        let sessions = vec![json!({ "id": "a" }), json!({ "id": "b", "parentId": "a", "parentKind": "subagent" }), json!({ "id": "c", "parentId": "a", "parentKind": "fork" })];
        let starts =
            vec![json!({ "actor": "actor:wren", "session": "a", "at": 1, "via": "registered" }), json!({ "actor": "actor:hermes", "session": "a", "at": 5, "via": "observed" })];
        let w = attribute(&sessions, &starts, None, None);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].1, Who { actor: "actor:hermes".into(), how: "started" });
        assert_eq!(w[1].1, Who { actor: "actor:hermes".into(), how: "parent" });
    }

    fn hermes(cost: Value, input: f64) -> Value {
        json!({ "id": "actor:hermes", "product": "hermes-agent", "usage": { "costUsd": cost, "tokens": { "input": input, "output": 0, "reasoning": 0, "cacheRead": 0, "cacheWrite": 0 } } })
    }

    fn opencode(id: &str, cost: Value, input: f64) -> Value {
        json!({ "id": id, "app": "opencode", "usage": { "costUsd": cost, "tokens": { "input": input, "output": 0, "reasoning": 0, "cacheRead": 0, "cacheWrite": 0 } } })
    }

    #[test]
    fn reported_counts_only_what_grows_after_first_sight() {
        let day = 1000.0;
        let (st, items) = reported_today(&Value::Null, day, &[hermes(json!(5.0), 100.0)], &[opencode("oc1", json!(2.0), 10.0)]);
        assert!(items.is_empty(), "first sight is the baseline");
        let (st, items) = reported_today(&st, day, &[hermes(json!(5.5), 150.0)], &[opencode("oc1", json!(2.0), 10.0), opencode("oc2", json!(0.25), 7.0)]);
        // oc2 first seen now: baseline too.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["key"], "actor:hermes");
        assert_eq!(items[0]["usd"], 0.5);
        assert_eq!(items[0]["tokens"], 50);
        assert_eq!(items[0]["source"], "Hermes");
        // Increases add up through the day, and a lower total adds nothing.
        let (st, items) = reported_today(&st, day, &[hermes(json!(5.75), 160.0)], &[opencode("oc1", json!(3.0), 30.0)]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["usd"], 0.75);
        assert_eq!(items[1]["key"], "oc1");
        assert_eq!(items[1]["usd"], 1);
        assert_eq!(items[1]["tokens"], 20);
        let (st, items) = reported_today(&st, day, &[hermes(json!(1.0), 5.0)], &[]);
        assert_eq!(items[0]["usd"], 0.75);
        // A new day starts from nothing, and keeps the baselines.
        let (_, items) = reported_today(&st, day + 1.0, &[hermes(json!(1.5), 15.0)], &[]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["usd"], 0.5);
        assert_eq!(items[0]["tokens"], 10);
    }

    #[test]
    fn unpriced_reports_add_tokens_and_no_dollars() {
        let (st, _) = reported_today(&Value::Null, 1.0, &[hermes(Value::Null, 10.0)], &[]);
        let (_, items) = reported_today(&st, 1.0, &[hermes(Value::Null, 40.0)], &[]);
        assert_eq!(items[0]["usd"], Value::Null);
        assert_eq!(items[0]["tokens"], 30);
    }

    #[test]
    fn reported_is_apart_from_the_estimate() {
        let names = vec![("actor:hermes".to_string(), json!("Hermes")), ("actor:wren".to_string(), json!("Wren"))];
        let sessions = vec![json!({ "id": "oc1", "app": "opencode" }), json!({ "id": "oc2", "app": "opencode" })];
        let starts = vec![json!({ "actor": "actor:wren", "session": "oc1", "at": 1, "via": "observed" })];
        let reported = vec![
            json!({ "key": "actor:hermes", "source": "Hermes", "actor": "actor:hermes", "usd": 0.5, "tokens": 50 }),
            json!({ "key": "oc1", "source": "OpenCode", "session": "oc1", "usd": 1, "tokens": 20 }),
            json!({ "key": "oc2", "source": "OpenCode", "session": "oc2", "usd": null, "tokens": 5 }),
            json!({ "key": "actor:ghost", "source": "Hermes", "actor": "actor:ghost", "usd": 9.0, "tokens": 9 }),
        ];
        let out = spend_by_actor(&[], &sessions, &starts, &names, None, &reported);
        assert_eq!(out["total"], 0.0);
        let rows = out["actors"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let row = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap();
        assert_eq!(row("actor:hermes")["cost"], 0.0);
        assert_eq!(row("actor:hermes")["reported"], json!({ "usd": 0.5, "tokens": 50, "source": "Hermes" }));
        assert_eq!(row("actor:wren")["reported"], json!({ "usd": 1, "tokens": 20, "source": "OpenCode" }));
        assert_eq!(out["unattributed"]["reported"], json!({ "usd": null, "tokens": 5, "source": "OpenCode" }));
        assert!(row("actor:wren").get("reported").is_some() && out["unattributed"]["cost"] == 0.0);
    }
}
