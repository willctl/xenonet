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
