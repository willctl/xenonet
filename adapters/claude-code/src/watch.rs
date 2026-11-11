//! Where Claude Code's sessions are written, for the watcher: the
//! transcripts under its projects folder (a session's, a subagent's, and a
//! session's custom-title.json beside them), and Claude desktop's records
//! of its sessions (`claude-code-sessions/**/local_*.json`).

use crate::reader::classify;
use std::path::{Path, PathBuf};
use xenonet_adapter::reader::ListerEnv;
use xenonet_adapter::watch::{self, Cursor, SessionEvent, WatchSpec};

const TRANSCRIPTS: &str = "transcripts";
const DESKTOP: &str = "desktop";

pub fn specs(env: &ListerEnv<'_>) -> Vec<WatchSpec> {
    let mut out = Vec::new();
    if !env.root.is_empty() {
        out.push(WatchSpec::tree(TRANSCRIPTS, env.root, &["*/*.jsonl", "*/*/subagents/*.jsonl", "*/*/custom-title.json"]));
    }
    if let Some(d) = env.desktop_dir.filter(|d| !d.is_empty()) {
        out.push(WatchSpec::tree(DESKTOP, Path::new(d).join("claude-code-sessions"), &["**/local_*.json"]));
    }
    out
}

// The session a desktop record is of, as its cliSessionId names it.
fn desktop_session(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v = xenonet_text::json::parse(&text).ok()?;
    v.get("cliSessionId").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn changed(paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    for p in paths {
        let Some(tag) = cursor.spec_of(p).map(|s| s.tag) else { continue };
        let norm = watch::slashed(p);
        match tag {
            TRANSCRIPTS if norm.ends_with("/custom-title.json") => {
                let session = p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
                watch::changed(p, session, &mut out);
            }
            TRANSCRIPTS => {
                if let Some(c) = classify(&norm) {
                    watch::transcript(cursor, p, &c.id, c.agent.as_deref(), &mut out);
                }
            }
            _ => watch::changed(p, desktop_session(p), &mut out),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use xenonet_adapter::watch::{Lines, Read};

    const S: &str = "0199abcd-0000-7000-8000-000000000001";

    #[test]
    fn reads_transcripts_sidecars_and_desktop_records() {
        let home = std::env::temp_dir().join(format!("xenonet-claude-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let projects = home.join("projects");
        let folder = projects.join("-w-atlas");
        std::fs::create_dir_all(folder.join(S).join("subagents")).unwrap();
        let desktop = home.join("desktop");
        std::fs::create_dir_all(desktop.join("claude-code-sessions/x")).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let env = Map::new();
        let (root, desk) = (projects.to_string_lossy().into_owned(), desktop.to_string_lossy().into_owned());
        let e = ListerEnv { home_dir: "", root: &root, desktop_dir: Some(&desk), state_dir: "", env: &env, version: "0", handle: rt.handle().clone() };
        let specs = specs(&e);
        assert_eq!(specs.len(), 2);
        let mut c = Cursor::new(specs);

        let own = folder.join(format!("{S}.jsonl"));
        std::fs::write(&own, "{\"type\":\"user\"}\n").unwrap();
        let sub = folder.join(S).join("subagents/agent-a1.jsonl");
        std::fs::write(&sub, "{}\n").unwrap();
        let title = folder.join(S).join("custom-title.json");
        std::fs::write(&title, "{}").unwrap();
        let record = desktop.join("claude-code-sessions/x/local_1.json");
        std::fs::write(&record, format!("{{\"cliSessionId\":\"{S}\"}}")).unwrap();
        let other = folder.join("notes.jsonl");
        std::fs::write(&other, "{}\n").unwrap();

        let ev = changed(&[own.clone(), sub.clone(), title.clone(), record.clone(), other], &mut c);
        assert_eq!(
            ev,
            vec![
                SessionEvent::Lines(Lines { session: S.into(), agent: None, file: own.clone(), from: 0, to: 16, read: Read::New, lines: vec!["{\"type\":\"user\"}".into()] }),
                SessionEvent::Lines(Lines { session: S.into(), agent: Some("a1".into()), file: sub, from: 0, to: 3, read: Read::New, lines: vec!["{}".into()] }),
                SessionEvent::Changed { session: Some(S.into()), file: title },
                SessionEvent::Changed { session: Some(S.into()), file: record },
            ]
        );
        std::fs::remove_file(&own).unwrap();
        assert_eq!(changed(std::slice::from_ref(&own), &mut c), vec![SessionEvent::Gone { session: S.into(), agent: None, file: own }]);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
