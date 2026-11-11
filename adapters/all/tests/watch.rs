//! Which adapters' sessions are watched, and where, for one home folder:
//! every adapter that lists sessions from files gives specs, and the
//! observers of actor products give none (their scan stays their path).

use serde_json::Map;
use std::path::{Path, PathBuf};
use xenonet_adapter::reader::ListerEnv;
use xenonet_adapter::watch::{Cursor, SessionEvent};
use xenonet_adapter::Platform;

fn env_for<'a>(p: &dyn Platform, home: &'a str, roots: &'a [(String, String)], env: &'a Map<String, serde_json::Value>, rt: &tokio::runtime::Runtime) -> ListerEnv<'a> {
    let root = roots.iter().find(|(id, _)| id == p.id()).map_or("", |(_, r)| r.as_str());
    ListerEnv { home_dir: home, root, desktop_dir: None, state_dir: home, env, version: "0", handle: rt.handle().clone() }
}

#[test]
fn the_file_listed_apps_are_watched() {
    let home = "/h";
    let roots = [("claude".to_string(), "/h/.claude/projects".to_string()), ("codex".into(), "/h/.codex".into()), ("grok".into(), "/h/.grok".into())];
    let env = Map::new();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let watched: Vec<(String, Vec<PathBuf>)> = xenonet_platforms::all()
        .iter()
        .map(|p| (p.id().to_string(), p.watch(&env_for(p.as_ref(), home, &roots, &env, &rt)).into_iter().map(|s| s.root).collect::<Vec<_>>()))
        .filter(|(_, roots)| !roots.is_empty())
        .collect();
    let ids: Vec<&str> = watched.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["claude", "codex", "grok", "opencode", "cursor", "gemini-cli", "copilot"]);
    let roots_of = |id: &str| watched.iter().find(|(i, _)| i == id).map(|(_, r)| r.clone()).unwrap();
    assert_eq!(roots_of("claude"), [PathBuf::from("/h/.claude/projects")]);
    assert_eq!(roots_of("codex"), [Path::new("/h/.codex/sessions"), Path::new("/h/.codex/archived_sessions"), Path::new("/h/.codex")]);
    assert_eq!(roots_of("opencode"), [PathBuf::from("/h/.local/share/opencode")]);
    assert_eq!(roots_of("copilot"), [PathBuf::from("/h/.copilot/session-state")]);
}

#[test]
fn a_copilot_log_names_its_session() {
    let copilot = xenonet_platforms::all().into_iter().find(|p| p.id() == "copilot").unwrap();
    let env = Map::new();
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let mut c = Cursor::new(copilot.watch(&env_for(copilot.as_ref(), "/h", &[], &env, &rt)));
    let (new, old) = (PathBuf::from("/h/.copilot/session-state/0f3c/events.jsonl"), PathBuf::from("/h/.copilot/session-state/9a1b.jsonl"));
    assert_eq!(
        copilot.changed(&[new.clone(), old.clone()], &mut c),
        [SessionEvent::Changed { session: Some("0f3c".into()), file: new }, SessionEvent::Changed { session: Some("9a1b".into()), file: old }]
    );
}
