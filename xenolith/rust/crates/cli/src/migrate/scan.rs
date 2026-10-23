//! What the wiki holds, counted and fingerprinted, so the migration can show it before and prove it
//! after that nothing was lost: pages, log days, notes waiting (and dead letters), filed notes, held
//! changes and vectors. A fingerprint is a SHA-256 over each file's relative path and content hash;
//! it says nothing about the text. PROTOCOL.md and index.md are not in any group: an install may
//! replace an unedited PROTOCOL.md, and index.md is generated.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    pub count: usize,
    pub digest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub pages: Tally,
    pub log_days: Tally,
    pub inbox: Tally,
    pub dead_letters: usize,
    pub filed: Tally,
    /// Held changes waiting for a decision (the digest covers decided ones too).
    pub held: Tally,
    pub vectors: Tally,
}

impl Scan {
    pub fn groups(&self) -> [(&'static str, &Tally); 6] {
        [("pages", &self.pages), ("log days", &self.log_days), ("notes waiting", &self.inbox), ("filed notes", &self.filed), ("held changes", &self.held), ("vectors", &self.vectors)]
    }

    pub fn summary(&self) -> String {
        let n = |c: usize, one: &str, many: &str| format!("{c} {}", if c == 1 { one } else { many });
        format!(
            "{}, {}, {} ({}), {}, {}, {}",
            n(self.pages.count, "page", "pages"),
            n(self.log_days.count, "log day", "log days"),
            n(self.inbox.count, "note waiting", "notes waiting"),
            n(self.dead_letters, "dead letter", "dead letters"),
            n(self.filed.count, "filed note", "filed notes"),
            n(self.held.count, "held change", "held changes"),
            n(self.vectors.count, "vector", "vectors"),
        )
    }
}

/// Regular files under `dir` (not following links), as (relative path with /, absolute path), sorted.
fn files(dir: &Path, keep: &dyn Fn(&str) -> bool) -> Vec<(String, std::path::PathBuf)> {
    fn go(base: &Path, dir: &Path, keep: &dyn Fn(&str) -> bool, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() {
                go(base, &p, keep, out);
            } else if t.is_file() {
                let rel = p.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                if keep(&rel) {
                    out.push((rel, p));
                }
            }
        }
    }
    let mut out = vec![];
    go(dir, dir, keep, &mut out);
    out.sort();
    out
}

/// Hashes the files on a few threads: a large wiki has a hundred thousand vectors.
fn tally(list: &[(String, std::path::PathBuf)]) -> Tally {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let chunk = list.len().div_ceil(threads).max(1);
    let parts: Vec<Vec<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = list
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(|(rel, p)| format!("{rel}\0{}\n", xenolith_core::text::sha256_hex(&std::fs::read(p).unwrap_or_default()))).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    let joined: String = parts.into_iter().flatten().collect();
    Tally { count: list.len(), digest: xenolith_core::text::sha256_hex(joined.as_bytes()) }
}

pub fn scan(wiki: &Path) -> Scan {
    let md = |r: &str| r.ends_with(".md");
    let cur = wiki.join(".curator");
    let inbox = files(&wiki.join("inbox"), &|r: &str| !r.contains('/') && r.ends_with(".md") && !r.starts_with('.'));
    let dead = inbox
        .iter()
        .filter(|(rel, _)| {
            let id = rel.trim_end_matches(".md");
            std::fs::read_to_string(cur.join("state").join(format!("{id}.json")))
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .is_some_and(|v| v["dead"].as_bool() == Some(true))
        })
        .count();
    let held_all = files(&cur.join("held"), &|r: &str| r.ends_with(".json") || r.ends_with(".jsonl"));
    let mut held = tally(&held_all);
    held.count = held_all.iter().filter(|(r, _)| !r.contains('/') && r.ends_with(".json")).count();
    Scan {
        pages: tally(&files(&wiki.join("pages"), &md)),
        log_days: tally(&files(&wiki.join("log"), &md)),
        inbox: tally(&inbox),
        dead_letters: dead,
        filed: tally(&files(&cur.join("done"), &|_| true)),
        held,
        vectors: tally(&files(&cur.join("vectors"), &|r: &str| r.ends_with(".f32"))),
    }
}

/// How `after` differs from `before`, in plain words; empty when nothing changed.
pub fn differences(before: &Scan, after: &Scan) -> Vec<String> {
    let mut out = vec![];
    for ((name, a), (_, b)) in before.groups().iter().zip(after.groups().iter()) {
        if a.count != b.count {
            out.push(format!("{name}: {} before, {} now", a.count, b.count));
        } else if a.digest != b.digest {
            out.push(format!("{name}: the same number ({}), but some changed", a.count));
        }
    }
    out
}
