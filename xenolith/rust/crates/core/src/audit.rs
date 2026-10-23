//! The audit chain: grants, tokens, refusals, writes, new spaces and sync, one JSON line each in
//! Xenolith's state folder (audit/chain.jsonl). Each line carries the SHA-256 of the line before
//! it (`prev`) and its own (`hash`, over `prev` and the line without `hash`), so a line changed,
//! removed or put in between breaks the chain at that point, and `xenolith audit verify` says
//! where. It holds no text a session wrote, only its hash.
//!
//! The chain shows tampering; it can't prevent it: whoever can write the file can rewrite the
//! whole chain. Keep a copy of the last hash elsewhere to pin it.
//!
//! Each line is on disk before append returns (the file synced, and its folder when the file is
//! new). A line that can't be written is never silent: best_effort counts it for this process,
//! and /status and `xenolith status` say so (health).

use crate::paths::AppPaths;
use crate::text::{local_iso, now, sha256_hex};
use crate::wiki::{self, Result, wiki_err};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lines this process could not write, and the last reason (health()).
static FAILURES: AtomicUsize = AtomicUsize::new(0);
static LAST_FAILURE: Mutex<Option<String>> = Mutex::new(None);

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

pub fn chain_file(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("audit").join("chain.jsonl")
}

/// The hash of a line: over the previous hash and the line's JSON without its own hash.
fn line_hash(prev: &str, body: &Map<String, Value>) -> String {
    let mut b = body.clone();
    b.remove("hash");
    sha256_hex(format!("{prev}\n{}", Value::Object(b)).as_bytes())
}

/// The last line of the chain, read from the end so a long chain costs one small read.
fn last_line(file: &Path) -> std::io::Result<Option<String>> {
    let mut f = match fs::File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    let mut take = 4096u64;
    loop {
        let start = len.saturating_sub(take);
        f.seek(SeekFrom::Start(start))?;
        let mut buf = String::new();
        f.read_to_string(&mut buf)?;
        let trimmed = buf.trim_end_matches('\n');
        if let Some(i) = trimmed.rfind('\n') {
            return Ok(Some(trimmed[i + 1..].to_string()));
        }
        if start == 0 {
            return Ok(Some(trimmed.to_string()).filter(|s| !s.is_empty()));
        }
        take *= 4;
    }
}

/// Appends one entry: `kind` and its fields, after `seq`, `at` and `who`. Returns the line written.
pub fn append(paths: &AppPaths, kind: &str, who: &str, fields: Vec<(&str, Value)>) -> Result<Value> {
    append_to(&chain_file(paths), kind, who, fields)
}

pub fn append_to(file: &Path, kind: &str, who: &str, fields: Vec<(&str, Value)>) -> Result<Value> {
    let dir = file.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    // One writer at a time: each line names the one before it.
    let mut guard = match crate::lock::acquire(dir, "chain", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return wiki_err(format!("The audit log is busy ({}).", b.0)),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.into()),
    };
    let (seq, prev) = match last_line(file)? {
        None => (0, GENESIS.to_string()),
        Some(l) => match serde_json::from_str::<Value>(&l) {
            Ok(v) => (v["seq"].as_i64().unwrap_or(0), v["hash"].as_str().unwrap_or(GENESIS).to_string()),
            // A torn or edited last line: chain on from its text, so verify still finds the break there.
            Err(_) => (0, sha256_hex(l.as_bytes())),
        },
    };
    let mut o = Map::new();
    o.insert("seq".into(), json!(seq + 1));
    o.insert("at".into(), json!(local_iso(&now())));
    o.insert("kind".into(), json!(kind));
    o.insert("who".into(), json!(who));
    for (k, v) in fields {
        if !v.is_null() {
            o.insert(k.into(), v);
        }
    }
    o.insert("prev".into(), json!(prev));
    let hash = line_hash(&prev, &o);
    o.insert("hash".into(), json!(hash));
    let line = Value::Object(o);
    let r = (|| -> std::io::Result<()> {
        let new = !file.exists();
        let mut f = fs::OpenOptions::new().create(true).append(true).open(file)?;
        f.write_all(format!("{line}\n").as_bytes())?;
        f.sync_data()?;
        if new {
            wiki::fsync_dir(dir);
        }
        Ok(())
    })();
    guard.release();
    r?;
    Ok(line)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Verified {
    pub lines: usize,
    /// The last hash: pin it elsewhere to notice the whole chain being rewritten.
    pub head: String,
    /// The first line (1-based) that does not follow from the one before it, and why.
    pub broken: Option<(usize, String)>,
}

/// Checks every line: each names the hash of the one before and hashes to its own `hash`.
pub fn verify(file: &Path) -> Result<Verified> {
    let text = match fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Verified { lines: 0, head: GENESIS.into(), broken: None }),
        Err(e) => return Err(e.into()),
    };
    let mut prev = GENESIS.to_string();
    let mut seq = 0;
    let mut n = 0;
    for (i, line) in text.lines().enumerate() {
        n = i + 1;
        let Ok(Value::Object(o)) = serde_json::from_str::<Value>(line) else {
            return Ok(Verified { lines: n, head: prev, broken: Some((n, "not a JSON object".into())) });
        };
        if o.get("prev").and_then(Value::as_str) != Some(prev.as_str()) {
            return Ok(Verified { lines: n, head: prev, broken: Some((n, "does not name the line before it (a line was removed, added or changed)".into())) });
        }
        let want = line_hash(&prev, &o);
        if o.get("hash").and_then(Value::as_str) != Some(want.as_str()) {
            return Ok(Verified { lines: n, head: prev, broken: Some((n, "its content changed after it was written".into())) });
        }
        let s = o.get("seq").and_then(Value::as_i64).unwrap_or(-1);
        if s != seq + 1 {
            return Ok(Verified { lines: n, head: prev, broken: Some((n, format!("sequence {s} after {seq}"))) });
        }
        seq = s;
        prev = want;
    }
    Ok(Verified { lines: n, head: prev, broken: None })
}

/// The last `n` entries, newest last.
pub fn tail(file: &Path, n: usize) -> Vec<Value> {
    let text = fs::read_to_string(file).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// The text a session wrote, as the audit log keeps it: a hash, never the words.
pub fn content_hash(text: &str) -> String {
    format!("sha256:{}", &sha256_hex(crate::text::to_lf(text).as_bytes())[..32])
}

/// For a refusal line: the folder may be gone or unwritable, and a refusal must still be answered.
/// A line that isn't written is counted, and /status says so.
pub fn best_effort(paths: &AppPaths, kind: &str, who: &str, fields: Vec<(&str, Value)>) {
    if let Err(e) = append(paths, kind, who, fields) {
        let why = wiki::Error::to_string(&e);
        eprintln!("[xenolith] audit log not written: {why}");
        FAILURES.fetch_add(1, Ordering::SeqCst);
        *LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{} {why}", local_iso(&now())));
    }
}

/// Whether a line could be added now: the folder exists or can be made, and the file opens for
/// appending (nothing is written).
pub fn writable(paths: &AppPaths) -> std::result::Result<(), String> {
    let file = chain_file(paths);
    fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))
        .and_then(|_| fs::OpenOptions::new().create(true).append(true).open(&file))
        .map(|_| ())
        .map_err(|e| format!("{}: {e}", wiki::disp(&file)))
}

/// The chain's health for /status: lines this process failed to write, the last reason, and
/// whether a line could be written now. `problem` is what to tell the person, if anything.
pub fn health(paths: &AppPaths) -> (Value, Option<String>) {
    let failures = FAILURES.load(Ordering::SeqCst);
    let last = LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let now = writable(paths);
    let problem = match (&now, failures) {
        (Err(e), _) => Some(format!("the audit chain can't be written: {e}")),
        (Ok(()), 0) => None,
        (Ok(()), n) => Some(format!("{n} audit line(s) were not written (last: {})", last.clone().unwrap_or_default())),
    };
    (json!({ "ok": problem.is_none(), "failures": failures, "lastError": last, "writable": now.is_ok() }), problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::random_hex;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenolith-audit-{}", random_hex(8)));
        fs::create_dir_all(&d).unwrap();
        d.join("chain.jsonl")
    }

    #[test]
    fn each_line_names_the_one_before_and_verify_follows_the_chain() {
        let f = tmp();
        assert_eq!(verify(&f).unwrap(), Verified { lines: 0, head: GENESIS.into(), broken: None });
        let a = append_to(&f, "grant", "you", vec![("subject", json!("actor:hermes")), ("scope", json!("ctx:read")), ("skip", Value::Null)]).unwrap();
        let b = append_to(&f, "write", "actor:hermes (tok_x)", vec![("space", json!("actor:hermes")), ("content", json!(content_hash("hello")))]).unwrap();
        assert_eq!(a["seq"], 1);
        assert_eq!(a["prev"], GENESIS);
        assert!(a.get("skip").is_none(), "empty fields are left out");
        assert_eq!(b["seq"], 2);
        assert_eq!(b["prev"], a["hash"]);
        let v = verify(&f).unwrap();
        assert_eq!(v, Verified { lines: 2, head: b["hash"].as_str().unwrap().into(), broken: None });
        assert!(!fs::read_to_string(&f).unwrap().contains("hello"), "text is kept only as a hash");
        crate::lock::remove_test_dir(f.parent().unwrap());
    }

    #[test]
    fn a_changed_removed_or_inserted_line_breaks_the_chain_there() {
        let f = tmp();
        for i in 0..4 {
            append_to(&f, "write", "you", vec![("n", json!(i))]).unwrap();
        }
        let good = fs::read_to_string(&f).unwrap();
        let lines: Vec<&str> = good.lines().collect();

        fs::write(&f, good.replace("\"n\":1", "\"n\":7")).unwrap();
        assert_eq!(verify(&f).unwrap().broken.unwrap().0, 2, "changed content");

        fs::write(&f, format!("{}\n{}\n{}\n", lines[0], lines[2], lines[3])).unwrap();
        assert_eq!(verify(&f).unwrap().broken.unwrap().0, 2, "a removed line");

        fs::write(&f, format!("{}\n{}\n{}\n{}\n{}\n", lines[0], lines[1], lines[1], lines[2], lines[3])).unwrap();
        assert_eq!(verify(&f).unwrap().broken.unwrap().0, 3, "a repeated line");

        fs::write(&f, &good).unwrap();
        fs::OpenOptions::new().append(true).open(&f).unwrap().write_all(b"{\"torn\":").unwrap();
        append_to(&f, "write", "you", vec![]).unwrap();
        assert_eq!(verify(&f).unwrap().broken.unwrap().0, 5, "a torn line stays visible");
        crate::lock::remove_test_dir(f.parent().unwrap());
    }

    #[test]
    fn a_line_that_cant_be_written_is_counted_and_reported() {
        let home = std::env::temp_dir().join(format!("xenolith-audit-health-{}", random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), home.to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        // (Other tests in this process may have counted failures of their own: only this chain's state.)
        assert_eq!(health(&paths).0["writable"].as_bool(), Some(true));
        best_effort(&paths, "write", "you", vec![]);
        assert_eq!(verify(&chain_file(&paths)).unwrap().lines, 1);
        // The chain's place is taken by a folder: nothing can be appended.
        fs::remove_file(chain_file(&paths)).unwrap();
        fs::create_dir_all(chain_file(&paths)).unwrap();
        let before = FAILURES.load(Ordering::SeqCst);
        best_effort(&paths, "refused", "you", vec![]);
        assert!(FAILURES.load(Ordering::SeqCst) > before);
        let (h, problem) = health(&paths);
        assert_eq!(h["writable"].as_bool(), Some(false));
        assert!(problem.unwrap().starts_with("the audit chain can't be written"));
        crate::lock::remove_test_dir(&home);
    }

    #[test]
    fn concurrent_writers_keep_one_chain() {
        let f = tmp();
        let threads: Vec<_> = (0..4)
            .map(|t| {
                let f = f.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        append_to(&f, "write", "you", vec![("t", json!(t)), ("i", json!(i))]).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let v = verify(&f).unwrap();
        assert_eq!((v.lines, v.broken), (40, None));
        assert_eq!(tail(&f, 3).len(), 3);
        crate::lock::remove_test_dir(f.parent().unwrap());
    }
}
