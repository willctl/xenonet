//! The state's orchestration on the demo's made-up world, as test/state.test.js
//! and test/server.test.js check it of the JS: what a refresh puts on the
//! map and says, refreshes asked for together, renaming, the projects
//! folder's rules and changes, identities, the live state's simulations,
//! handoffs, the live snapshot's throttle and the nudge to commit.

use super::*;
use crate::config::{process_env, MapConfig};
use std::sync::atomic::AtomicU16;

static PORTS: AtomicU16 = AtomicU16::new(0);

struct World {
    state: Arc<MapState>,
    root: String,
    home: String,
}

impl Drop for World {
    fn drop(&mut self) {
        self.state.end();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// A demo world of its own, on a port no other test uses.
fn world() -> World {
    let port = 52000 + (std::process::id() % 900) as u16 * 10 + PORTS.fetch_add(1, Ordering::SeqCst);
    let demo = Arc::new(DemoWorld::new(port));
    let w = demo.create().expect("the demo's world");
    let home = std::env::var("HOME").unwrap_or_default();
    let cfg = MapConfig::new(true, false, process_env(), &home, Some(&w));
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let vendor = Vendor::new(dir, Arc::new(Mutex::new(xenonet_schema::drift::DriftLog::default())));
    let store = StateStore::new(&cfg.state_dir, &cfg.projects_dir);
    let codex_sessions = std::path::Path::new(&cfg.codex_home).join("sessions").to_string_lossy().into_owned();
    let live = Live::new(&cfg.claude_projects_dir, &codex_sessions, "", cfg.claude_desktop_dir.as_deref(), false, &cfg.home_dir, vendor.clone());
    let codex = CodexClient::new("/nonexistent/codex".into(), "0.0.0".into(), vendor.clone());
    let state = MapState::new(cfg, store, vendor, Emitter::new(), Readers::Demo(demo), live, codex, "0.0.0");
    let root = w["root"].as_str().unwrap_or_default().to_string();
    World { state, root, home: w["home"].as_str().unwrap_or_default().to_string() }
}

const SESSION1: &str = "claude:0190d3e0-0000-7000-8000-000000000001";

fn sent(s: &MapState, name: &str) -> Vec<Value> {
    lock(&s.sent).iter().filter(|(n, _)| n == name).map(|(_, v)| v.clone()).collect()
}

fn clear(s: &MapState) {
    lock(&s.sent).clear();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refresh_maps_the_world_and_says_so_once() {
    let w = world();
    let s = &w.state;
    let g = s.refresh_map().await.expect("a map");
    let nodes = items(g.get("nodes"));
    assert!(nodes.iter().any(|n| s_of(n, "id") == Some(SESSION1) && truthy(n.get("actions"))), "the demo's first session, with what it offers");
    assert!(nodes.iter().any(|n| s_of(n, "kind") == Some("project") && s_of(n, "name") == Some("atlas-api")));
    assert_eq!(g["demo"], true);
    let names: Vec<&str> = items(g.get("sources")).iter().filter_map(|x| s_of(x, "name")).collect();
    assert_eq!(names, ["projects", "codex", "codex-projects", "claude", "live", "wiki", "search", "identities"]);
    assert!(items(g.get("sources")).iter().all(|x| x["ok"] == true || s_of(x, "name") == Some("live") || s_of(x, "name") == Some("search")), "{}", g["sources"]);
    assert!(!items(g.get("identities")).is_empty(), "who the demo's work ran as");
    assert_eq!(sent(s, "graph").len(), 1, "a new map goes out");
    // Nothing changed: no map goes out again, and nothing pulses.
    clear(s);
    s.refresh_map().await.expect("a map");
    assert!(sent(s, "graph").is_empty() && sent(s, "pulse").is_empty());
    assert_eq!(s.graph_version(), 2);
    // A session written to since pulses.
    let file = s.stats.files(SESSION1)[0].0.clone();
    std::thread::sleep(Duration::from_millis(20));
    let mut f = std::fs::OpenOptions::new().append(true).open(&file).expect("its transcript");
    std::io::Write::write_all(&mut f, b"{}\n").expect("a line");
    s.refresh_map().await.expect("a map");
    assert_eq!(sent(s, "pulse")[0], json!({ "nodeId": SESSION1, "reason": "updated" }));
    // The index follows the map.
    assert_eq!(s.node_of(SESSION1).and_then(|n| n.get("title").cloned()), Some(json!("Add partial refunds to the orders API")));
    assert!(s.home(SESSION1).is_some_and(|h| h.starts_with("project:")), "a session lives in its project");
    let _ = &w.home;
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshes_asked_for_together_are_one() {
    let w = world();
    let s = &w.state;
    let (a, b) = tokio::join!(s.refresh_map(), s.refresh_map());
    assert_eq!(a.expect("a map")["generatedAt"], b.expect("a map")["generatedAt"], "the same refresh");
    assert_eq!(lock(&s.claude_sessions.state).runs(), 1.0);
    // One after a change sees it, whatever was running.
    let (_, c) = tokio::join!(s.refresh_map(), s.refresh_after());
    c.expect("a map");
    assert_eq!(lock(&s.claude_sessions.state).runs(), 3.0);
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_demo_session_keeps_the_name_here() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    clear(s);
    let done = s.rename_session(json!(SESSION1), json!("  Refunds,\n partial\u{7}  ")).await.expect("renamed");
    assert_eq!(done, json!({ "id": SESSION1, "title": "Refunds, partial", "saved": [], "note": null, "demo": true }));
    assert!(sent(s, "graph").iter().any(|g| items(g.get("nodes")).iter().any(|n| s_of(n, "title") == Some("Refunds, partial"))), "the map goes out with it");
    assert_eq!(s.rename_session(json!(SESSION1), json!(" \n\t ")).await.map_err(|e| e.status).err(), Some(Some(400)));
    assert_eq!(s.rename_session(json!("claude:nobody"), json!("x")).await.map_err(|e| e.status).err(), Some(Some(404)));
    assert_eq!(s.rename_session(json!("core"), json!("x")).await.map_err(|e| e.status).err(), Some(Some(404)), "only a session has a name to change");
    let long = s.rename_session(json!(SESSION1), json!("x".repeat(500))).await.expect("renamed");
    assert_eq!(long["title"].as_str().map(str::len), Some(120));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_projects_folders_rules_a_fresh_scan_and_codexs_pass() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    let d = s.project_detection().await.expect("detection");
    assert_eq!(d["scan"]["ok"], true);
    let labs = std::path::Path::new(d["root"].as_str().unwrap_or_default()).join("labs").to_string_lossy().into_owned();
    let ruled = s.set_project_rule(json!(labs), json!("project")).await.expect("a rule");
    assert_eq!(ruled["rules"][0]["as"], "project");
    assert_eq!(s.set_project_rule(json!(labs), json!("planet")).await.map_err(|e| e.message).err().as_deref(), Some("unknown rule"));
    assert!(s.set_project_rule(json!("/tmp"), json!("project")).await.is_err(), "only folders in the projects folder");
    assert_eq!(s.clear_project_rules(false).await.expect("cleared")["rules"], json!([]));
    assert_eq!(s.rescan_projects().await.expect("scanned")["scan"]["ok"], true);
    let outline = s.codex_outline().await.expect("an outline");
    assert_ne!(items(outline.get("outline")), &[] as &[Value]);
    let stale = s.ask_codex_about_projects(json!("stale"), false).await.map_err(|e| e.status).err();
    assert_eq!(stale, Some(Some(409)));
    let asked = s.ask_codex_about_projects(outline["fingerprint"].clone(), false).await.expect("picks");
    assert_eq!((asked["codex"]["picks"][0]["path"].as_str(), asked["codex"]["stale"].as_bool()), (Some("labs/snippets"), Some(false)));
    assert_eq!(s.forget_codex_picks().await.expect("forgotten")["codex"], Value::Null);
}

#[tokio::test(flavor = "multi_thread")]
async fn another_projects_folder_and_back() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    let before = s.root_info().expect("the root");
    assert_eq!((before["source"].as_str(), before["scan"]["ok"].as_bool()), (Some("default"), Some(true)));
    let check = s.check_root(json!("~/Clients")).await.expect("a check");
    assert_eq!(
        (check["ok"].as_bool(), check["current"].as_bool(), check["scan"]["projects"].as_u64(), check["scan"]["groups"].as_u64()),
        (Some(true), Some(false), Some(4), Some(1))
    );
    assert!(s.check_root(json!("~")).await.expect("a check")["error"].as_str().is_some_and(|e| e.contains("your home folder")));
    assert_eq!(s.set_root(json!("~/nowhere")).await.map_err(|e| e.status).err(), Some(Some(400)));
    let moved = s.set_root(json!("~/Clients")).await.expect("moved");
    assert_eq!((moved["source"].as_str(), moved["changed"].as_bool(), moved["scan"]["projects"].as_u64()), (Some("chosen"), Some(true), Some(4)));
    let core = s.node_of("core").expect("the core");
    assert_eq!(core["name"], "~/Clients", "the map follows at once");
    let back = s.set_root(Value::Null).await.expect("back");
    assert_eq!((back["source"].as_str(), back["changed"].as_bool(), back["path"].clone()), (Some("default"), Some(true), before["path"].clone()));
}

// A second pass over repos already read, which once took a gatherer's lock
// while it held it, and hung the service a minute after it started. A
// deadlock leaves its runtime nothing to time out with, so the passes run
// on a runtime of their own, and the test waits from outside it.
#[test]
fn identities_read_again_without_waiting_on_themselves() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("a runtime");
        rt.block_on(async {
            let w = world();
            let s = &w.state;
            s.refresh_map().await.expect("a map");
            for _ in 0..3 {
                s.refresh_repos().await;
            }
            let _ = tx.send(!items(s.identity_detection().expect("identities").get("identities")).is_empty());
        });
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(60)), Ok(true), "the repos' passes never ended, or found no identity");
}

// The repos' pass while the blocks' health, the identities and the map are
// asked for from other threads, as /api/health, the doctor and a refresh
// ask for them: one lock order everywhere, or this never ends.
#[test]
fn identities_read_while_their_health_and_the_map_are_asked_for() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
        rt.block_on(async {
            let w = world();
            let s = w.state.clone();
            s.refresh_map().await.expect("a map");
            let stop = Arc::new(AtomicBool::new(false));
            let mut askers = Vec::new();
            for _ in 0..3 {
                let (s, stop) = (s.clone(), stop.clone());
                askers.push(tokio::task::spawn_blocking(move || {
                    while !stop.load(Ordering::SeqCst) {
                        let _ = s.blocks();
                        let _ = s.identity_detection();
                    }
                }));
            }
            let mapper = {
                let s = s.clone();
                tokio::spawn(async move {
                    for _ in 0..5 {
                        let _ = s.refresh_map().await;
                    }
                })
            };
            for _ in 0..20 {
                s.refresh_repos().await;
            }
            let _ = mapper.await;
            stop.store(true, Ordering::SeqCst);
            for a in askers {
                let _ = a.await;
            }
            let _ = tx.send(true);
        });
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(90)), Ok(true), "the passes and the asks never ended");
}

#[tokio::test(flavor = "multi_thread")]
async fn identities_named_hidden_pinned_and_found_again() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    let d = s.identity_detection().expect("identities");
    assert_eq!(d["enabled"], true);
    let id = d["identities"][0]["id"].clone();
    assert!(s.name_identity(json!("github:nowhere/nobody"), json!("X")).await.is_err());
    let named = s.name_identity(id.clone(), json!(" Work\u{7} ")).await.expect("named");
    assert_eq!(items(named.get("identities")).iter().find(|i| i["id"] == id).map(|i| i["name"].clone()), Some(json!("Work")));
    let hidden = s.hide_identity(id.clone(), json!(true)).await.expect("hidden");
    assert_eq!(items(hidden.get("identities")).iter().find(|i| i["id"] == id).map(|i| i["hidden"].clone()), Some(json!(true)));
    let pinned = s.pin_identity(json!(SESSION1), id.clone()).await.expect("pinned");
    assert_eq!(items(pinned.get("pins")).iter().find(|p| p["node"] == SESSION1).map(|p| p["identity"].clone()), Some(id.clone()));
    assert!(s.pin_identity(json!("core"), id.clone()).await.is_err(), "only a session or a project");
    assert_eq!(s.pin_identity(json!(SESSION1), Value::Null).await.expect("unpinned")["pins"], json!([]));
    assert_ne!(items(s.rescan_identities().await.expect("again").get("identities")), &[] as &[Value]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_consoles_simulations_and_clear() {
    let w = world();
    let s = &w.state;
    s.live.start().await;
    s.refresh_map().await.expect("a map");
    for kind in ["needs-you", "collision", "stuck", "turn-end", "wiki-write"] {
        assert_eq!(s.simulate_live(kind, None, None).map(|d| d["kind"].clone()), Some(json!(kind)), "{kind}");
        let live = s.snapshot();
        if kind == "needs-you" {
            assert!(items(live.get("sessions")).iter().any(|x| x["needsYou"]["text"].as_str().is_some_and(|t| t.starts_with("Should I run the migration"))));
        }
        if kind == "stuck" {
            assert!(items(live.get("sessions")).iter().any(|x| items(x["stuck"].get("reasons")).iter().any(|r| r == "4 failed tool calls in a row")), "{}", live["sessions"]);
        }
        if kind == "collision" {
            assert_ne!(items(live.get("collisions")), &[] as &[Value]);
        }
    }
    assert_eq!(s.simulate_live("clear", None, None), Some(json!({ "ok": true, "kind": "clear" })));
    assert!(!items(s.snapshot().get("sessions")).iter().any(|x| x["needsYou"]["text"].as_str().is_some_and(|t| t.starts_with("Should I run the migration"))));
    assert!(s.simulate_live("earthquake", None, None).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handoff_brief_from_what_the_tailer_saw() {
    let w = world();
    let s = &w.state;
    s.live.start().await;
    s.refresh_map().await.expect("a map");
    let h = s.handoff(Some(SESSION1), "codex").await.expect("a brief");
    assert!(h["brief"].as_str().is_some_and(|b| b.contains("Continue the work from a Claude Code session titled \"Add partial refunds")), "{h}");
    assert!(h["folder"].as_str().is_some_and(|f| f.ends_with("atlas-api")));
    assert_eq!(s.handoff(Some("core"), "codex").await.map_err(|e| e.message).err().as_deref(), Some("unknown session"));
    let t = s.read_transcript(&s.node_of(SESSION1).expect("its node"), None, 20.0).await.expect("a transcript");
    assert_ne!(items(t.get("entries")), &[] as &[Value]);
}

#[tokio::test(flavor = "multi_thread")]
async fn activity_goes_out_and_the_live_state_after_it_at_most_twice_a_second() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    s.live.start().await;
    s.listening.store(true, Ordering::SeqCst);
    let weak = Arc::downgrade(s);
    s.live.listen(Arc::new(move |events| {
        if let Some(s) = weak.upgrade() {
            s.heard(events);
        }
    }));
    clear(s);
    let id = s.live.ids().into_iter().map(|(i, _)| i).find(|i| i == SESSION1).expect("a live session");
    for _ in 0..3 {
        s.live.activity(&id, vec![("kind".into(), Some(json!("tool"))), ("tool".into(), Some(json!("Bash"))), ("ts".into(), Some(num(now_ms())))]);
    }
    assert_eq!(sent(s, "activity").len(), 3);
    tokio::time::sleep(Duration::from_millis(LIVE_THROTTLE_MS + 300)).await;
    assert_eq!(sent(s, "live").len(), 1, "one snapshot for the three");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_ends_with_uncommitted_work_is_nudged_once() {
    let w = world();
    let s = &w.state;
    s.refresh_map().await.expect("a map");
    s.repo_pass().await;
    s.refresh_map().await.expect("a map");
    let home = s.home(SESSION1).expect("a home");
    assert!(s.node_of(&home).is_some_and(|n| truthy(n["repo"].get("dirty"))), "atlas-api has uncommitted work");
    clear(s);
    let ev = json!({ "kind": "turn-end", "sessionId": SESSION1, "app": "claude", "ts": num(now_ms()) });
    s.maybe_ship_it(&ev);
    s.maybe_ship_it(&ev);
    let nudges: Vec<Value> = sent(s, "activity").into_iter().filter(|a| a["kind"] == "ship-it").collect();
    assert_eq!(nudges.len(), 1, "at most every 30 minutes a project");
    assert!(nudges[0]["detail"].as_str().is_some_and(|d| d.contains("uncommitted file") && d.ends_with("atlas-api")));
    s.maybe_ship_it(&json!({ "kind": "turn-end", "sessionId": SESSION1, "interrupted": true }));
    assert_eq!(sent(s, "activity").len(), 1, "not a turn you stopped");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_actors_report_are_sessions_on_the_map() {
    let w = world();
    let s = &w.state;
    let runs = RunsCore(s.clone());
    let r = runs.report("actor:wren", json!({ "run": "sbx_1", "app": "claude", "title": "Running the tests" }));
    assert_eq!(r["ok"], true);
    let id = r["id"].as_str().unwrap_or_default().to_string();
    assert_eq!(runs.report("actor:wren", json!({ "run": "../x", "app": "claude" }))["ok"], false);
    s.refresh_map().await.expect("a map");
    let n = s.node_of(&id).expect("on the map");
    assert_eq!((n["observed"].as_str(), n["reportedBy"].as_str()), (Some("gateway"), Some("actor:wren")));
    assert!(s.observed_live(&id).expect("observed"));
    assert_eq!(s.live_info(&id, now_ms()).expect("its info").map(|i| i["status"].clone()), Some(json!("working")));
}

#[test]
fn a_page_written_in_the_wiki_changes_its_stamp() {
    let dir = std::env::temp_dir().join(format!("xenonet-wire-wiki-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("pages")).expect("a folder");
    let d = dir.to_string_lossy().into_owned();
    let before = wiki_stamp(&d);
    std::fs::write(dir.join("pages").join("atlas.md"), "# Atlas\n").expect("a page");
    let after = wiki_stamp(&d);
    assert_ne!(before, after);
    assert_eq!(after, wiki_stamp(&d), "the same until it changes");
    std::fs::remove_dir_all(&dir).expect("cleaned up");
}

// A made-up home folder, not the demo: a project with a repo, Claude
// Code's transcript of a session in it, and its sign-in.
#[tokio::test(flavor = "multi_thread")]
async fn a_home_folders_own_sessions_and_projects() {
    let home = std::env::temp_dir().join(format!("xenonet-wire-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let p = home.join("Projects").join("atlas");
    std::fs::create_dir_all(p.join("src")).expect("a project");
    std::fs::write(p.join("package.json"), "{}").expect("its package");
    let home_s = std::fs::canonicalize(&home).expect("the home").to_string_lossy().into_owned();
    let cwd = std::path::Path::new(&home_s).join("Projects").join("atlas").to_string_lossy().into_owned();
    let id = "0190d3e0-0000-7000-8000-00000000aaaa";
    let folder = home.join(".claude").join("projects").join(cwd.replace(|c: char| !c.is_ascii_alphanumeric(), "-"));
    std::fs::create_dir_all(&folder).expect("Claude Code's folder");
    let ts = |ago: f64| xenonet_base::time::iso(now_ms() - ago);
    let lines = [
        json!({ "type": "user", "uuid": "u1", "parentUuid": null, "sessionId": id, "cwd": cwd, "gitBranch": "main", "version": "2.0.1", "timestamp": ts(60e3), "message": { "role": "user", "content": "Add a health check route" } }),
        json!({ "type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": id, "cwd": cwd, "version": "2.0.1", "timestamp": ts(50e3), "message": { "id": "m1", "role": "assistant", "model": "claude-sonnet-4-5", "stop_reason": "end_turn", "usage": { "input_tokens": 10, "output_tokens": 20 }, "content": [{ "type": "text", "text": "Added GET /health." }] } }),
    ];
    let text: String = lines.iter().map(|l| xenonet_base::json::stringify(l, 0) + "\n").collect();
    std::fs::write(folder.join(format!("{id}.jsonl")), text).expect("a transcript");
    let mut env = process_env();
    env.insert("XENONET_CODEX_BIN".into(), json!("/nonexistent/codex"));
    env.insert("XENONET_CLAUDE_USAGE".into(), json!("off"));
    env.insert("XENONET_CLAUDE_DESKTOP_DIR".into(), json!(home.join("no-desktop").to_string_lossy()));
    env.insert("XENONET_STATE_DIR".into(), json!(home.join(".state").to_string_lossy()));
    env.insert("XENONET_CACHE_DIR".into(), json!(home.join(".cache").to_string_lossy()));
    env.insert("XENONET_WIKI_DIR".into(), json!(home.join("no-wiki").to_string_lossy()));
    env.insert("HOME".into(), json!(home_s));
    for k in ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "XENONET_PROJECTS_DIR", "XENONET_READERS", "XENONET_NUMBAT_FILE", "GH_CONFIG_DIR", "XDG_CONFIG_HOME"] {
        env.shift_remove(k);
    }
    let cfg = MapConfig::new(false, false, env, &home_s, None);
    assert!(!cfg.demo && cfg.claude_projects_dir.starts_with(&home_s), "nothing of yours is read");
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let vendor = Vendor::new(dir, Arc::new(Mutex::new(xenonet_schema::drift::DriftLog::default())));
    let store = StateStore::new(&cfg.state_dir, &cfg.projects_dir);
    let live = Live::new(&cfg.claude_projects_dir, "", "", None, false, &cfg.home_dir, vendor.clone());
    let codex = CodexClient::new(cfg.codex_binary(), "0.0.0".into(), vendor.clone());
    let claude = crate::claude::Claude::new(&cfg.claude_projects_dir, None, vendor.clone());
    let s = MapState::new(cfg, store, vendor, Emitter::new(), Readers::Real { claude }, live, codex, "0.0.0");
    let g = s.refresh_map().await.expect("a map");
    let session = items(g.get("nodes")).iter().find(|n| s_of(n, "id") == Some(&format!("claude:{id}"))).cloned().expect("the session, on the map");
    assert_eq!((session["title"].as_str(), session["cwd"].as_str()), (Some("Add a health check route"), Some(cwd.as_str())));
    assert!(s.home(&format!("claude:{id}")).is_some_and(|h| h.ends_with("/atlas")), "in its project");
    let st = |name: &str| items(g.get("sources")).iter().find(|x| s_of(x, "name") == Some(name)).cloned().unwrap_or(Value::Null);
    assert_eq!((st("claude")["ok"].as_bool(), st("claude")["count"].as_u64()), (Some(true), Some(1)));
    assert_eq!(st("codex")["error"], "spawn /nonexistent/codex ENOENT", "no Codex here, said as Node says it");
    let t = s.read_transcript(&session, None, 120.0).await.expect("its transcript");
    assert_eq!(t["entries"][0]["text"], "Add a health check route");
    let renamed = s.rename_session(json!(format!("claude:{id}")), json!("Health check")).await.expect("renamed");
    assert_eq!((renamed["title"].as_str(), renamed["saved"].clone()), (Some("Health check"), json!(["Claude Code"])));
    s.end();
    std::fs::remove_dir_all(&home).expect("cleaned up");
}
