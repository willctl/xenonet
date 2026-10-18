//! A SQLite database with a version (core/blocks/sqlite.js): PRAGMA
//! user_version counts the migration steps taken, and each step runs in a
//! transaction of its own with the version it reaches. A database a newer
//! XenoNet has moved further is left at its version. With `backup`, a
//! database about to be migrated is copied first (VACUUM INTO
//! `<name>.v<n>.sqlite`), and the copy is deleted a week later: it holds what
//! the database itself keeps only so long (agents' messages, actors'
//! events), so it doesn't keep them for good.

use crate::lifecycle::Lifecycle;
use crate::store::free_name;
use rusqlite::Connection;
use std::path::Path;

pub const WEEK_MS: f64 = 7.0 * 24.0 * 3600e3;

/// A migration step: a function of the database. Send, so a store can be
/// used from a machine's thread.
pub type Step = Box<dyn Fn(&Connection) -> rusqlite::Result<()> + Send>;

/// Where a database's version started, where it is, and whether a newer
/// XenoNet moved it further than this one knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Version {
    pub from: i64,
    pub version: i64,
    pub newer: bool,
}

fn user_version(db: &Connection) -> rusqlite::Result<i64> {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// Takes `db` from its version up to the last of `steps`, each step in a
/// transaction of its own with the version it reaches.
pub fn migrate_sqlite(db: &Connection, steps: &[Step]) -> rusqlite::Result<Version> {
    let from = user_version(db)?;
    let last = steps.len() as i64;
    if from > last {
        return Ok(Version { from, version: from, newer: true });
    }
    for v in from..last {
        db.execute_batch("BEGIN")?;
        let step = steps[v as usize](db).and_then(|()| db.execute_batch(&format!("PRAGMA user_version = {}", v + 1)));
        match step {
            Ok(()) => db.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = db.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    Ok(Version { from, version: last, newer: false })
}

pub struct SqliteStore {
    pub name: String,
    pub file: String,
    steps: Vec<Step>,
    pragmas: Vec<String>,
    backup: bool,
    pub keep_backup_ms: f64,
    pub db: Option<Connection>,
    pub version: Option<Version>,
    pub backup_file: Option<String>,
    pub life: Lifecycle,
}

fn split_name(file: &str) -> (String, String) {
    let p = Path::new(file);
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (name.strip_suffix(&ext).unwrap_or(&name).to_string(), ext)
}

impl SqliteStore {
    pub fn new(name: &str, file: &str, steps: Vec<Step>, pragmas: Vec<String>, backup: bool, now: f64) -> Self {
        SqliteStore { name: name.into(), file: file.into(), steps, pragmas, backup, keep_backup_ms: WEEK_MS, db: None, version: None, backup_file: None, life: Lifecycle::new(now) }
    }

    /// Opens the database and brings it up to date.
    pub fn open(&mut self, now: f64) -> Result<&Connection, String> {
        // Taken and put back: a borrow of it, returned from here, would
        // outlast the rest of the function, which changes self.
        if let Some(db) = self.db.take() {
            return Ok(self.db.insert(db));
        }
        let opened = (|| -> Result<(Connection, Version), String> {
            if let Some(dir) = Path::new(&self.file).parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let db = Connection::open(&self.file).map_err(|e| e.to_string())?;
            for p in &self.pragmas {
                db.execute_batch(&format!("PRAGMA {p}")).map_err(|e| e.to_string())?;
            }
            if self.backup {
                self.prune_backups(now);
                self.copy_first(&db, now).map_err(|e| e.to_string())?;
            }
            let v = migrate_sqlite(&db, &self.steps).map_err(|e| e.to_string())?;
            Ok((db, v))
        })();
        match opened {
            Err(e) => {
                // A connection that failed to migrate is closed as it drops,
                // so Windows can remove or reopen the file.
                self.life.fail(&e, now);
                Err(e)
            }
            Ok((db, v)) => {
                self.life.count("opens", 1.0);
                if v.newer {
                    let base = Path::new(&self.file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.life.problem(&format!("{base} is from a newer XenoNet (version {}), so this one leaves its shape as it is", v.version), now);
                } else {
                    self.life.ok(now);
                }
                self.version = Some(v);
                Ok(self.db.insert(db))
            }
        }
    }

    // A copy of a database with data in it, before its first step up.
    fn copy_first(&mut self, db: &Connection, now: f64) -> rusqlite::Result<()> {
        let from = user_version(db)?;
        let tables: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))?;
        if from >= self.steps.len() as i64 || (from == 0 && tables == 0) {
            return Ok(());
        }
        let (stem, ext) = split_name(&self.file);
        let target = free_name(&self.file, &format!("{stem}.v{from}"), now, &ext);
        db.execute_batch(&format!("VACUUM INTO '{}'", target.replace('\'', "''")))?;
        self.backup_file = Some(target);
        Ok(())
    }

    /// Deletes the copies made before migrations (`<name>.v<n>.sqlite`, or
    /// `<name>.v<n>-<time>.sqlite` when that name was taken) that are
    /// `keep_backup_ms` old, by when they were written. Returns the files
    /// deleted.
    pub fn prune_backups(&mut self, now: f64) -> Vec<String> {
        let (stem, ext) = split_name(&self.file);
        let dir = Path::new(&self.file).parent().unwrap_or(Path::new("")).to_path_buf();
        let Ok(names) = std::fs::read_dir(&dir) else { return Vec::new() };
        // ^<stem>\.v\d+(-[0-9TZ.-]+)?<ext>$
        let is_copy = |name: &str| {
            let Some(rest) = name.strip_prefix(&format!("{stem}.v")).and_then(|r| r.strip_suffix(&ext)) else { return false };
            let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            let tail = &rest[digits..];
            digits > 0 && (tail.is_empty() || tail.strip_prefix('-').is_some_and(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || "TZ.-".contains(c))))
        };
        let mut gone = Vec::new();
        for e in names.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_copy(&name) {
                continue;
            }
            let file = dir.join(&name).to_string_lossy().into_owned();
            let Ok(meta) = std::fs::metadata(&file) else { continue };
            let Ok(mtime) = xenonet_base::fs::mtime_ms_of(&meta) else { continue };
            if now - mtime < self.keep_backup_ms {
                continue;
            }
            if std::fs::remove_file(&file).is_ok() {
                gone.push(file);
            }
        }
        gone.sort();
        if self.backup_file.as_ref().is_some_and(|b| gone.contains(b)) {
            self.backup_file = None;
        }
        gone
    }

    /// Closes it.
    pub fn close(&mut self, now: f64) {
        self.db = None;
        self.life.stop(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-sqlite-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn steps(n: usize) -> Vec<Step> {
        let all: Vec<Step> = vec![
            Box::new(|db: &Connection| db.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)")),
            Box::new(|db: &Connection| db.execute_batch("ALTER TABLE notes ADD COLUMN at INTEGER")),
        ];
        all.into_iter().take(n).collect()
    }

    #[test]
    fn steps_taken_a_copy_kept_and_a_newer_one_left_alone() {
        let d = temp("steps");
        let file = d.join("state/notes.sqlite").to_string_lossy().into_owned();
        let mut fresh = SqliteStore::new("notes", &file, steps(1), vec!["journal_mode = WAL".into()], true, 0.0);
        let db = fresh.open(0.0).unwrap();
        db.execute("INSERT INTO notes (body) VALUES (?1)", ["hello"]).unwrap();
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!((fresh.version, fresh.backup_file.clone(), mode.as_str()), (Some(Version { from: 0, version: 1, newer: false }), None, "wal"));
        fresh.close(0.0);

        let mut later = SqliteStore::new("notes", &file, steps(2), vec![], true, 0.0);
        later.open(1e12).unwrap();
        assert_eq!(later.version, Some(Version { from: 1, version: 2, newer: false }));
        let copy = later.backup_file.clone().unwrap();
        assert!(copy.ends_with("notes.v1.sqlite"));
        let c = Connection::open(&copy).unwrap();
        let cols: Vec<String> = c.prepare("PRAGMA table_info(notes)").unwrap().query_map([], |r| r.get(1)).unwrap().map(Result::unwrap).collect();
        assert_eq!(cols, ["id", "body"], "as it was before the step");
        later.close(0.0);

        Connection::open(&file).unwrap().execute_batch("PRAGMA user_version = 9").unwrap();
        let mut newer = SqliteStore::new("notes", &file, steps(2), vec![], false, 0.0);
        newer.open(0.0).unwrap();
        assert_eq!((newer.version.map(|v| v.newer), newer.life.state.as_str()), (Some(true), "degraded"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_step_that_fails_takes_nothing_with_it() {
        let d = temp("fails");
        let file = d.join("broken.sqlite").to_string_lossy().into_owned();
        let bad: Vec<Step> = vec![
            Box::new(|db: &Connection| db.execute_batch("CREATE TABLE a (x INTEGER)")),
            Box::new(|db: &Connection| db.execute_batch("CREATE TABLE b (y INTEGER); INSERT INTO nowhere VALUES (1)")),
        ];
        let mut s = SqliteStore::new("broken", &file, bad, vec![], false, 0.0);
        assert!(s.open(0.0).unwrap_err().contains("no such table: nowhere"));
        assert_eq!(s.life.state.as_str(), "failed");
        let c = Connection::open(&file).unwrap();
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        let tables: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!((v, tables), (1, vec!["a".to_string()]));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fts5_is_there() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE VIRTUAL TABLE docs USING fts5(text, session_id unindexed, tokenize='porter unicode61 remove_diacritics 2'); INSERT INTO docs VALUES ('the parsers parse', 'claude:x');").unwrap();
        let n: i64 = db.query_row("SELECT count(*) FROM docs WHERE docs MATCH 'parser'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "porter stems parsers to parser");
    }
}
