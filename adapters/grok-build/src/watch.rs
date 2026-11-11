//! Where Grok Build's sessions are written, for the watcher: each
//! session's folder under `sessions/<encoded cwd>/<session id>/`, its
//! updates.jsonl tailed, its summary.json, usage.json and subagents'
//! meta.json read whole, and the `.cwd` a long folder name leaves its
//! folder in.

use std::path::{Path, PathBuf};
use xenonet_adapter::reader::ListerEnv;
use xenonet_adapter::watch::{self, Cursor, SessionEvent, WatchSpec};

pub fn specs(env: &ListerEnv<'_>) -> Vec<WatchSpec> {
    if env.root.is_empty() {
        return Vec::new();
    }
    vec![WatchSpec::tree("sessions", Path::new(env.root).join("sessions"), &["*/*/updates.jsonl", "*/*/summary.json", "*/*/usage.json", "*/*/subagents/*/meta.json", "*/.cwd"])]
}

pub fn changed(paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    for p in paths {
        let Some(rel) = cursor.spec_of(p).and_then(|s| s.relative(p)) else { continue };
        let parts: Vec<&str> = rel.split('/').collect();
        match parts.as_slice() {
            [_, id, "updates.jsonl"] => watch::transcript(cursor, p, id, None, &mut out),
            [_, id, _, ..] => watch::changed(p, Some((*id).to_string()), &mut out),
            _ => watch::changed(p, None, &mut out),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn tails_updates_and_reads_the_rest_whole() {
        let home = std::env::temp_dir().join(format!("xenonet-grok-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let s = home.join("sessions/%2Fw/abc-1");
        std::fs::create_dir_all(s.join("subagents/x")).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let env = Map::new();
        let root = home.to_string_lossy().into_owned();
        let e = ListerEnv { home_dir: "", root: &root, desktop_dir: None, state_dir: "", env: &env, version: "0", handle: rt.handle().clone() };
        let mut c = Cursor::new(specs(&e));
        let (updates, summary, meta, cwd) = (s.join("updates.jsonl"), s.join("summary.json"), s.join("subagents/x/meta.json"), home.join("sessions/%2Fw/.cwd"));
        std::fs::write(&updates, "{}\n").unwrap();
        for f in [&summary, &meta, &cwd] {
            std::fs::write(f, "{}").unwrap();
        }
        let ev = changed(&[updates, summary.clone(), meta.clone(), cwd.clone(), s.join("history.jsonl")], &mut c);
        assert!(matches!(&ev[0], SessionEvent::Lines(l) if l.session == "abc-1" && l.lines == ["{}"]));
        assert_eq!(
            ev[1..],
            [
                SessionEvent::Changed { session: Some("abc-1".into()), file: summary },
                SessionEvent::Changed { session: Some("abc-1".into()), file: meta },
                SessionEvent::Changed { session: None, file: cwd },
            ]
        );
        std::fs::remove_dir_all(&home).unwrap();
    }
}
