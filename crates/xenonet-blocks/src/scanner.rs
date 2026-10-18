//! A scanner walks a folder for the files it wants, within a budget of
//! folders read, levels deep and time spent, and remembers each folder's
//! mtime, so the next walk reads only the folders that changed
//! (core/blocks/scanner.js).
//!
//! A folder's mtime changes when a file is added to it, removed or renamed,
//! not when a file in it is written to, so a walk with the cache finds new
//! files at the cost of a stat for each folder that didn't change. Folders
//! that went are forgotten, with everything under them.
//!
//! An mtime only moves when the filesystem's clock does. Linux keeps coarse
//! timestamps, so a file added a few milliseconds after a folder last
//! changed usually leaves its mtime as it was. A folder read that soon
//! after its last change could be hiding one (git calls such entries
//! racy), so the next walk reads it again, whatever its mtime says.
//!
//! `scan` walks depth first, in the order each folder lists its entries: a
//! folder's files, then its subfolders, reading the disk as it goes.
//!
//! The breadth-first walk goes a level at a time, shallow levels first, and
//! leaves the reading to whoever runs it (several folders at once, and a
//! `look` at each folder that may wait on other work): `begin_breadth`,
//! then for each level `plan`, each folder's `begin_read`, `read_failed` or
//! `look_failed`, and `finish_read`, and `end_level`; then `finish_breadth`.
//!
//! The clock is read where the JS reads it, no more and no less: a test's
//! clock may move each time it's read.

use crate::lifecycle::Lifecycle;
use xenonet_base::fs::{DirEntry, Fs, FsError};
use xenonet_base::json::number_string;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::join;

/// How close to a folder's mtime a read of it is racy: wide enough for the
/// coarsest clocks, FAT's 2 s, and either side, for a filesystem whose
/// clock runs ahead.
pub const RACY_MS: f64 = 2000.0;

fn racy(at: f64, mtime_ms: Option<f64>) -> bool {
    mtime_ms.is_some_and(|m| (at - m).abs() < RACY_MS)
}

#[cfg(windows)]
const SEP: char = '\\';
#[cfg(not(windows))]
const SEP: char = '/';

/// What one walk may spend: levels below the root, folders read, and ms.
/// Each may be infinite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub depth: f64,
    pub folders: f64,
    pub ms: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { depth: f64::INFINITY, folders: f64::INFINITY, ms: f64::INFINITY }
    }
}

/// How the last walk went.
#[derive(Clone, Debug, PartialEq)]
pub struct Last {
    pub files: u64,
    pub read: u64,
    pub seen: u64,
    pub truncated: Option<&'static str>,
    pub ms: f64,
}

/// A folder as it was when last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub mtime_ms: Option<f64>,
    pub subdirs: Vec<String>,
    pub racy: bool,
}

#[derive(Debug)]
struct Run {
    files: u64,
    read: u64,
    seen: u64,
    truncated: Option<&'static str>,
    problem: Option<String>,
    started: f64,
}

impl Run {
    fn new(started: f64) -> Self {
        Run { files: 0, read: 0, seen: 0, truncated: None, problem: None, started }
    }
}

pub use crate::Halt;

/// The files a walk wants and the folders it enters. `pick(name, dir)` and
/// `enter(name, dir, level)`.
pub trait Rules {
    fn pick(&mut self, name: &str, dir: &str) -> Result<bool, Halt>;
    fn enter(&mut self, name: &str, dir: &str, level: u32) -> Result<bool, Halt>;
}

/// Rules from two closures.
pub struct FnRules<P, E>(pub P, pub E);

impl<P: FnMut(&str, &str) -> bool, E: FnMut(&str, &str, u32) -> bool> Rules for FnRules<P, E> {
    fn pick(&mut self, name: &str, dir: &str) -> Result<bool, Halt> {
        Ok((self.0)(name, dir))
    }
    fn enter(&mut self, name: &str, dir: &str, level: u32) -> Result<bool, Halt> {
        Ok((self.1)(name, dir, level))
    }
}

pub struct Scanner {
    pub name: String,
    pub root: String,
    pub limits: Limits,
    pub use_cache: bool,
    /// folder → what it held the last time it was read.
    pub folders: OrderedMap<Folder>,
    pub last: Option<Last>,
    /// What the last walk was allowed, and how it went.
    pub budget: Limits,
    pub life: Lifecycle,
}

impl Scanner {
    pub fn new(name: &str, root: &str, limits: Limits, use_cache: bool, now: f64) -> Self {
        Scanner { name: name.into(), root: root.into(), limits, use_cache, folders: OrderedMap::new(), last: None, budget: limits, life: Lifecycle::new(now) }
    }

    /// Forgets every folder, so the next walk reads them all.
    pub fn forget(&mut self) {
        self.folders.clear();
    }

    // Whether a folder's last read still holds: its mtime hasn't moved,
    // and it wasn't read so soon after a change that another could hide.
    fn kept(known: Option<&Folder>, mtime_ms: f64) -> bool {
        known.is_some_and(|k| !k.racy && k.mtime_ms == Some(mtime_ms))
    }

    fn forget_under(&mut self, dir: &str) {
        let under = format!("{dir}{SEP}");
        self.folders.retain(|k, _| k != dir && !k.starts_with(&under));
    }

    // Folders that went since a folder was last read, and everything that
    // was under them.
    fn gone(&mut self, before: Option<&Folder>, now: &Folder) {
        let went: Vec<String> = before.map(|b| b.subdirs.iter().filter(|d| !now.subdirs.contains(d)).cloned().collect()).unwrap_or_default();
        for d in went {
            self.forget_under(&d);
        }
    }

    // How a walk went: `last`, the counters, and the block's state.
    fn report(&mut self, run: Run, limits: Limits, now: &mut dyn FnMut() -> f64) {
        let started = run.started;
        let ms = now() - started;
        let last = Last { files: run.files, read: run.read, seen: run.seen, truncated: run.truncated, ms };
        self.last = Some(last);
        self.life.count("scans", 1.0);
        self.life.count("read", run.read as f64);
        // A walk stopped as no longer wanted says nothing of the scanner.
        match run.truncated {
            Some("cancelled") => {}
            Some("folders") => self.life.problem(&format!("stopped at the {}-folder budget", number_string(limits.folders)), started),
            Some(_) => self.life.problem(&format!("stopped after {} ms", number_string(limits.ms)), started),
            None => match run.problem {
                Some(p) => self.life.problem(&p, started),
                None => self.life.ok(started),
            },
        }
    }

    /// The files picked in the folders this walk read, depth first.
    pub fn scan(&mut self, fs: &mut dyn Fs, rules: &mut dyn Rules, now: &mut dyn FnMut() -> f64) -> Result<Vec<String>, Halt> {
        let mut run = Run::new(now());
        let mut out = Vec::new();
        let root = self.root.clone();
        self.walk(&root, 0, &mut out, &mut run, fs, rules, now)?;
        let limits = self.limits;
        self.report(run, limits, now);
        Ok(out)
    }

    #[expect(clippy::too_many_arguments, reason = "each level of the walk passes on what scan was given: the run, the filesystem, the rules and the clock")]
    fn walk(&mut self, dir: &str, level: u32, out: &mut Vec<String>, run: &mut Run, fs: &mut dyn Fs, rules: &mut dyn Rules, now: &mut dyn FnMut() -> f64) -> Result<(), Halt> {
        if run.truncated.is_some() {
            return Ok(());
        }
        let Ok(mtime) = fs.mtime_ms(dir) else {
            self.folders.remove(dir);
            return Ok(());
        };
        run.seen += 1;
        let known = self.folders.get(dir);
        let f = if !self.use_cache || !Self::kept(known, mtime) {
            if run.read as f64 >= self.limits.folders {
                run.truncated = Some("folders");
                return Ok(());
            }
            let at = now();
            if at - run.started > self.limits.ms {
                run.truncated = Some("time");
                return Ok(());
            }
            run.read += 1;
            let before = known.cloned();
            let mut f = Folder { mtime_ms: Some(mtime), subdirs: Vec::new(), racy: racy(at, Some(mtime)) };
            let ents = match fs.read_dir(dir) {
                Ok(e) => e,
                Err(e) => {
                    if !e.is("ENOENT") && run.problem.is_none() {
                        run.problem = Some(format!("couldn't read {dir} ({})", e.code_or_message()));
                    }
                    Vec::new()
                }
            };
            for e in &ents {
                let full = join(dir, &e.name);
                if e.is_dir {
                    if f64::from(level) < self.limits.depth && rules.enter(&e.name, dir, level + 1)? {
                        f.subdirs.push(full);
                    }
                } else if e.is_file && rules.pick(&e.name, dir)? {
                    out.push(full);
                    run.files += 1;
                }
            }
            self.gone(before.as_ref(), &f);
            self.folders.insert(dir, f.clone());
            f
        } else {
            known.cloned().expect("a kept folder")
        };
        for sub in &f.subdirs {
            self.walk(sub, level + 1, out, run, fs, rules, now)?;
        }
        Ok(())
    }

    /// A breadth-first walk begins, with its own root and limits.
    pub fn begin_breadth(&mut self, root: &str, limits: Limits, now: &mut dyn FnMut() -> f64) -> Breadth {
        self.budget = limits;
        Breadth { limits, run: Run::new(now()), out: Vec::new(), level: vec![root.to_string()], n: 0, slots: Vec::new() }
    }

    /// The folders of this level to read, in order: the slot each fills,
    /// and its mtime. `mtimes` are the level's folders' (None for one that
    /// couldn't be stat'ed), when the walk uses the cache.
    pub fn plan(&mut self, w: &mut Breadth, mtimes: Option<&[Option<f64>]>) -> Vec<Read> {
        let mut reads = Vec::new();
        let mut taken = 0u64;
        w.slots.clear();
        let level = std::mem::take(&mut w.level);
        for (i, dir) in level.iter().enumerate() {
            let st = mtimes.map(|m| m.get(i).copied().flatten());
            if st == Some(None) {
                self.forget_under(dir);
                continue;
            }
            let st = st.flatten();
            if w.n > 0 {
                w.run.seen += 1;
            }
            let known = self.folders.get(dir);
            if let (Some(m), Some(k)) = (st, known) {
                if Self::kept(known, m) {
                    w.slots.push(Slot::Found(Some(Found { files: Vec::new(), subdirs: k.subdirs.clone() })));
                    continue;
                }
            }
            // The budget is spent in order, so the same folders are read
            // every time: the first ones on the level.
            if w.n > 0 && (w.run.read + taken) as f64 >= w.limits.folders {
                w.run.truncated = Some("folders");
                break;
            }
            if w.n > 0 {
                taken += 1;
            }
            reads.push(Read { slot: w.slots.len(), dir: dir.clone(), mtime_ms: st });
            w.slots.push(Slot::Reading);
        }
        reads
    }

    /// A folder's read begins: when (the clock read once), or None when the
    /// walk is no longer wanted or out of time.
    pub fn begin_read(&mut self, w: &mut Breadth, aborted: bool, now: &mut dyn FnMut() -> f64) -> Option<f64> {
        if aborted {
            w.run.truncated.get_or_insert("cancelled");
            return None;
        }
        let at = now();
        if at - w.run.started > w.limits.ms {
            w.run.truncated.get_or_insert("time");
            return None;
        }
        if w.n > 0 {
            w.run.read += 1;
        }
        Some(at)
    }

    /// Reading a folder failed: whether to go on with it (as a folder with
    /// nothing in it), or pass it over (it went).
    pub fn read_failed(&mut self, w: &mut Breadth, dir: &str, err: &FsError) -> bool {
        if err.is("ENOENT") {
            self.forget_under(dir);
            return false;
        }
        w.run.problem.get_or_insert_with(|| format!("couldn't read {dir} ({})", err.code_or_message()));
        true
    }

    /// What looks at the folders is the caller's: a folder it couldn't
    /// look at is passed over, like one that couldn't be read.
    pub fn look_failed(&mut self, w: &mut Breadth, dir: &str, message: &str) {
        w.run.problem.get_or_insert_with(|| format!("couldn't look at {dir} ({message})"));
    }

    /// A folder the walk didn't read after all.
    pub fn skip(&mut self, w: &mut Breadth, slot: usize) {
        w.slots[slot] = Slot::Found(None);
    }

    /// A folder read: its entries (none when it couldn't be read or looked
    /// at), the files picked and the subfolders entered.
    pub fn finish_read(&mut self, w: &mut Breadth, read: &Read, at: f64, entries: &[DirEntry], rules: &mut dyn Rules) -> Result<(), Halt> {
        let mut f = Found { files: Vec::new(), subdirs: Vec::new() };
        for e in entries {
            let full = join(&read.dir, &e.name);
            if e.is_dir {
                if f64::from(w.n) < w.limits.depth && rules.enter(&e.name, &read.dir, w.n + 1)? {
                    f.subdirs.push(full);
                }
            } else if e.is_file && rules.pick(&e.name, &read.dir)? {
                f.files.push(full);
                w.run.files += 1;
            }
        }
        if self.use_cache {
            let now = Folder { mtime_ms: read.mtime_ms, subdirs: f.subdirs.clone(), racy: racy(at, read.mtime_ms) };
            let before = self.folders.get(&read.dir).cloned();
            self.gone(before.as_ref(), &now);
            self.folders.insert(&read.dir, now);
        }
        w.slots[read.slot] = Slot::Found(Some(f));
        Ok(())
    }

    /// The level is done: its files, in order, and the next level.
    pub fn end_level(&mut self, w: &mut Breadth) {
        for s in std::mem::take(&mut w.slots) {
            if let Slot::Found(Some(f)) = s {
                w.out.extend(f.files);
                // (A folder kept from an earlier walk may have been read
                // for a deeper one.)
                if f64::from(w.n) < w.limits.depth {
                    w.level.extend(f.subdirs);
                }
            }
        }
        w.n += 1;
    }

    /// The walk is done: the files picked, shallow levels first.
    pub fn finish_breadth(&mut self, mut w: Breadth, now: &mut dyn FnMut() -> f64) -> Vec<String> {
        let out = std::mem::take(&mut w.out);
        self.report(w.run, w.limits, now);
        out
    }
}

/// A folder of a breadth-first walk to read.
#[derive(Clone, Debug)]
pub struct Read {
    pub slot: usize,
    pub dir: String,
    pub mtime_ms: Option<f64>,
}

#[derive(Debug)]
struct Found {
    files: Vec<String>,
    subdirs: Vec<String>,
}

#[derive(Debug)]
enum Slot {
    Reading,
    Found(Option<Found>),
}

/// A breadth-first walk under way.
#[derive(Debug)]
pub struct Breadth {
    pub limits: Limits,
    run: Run,
    out: Vec<String>,
    level: Vec<String>,
    /// The level being read: 0 is the root.
    pub n: u32,
    slots: Vec<Slot>,
}

impl Breadth {
    /// Whether there's another level to read.
    pub fn more(&self) -> bool {
        !self.level.is_empty() && self.run.truncated.is_none()
    }

    /// The folders of the level about to be read.
    pub fn level(&self) -> &[String] {
        &self.level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_base::fs::StdFs;

    #[test]
    fn walks_depth_and_breadth_first_alike() {
        let dir = std::env::temp_dir().join(format!("xenonet-scanner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in ["p/a/s1.jsonl", "p/a/n.txt", "p/a/s1/sub/x.jsonl", "p/b/s2.jsonl"] {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}\n").unwrap();
        }
        let root = dir.join("p").to_string_lossy().into_owned();
        let mut s = Scanner::new("t", &root, Limits::default(), true, 0.0);
        // A clock far from the folders' mtimes: no read is racy.
        let mut now = || 1e15;
        let mut rules = FnRules(|n: &str, _: &str| std::path::Path::new(n).extension().is_some_and(|e| e == "jsonl"), |_: &str, _: &str, _| true);
        let found = s.scan(&mut StdFs, &mut rules, &mut now).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(s.scan(&mut StdFs, &mut rules, &mut now).unwrap().len(), 0, "nothing changed");

        let mut b = Scanner::new("t", &root, Limits::default(), false, 0.0);
        let mut w = b.begin_breadth(&root, Limits::default(), &mut now);
        while w.more() {
            for r in b.plan(&mut w, None) {
                let at = b.begin_read(&mut w, false, &mut now).unwrap();
                let ents = StdFs.read_dir(&r.dir).unwrap();
                b.finish_read(&mut w, &r, at, &ents, &mut rules).unwrap();
            }
            b.end_level(&mut w);
        }
        let found = b.finish_breadth(w, &mut now);
        assert_eq!(found.len(), 3);
        assert!(found[2].ends_with("x.jsonl"), "the shallow files first");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
