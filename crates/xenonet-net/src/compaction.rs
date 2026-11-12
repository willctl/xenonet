//! When a machine compacts its own feed in a network
//! (docs/design/network.md, "Retention and compaction"): after
//! `AFTER` kept entries since its last compact entry, and on a look every
//! `EVERY_MS` for entries that aged out, whenever the compaction would
//! drop something. What stays live is the kinds' retention
//! (`xenonet_log::feed::live_set`); which entries of the kinds kept while
//! open still are is read from the feed itself (`open_set`).

use serde_json::Value;
use std::collections::HashSet;
use xenonet_log::feed::{FeedEntry, Kind};

/// Kept entries written since the last compact entry that make a
/// compaction due.
pub const AFTER: u64 = 256;

/// How often a machine looks at its feeds anyway, for entries whose
/// retention ran out while nothing was written.
pub const EVERY_MS: u64 = 6 * 60 * 60 * 1000;

fn text<'a>(b: &'a Value, k: &str) -> &'a str {
    b.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The places of `held` (this machine's kept entries in one incarnation,
/// in order) still open, of the kinds kept while open, as of `now` (ms):
///
/// - a claim, until a later claim or release by the same agent of the
///   same path in the same project (a release with no path lets all of the
///   agent's claims there go), or until it expires;
/// - a handoff offer, until a later entry of the feed names the same
///   handoff (taken or lapsed).
pub fn open_set(held: &[FeedEntry], now: u64) -> HashSet<u64> {
    let mut open = HashSet::new();
    for (i, e) in held.iter().enumerate() {
        let later = &held[i + 1..];
        let b = &e.body;
        let still = match e.kind {
            Kind::Claim if text(b, "op") == "claim" => {
                let (agent, project, path) = (text(b, "agent"), text(b, "project"), text(b, "path"));
                let expired = b.get("expiresAt").and_then(Value::as_u64).is_some_and(|t| t <= now);
                let ended = later.iter().any(|l| {
                    let lb = &l.body;
                    l.kind == Kind::Claim
                        && text(lb, "agent") == agent
                        && text(lb, "project") == project
                        && (text(lb, "path") == path || (text(lb, "op") == "release" && text(lb, "path").is_empty()))
                });
                !expired && !ended
            }
            Kind::Handoff if text(b, "op") == "offer" => {
                let id = text(b, "id");
                !later.iter().any(|l| l.kind == Kind::Handoff && text(&l.body, "id") == id)
            }
            _ => false,
        };
        if still {
            open.insert(e.seq);
        }
    }
    open
}

/// Whether a compaction of `held` keeping `live` drops anything worth a
/// compact entry: an entry of the range that isn't live, other than an
/// earlier compact entry, which a new one would only supersede.
pub fn drops(held: &[FeedEntry], live: &[u64]) -> bool {
    held.iter().any(|e| e.kind != Kind::Compact && live.binary_search(&e.seq).is_err())
}

/// The kept entries written since the last compact entry among `held`.
pub fn since_last(held: &[FeedEntry], kept: u64) -> u64 {
    let last = held.iter().rev().find(|e| e.kind == Kind::Compact).map_or(0, |e| e.seq);
    kept.saturating_sub(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
    use xenonet_log::feed::Writer;

    const T: u64 = 1_792_512_000_000;

    fn feed(bodies: &[(Kind, Value)]) -> Vec<FeedEntry> {
        let key = SeedSigner::from_seed(&[7; 32], KeyHome::File).unwrap();
        let mut w = Writer::start(key.public().id(), "net_00112233445566778899aabbccddeeff", T);
        bodies.iter().map(|(k, b)| w.write(&key, *k, T, b.clone()).unwrap()).collect()
    }

    fn claim(op: &str, agent: &str, path: &str) -> (Kind, Value) {
        (Kind::Claim, json!({ "op": op, "agent": agent, "project": "prj_a", "path": path }))
    }

    #[test]
    fn a_claim_is_open_until_its_release_and_a_handoff_until_its_end() {
        let es = feed(&[
            claim("claim", "agt_a", "src/"),
            claim("claim", "agt_a", "lib/"),
            claim("claim", "agt_b", "src/"),
            claim("release", "agt_a", "src/"),
            (Kind::Handoff, json!({ "id": "hof_1", "op": "offer" })),
            (Kind::Handoff, json!({ "id": "hof_2", "op": "offer" })),
            (Kind::Handoff, json!({ "id": "hof_1", "op": "taken" })),
            claim("release", "agt_b", ""),
        ]);
        let mut open: Vec<u64> = open_set(&es, T).into_iter().collect();
        open.sort_unstable();
        assert_eq!(open, [2, 6], "agt_a's lib/ and the second offer; a release with no path ends all of agt_b's");
        let expiring = feed(&[(Kind::Claim, json!({ "op": "claim", "agent": "agt_a", "project": "prj_a", "path": "src/", "expiresAt": T + 10 }))]);
        assert_eq!(open_set(&expiring, T).len(), 1);
        assert!(open_set(&expiring, T + 10).is_empty(), "expired");
    }

    #[test]
    fn a_compaction_is_due_when_it_drops_something() {
        let es = feed(&[claim("claim", "agt_a", "src/"), claim("release", "agt_a", "src/")]);
        assert!(drops(&es, &[]));
        assert!(!drops(&es, &[1, 2]));
        assert_eq!(since_last(&es, 2), 2);
    }
}
