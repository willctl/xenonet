//! What the doctor, `xenonet diagnose --ask` and the map ask of Codex: the
//! command that reads a diagnostics bundle, the command the doctor's
//! checks look for, whether it's installed, the words for how many of its
//! threads the map reads, and the folders it makes for a task without a
//! project.

use async_trait::async_trait;
use serde_json::{json, Value};
use xenonet_adapter::dirs;
use xenonet_adapter::platform::{Ask, Presence, Scratch};
use xenonet_adapter::{CheckCtx, Stop};
use xenonet_text::text::to_number;
use xenonet_text::units::plural;

/// Codex read-only and asking before it does anything, with the model
/// chosen and XenoNet's docs and code to read.
pub fn ask(a: &Ask<'_>) -> (String, Vec<String>) {
    let mut args: Vec<String> = ["-C", a.dir, "-s", "read-only", "-a", "on-request"].into_iter().map(str::to_string).collect();
    if let Some(m) = a.model {
        args.extend(["-m".into(), m.to_string()]);
    }
    args.extend(a.sources.iter().flat_map(|s| ["--add-dir".to_string(), s.clone()]));
    args.push(a.prompt.to_string());
    ("codex".into(), args)
}

/// findCodexBinary() (core/dirs.js), as the checks' `codexBin`:
/// XENONET_CODEX_BIN, the ChatGPT app's own codex, or "codex".
pub fn facts(home: &str, env: &dirs::Env, look: &mut dyn dirs::Look) -> Vec<(&'static str, Value)> {
    let bin = std::env::var("XENONET_CODEX_BIN")
        .ok()
        .unwrap_or_else(|| dirs::bundled_codex_binaries(env, xenonet_os::platform(), home, look).into_iter().next().unwrap_or_else(|| "codex".into()));
    vec![("codexBin", json!(bin))]
}

/// Installed when there's a command to run (`checks::codex_bin`).
pub struct Installed;

#[async_trait(?Send)]
impl Presence for Installed {
    async fn installed(&self, c: &dyn CheckCtx) -> Result<bool, Stop> {
        Ok(crate::checks::codex_bin(c).await?.is_some())
    }
}

pub fn counted(count: &Value) -> Option<String> {
    (!count.is_null()).then(|| plural(to_number(Some(count)), "Codex thread", None))
}

/// The ChatGPT app's folders for a task without a project, one a day.
pub fn scratch() -> Scratch {
    Scratch { id: "zone:codex-projectless", name: "Codex without a project", holds: day_folder }
}

// /documents/codex/\d{4}-\d{2}-\d{2}(\/|$)
fn day_folder(k: &str) -> bool {
    const AT: &str = "/documents/codex/";
    k.match_indices(AT).any(|(i, _)| {
        let d = &k.as_bytes()[i + AT.len()..];
        let digits = |r: std::ops::Range<usize>| d.len() >= r.end && d[r].iter().all(u8::is_ascii_digit);
        digits(0..4) && d.get(4) == Some(&b'-') && digits(5..7) && d.get(7) == Some(&b'-') && digits(8..10) && matches!(d.get(10), None | Some(b'/'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_folder_is_a_task_without_a_project() {
        assert!(day_folder("/users/me/documents/codex/2026-10-02/regex") && day_folder("/users/me/documents/codex/2026-10-02"));
        assert!(!day_folder("/users/me/documents/codex/2026-10-0x") && !day_folder("/users/me/documents/codex/notes"));
        let a = ask(&Ask { model: Some("m"), dir: "/d", sources: &["/x".into()], prompt: "Read AGENTS.md" });
        assert_eq!(a, ("codex".into(), ["-C", "/d", "-s", "read-only", "-a", "on-request", "-m", "m", "--add-dir", "/x", "Read AGENTS.md"].map(String::from).to_vec()));
        assert_eq!((counted(&json!(1)).as_deref(), counted(&Value::Null)), (Some("1 Codex thread"), None));
    }
}
