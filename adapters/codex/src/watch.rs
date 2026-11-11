//! Where Codex's sessions are written, for the watcher: its rollouts
//! (`sessions/**/rollout-*.jsonl`), the ones it archived
//! (`archived_sessions/`, read whole, never tailed), and its state
//! database, where Codex desktop keeps its projects.

use crate::reader::rollout_id;
use std::path::{Path, PathBuf};
use xenonet_adapter::reader::ListerEnv;
use xenonet_adapter::watch::{self, Cursor, SessionEvent, WatchSpec};

const ROLLOUTS: &str = "rollouts";
const ARCHIVED: &str = "archived";
const STATE: &str = "state";

pub fn specs(env: &ListerEnv<'_>) -> Vec<WatchSpec> {
    if env.root.is_empty() {
        return Vec::new();
    }
    let home = Path::new(env.root);
    vec![
        WatchSpec::tree(ROLLOUTS, home.join("sessions"), &["**/rollout-*.jsonl"]),
        WatchSpec::tree(ARCHIVED, home.join("archived_sessions"), &["**/rollout-*.jsonl", "**/rollout-*.jsonl.zst"]),
        // The database and its write-ahead log, where a write lands first.
        WatchSpec::flat(STATE, home, &["state_*.sqlite", "state_*.sqlite-wal"]),
    ]
}

pub fn changed(paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    let mut state = false;
    for p in paths {
        let Some(tag) = cursor.spec_of(p).map(|s| s.tag) else { continue };
        let norm = watch::slashed(p);
        match tag {
            ROLLOUTS => {
                if let Some(id) = rollout_id(&norm) {
                    watch::transcript(cursor, p, &id, None, &mut out);
                }
            }
            ARCHIVED => {
                let id = rollout_id(norm.strip_suffix(".zst").unwrap_or(&norm));
                watch::changed(p, id, &mut out);
            }
            // The database and its log change together: once is enough.
            _ if !state => {
                state = true;
                watch::changed(p, None, &mut out);
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use xenonet_adapter::watch::Read;

    const T: &str = "0199abcd-0000-7000-8000-000000000001";

    #[test]
    fn reads_rollouts_archives_and_the_state_database() {
        let home = std::env::temp_dir().join(format!("xenonet-codex-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let day = home.join("sessions/2026/11/11");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(home.join("archived_sessions")).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let env = Map::new();
        let root = home.to_string_lossy().into_owned();
        let e = ListerEnv { home_dir: "", root: &root, desktop_dir: None, state_dir: "", env: &env, version: "0", handle: rt.handle().clone() };
        let mut c = Cursor::new(specs(&e));

        let rollout = day.join(format!("rollout-2026-11-11T08-00-00-{T}.jsonl"));
        std::fs::write(&rollout, "{\"type\":\"session_meta\"}\n").unwrap();
        let archived = home.join(format!("archived_sessions/rollout-2026-11-10T08-00-00-{T}.jsonl.zst"));
        std::fs::write(&archived, "x").unwrap();
        let (db, wal) = (home.join("state_5.sqlite"), home.join("state_5.sqlite-wal"));
        std::fs::write(&db, "x").unwrap();
        std::fs::write(&wal, "x").unwrap();
        let log = home.join("log/codex-tui.log");

        let ev = changed(&[rollout.clone(), archived.clone(), db.clone(), wal, log], &mut c);
        assert_eq!(ev.len(), 3, "{ev:?}");
        assert!(matches!(&ev[0], SessionEvent::Lines(l) if l.session == T && l.read == Read::New && l.lines.len() == 1));
        assert_eq!(ev[1], SessionEvent::Changed { session: Some(T.into()), file: archived });
        assert_eq!(ev[2], SessionEvent::Changed { session: None, file: db });
        std::fs::remove_dir_all(&home).unwrap();
    }
}
