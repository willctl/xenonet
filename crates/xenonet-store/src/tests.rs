use super::*;
use xenonet_id::sign::{KeyHome, SeedSigner};
use xenonet_log::person::{self, PersonChain, PersonPolicy, SALT_BYTES};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xenonet-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn root(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

fn found(n: u8) -> person::Entry {
    person::found_salted(&root(n), [n; SALT_BYTES], 1_792_000_000_000).unwrap()
}

#[test]
fn a_new_folder_gets_a_store_with_the_schemas_version() {
    let dir = folder("new");
    assert_eq!(standing(&dir), Standing::None);
    let db = Db::open(&dir).unwrap();
    assert_eq!(db.path(), Some(dir.join(FILE).as_path()));
    assert_eq!(db.stored().unwrap(), 0);
    let mode: String = db.read(|c| Ok(c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)).unwrap();
    assert_eq!(mode, "wal");
    drop(db);
    assert_eq!(standing(&dir), Standing::Ready);
    let c = Connection::open(dir.join(FILE)).unwrap();
    assert_eq!(found_version(&c).unwrap(), schema::version());
    assert!(Db::open(&dir).is_ok(), "and opens again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn another_version_is_left_as_it_is_and_refused() {
    let dir = folder("other");
    {
        let c = Connection::open(dir.join(FILE)).unwrap();
        c.execute_batch("CREATE TABLE kept (x); INSERT INTO kept VALUES (7); PRAGMA user_version = 3;").unwrap();
    }
    let e = Db::open(&dir).unwrap_err();
    assert!(e.set_up_again() && e.message.contains("set this machine up again"), "{e}");
    assert!(matches!(standing(&dir), Standing::SetUpAgain(_)));
    let c = Connection::open(dir.join(FILE)).unwrap();
    assert_eq!((found_version(&c).unwrap(), c.query_row("SELECT x FROM kept", [], |r| r.get::<_, i64>(0)).unwrap()), (3, 7), "untouched");
    // A database with tables and no version is another XenoNet's too.
    let dir2 = folder("unversioned");
    Connection::open(dir2.join(FILE)).unwrap().execute_batch("CREATE TABLE t (x)").unwrap();
    assert!(Db::open(&dir2).unwrap_err().set_up_again());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn an_old_engine_sqlite_is_left_as_it_is_and_refused() {
    let dir = folder("engine");
    std::fs::write(dir.join(OLD_ENGINE), b"what an earlier build kept").unwrap();
    let e = Db::open(&dir).unwrap_err();
    assert_eq!(e.code, "set-up-again");
    assert!(e.message.contains(OLD_ENGINE), "{e}");
    assert!(matches!(standing(&dir), Standing::SetUpAgain(why) if why.contains("set this machine up again")));
    assert!(!dir.join(FILE).exists(), "no store made beside it");
    assert_eq!(std::fs::read(dir.join(OLD_ENGINE)).unwrap(), b"what an earlier build kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_version_is_the_schemas_hash() {
    assert_eq!(schema::version(), schema::version());
    assert!(schema::version() > 0);
}

#[test]
fn every_entry_takes_the_next_number_in_one_order() {
    let db = Db::memory().unwrap();
    let (a, b) = (found(1), found(2));
    assert_eq!(db.put_entry("person:a", &a).unwrap(), Put::Stored(1));
    assert_eq!(db.put_entry("person:b", &b).unwrap(), Put::Stored(2), "one order across chains");
    assert_eq!(db.put_entry("person:a", &a).unwrap(), Put::Had(1), "the same entry again");
    assert_eq!(db.put_entry("person:a", &b).unwrap_err().code, "conflict", "another at that height");
    assert_eq!(db.stored().unwrap(), 2, "nothing counted for what wasn't stored");
    assert_eq!((db.entry_stored("person:b", 0).unwrap(), db.entry_stored("person:b", 1).unwrap()), (Some(2), None));
}

#[test]
fn a_chain_is_rebuilt_from_its_stored_entries() {
    let dir = folder("chain");
    let mut chain = PersonChain::new();
    chain.offer(found(5)).unwrap();
    {
        let db = Db::open(&dir).unwrap();
        for e in chain.entries() {
            db.put_entry("person:x", e).unwrap();
        }
    }
    let db = Db::open(&dir).unwrap();
    let back: Vec<_> = db.entries::<PersonPolicy>("person:x").unwrap();
    assert_eq!(back.iter().map(|(_, s)| *s).collect::<Vec<_>>(), [1]);
    let rebuilt = PersonChain::from_entries(back.into_iter().map(|(e, _)| e).collect()).unwrap();
    assert_eq!((rebuilt.id(), rebuilt.head()), (chain.id(), chain.head()));
    assert_eq!(rebuilt.person().unwrap().salt, [5; SALT_BYTES]);
    assert_eq!(db.entries::<PersonPolicy>("person:nobody").unwrap().len(), 0, "no such chain");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn another_threads_call_waits_for_a_transaction_to_end() {
    let db = std::sync::Arc::new(Db::memory().unwrap());
    let other = db.clone();
    let (ready, go) = std::sync::mpsc::channel();
    let out: Result<(), sql::SqlError> = db.transaction(|| {
        db.write(|c| Ok(c.execute_batch("CREATE TABLE t (x)")?)).map_err(|e| sql::SqlError::Other(e.message))?;
        // Another thread's write, started while this transaction is open,
        // isn't part of it: it waits, and lands after the rollback.
        let h = std::thread::spawn(move || {
            ready.send(()).unwrap();
            other.write(|c| Ok(c.execute_batch("CREATE TABLE u (x)")?)).unwrap();
        });
        go.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(db.in_transaction() && !h.is_finished(), "the other write waits");
        Err(sql::SqlError::Other("give up".into()))
    });
    assert!(out.is_err());
    // The other thread's table is made once the transaction is gone.
    for _ in 0..100 {
        if db.read(|c| Ok(c.query_row("SELECT count(*) FROM sqlite_schema WHERE name = 'u'", [], |r| r.get::<_, i64>(0))?)).unwrap() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let names: Vec<String> = db
        .read(|c| {
            let mut q = c.prepare("SELECT name FROM sqlite_schema WHERE name IN ('t', 'u') ORDER BY name")?;
            let rows = q.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(names, ["u"], "the rolled back transaction's table is gone, the other thread's stays");
    assert!(!db.in_transaction());
}

#[test]
fn a_network_and_what_goes_with_it_are_forgotten_together() {
    use net::{Gateway, Network};
    let db = Db::memory().unwrap();
    let n = Network {
        id: "net_a".into(),
        name: "home".into(),
        host: "laptop".into(),
        founder: "laptop".into(),
        gateway: Gateway { host: "laptop".into(), key_id: "k".into(), sign: "s".into(), seal: "x".into(), address: None },
        lan: None,
        titles_only: false,
        share_load: true,
        joined: 5,
    };
    db.put_link_status("net_a", &serde_json::json!({ "state": "live" })).unwrap();
    assert_eq!(db.link_status("net_a").unwrap(), None, "nothing kept for a network this machine isn't in");
    db.put_network(&n).unwrap();
    db.put_network(&Network { name: "house".into(), ..n.clone() }).unwrap();
    assert_eq!(db.networks().unwrap().iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["house"], "one row a network");
    db.put_link_status("net_a", &serde_json::json!({ "state": "live" })).unwrap();
    db.put_services("net_a", &[serde_json::json!({ "name": "b" }), serde_json::json!({ "name": "a" })]).unwrap();
    assert_eq!(db.services("net_a").unwrap()[0]["name"], "b", "in the order published");
    db.forget_network("net_a").unwrap();
    assert!(db.networks().unwrap().is_empty() && db.link_status("net_a").unwrap().is_none() && db.services("net_a").unwrap().is_empty());
}

#[test]
fn a_new_own_person_takes_the_place_of_the_one_before() {
    let db = Db::memory().unwrap();
    let (a, b) = (found(1), found(2));
    db.put_person("prs_a", true, std::slice::from_ref(&a)).unwrap();
    db.put_person("prs_x", false, std::slice::from_ref(&b)).unwrap();
    db.put_person("prs_a", true, std::slice::from_ref(&a)).unwrap();
    assert_eq!(db.stored().unwrap(), 2, "an entry held is kept once");
    db.put_person("prs_b", true, std::slice::from_ref(&b)).unwrap();
    let persons: Vec<(String, bool)> = db.persons().unwrap().into_iter().map(|p| (p.id, p.own)).collect();
    assert_eq!(persons, [("prs_b".to_string(), true), ("prs_x".to_string(), false)]);
    assert!(db.person::<PersonPolicy>("prs_a").unwrap().is_empty(), "the old own chain goes");
    db.forget_others().unwrap();
    assert_eq!(db.persons().unwrap().len(), 1);
    assert_eq!(db.person::<PersonPolicy>("prs_b").unwrap().len(), 1);
}

#[test]
fn what_you_saved_keeps_the_order_it_was_saved_in() {
    let db = Db::memory().unwrap();
    db.set_layout(&[("b".into(), Some([1.0, 2.0, 3.5])), ("a".into(), Some([0.0, 0.0, 0.0]))]).unwrap();
    db.set_layout(&[("b".into(), Some([4.0, 4.0, 4.0]))]).unwrap();
    assert_eq!(xenonet_text::json::stringify(&db.layout().unwrap(), 0), r#"{"b":{"x":4,"y":4,"z":4},"a":{"x":0,"y":0,"z":0}}"#, "a place moved keeps its turn");
    db.set_layout(&[("b".into(), None)]).unwrap();
    assert_eq!(db.layout().unwrap().as_object().unwrap().len(), 1);
    db.set_setting("theme", Some(&serde_json::json!("rime"))).unwrap();
    assert_eq!((db.setting("theme").unwrap(), db.setting("environment").unwrap()), (serde_json::json!("rime"), serde_json::Value::Null));
    db.set_app_setting("sorter", "picks", "/p", Some(&serde_json::json!({ "picks": [] }))).unwrap();
    assert_eq!(db.app_setting("sorter", "picks", "/p").unwrap(), Some(serde_json::json!({ "picks": [] })));
    db.set_identity_name("git:a", Some("work")).unwrap();
    db.set_identity_pin("claude:x", Some(("git:a", 5.0))).unwrap();
    db.set_signins("an-app", &serde_json::json!([{ "id": "claude:x", "from": 1, "to": 2 }])).unwrap();
    let ids = db.identities().unwrap();
    assert_eq!(ids["names"]["git:a"], "work");
    assert_eq!(ids["pins"]["claude:x"], serde_json::json!({ "identity": "git:a", "at": 5 }));
    assert_eq!(ids["signins"]["an-app"][0]["to"], 2);
}

#[test]
fn delivery_keeps_its_policy_runs_and_delegations_across_a_restart() {
    use delivery::{Delegated, RunRow};
    let dir = folder("delivery");
    {
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.delivery_policy().unwrap(), None, "nothing kept yet");
        let data = serde_json::json!({ "paused": true, "consent": { "claude:a": { "until": 5 } } });
        db.put_delivery_policy(data.as_object().unwrap()).unwrap();
        db.put_wake_budget(&[("w:actor:wren".into(), vec![1.0, 2.0])]).unwrap();
        db.put_runs(&[RunRow { id: "run_1".into(), key: "claude:a".into(), app: "an-app".into(), pid: Some(7.0), at: 1.0, end: None, exit: None, ms: None, timed_out: false }])
            .unwrap();
        db.put_delegated("claude:a", &Delegated { cwd: "/p".into(), from: Some("studio".into()), at: 10.0 }, 100.0).unwrap();
        db.put_delegated("claude:b", &Delegated { cwd: "/q".into(), from: None, at: 500.0 }, 100.0).unwrap();
        for i in 0..delivery::AUDIT_LINES + 2 {
            db.add_delivery_audit(&serde_json::json!({ "i": i })).unwrap();
        }
    }
    let db = Db::open(&dir).unwrap();
    let kept = db.delivery_policy().unwrap().unwrap();
    assert_eq!((kept["paused"].clone(), kept["consent"]["claude:a"]["until"].clone()), (serde_json::json!(true), serde_json::json!(5)));
    assert_eq!(db.wake_budget().unwrap(), [("w:actor:wren".to_string(), vec![1.0, 2.0])]);
    assert_eq!(db.runs().unwrap()[0].pid, Some(7.0));
    assert_eq!(db.delegated("claude:a").unwrap(), None, "a delegation older than what's kept goes");
    assert_eq!(db.delegated("claude:b").unwrap().map(|d| d.cwd), Some("/q".into()));
    let audit = db.delivery_audit().unwrap();
    assert_eq!((audit.len() as i64, audit[0]["i"].clone()), (delivery::AUDIT_LINES, serde_json::json!(2)), "the oldest lines go");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_store_is_the_owners_alone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder("mode");
    let db = Db::open(&dir).unwrap();
    db.set_setting("theme", Some(&serde_json::json!("rime"))).unwrap();
    assert_eq!(std::fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(dir.join(format!("{FILE}-wal"))).unwrap().permissions().mode() & 0o777, 0o600, "and its write-ahead log");
    let _ = std::fs::remove_dir_all(&dir);
}
