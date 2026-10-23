//! The migration's journal: what it found, what it backed up (with SHA-256), and each step's status,
//! written atomically and readable only by you before and after every change. A run that stops
//! anywhere resumes from it, and --rollback undoes what it says was done. It holds paths, hashes,
//! counts and statuses, never a note's text, a setting's value or a secret.

use super::edits::fwd;
use super::scan::Scan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FORMAT: u32 = 1;

/// One file the migration (or the install it runs) may change, and its state before and after.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracked {
    pub path: String,
    /// What it is, for the report ("Claude Code settings", "LaunchAgent com.agentwiki.server").
    pub role: String,
    /// SHA-256 of the file before the migration; None: it did not exist.
    pub pre: Option<String>,
    /// The copy of it in the backup folder (relative to it).
    pub backup: Option<String>,
    /// Every state the migration has left it in (None: removed), oldest first. Its own edits are
    /// recorded here before they are written, so a run stopped mid-write still knows the file.
    pub known: Vec<Option<String>>,
    /// An installer or an app's CLI may be changing it right now: a run stopped while one ran
    /// leaves this set, and --rollback then restores the file whatever it holds.
    pub busy: bool,
    /// Whether <file>.bak-xenolith existed before (the installer makes one on its first edit).
    pub bak_xenolith_pre: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepRec {
    pub status: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// A folder moved from one place to another.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Move {
    pub from: String,
    pub to: String,
    /// planned | moving | moved | kept | absent | rolled-back
    pub status: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub format: u32,
    pub started_at: String,
    pub xenolith_version: String,
    pub os: String,
    /// running | done | rolling-back | rolled-back | finished
    pub state: String,
    pub aw_era: String,
    pub aw_version: Option<String>,
    pub aw_folders: Vec<String>,
    pub aw_config: Option<String>,
    pub codex_plugin_id: String,
    pub wiki_dir: String,
    pub xenolith_installed_before: bool,
    /// Whether agent-wiki ran its tools without asking (else Xenolith gets --no-approve).
    pub approve: bool,
    /// What agent-wiki's installer created (from its install-state.json), for Xenolith's uninstall.
    pub aw_created: Vec<String>,
    pub snapshot: Option<Scan>,
    pub protocol_before: Option<String>,
    pub files: Vec<Tracked>,
    pub steps: BTreeMap<String, StepRec>,
    /// Step 6's parts: id -> done | pending | skipped.
    pub items: BTreeMap<String, String>,
    pub pending: Vec<String>,
    pub warnings: Vec<String>,
    pub codex_home: Option<Move>,
    pub paused_by_us: bool,
    /// Windows: the AgentWiki service before the migration (its state), the tray's Run value and
    /// whether the logon task existed, for rollback.
    pub aw_service: Option<String>,
    pub aw_run_value: Option<String>,
    pub aw_task: bool,
    pub verify: Option<serde_json::Value>,
    pub rollback: BTreeMap<String, String>,
    pub finish: BTreeMap<String, String>,
    pub wiki_move: Option<Move>,
}

pub fn now() -> String {
    xenolith_core::text::local_iso(&xenolith_core::text::now())
}

pub fn sha_file(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| xenolith_core::text::sha256_hex(&b))
}

impl Journal {
    pub fn file(work: &Path) -> PathBuf {
        work.join("journal.json")
    }

    pub fn load(work: &Path) -> Result<Option<Journal>, String> {
        let f = Self::file(work);
        match std::fs::read_to_string(&f) {
            Ok(t) => serde_json::from_str::<Journal>(&t).map(Some).map_err(|e| format!("the migration journal {} cannot be read ({e}); move it aside to start over", fwd(&f))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", fwd(&f))),
        }
    }

    /// Writes the journal atomically, readable only by you.
    pub fn save(&self, work: &Path) -> Result<(), String> {
        crash::point()?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_private(&Self::file(work), &format!("{text}\n")).map_err(|e| format!("cannot write the journal in {}: {e}", fwd(work)))
    }

    pub fn step(&self, n: &str) -> &str {
        self.steps.get(n).map(|s| s.status.as_str()).unwrap_or("")
    }

    pub fn set_step(&mut self, n: &str, status: &str) {
        let rec = self.steps.entry(n.to_string()).or_default();
        rec.status = status.into();
        rec.at = now();
    }

    pub fn note(&mut self, n: &str, line: String) {
        let rec = self.steps.entry(n.to_string()).or_default();
        if !rec.notes.contains(&line) {
            rec.notes.push(line);
        }
    }

    /// Records the state a tracked file is about to be put in (before the write).
    pub fn expect(&mut self, p: &Path, sha: Option<String>) {
        let k = fwd(p);
        if let Some(t) = self.files.iter_mut().find(|t| t.path == k) {
            if !t.known.contains(&sha) {
                t.known.push(sha);
            }
        }
    }

    /// Records a tracked file's state after the migration (or a program it ran) changed it.
    pub fn mark_post(&mut self, p: &Path) {
        let k = fwd(p);
        let sha = sha_file(p);
        if let Some(t) = self.files.iter_mut().find(|t| t.path == k) {
            t.busy = false;
            if sha != t.pre && !t.known.contains(&sha) {
                t.known.push(sha);
            }
        }
    }

    pub fn mark_all(&mut self) {
        for p in self.files.iter().map(|t| PathBuf::from(&t.path)).collect::<Vec<_>>() {
            self.mark_post(&p);
        }
    }

    /// Before a program that may change these files runs.
    pub fn set_busy(&mut self, paths: &[&Path]) {
        let keys: Vec<String> = paths.iter().map(|p| fwd(p)).collect();
        for t in self.files.iter_mut().filter(|t| keys.contains(&t.path)) {
            t.busy = true;
        }
    }

    pub fn pend(&mut self, line: String) {
        if !self.pending.contains(&line) {
            self.pending.push(line);
        }
    }

    pub fn warn(&mut self, line: String) {
        if !self.warnings.contains(&line) {
            self.warnings.push(line);
        }
    }
}

/// A file only you may read, written atomically and flushed to disk (private::write). The unit
/// tests, which write the journal some ten thousand times to stop a run at every write, skip the
/// flush on Unix; nothing else differs.
fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    #[cfg(all(unix, test))]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
        let tmp = PathBuf::from(format!("{}.{}.tmp", file.display(), xenolith_core::text::random_hex(8)));
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?.write_all(text.as_bytes())?;
        std::fs::rename(&tmp, file)
    }
    #[cfg(not(all(unix, test)))]
    {
        xenolith_core::private::write(file, text, &[])
    }
}

/// Copies `file` into the backup folder, readable only by you: (relative name, SHA-256), or None
/// when the file does not exist.
pub fn backup(work: &Path, idx: usize, file: &Path) -> Result<Option<(String, String)>, String> {
    let Some(text) = super::edits::read(file)? else { return Ok(None) };
    let name = format!("{idx:02}-{}", file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into()));
    let dest = work.join("backup").join(&name);
    write_private(&dest, &text).map_err(|e| format!("cannot back up {}: {e}", fwd(file)))?;
    let sha = xenolith_core::text::sha256_hex(text.as_bytes());
    if sha_file(&dest).as_deref() != Some(sha.as_str()) {
        return Err(format!("the backup of {} does not match it", fwd(file)));
    }
    Ok(Some((format!("backup/{name}"), sha)))
}

/// One line per event in <work>/migrate.log: what was done, with paths and counts only.
pub fn log(work: &Path, line: &str) {
    use std::io::Write;
    if std::fs::create_dir_all(work).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(work.join("migrate.log")) {
        let _ = writeln!(f, "{} {line}", now());
    }
}

/// Tests stop the migration at a chosen journal write, the way a kill would: the write does not
/// happen and nothing after it runs.
pub mod crash {
    use std::cell::Cell;

    thread_local! {
        static AT: Cell<Option<usize>> = const { Cell::new(None) };
        static SEEN: Cell<usize> = const { Cell::new(0) };
    }

    /// Outside the tests nothing is armed, so this never fails.
    pub fn point() -> Result<(), String> {
        let n = SEEN.with(|s| {
            s.set(s.get() + 1);
            s.get()
        });
        if AT.with(Cell::get) == Some(n) {
            return Err(format!("{KILLED} at journal write {n}"));
        }
        Ok(())
    }

    pub fn is_kill(e: &str) -> bool {
        e.starts_with(KILLED)
    }

    pub const KILLED: &str = "simulated kill";

    #[cfg(test)]
    pub fn arm(at: Option<usize>) {
        AT.with(|a| a.set(at));
        SEEN.with(|s| s.set(0));
    }

    #[cfg(test)]
    pub fn seen() -> usize {
        SEEN.with(Cell::get)
    }
}
