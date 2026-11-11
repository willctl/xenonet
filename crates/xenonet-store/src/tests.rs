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
    db.put_sharing("net_b", &serde_json::json!("all")).unwrap();
    assert_eq!(db.sharing("net_b").unwrap(), None, "nothing shared into a network this machine isn't in");
    db.put_sharing("net_a", &serde_json::json!(["prj_web"])).unwrap();
    assert_eq!(db.sharing("net_a").unwrap(), Some(serde_json::json!(["prj_web"])));
    db.forget_network("net_a").unwrap();
    assert!(db.networks().unwrap().is_empty() && db.link_status("net_a").unwrap().is_none() && db.services("net_a").unwrap().is_empty());
    assert_eq!(db.sharing("net_a").unwrap(), None);
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
        db.put_delegated("claude:a", &Delegated { cwd: "/p".into(), from: Some("studio".into()), at: 10.0, intent: None }, 100.0).unwrap();
        db.put_delegated("claude:b", &Delegated { cwd: "/q".into(), from: None, at: 500.0, intent: Some(("int_a".into(), "dlg_a".into())) }, 100.0).unwrap();
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
    assert_eq!(db.delegated("claude:b").unwrap().map(|d| (d.cwd, d.intent)), Some(("/q".into(), Some(("int_a".into(), "dlg_a".into())))));
    assert_eq!(db.intent_run("int_a").unwrap().as_deref(), Some("claude:b"), "the run an intent started");
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

mod feeds {
    use super::*;
    use crate::feed::WriterAt;
    use xenonet_id::sign::Signer;
    use xenonet_log::feed::{Feed, Kind, Offer, Pos, Verified, Writer};

    const NET: &str = "net_00112233445566778899aabbccddeeff";

    fn author() -> SeedSigner {
        root(9)
    }

    fn entries(n: u64) -> Vec<Verified> {
        let k = author();
        let mut w = Writer::start(k.public().id(), NET, 1_792_000_000_000);
        (0..n).map(|i| w.write(&k, Kind::Handoff, 1_792_000_000_000 + i, serde_json::json!({ "n": i })).unwrap().verify(&k.public()).unwrap()).collect()
    }

    fn seqs(o: &Offer) -> Vec<u64> {
        match o {
            Offer::New(es) => es.iter().map(|e| e.seq).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_feed_over_the_store_applies_each_entry_once_in_order() {
        let db = Db::memory().unwrap();
        let es = entries(3);
        let id = author().public().id();
        let mut f = Feed::new(db.feeds());
        assert!(matches!(f.offer(es[1].clone()).unwrap(), Offer::Gap(_)));
        assert_eq!(db.feed_held_count(NET).unwrap(), 1, "held, in the store");
        assert_eq!(seqs(&f.offer(es[0].clone()).unwrap()), [1, 2], "the gap filled");
        assert_eq!(f.offer(es[0].clone()).unwrap(), Offer::Duplicate);
        assert_eq!(seqs(&f.offer(es[2].clone()).unwrap()), [3]);
        assert_eq!(f.ack(&id, NET).unwrap().map(|a| (a.inc, a.high)), Some((es[0].inc, 3)));
        assert_eq!(db.stored().unwrap(), 3, "each applied entry has its number in the store's order");
        // A fork freezes the feed, which the store keeps until unfrozen.
        let k = author();
        let mut other = es[2].entry().clone();
        other.body = serde_json::json!({ "n": "another" });
        other.sign(&k).unwrap();
        assert!(matches!(f.offer(other.verify(&k.public()).unwrap()).unwrap(), Offer::Fork(_)));
        assert_eq!(db.frozen_feeds(NET).unwrap().len(), 1);
        assert!(matches!(Feed::new(db.feeds()).offer(es[2].clone()).unwrap(), Offer::Frozen(_)), "a feed over the same store sees it frozen");
        Feed::new(db.feeds()).unfreeze(&id, NET).unwrap();
        assert_eq!(db.frozen_feeds(NET).unwrap(), []);
    }

    #[test]
    fn the_entries_applied_are_kept_as_signed_to_serve_as_a_holder() {
        let db = Db::memory().unwrap();
        let es = entries(4);
        let id = author().public().id();
        let mut f = Feed::new(db.feeds());
        for e in &es {
            f.offer(e.clone()).unwrap();
        }
        let from = db.feed_in_from(&id, NET, Pos::new(es[0].inc, 2), 2).unwrap();
        assert_eq!(from.iter().map(|(e, open)| (e.seq, *open)).collect::<Vec<_>>(), [(2, true), (3, true)]);
        assert_eq!(from[0].0, *es[1].entry(), "as its author signed it");
        assert_eq!(db.feed_in_from(&id, NET, Pos::new(es[0].inc, 5), 10).unwrap().len(), 0, "nothing past the head");
    }

    #[test]
    fn what_a_transaction_applies_goes_with_it_or_not_at_all() {
        let db = Db::memory().unwrap();
        let es = entries(1);
        let failed: Result<(), sql::SqlError> = db.transaction(|| {
            let o = Feed::new(db.feeds()).offer(es[0].clone()).map_err(|e| sql::SqlError::from(rusqlite::Error::InvalidParameterName(e.message)))?;
            assert_eq!(seqs(&o), [1]);
            Err(sql::SqlError::from(rusqlite::Error::InvalidQuery))
        });
        assert!(failed.is_err());
        assert_eq!(seqs(&Feed::new(db.feeds()).offer(es[0].clone()).unwrap()), [1], "rolled back, so it applies again");
    }

    #[test]
    fn the_author_keeps_its_incarnation_and_kept_entries() {
        let db = Db::memory().unwrap();
        let k = author();
        let mut w = Writer::start(k.public().id(), NET, 1_792_000_000_000);
        let e = w.write(&k, Kind::Handoff, 1, serde_json::json!({})).unwrap();
        let at = WriterAt { inc: w.inc(), kept: 1, loose: 0, link: w.link(), head: w.head() };
        db.put_feed_writer(NET, &k.public().id(), at, Some(&e)).unwrap();
        assert_eq!(db.feed_writer(NET).unwrap(), Some(at));
        let back = Writer::resume(k.public().id(), NET, at.inc, at.kept, at.loose, at.link, at.head);
        assert_eq!(back, w, "the writer comes back as it was");
        assert_eq!(db.feed_out_after(NET, Pos::ORIGIN, 10).unwrap(), std::slice::from_ref(&e));
        assert_eq!(db.feed_out_after(NET, e.pos(), 10).unwrap(), []);
        db.put_feed_ack(NET, "laptop", e.pos()).unwrap();
        db.put_feed_ack(NET, "laptop", Pos::ORIGIN).unwrap();
        assert_eq!(db.feed_ack(NET, "laptop").unwrap(), Some(e.pos()), "acks only move on");
    }

    /// The author writes through the store, as the network does.
    fn put(db: &Db, w: &Writer, e: &xenonet_log::feed::FeedEntry) {
        let at = WriterAt { inc: w.inc(), kept: w.kept(), loose: w.loose(), link: w.link(), head: w.head() };
        db.put_feed_writer(NET, &author().public().id(), at, Some(e)).unwrap();
    }

    #[test]
    fn a_compact_entry_drops_what_it_doesnt_keep_and_answers_a_member_behind() {
        let (author_db, db) = (Db::memory().unwrap(), Db::memory().unwrap());
        let k = author();
        let t = 1_792_000_000_000;
        let mut w = Writer::start(k.public().id(), NET, t);
        let mut f = Feed::at(db.feeds(), t + 10);
        let mut written = Vec::new();
        for (i, path) in ["kept", "released", "held"].into_iter().enumerate() {
            let e = w.write(&k, Kind::Claim, t + i as u64, serde_json::json!({ "path": path })).unwrap();
            put(&author_db, &w, &e);
            written.push(e);
        }
        // The member applied the first and holds the third behind a gap.
        assert_eq!(seqs(&f.offer(written[0].clone().verify(&k.public()).unwrap()).unwrap()), [1]);
        assert!(matches!(f.offer(written[2].clone().verify(&k.public()).unwrap()).unwrap(), Offer::Gap(_)));
        let c = w.compact(&k, t + 5, &[1]).unwrap();
        put(&author_db, &w, &c);
        assert_eq!(author_db.feed_out_after(NET, Pos::ORIGIN, 10).unwrap(), [written[0].clone(), c.clone()], "the rest are dropped");
        assert_eq!(author_db.feed_compacted(NET, c.pos()).unwrap(), None, "nothing compacted past it");
        let answer = author_db.feed_compacted(NET, written[0].pos()).unwrap().unwrap();
        assert_eq!((answer.compact.clone(), answer.live.len(), answer.headers.len()), (c.clone(), 0, 2));
        let o = f.offer_compacted(answer.verify(&k.public()).unwrap()).unwrap();
        let Offer::New(applied) = o else { panic!("{o:?}") };
        assert_eq!((applied.entries.len(), applied.ended.clone()), (1, vec![]), "only the compact entry: the released claim was never applied here");
        assert_eq!(db.feed_held_count(NET).unwrap(), 0, "the held claim is dropped");
        assert_eq!(f.offer(written[1].clone().verify(&k.public()).unwrap()).unwrap(), Offer::Duplicate, "the chain's id is kept for the place it skipped");
        let id = k.public().id();
        let open: Vec<Pos> = xenonet_log::feed::FeedStore::open_upto(&db.feeds(), &id, NET, c.pos()).unwrap().into_iter().map(|o| o.at).collect();
        assert_eq!(open, [written[0].pos(), c.pos()]);
        // A new member gets the whole range: the live claim and three headers.
        let fresh = author_db.feed_compacted(NET, Pos::ORIGIN).unwrap().unwrap();
        assert_eq!((fresh.live, fresh.headers.len()), (vec![written[0].clone()], 3));
    }

    #[test]
    fn loose_records_are_pruned_past_their_dedupe_period() {
        use xenonet_log::feed::LOOSE_DEDUPE_MS;
        let db = Db::memory().unwrap();
        let k = author();
        let t = 1_792_000_000_000;
        let mut w = Writer::start(k.public().id(), NET, t);
        let old = w.write(&k, Kind::Message, t, serde_json::json!({ "text": "old" })).unwrap();
        assert_eq!(seqs(&Feed::at(db.feeds(), t).offer(old.clone().verify(&k.public()).unwrap()).unwrap()), [1]);
        let new = w.write(&k, Kind::Message, t + LOOSE_DEDUPE_MS, serde_json::json!({ "text": "new" })).unwrap();
        let mut later = Feed::at(db.feeds(), t + LOOSE_DEDUPE_MS + 1);
        assert_eq!(seqs(&later.offer(new.clone().verify(&k.public()).unwrap()).unwrap()), [2]);
        assert_eq!(later.offer(old.verify(&k.public()).unwrap()).unwrap(), Offer::Expired, "refused by its age once its record is gone");
        assert_eq!(later.offer(new.verify(&k.public()).unwrap()).unwrap(), Offer::Duplicate);
        let rows: i64 = db.read(|c| Ok(c.query_row("SELECT count(*) FROM feed_loose", [], |r| r.get(0))?)).unwrap();
        assert_eq!(rows, 1);
    }
}
