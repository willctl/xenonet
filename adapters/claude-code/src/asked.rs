//! What the doctor, `xenonet diagnose --ask` and the map ask of Claude
//! Code: the command that reads a diagnostics bundle, the words for how
//! many of its sessions the map reads, and Claude desktop's scratch
//! folders.

use serde_json::Value;
use xenonet_adapter::platform::{Ask, Scratch};
use xenonet_text::text::to_number;
use xenonet_text::units::plural;

/// Claude Code in plan mode, with the model chosen and XenoNet's docs and
/// code to read; it reads CLAUDE.md, the brief's copy.
pub fn ask(a: &Ask<'_>) -> (String, Vec<String>) {
    let mut args: Vec<String> = vec!["--permission-mode".into(), "plan".into()];
    if let Some(m) = a.model {
        args.extend(["--model".into(), m.to_string()]);
    }
    args.extend(a.sources.iter().flat_map(|s| ["--add-dir".to_string(), s.clone()]));
    args.push(a.prompt.replace("AGENTS.md", "CLAUDE.md"));
    ("claude".into(), args)
}

pub fn counted(count: &Value) -> Option<String> {
    (!count.is_null()).then(|| plural(to_number(Some(count)), "Claude session", None))
}

/// The folders Claude desktop makes for a task without a project.
pub fn scratch() -> Scratch {
    Scratch { id: "zone:claude-scratch", name: "Claude scratch", holds: |k| k.contains("/claude/scratch-workspaces/") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_mode_reads_claude_md() {
        let a = ask(&Ask { model: None, dir: "/d", sources: &[], prompt: "Read bundle.md and AGENTS.md here" });
        assert_eq!(a, ("claude".into(), ["--permission-mode", "plan", "Read bundle.md and CLAUDE.md here"].map(String::from).to_vec()));
        assert!((scratch().holds)("c:/users/me/appdata/roaming/claude/scratch-workspaces/a") && !(scratch().holds)("/tmp/x"));
        assert_eq!((counted(&json!(0)).as_deref(), counted(&Value::Null)), (Some("0 Claude sessions"), None));
    }
}
