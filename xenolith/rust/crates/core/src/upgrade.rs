//! The upgrade from 1.x: what 2.0 changes in a folder and a state folder 1.x left, done only when
//! the person asks, recorded, and taken back on request.
//!
//! - **Waiting.** Until it runs, 2.0 founds no chain, writes no `xenolith-space.json`, moves no
//!   copy aside and binds nothing (chains.rs). The service reads and writes the folder as 1.x did:
//!   notes go to each space's inbox and its curator files them. `/status`, `xenolith status` and
//!   `xenolith doctor` say the upgrade is waiting.
//! - **The dry run.** `xenolith upgrade` lists what `--run` would found, move aside, mark and bind,
//!   reading only (`chains::plan`).
//! - **Running.** `xenolith upgrade --run` (through the service, with the owner key) moves 1.x's
//!   copies of followed spaces aside and founds and marks the rest, saying each change in the
//!   manifest, `<state>/upgrade.json`, before making it. From then on the service's later passes
//!   (a space folder appearing) record their changes there too.
//! - **Undo.** `xenolith upgrade --undo` takes back every change the manifest records, newest
//!   first: files it wrote go back to what they held (or go, when they weren't there), folders it
//!   made go when empty, copies it moved come back. Nothing else is touched, and no 1.x file is ever
//!   deleted: `xenolith doctor` names 1.x's leftovers for the person to delete (`leftovers`).
//! - **A new folder.** A folder 2.0 makes itself, where nothing was before and the config folder
//!   holds nothing of 1.x's sync or pairing, counts as upgraded at once (`open_folder`): there's no
//!   1.x data to keep, and nothing to roll back to, so asking first would only add a step.
//!
//! Deferred: comparing a restored state folder's stored feed incarnation with the feed's last entry
//! (review revision 33f). Xenolith keeps no feeds until sync/2, so there's no incarnation yet.

use crate::chains::{self, Change, Report};
use crate::paths::AppPaths;
use crate::spaces::{self, SpaceId};
use crate::wiki;
use serde_json::{Value, json};
use std::fs;
use std::path::{Component, Path, PathBuf};
use xenonet_id::codec::{b64, unb64};
use xenonet_id::sign::Signer;

/// The manifest's file in the state folder.
pub fn file(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("upgrade.json")
}

/// Where the upgrade stands. No manifest is `Waiting`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// 1.x's folder and state, as they were: nothing founded, marked or moved.
    Waiting,
    /// A folder 2.0 made itself: upgraded from the start.
    Fresh,
    /// `--run` started and hasn't finished (or stopped part way: running it again finishes).
    Running,
    /// `--run` finished.
    Done,
    /// `--undo` started and hasn't finished (running it again finishes).
    Undoing,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Waiting => "waiting",
            Stage::Fresh => "fresh",
            Stage::Running => "running",
            Stage::Done => "done",
            Stage::Undoing => "undoing",
        }
    }

    fn parse(s: &str) -> Option<Stage> {
        [Stage::Fresh, Stage::Running, Stage::Done, Stage::Undoing].into_iter().find(|t| t.as_str() == s)
    }

    /// Whether the service founds and marks spaces at this stage.
    pub fn upgraded(self) -> bool {
        matches!(self, Stage::Fresh | Stage::Done)
    }
}

/// The manifest: where the upgrade stands, and each change it made, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub stage: Stage,
    /// When it reached this stage (ms).
    pub at: i64,
    /// The Xenolith that wrote it.
    pub version: String,
    pub changes: Vec<Change>,
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn change_json(c: &Change) -> Value {
    match c {
        Change::Dir { path } => json!({ "dir": path_text(path) }),
        Change::File { path, before } => json!({ "file": path_text(path), "before": before.as_deref().map(b64) }),
        Change::Moved { address, from, to } => json!({ "moved": address.to_string(), "from": path_text(from), "to": path_text(to) }),
    }
}

fn change_of(v: &Value) -> Option<Change> {
    if let Some(p) = v["dir"].as_str() {
        return Some(Change::Dir { path: p.into() });
    }
    if let Some(p) = v["file"].as_str() {
        let before = match &v["before"] {
            Value::Null => None,
            b => Some(unb64(b.as_str()?)?),
        };
        return Some(Change::File { path: p.into(), before });
    }
    let address = SpaceId::parse(v["moved"].as_str()?).ok()?;
    Some(Change::Moved { address, from: v["from"].as_str()?.into(), to: v["to"].as_str()?.into() })
}

impl Manifest {
    fn new(stage: Stage, now: i64) -> Manifest {
        Manifest { stage, at: now, version: crate::VERSION.to_string(), changes: vec![] }
    }

    /// The manifest; None when there's none (the upgrade is waiting). One that's there and can't be
    /// read is an error, never "waiting", so an undo can't be lost and nothing is founded on a guess.
    pub fn load(paths: &AppPaths) -> Result<Option<Manifest>, String> {
        let f = file(paths);
        let text = match fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read {}: {e}", wiki::disp(&f))),
        };
        let bad = || format!("{} isn't an upgrade manifest; it was left as it is", wiki::disp(&f));
        let v: Value = serde_json::from_str(&text).map_err(|_| bad())?;
        let stage = v["stage"].as_str().and_then(Stage::parse).filter(|_| v["v"] == 1).ok_or_else(bad)?;
        let changes = v["changes"].as_array().ok_or_else(bad)?.iter().map(change_of).collect::<Option<Vec<_>>>().ok_or_else(bad)?;
        Ok(Some(Manifest { stage, at: v["at"].as_i64().unwrap_or(0), version: v["version"].as_str().unwrap_or("").to_string(), changes }))
    }

    fn save(&self, paths: &AppPaths) -> Result<(), String> {
        let v = json!({ "v": 1, "stage": self.stage.as_str(), "at": self.at, "version": self.version, "changes": self.changes.iter().map(change_json).collect::<Vec<_>>() });
        let text = format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default());
        wiki::atomic_write(&file(paths), &text, None, None).map_err(|e| e.to_string())
    }

    /// Says a change (saves the manifest with it) before it's made.
    fn record(&mut self, paths: &AppPaths, c: Change) -> Result<(), String> {
        self.changes.push(c);
        self.save(paths)
    }
}

/// Where the upgrade stands.
pub fn stage(paths: &AppPaths) -> Result<Stage, String> {
    Ok(Manifest::load(paths)?.map_or(Stage::Waiting, |m| m.stage))
}

/// 1.x's sync and pairing files in the config folder: any of them means this machine ran 1.x sync.
fn sync_1x_files(paths: &AppPaths) -> [PathBuf; 3] {
    [chains::sync_1x_file(paths), paths.config_dir.join("pair.json"), paths.config_dir.join("peers")]
}

/// Sets the shared folder up (wiki::ensure_wiki). When there was nothing at all there before and
/// the config folder holds nothing of 1.x's sync or pairing, 2.0 made this folder, so it counts as
/// upgraded at once (the module's notes say why). Every program that may make the folder first
/// calls this rather than ensure_wiki.
pub fn open_folder(paths: &AppPaths, root: &Path) -> wiki::Result<()> {
    let new = match fs::read_dir(root) {
        Ok(mut d) => d.next().is_none(),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    };
    wiki::ensure_wiki(root)?;
    if new && !sync_1x_files(paths).iter().any(|f| fs::symlink_metadata(f).is_ok()) && fs::symlink_metadata(file(paths)).is_err() {
        // Best effort: without it the upgrade waits, which is the safe side.
        let _ = fs::create_dir_all(&paths.state_dir).map_err(|e| e.to_string()).and_then(|_| Manifest::new(Stage::Fresh, crate::text::now_ms()).save(paths));
    }
    Ok(())
}

/// The service's pass (at its start, and when a space folder appears): founds and marks spaces
/// once the upgrade has run, recording each change in the manifest. None while it's waiting (or
/// part way, or being undone).
pub fn reconcile(paths: &AppPaths, root: &Path, context: &dyn Signer, owner: &dyn Signer, now: i64) -> Result<Option<Report>, String> {
    chains::locked(paths, || {
        let Some(mut m) = Manifest::load(paths)?.filter(|m| m.stage.upgraded()) else { return Ok(None) };
        chains::reconcile(paths, root, context, owner, now, &mut |c: Change| m.record(paths, c)).map(Some)
    })
}

/// What `--run` did: whether the upgrade had run already, and what this run changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub already: Option<Stage>,
    pub report: Report,
}

impl Ran {
    pub fn to_json(&self) -> Value {
        let mut v = self.report.to_json();
        v["already"] = json!(self.already.map(Stage::as_str));
        v
    }
}

/// `xenolith upgrade --run`: moves 1.x's copies of followed spaces aside, then founds and marks the
/// spaces this Xenolith curates, saying each change in the manifest first. Once it has run, running
/// it again changes nothing. A run that stopped part way finishes; an undo that stopped part way
/// has to finish first.
pub fn run(paths: &AppPaths, root: &Path, context: &dyn Signer, owner: &dyn Signer, now: i64) -> Result<Ran, String> {
    chains::locked(paths, || {
        let mut m = match Manifest::load(paths)? {
            Some(m) if m.stage.upgraded() => return Ok(Ran { already: Some(m.stage), report: Report::default() }),
            Some(m) if m.stage == Stage::Undoing => return Err("an undo stopped part way: run `xenolith upgrade --undo` again to finish it first".into()),
            Some(m) => m,
            None => Manifest::new(Stage::Running, now),
        };
        m.stage = Stage::Running;
        m.save(paths)?;
        let mut rec = |c: Change| m.record(paths, c);
        let moved = chains::move_1x_copies(paths, root, &mut rec)?;
        let mut report = chains::reconcile(paths, root, context, owner, now, &mut rec)?;
        report.moved = moved.moved;
        m.stage = Stage::Done;
        m.at = now;
        m.save(paths)?;
        Ok(Ran { already: None, report })
    })
}

/// What `--undo` did: how many changes it took back, and the ones it left, with why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Undone {
    /// None when there was nothing to undo (no manifest).
    pub stage: Option<Stage>,
    pub undone: usize,
    pub left: Vec<(PathBuf, String)>,
}

impl Undone {
    pub fn to_json(&self) -> Value {
        json!({
            "was": self.stage.map(Stage::as_str),
            "undone": self.undone,
            "left": self.left.iter().map(|(p, why)| json!({ "path": wiki::disp(p), "why": why })).collect::<Vec<_>>(),
        })
    }
}

/// Whether `p` lies under `base`, by its components, with no `..` in it.
fn inside(p: &Path, base: &Path) -> bool {
    p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir)) && p.starts_with(base) && p != base
}

/// Puts `bytes` back in `file` as they were: a new file beside it, then a rename.
fn put_back(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = file.with_file_name(format!("{}.{}.undo.tmp", file.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    fs::write(&tmp, bytes)?;
    wiki::retrying(|| fs::rename(&tmp, file)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// `xenolith upgrade --undo`: takes back each change the manifest records, newest first, and
/// then the manifest itself, so the upgrade is waiting again. Touches nothing it didn't record;
/// a change it can't take back (a copy whose old place is taken again, a folder that isn't empty)
/// is left and named, and the manifest stays only when something was left.
pub fn undo(paths: &AppPaths, root: &Path) -> Result<Undone, String> {
    chains::locked(paths, || {
        let Some(mut m) = Manifest::load(paths)? else { return Ok(Undone::default()) };
        let mut out = Undone { stage: Some(m.stage), ..Undone::default() };
        m.stage = Stage::Undoing;
        m.save(paths)?;
        let ours = |p: &Path| inside(p, &paths.state_dir) || inside(p, root);
        let mut keep = vec![];
        for c in m.changes.iter().rev() {
            let left = match c {
                Change::File { path, .. } | Change::Dir { path } if !ours(path) => Some((path.clone(), "it isn't in the state folder or the shared folder".to_string())),
                Change::File { path, before: Some(b) } => put_back(path, b).err().map(|e| (path.clone(), format!("cannot put it back: {e}"))),
                Change::File { path, before: None } => match fs::remove_file(path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Some((path.clone(), format!("cannot remove it: {e}"))),
                    _ => None,
                },
                Change::Dir { path } => match fs::remove_dir(path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Some((path.clone(), "it isn't empty: something else is in it, so it stays".to_string())),
                    _ => None,
                },
                Change::Moved { from, to, .. } if !inside(from, root) || !inside(to, &paths.state_dir) => {
                    Some((to.clone(), "it isn't a move from the shared folder into the state folder".to_string()))
                }
                Change::Moved { from, .. } if fs::symlink_metadata(from).is_ok() => Some((from.clone(), "a folder is there again, so the copy stays where it was moved".to_string())),
                Change::Moved { to, .. } if fs::symlink_metadata(to).is_err() => Some((to.clone(), "the copy isn't there any more".to_string())),
                Change::Moved { from, to, .. } => chains::move_aside(to, from).err().map(|e| (to.clone(), e)),
            };
            match left {
                Some(l) => {
                    out.left.push(l);
                    keep.push(c.clone());
                }
                None => out.undone += 1,
            }
        }
        if keep.is_empty() {
            let f = file(paths);
            fs::remove_file(&f).map_err(|e| format!("took back every change but cannot remove {}: {e}", wiki::disp(&f)))?;
        } else {
            keep.reverse();
            m.changes = keep;
            m.save(paths)?;
        }
        Ok(out)
    })
}

/// The dry run: where the upgrade stands, and what `--run` would do now.
pub fn plan_json(paths: &AppPaths, root: &Path) -> Result<Value, String> {
    let m = Manifest::load(paths)?;
    let stage = m.as_ref().map_or(Stage::Waiting, |m| m.stage);
    let plan = if stage.upgraded() { chains::Plan::default() } else { chains::plan(paths, root)? };
    let names = |v: &[SpaceId]| v.iter().map(ToString::to_string).collect::<Vec<_>>();
    Ok(json!({
        "stage": stage.as_str(),
        "at": m.as_ref().map(|m| m.at),
        "recorded": m.as_ref().map_or(0, |m| m.changes.len()),
        "manifest": wiki::disp(&file(paths)),
        "plan": {
            "found": names(&plan.founded),
            "mark": names(&plan.marked),
            "moveAside": plan.moved.iter().map(|(s, to)| json!({ "space": s.to_string(), "to": wiki::disp(to) })).collect::<Vec<_>>(),
            "bind": [],
            "bindWhy": "a space is founded under its name and owned by this Xenolith's owner key, which can't bind it; `xenolith spaces bind` ties it to its project once shared mode moves it to you",
        },
    }))
}

/// For /status and `xenolith status`: where the upgrade stands, and the next step.
pub fn status_json(paths: &AppPaths) -> Value {
    match Manifest::load(paths) {
        Ok(m) => {
            let stage = m.as_ref().map_or(Stage::Waiting, |m| m.stage);
            let next = match stage {
                Stage::Waiting => Some("the upgrade from 1.x is waiting: `xenolith upgrade` shows what it will do, `xenolith upgrade --run` does it"),
                Stage::Running => Some("the upgrade stopped part way: `xenolith upgrade --run` finishes it"),
                Stage::Undoing => Some("an undo stopped part way: `xenolith upgrade --undo` finishes it"),
                Stage::Fresh | Stage::Done => None,
            };
            json!({ "stage": stage.as_str(), "at": m.as_ref().map(|m| m.at), "changes": m.as_ref().map_or(0, |m| m.changes.len()), "next": next })
        }
        Err(e) => json!({ "stage": "unreadable", "error": e, "next": "the upgrade manifest can't be read, so nothing is founded: see `xenolith doctor`" }),
    }
}

/// Something 1.x left that 2.0 doesn't read and never deletes: what it is, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    pub what: String,
    pub path: PathBuf,
}

fn md_count(dir: &Path) -> usize {
    fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".md")).count()
}

/// 1.x's leftovers, for `xenolith doctor`: its sync and pairing files, the tokens access.json still
/// lists in 1.x's form, each space's 1.x sync state, and the copies the upgrade moved aside, with
/// the notes in them that never reached their curator. Reads only.
pub fn leftovers(paths: &AppPaths, root: &Path) -> Vec<Leftover> {
    let mut out = vec![];
    let mut push = |what: String, path: PathBuf| out.push(Leftover { what, path });
    let [sync, pair, peers] = sync_1x_files(paths);
    if sync.is_file() {
        push("1.x's sync settings (sync.json); 2.0 reads only its `follow` list".into(), sync);
    }
    if pair.is_file() {
        push("1.x's pairing offers".into(), pair);
    }
    if peers.is_dir() {
        let n = fs::read_dir(&peers).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".token")).count();
        push(format!("1.x's peer tokens ({n}), which nothing accepts any more"), peers);
    }
    let access = crate::access::policy_file(paths);
    let old: usize =
        fs::read_to_string(&access).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map_or(0, |v| v["tokens"].as_array().map_or(0, |a| a.iter().filter(|t| t.get("hash").is_some()).count()));
    if old > 0 {
        push(format!("{old} 1.x token(s) in access.json, no longer read: make new ones with `xenolith token create`"), access);
    }
    let followed = chains::followed(paths);
    for s in spaces::list(root) {
        let dir = spaces::space_dir(root, &s);
        let sync = crate::inbox::curator_paths(&dir).cur.join("sync");
        if sync.is_dir() {
            let dead = md_count(&sync.join("dead"));
            push(format!("1.x's sync state for {s}{}", if dead > 0 { format!(", with {dead} note(s) its curator refused") } else { String::new() }), sync);
        }
        let inbox = md_count(&dir.join("inbox"));
        if followed.contains(&s.to_string()) && inbox > 0 {
            push(format!("{inbox} note(s) in {s}'s inbox that 1.x never sent to its curator on another machine"), dir.join("inbox"));
        }
    }
    let copies = chains::copies_dir(paths);
    for kind in fs::read_dir(&copies).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
        for copy in fs::read_dir(kind.path()).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
            let space = format!("{}:{}", kind.file_name().to_string_lossy(), copy.file_name().to_string_lossy());
            let inbox = md_count(&copy.path().join("inbox"));
            push(format!("1.x's copy of {space}, moved aside by the upgrade{}", if inbox > 0 { format!(", with {inbox} note(s) never sent to its curator") } else { String::new() }), copy.path());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::random_hex;
    use std::collections::BTreeMap;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    const T: i64 = 1_792_512_000_000;

    struct Home {
        base: PathBuf,
        paths: AppPaths,
        root: PathBuf,
        context: SeedSigner,
        owner: SeedSigner,
    }

    impl Drop for Home {
        fn drop(&mut self) {
            crate::lock::remove_test_dir(&self.base);
        }
    }

    fn home() -> Home {
        let base = std::env::temp_dir().join(format!("xenolith-upgrade-{}", random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), base.join("home").to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        fs::create_dir_all(&paths.state_dir).unwrap();
        let key = |m: u8| SeedSigner::from_seed(&[m; 32], KeyHome::File).unwrap();
        Home { root: base.join("wiki"), paths, base, context: key(7), owner: key(107) }
    }

    fn sp(s: &str) -> SpaceId {
        SpaceId::parse(s).unwrap()
    }

    /// Each path under a folder: a file's bytes, or None for a folder.
    type Tree = BTreeMap<String, Option<Vec<u8>>>;

    /// Every file under `dir` and its bytes, and every folder, leaving out lock bookkeeping.
    fn snapshot(dir: &Path) -> Tree {
        fn walk(base: &Path, d: &Path, out: &mut Tree) {
            for e in fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if e.file_name() == ".locks" {
                    continue;
                }
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                if p.is_dir() {
                    out.insert(rel, None);
                    walk(base, &p, out);
                } else {
                    out.insert(rel, Some(fs::read(&p).unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    /// A 1.x satellite: a space it curates, two it follows (one with a note never sent), its sync
    /// files, and a marker someone wrote by hand in the root.
    fn satellite(h: &Home) {
        wiki::ensure_wiki(&h.root).unwrap();
        for s in ["project:mine", "project:atlas", "project:shared"] {
            let d = spaces::create(&h.root, &sp(s)).unwrap();
            fs::write(d.join("pages").join("p.md"), format!("# {s}\n")).unwrap();
        }
        fs::write(spaces::space_dir(&h.root, &sp("project:atlas")).join("inbox").join("n1.md"), "a note\n").unwrap();
        fs::write(h.root.join(chains::MARKER), "{\"by\": \"hand\"}\n").unwrap();
        let cfg = json!({ "machine": "neo", "home": "inix", "follow": { "project:atlas": { "peer": "inix" }, "project:shared": { "peer": "inix", "auto": true } } });
        fs::write(chains::sync_1x_file(&h.paths), cfg.to_string()).unwrap();
        fs::create_dir_all(h.paths.config_dir.join("peers")).unwrap();
        fs::write(h.paths.config_dir.join("peers").join("inix.token"), "xenolith_0123\n").unwrap();
    }

    fn both(h: &Home) -> (Tree, Tree) {
        (snapshot(&h.root), snapshot(&h.base.join("home")))
    }

    #[test]
    fn nothing_happens_until_the_run_and_the_undo_puts_every_byte_back() {
        let h = home();
        satellite(&h);
        let before = both(&h);
        assert_eq!(stage(&h.paths).unwrap(), Stage::Waiting);
        // The service's pass while waiting, and the dry run: nothing changes.
        assert_eq!(reconcile(&h.paths, &h.root, &h.context, &h.owner, T).unwrap(), None);
        let plan = plan_json(&h.paths, &h.root).unwrap();
        assert_eq!(plan["stage"], "waiting");
        assert_eq!(plan["plan"]["found"], json!(["machine:local", "project:mine"]));
        assert_eq!(plan["plan"]["mark"], json!(["machine:local", "project:mine"]));
        assert_eq!(plan["plan"]["moveAside"].as_array().unwrap().iter().map(|m| m["space"].clone()).collect::<Vec<_>>(), [json!("project:atlas"), json!("project:shared")]);
        assert_eq!(plan["plan"]["bind"], json!([]));
        assert_eq!(both(&h), before, "the waiting pass and the dry run changed nothing");
        // The run does what the plan said.
        let ran = run(&h.paths, &h.root, &h.context, &h.owner, T).unwrap();
        assert_eq!(ran.already, None);
        assert_eq!(ran.report.founded, vec![sp("machine:local"), sp("project:mine")]);
        assert_eq!(ran.report.marked, ran.report.founded);
        assert_eq!(ran.report.moved, vec![sp("project:atlas"), sp("project:shared")]);
        assert_eq!(stage(&h.paths).unwrap(), Stage::Done);
        assert!(!spaces::exists(&h.root, &sp("project:atlas")));
        assert!(chains::of_address(&h.paths, &sp("project:mine")).is_some());
        let names: Vec<String> = leftovers(&h.paths, &h.root).iter().map(|l| l.what.clone()).collect();
        assert!(names.iter().any(|w| w.contains("copy of project:atlas") && w.contains("1 note(s) never sent")), "{names:?}");
        assert!(names.iter().any(|w| w.contains("peer tokens (1)")), "{names:?}");
        // Again: nothing.
        let after = both(&h);
        assert_eq!(run(&h.paths, &h.root, &h.context, &h.owner, T).unwrap().already, Some(Stage::Done));
        assert_eq!(both(&h), after, "a second run is a no-op");
        // A space that appears later is founded by the service's pass, and recorded.
        spaces::create(&h.root, &sp("project:later")).unwrap();
        assert_eq!(reconcile(&h.paths, &h.root, &h.context, &h.owner, T).unwrap().unwrap().founded, vec![sp("project:later")]);
        // The undo: back to the folder and state as they were, but for what wasn't the upgrade's.
        let undone = undo(&h.paths, &h.root).unwrap();
        assert_eq!(undone.stage, Some(Stage::Done));
        assert!(undone.left.is_empty(), "{:?}", undone.left);
        assert_eq!(stage(&h.paths).unwrap(), Stage::Waiting);
        fs::remove_dir_all(spaces::space_dir(&h.root, &sp("project:later"))).unwrap();
        assert_eq!(both(&h), before, "every byte back, the hand-written marker included");
        assert_eq!(undo(&h.paths, &h.root).unwrap(), Undone::default(), "nothing more to undo");
    }

    #[test]
    fn an_undo_leaves_what_it_cant_take_back_and_says_why() {
        let h = home();
        satellite(&h);
        run(&h.paths, &h.root, &h.context, &h.owner, T).unwrap();
        // 1.x made atlas again in the meantime, and someone put a file in the chains' folder.
        spaces::create(&h.root, &sp("project:atlas")).unwrap();
        fs::write(chains::dir(&h.paths).join("mine.txt"), "x").unwrap();
        let undone = undo(&h.paths, &h.root).unwrap();
        let why: Vec<&str> = undone.left.iter().map(|(_, w)| w.as_str()).collect();
        assert!(why.iter().any(|w| w.contains("a folder is there again")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("isn't empty")), "{why:?}");
        assert!(chains::copies_dir(&h.paths).join("project").join("atlas").join("inbox").join("n1.md").exists(), "the copy is kept");
        assert!(spaces::exists(&h.root, &sp("project:shared")), "the rest came back");
        assert_eq!(stage(&h.paths).unwrap(), Stage::Undoing, "the manifest keeps what's left");
        assert_eq!(Manifest::load(&h.paths).unwrap().unwrap().changes.len(), undone.left.len());
        assert!(run(&h.paths, &h.root, &h.context, &h.owner, T).unwrap_err().contains("--undo"));
    }

    #[test]
    fn a_folder_2_0_makes_itself_counts_as_upgraded_and_one_1x_left_waits() {
        let h = home();
        open_folder(&h.paths, &h.root).unwrap();
        assert_eq!(stage(&h.paths).unwrap(), Stage::Fresh);
        let r = reconcile(&h.paths, &h.root, &h.context, &h.owner, T).unwrap().unwrap();
        assert_eq!(r.founded, vec![sp("machine:local")]);
        assert_eq!(Manifest::load(&h.paths).unwrap().unwrap().changes.iter().filter(|c| matches!(c, Change::File { .. })).count(), 3, "the chain, its marker and the index, recorded");

        // A folder with something in it, or a config folder 1.x synced from: waiting.
        let (a, b) = (home(), home());
        fs::create_dir_all(&a.root).unwrap();
        fs::write(a.root.join("notes.md"), "mine\n").unwrap();
        open_folder(&a.paths, &a.root).unwrap();
        fs::write(chains::sync_1x_file(&b.paths), "{}").unwrap();
        open_folder(&b.paths, &b.root).unwrap();
        for x in [&a, &b] {
            assert_eq!(stage(&x.paths).unwrap(), Stage::Waiting);
            assert_eq!(reconcile(&x.paths, &x.root, &x.context, &x.owner, T).unwrap(), None);
        }
        // Opened again later: still what it was.
        open_folder(&h.paths, &h.root).unwrap();
        open_folder(&a.paths, &a.root).unwrap();
        assert_eq!((stage(&h.paths).unwrap(), stage(&a.paths).unwrap()), (Stage::Fresh, Stage::Waiting));
    }

    #[test]
    fn a_manifest_that_cant_be_read_founds_nothing_and_undoes_nothing() {
        let h = home();
        open_folder(&h.paths, &h.root).unwrap();
        fs::write(file(&h.paths), "{ broken").unwrap();
        assert!(stage(&h.paths).is_err());
        assert!(reconcile(&h.paths, &h.root, &h.context, &h.owner, T).is_err());
        assert!(undo(&h.paths, &h.root).is_err());
        assert_eq!(status_json(&h.paths)["stage"], "unreadable");
        assert_eq!(fs::read_to_string(file(&h.paths)).unwrap(), "{ broken");
        // A change outside both folders is never touched.
        let outside = h.base.join("elsewhere.txt");
        fs::write(&outside, "keep").unwrap();
        let m = Manifest { stage: Stage::Done, at: T, version: "x".into(), changes: vec![Change::File { path: outside.clone(), before: None }] };
        m.save(&h.paths).unwrap();
        let undone = undo(&h.paths, &h.root).unwrap();
        assert_eq!(undone.left.len(), 1);
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    }
}
