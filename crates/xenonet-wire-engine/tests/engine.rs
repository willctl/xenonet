//! The engine as the binary drives it, on a made-up map (test/engine.test.js's
//! world): messages and the inbox that waits, the listening Stop hook and
//! its going away, the hooks' briefing and edit warnings, claims, the
//! policy's refusals and a Claude Code channel's poll.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use xenonet_service::config::Config;
use xenonet_service::cores::{Channels, Cores, Engine, State, R};
use xenonet_service::wire::{Emitter, WireEnv};
use xenonet_wire_engine::world::World;

const A: &str = "claude:aaaaaaaa-0001";
const B: &str = "codex:bbbbbbbb-0002";
const C: &str = "claude:cccccccc-0003";
const D: &str = "codex:dddddddd-0004";

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// A made-up session: its id, title, folder, last event, edits and status.
type Session = (&'static str, &'static str, &'static str, f64, Vec<(&'static str, f64)>, &'static str);

/// Sessions, their live view and where each lives, as the JS test's
/// stand-in for XenoNet's state has them.
struct Fake {
    at: f64,
}

impl Fake {
    fn sessions(&self) -> Vec<Session> {
        let t = self.at;
        vec![
            (A, "Add partial refunds", "/w/atlas", t - 30e3, vec![("/w/atlas/src/refunds.ts", t - 60e3)], "tool"),
            (B, "Rate-limit the API", "/w/atlas", t - 20e3, vec![("/w/atlas/src/routes.ts", t - 120e3)], "tool"),
            (C, "Fix the ledger test", "/w/ledger", t - 10e3, vec![], "tool"),
            (D, "Zoom on images", "/wt/atlas-zoom", t - 40e3, vec![("/wt/atlas-zoom/src/refunds.ts", t - 300e3)], "tool"),
        ]
    }
}

#[async_trait]
impl State for Fake {
    fn graph(&self) -> R<Option<Value>> {
        Ok(Some(json!({})))
    }
    fn graph_version(&self) -> u64 {
        1
    }
    fn nodes(&self) -> R<Vec<Value>> {
        let mut out = vec![
            json!({ "id": "project:atlas", "kind": "project", "name": "atlas", "path": "/w/atlas" }),
            json!({ "id": "project:ledger", "kind": "project", "name": "ledger", "path": "/w/ledger" }),
        ];
        for (id, title, cwd, at, _, _) in self.sessions() {
            out.push(json!({ "id": id, "kind": "session", "title": title, "cwd": cwd, "updatedAt": at }));
        }
        Ok(out)
    }
    fn node(&self, id: &str) -> R<Option<Value>> {
        Ok(self.nodes()?.into_iter().find(|n| n["id"] == id))
    }
    fn home_of(&self, id: &str) -> R<Value> {
        let cwd = self.node(id)?.and_then(|n| n["cwd"].as_str().map(str::to_string));
        Ok(match cwd.as_deref() {
            Some("/w/atlas" | "/wt/atlas-zoom") => json!("project:atlas"),
            Some("/w/ledger") => json!("project:ledger"),
            _ => Value::Null,
        })
    }
    fn live_ids(&self) -> R<Vec<(String, Value)>> {
        Ok(self.sessions().into_iter().map(|(id, _, _, at, _, _)| (id.to_string(), json!(at))).collect())
    }
    fn live_info(&self, id: &str, _now: f64) -> R<Option<Value>> {
        Ok(self.sessions().into_iter().find(|s| s.0 == id).map(|(_, _, _, at, edits, status)| {
            json!({ "lastEventAt": at, "endedAt": null, "status": status, "needsYou": null, "edits": edits.iter().map(|(p, ts)| json!({ "path": p, "ts": ts })).collect::<Vec<_>>() })
        }))
    }
    fn observed_live(&self, _id: &str) -> R<bool> {
        Ok(false)
    }
    fn identities(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
}

/// The area's parts on the made-up map, with its files in a folder of the
/// test's own and actors off.
fn world(name: &str) -> (Arc<World>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xenonet-wire-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp folder");
    let home = dir.to_string_lossy().into_owned();
    let state = dir.join("state").to_string_lossy().into_owned();
    let vars = [("HOME", home.as_str()), ("XENONET_STATE_DIR", state.as_str()), ("XENONET_CACHE_DIR", state.as_str()), ("XENONET_ACTORS", "off"), ("XENONET_PORT", "47998")];
    let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
    let mut config = Config::from(&[], &get);
    let mut cores = Cores::pending(false, false, false);
    cores.state = Arc::new(Fake { at: now() });
    let env = WireEnv { version: "0.0.0".into(), emitter: Emitter::new(), drift: Default::default() };
    let w = xenonet_wire_engine::world(&cores, &mut config, &env).expect("the engine's parts");
    (w, dir)
}

fn engine(w: &Arc<World>) -> xenonet_wire_engine::engine::EngineDriver {
    xenonet_wire_engine::engine::EngineDriver(w.clone())
}

fn ids(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v.as_array().into_iter().flatten().filter_map(|a| a["id"].as_str().map(str::to_string)).collect();
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_to_one_agent_to_the_repo_replies_and_an_inbox_that_waits() {
    let (w, dir) = world("messages");
    let e = engine(&w);
    let to_repo = e.send(A, json!({ "to": "repo", "text": "I'm moving refunds into its own module." }), None).await.unwrap();
    assert_eq!(ids(&to_repo["to"]), vec![B.to_string(), D.to_string()]);
    assert!(to_repo["to"][0]["delivery"].as_str().unwrap().contains("isn't connected"), "{to_repo}");
    let got = e.inbox(B, Duration::ZERO).await.unwrap();
    assert_eq!(got.as_array().unwrap().len(), 1);
    assert_eq!((got[0]["from"]["id"].clone(), got[0]["to"].clone()), (json!(A), json!("repo:atlas")));
    assert_eq!(e.inbox(B, Duration::ZERO).await.unwrap(), json!([]), "read once");
    // A reply goes back to the sender without naming it.
    let reply = e.send(B, json!({ "replyTo": got[0]["id"], "text": "OK, I'll stay out of src/refunds.ts." }), None).await.unwrap();
    assert_eq!(ids(&reply["to"]), vec![A.to_string()]);
    // By words from the title, and an inbox that waits for mail.
    let waiting = {
        let e = engine(&w);
        tokio::spawn(async move { e.inbox(C, Duration::from_secs(5)).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    e.send(A, json!({ "to": "ledger test", "text": "Is the ledger fixture stable now?" }), None).await.unwrap();
    let got = waiting.await.unwrap().unwrap();
    assert_eq!(got[0]["text"], "Is the ledger fixture stable now?");
    assert!(started.elapsed() < Duration::from_secs(3), "the wait ended when the message came");
    assert_eq!(e.inbox(A, Duration::ZERO).await.unwrap().as_array().unwrap().len(), 1, "the reply");
    let none = e.send(A, json!({ "to": "nobody here", "text": "hi" }), None).await.unwrap_err();
    assert!(none.message.contains("no session or actor matches"), "{}", none.message);
    assert!(e.send(A, json!({ "to": B, "text": "" }), None).await.unwrap_err().message.contains("empty"));
    // The same message twice, at once: the policy's refusal, 429.
    e.send(A, json!({ "to": C, "text": "twice" }), None).await.unwrap();
    let again = e.send(A, json!({ "to": C, "text": "twice" }), None).await.unwrap_err();
    assert_eq!((again.status, again.code.as_deref()), (Some(429), Some("duplicate")), "{}", again.message);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listening_stop_hook_hands_mail_over_and_stops_when_the_hook_goes() {
    let (w, dir) = world("listen");
    let e = engine(&w);
    // The hook goes away: the wait ends at once, with nothing taken.
    let (gone_tx, gone_rx) = tokio::sync::watch::channel(false);
    let listening = {
        let e = engine(&w);
        tokio::spawn(async move { e.listen(C, Duration::from_secs(30), gone_rx).await })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(e.listening().unwrap().iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(), vec![C]);
    let started = Instant::now();
    drop(gone_tx);
    assert_eq!(listening.await.unwrap().unwrap(), None);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(e.listening().unwrap().is_empty());
    // A message while it listens: handed over as a block, the session's
    // turn going on.
    let (_keep, gone_rx) = tokio::sync::watch::channel(false);
    let listening = {
        let e = engine(&w);
        tokio::spawn(async move { e.listen(C, Duration::from_secs(10), gone_rx).await })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    e.send(A, json!({ "to": C, "text": "the fixture moved" }), None).await.unwrap();
    let out = listening.await.unwrap().unwrap().expect("the message");
    assert_eq!(out["decision"], "block");
    assert!(out["reason"].as_str().unwrap().contains("the fixture moved"));
    assert_eq!(out["ids"].as_array().unwrap().len(), 1);
    // A hook that went before the answer leaves the message unread.
    e.mark_unseen(C, &out["ids"]).unwrap();
    assert_eq!(e.inbox(C, Duration::ZERO).await.unwrap().as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn delivery_records_a_message_handed_to_a_listening_hook_and_the_pause_switch_holds_the_rest() {
    let (w, dir) = world("delivery");
    let e = engine(&w);
    let router = xenonet_wire_engine::delivery::RouterDriver(w.clone());
    let policy = xenonet_wire_engine::delivery::PolicyDriver(w.clone());
    let (_keep, gone_rx) = tokio::sync::watch::channel(false);
    let listening = {
        let e = engine(&w);
        tokio::spawn(async move { e.listen(C, Duration::from_secs(10), gone_rx).await })
    };
    tokio::time::sleep(Duration::from_millis(150)).await;
    let sent = e.send(A, json!({ "to": C, "text": "ping" }), None).await.unwrap();
    assert_eq!(sent["to"][0]["delivery"], "right away: it's listening for messages", "{sent}");
    listening.await.unwrap().unwrap().expect("handed over");
    // The router's record, once its route has run.
    let mut recent = Value::Null;
    for _ in 0..50 {
        recent = router.list(50).unwrap();
        if recent.as_array().is_some_and(|l| !l.is_empty()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!((recent[0]["agent"].clone(), recent[0]["state"].clone(), recent[0]["how"].clone()), (json!(C), json!("live"), json!("listen")), "{recent}");
    // Paused: kept in delivery.json, and a session's message refused.
    use xenonet_service::cores::{Policy, Router};
    policy.set_paused(true).unwrap();
    assert!(policy.paused().unwrap());
    let kept = std::fs::read_to_string(dir.join("state").join("delivery.json")).unwrap();
    assert!(kept.contains("\"paused\": true"), "{kept}");
    let held = e.send(A, json!({ "to": C, "text": "while paused" }), None).await.unwrap_err();
    assert_eq!((held.status, held.code.as_deref()), (Some(429), Some("paused")));
    assert_eq!(policy.read_audit(10.0).unwrap().as_array().unwrap().last().unwrap()["kind"], "refused");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hooks_brief_a_starting_session_and_warn_before_an_edit() {
    let (w, dir) = world("hooks");
    let e = engine(&w);
    let start = e.hook("claude", Some("SessionStart"), &json!({ "session_id": "aaaaaaaa-0001", "cwd": "/w/atlas" })).await.unwrap().expect("a briefing");
    let said = start["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(said.contains("Rate-limit the API") && said.contains(&format!("You are {A}")), "{said}");
    let warn = e
        .hook(
            "claude",
            Some("PreToolUse"),
            &json!({ "session_id": "aaaaaaaa-0001", "cwd": "/w/atlas", "tool_name": "Edit", "tool_input": { "file_path": "/w/atlas/src/routes.ts" } }),
        )
        .await
        .unwrap()
        .expect("a warning");
    let said = warn["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
    assert!(said.starts_with("XenoNet: before you edit routes.ts") && said.contains(B), "{said}");
    // Once is enough for the same file and session.
    assert_eq!(
        e.hook("claude", Some("PreToolUse"), &json!({ "session_id": "aaaaaaaa-0001", "tool_name": "Edit", "tool_input": { "file_path": "/w/atlas/src/routes.ts" } }))
            .await
            .unwrap(),
        None
    );
    // A claim, and who has the file.
    let claimed = e.claim(C, json!({ "paths": ["/w/atlas/docs/"], "note": "rewriting the docs" })).await.unwrap();
    assert_eq!(claimed["claimed"], json!(["/w/atlas/docs/"]));
    let who = e.who_has(Some(A), "/w/atlas/docs/guide.md").await.unwrap();
    assert_eq!((who[0]["agent"]["id"].clone(), who[0]["how"].clone()), (json!(C), json!("claimed")));
    assert_eq!(e.release(C, Value::Null).await.unwrap(), json!({ "released": 1 }));
    // SessionEnd: the session's contact ends.
    assert_eq!(e.hook("claude", Some("SessionEnd"), &json!({ "session_id": "aaaaaaaa-0001" })).await.unwrap(), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_channel_poll_waits_for_its_session_and_ends_empty_at_its_time() {
    let (w, dir) = world("channel");
    let channels = xenonet_wire_engine::delivery::ChannelsDriver(w.clone());
    // No poll yet: nothing to hand over to.
    assert!(!w.channels.push(A, xenonet_engine::sql::Arg::text("early")));
    let polling = {
        let w = w.clone();
        tokio::spawn(async move { xenonet_wire_engine::delivery::ChannelsDriver(w).poll(A, Duration::from_secs(10)).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(w.channels.live(A));
    assert!(w.channels.push(A, xenonet_engine::sql::Arg::text("hello")));
    assert_eq!(polling.await.unwrap().unwrap(), json!(["hello"]));
    // Just polled, so still live: a push waits for the next poll.
    assert!(w.channels.push(A, xenonet_engine::sql::Arg::text("next")));
    assert_eq!(channels.poll(A, Duration::from_secs(1)).await.unwrap(), json!(["next"]));
    let started = Instant::now();
    assert_eq!(channels.poll(A, Duration::from_millis(200)).await.unwrap(), json!([]));
    assert!(started.elapsed() >= Duration::from_millis(150));
    std::fs::remove_dir_all(dir).unwrap();
}
