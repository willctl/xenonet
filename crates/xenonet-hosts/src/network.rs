//! The relay over a XenoNet network (docs/design/network.md, section 7;
//! `network/relay/v1`): a window host's console subscribes to another
//! host's map, and that host answers with the relay's own frames, a list
//! of them in each body, sealed between the two hosts as every host frame
//! is. This has the bodies, their checks and a subscription's order; the
//! service's relay and the hub do the rest.
//!
//! - From the subscribing host: `{ op, sub, … }`. `open` starts it, with a
//!   hello and a snapshot; `get` is one read from the relay's allowlist,
//!   as GET, and nothing else; `time` asks the host's clock; `renew` says
//!   the console still watches (every RENEW_MS; it lapses after LAPSE_MS
//!   without one), with `fresh` for a new snapshot; `close` ends it.
//! - Back: `{ sub, seq, frames }`, at most one every FRAME_MS for each
//!   subscription. `seq` counts 1, 2, 3… within it, so a hole says frames
//!   were lost, and the console asks for a fresh snapshot.

use crate::frames::{shallow, FROM_RELAY, MAX_DEPTH};
use serde_json::{json, Value};

pub const SCHEMA: &str = "network/relay/v1";
/// How often a console renews its subscription, and how long a host keeps
/// one that isn't renewed.
pub const RENEW_MS: f64 = 20e3;
pub const LAPSE_MS: f64 = 60e3;
/// At most one body for each subscription this often: the map at most
/// once a second and the live state every half second within it, other
/// events gathered into it.
pub const FRAME_MS: f64 = 500.0;
pub const GRAPH_MS: f64 = 1000.0;
pub const LIVE_MS: f64 = 500.0;
/// Frames in one body, at most.
pub const MAX_FRAMES: usize = 1000;
const MAX_PATH: usize = 2048;

/// What a subscribing host asks.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Open,
    Get { id: u64, path: String },
    Time { id: u64 },
    Renew { fresh: bool },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub sub: String,
    pub op: Op,
}

/// A subscription's id: sub_ and 32 hex digits, made at random.
pub fn is_sub(s: &str) -> bool {
    s.strip_prefix("sub_").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Whether a body is a subscriber's ask, not a host's answer.
pub fn is_ask(v: &Value) -> bool {
    v.get("op").is_some()
}

/// An ask's body.
pub fn ask(sub: &str, op: &Op) -> Value {
    let mut v = json!({ "schema": SCHEMA, "sub": sub });
    match op {
        Op::Open => v["op"] = json!("open"),
        Op::Get { id, path } => {
            v["op"] = json!("get");
            v["id"] = json!(id);
            v["path"] = json!(path);
        }
        Op::Time { id } => {
            v["op"] = json!("time");
            v["id"] = json!(id);
        }
        Op::Renew { fresh } => {
            v["op"] = json!("renew");
            if *fresh {
                v["fresh"] = json!(true);
            }
        }
        Op::Close => v["op"] = json!("close"),
    }
    v
}

fn id_of(v: &Value) -> Result<u64, String> {
    v.get("id").and_then(Value::as_u64).filter(|n| (1..=9_007_199_254_740_991).contains(n)).ok_or_else(|| "an ask with no id".to_string())
}

/// An ask, checked: the schema, a subscription id, an op the relay knows,
/// and what that op takes. A get is a read and nothing else: one that says
/// it's another method is refused, and its path is checked again against
/// the allowlist where it's read.
pub fn parse_ask(v: &Value) -> Result<Ask, String> {
    if !v.is_object() || v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err("not a relay body".into());
    }
    let sub = v.get("sub").and_then(Value::as_str).filter(|s| is_sub(s)).ok_or("an ask with no subscription")?.to_string();
    if v.get("method").is_some_and(|m| m.as_str() != Some("GET")) {
        return Err("the relay only reads, with GET".into());
    }
    let op = match v.get("op").and_then(Value::as_str) {
        Some("open") => Op::Open,
        Some("get") => {
            let path = v.get("path").and_then(Value::as_str).filter(|p| !p.is_empty() && p.len() <= MAX_PATH).ok_or("a get with no path")?;
            Op::Get { id: id_of(v)?, path: path.to_string() }
        }
        Some("time") => Op::Time { id: id_of(v)? },
        Some("renew") => Op::Renew { fresh: v.get("fresh") == Some(&Value::Bool(true)) },
        Some("close") => Op::Close,
        _ => return Err("an op the relay doesn't know".into()),
    };
    Ok(Ask { sub, op })
}

/// A host's answer: the relay's frames for one subscription, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub sub: String,
    pub seq: u64,
    pub frames: Vec<Value>,
}

pub fn batch(sub: &str, seq: u64, frames: &[Value]) -> Value {
    json!({ "schema": SCHEMA, "sub": sub, "seq": seq, "frames": frames })
}

/// An answer, checked: the schema, a subscription id, a seq from 1, and
/// at most MAX_FRAMES frames, each an object of a kind a relay sends,
/// nested no deeper than a relay's frame may be. The link checks each
/// frame again, as it checks a line from ssh.
pub fn parse_batch(v: &Value) -> Result<Batch, String> {
    if !v.is_object() || v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err("not a relay body".into());
    }
    let sub = v.get("sub").and_then(Value::as_str).filter(|s| is_sub(s)).ok_or("an answer with no subscription")?.to_string();
    let seq = v.get("seq").and_then(Value::as_u64).filter(|n| (1..=9_007_199_254_740_991).contains(n)).ok_or("an answer with no seq")?;
    let list = v.get("frames").and_then(Value::as_array).ok_or("an answer with no frames")?;
    if list.len() > MAX_FRAMES {
        return Err(format!("an answer with more than {MAX_FRAMES} frames"));
    }
    let mut frames = Vec::with_capacity(list.len());
    for f in list {
        let known = f.get("t").and_then(Value::as_str).is_some_and(|t| FROM_RELAY.contains(&t));
        if !known || !shallow(f, MAX_DEPTH) {
            return Err("a frame a relay doesn't send".into());
        }
        frames.push(f.clone());
    }
    Ok(Batch { sub, seq, frames })
}

/// Where a body's seq stands against the last one taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// The next one: take it.
    Next,
    /// One or more were lost before it: take it, and ask for a fresh
    /// snapshot.
    Hole,
    /// One taken already, or older: drop it.
    Old,
}

pub fn order(last: u64, seq: u64) -> Order {
    if seq <= last {
        Order::Old
    } else if seq == last + 1 {
        Order::Next
    } else {
        Order::Hole
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUB: &str = "sub_0123456789abcdef0123456789abcdef";

    #[test]
    fn asks_round_trip_and_only_reads_pass() {
        for op in [Op::Open, Op::Get { id: 3, path: "/api/graph".into() }, Op::Time { id: 4 }, Op::Renew { fresh: true }, Op::Renew { fresh: false }, Op::Close] {
            let v = ask(SUB, &op);
            assert!(is_ask(&v));
            assert_eq!(parse_ask(&v).unwrap(), Ask { sub: SUB.into(), op });
        }
        let mut post = ask(SUB, &Op::Get { id: 1, path: "/api/open".into() });
        post["method"] = json!("POST");
        assert_eq!(parse_ask(&post).unwrap_err(), "the relay only reads, with GET");
        for bad in [
            json!({ "schema": SCHEMA, "op": "post", "sub": SUB, "id": 1, "path": "/api/open" }),
            json!({ "schema": SCHEMA, "op": "get", "sub": SUB, "path": "/api/graph" }),
            json!({ "schema": SCHEMA, "op": "get", "sub": SUB, "id": 0, "path": "/api/graph" }),
            json!({ "schema": SCHEMA, "op": "get", "sub": SUB, "id": 1, "path": "" }),
            json!({ "schema": SCHEMA, "op": "open", "sub": "sub_x" }),
            json!({ "schema": "network/presence/v1", "op": "open", "sub": SUB }),
            json!([]),
        ] {
            assert!(parse_ask(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn answers_carry_known_frames_in_order() {
        let v = batch(SUB, 2, &[json!({ "t": "ping", "now": 1 }), json!({ "t": "event", "name": "live", "data": {} })]);
        assert!(!is_ask(&v));
        let b = parse_batch(&v).unwrap();
        assert_eq!((b.seq, b.frames.len()), (2, 2));
        for bad in [batch(SUB, 0, &[]), batch(SUB, 1, &[json!({ "t": "exec" })]), batch(SUB, 1, &[json!("ping")]), batch("sub_1", 1, &[])] {
            assert!(parse_batch(&bad).is_err(), "{bad}");
        }
        let deep = (0..100).fold(json!(1), |v, _| json!([v]));
        assert!(parse_batch(&batch(SUB, 1, &[json!({ "t": "event", "data": deep })])).is_err());
        let many: Vec<Value> = (0..=MAX_FRAMES).map(|_| json!({ "t": "ping" })).collect();
        assert!(parse_batch(&batch(SUB, 1, &many)).is_err());
        assert_eq!([order(0, 1), order(1, 3), order(3, 3), order(3, 2)], [Order::Next, Order::Hole, Order::Old, Order::Old]);
    }
}
