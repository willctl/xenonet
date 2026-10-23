//! digest.js's askCodex, run: one `codex exec` turn on a prompt (the
//! words for the day or for while you were away, or Codex's pass over the
//! projects folder), resolving to its last message. Its arguments, the
//! prompt and the run's own Codex home are xenonet-state's digest.rs and
//! classify.rs: the run shares only your sign-in, none of your Codex's
//! plugins, hooks or instructions.

use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(120);

/// `codex exec` on `prompt`; with a JSON Schema in `schema`, Codex answers
/// in that shape (--output-schema). `codex_home` is your Codex's home,
/// whose sign-in the run uses.
pub async fn ask_codex(bin: &str, prompt: &str, schema: Option<Value>, codex_home: &str) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("xenonet-codex-{}-{}", std::process::id(), crate::util::now_ms() as u64));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join("last.txt");
    let ready = (|| -> Result<(Option<String>, String), String> {
        let schema_file = match schema {
            Some(s) => {
                let f = dir.join("schema.json");
                std::fs::write(&f, xenonet_base::json::stringify(&s, 0)).map_err(|e| e.to_string())?;
                Some(f.to_string_lossy().into_owned())
            }
            None => None,
        };
        let home = xenonet_state::digest::codex_run_home(&dir.to_string_lossy(), codex_home).map_err(|e| e.to_string())?;
        Ok((schema_file, home))
    })();
    let (schema_file, home) = match ready {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };
    let args = xenonet_state::digest::codex_exec_args(&dir.to_string_lossy(), &out.to_string_lossy(), schema_file.as_deref());
    let ran = async {
        let mut child = Command::new(bin)
            .args(&args)
            .env("CODEX_HOME", &home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        // A codex that quits before reading all of a long prompt breaks the
        // pipe; its exit says what happened.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
        }
        let done = tokio::time::timeout(TIMEOUT, child.wait_with_output()).await.map_err(|_| "codex exec timed out".to_string())?.map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&done.stderr).into_owned();
        Ok::<_, String>((done.status.code(), stderr))
    }
    .await;
    let msg = std::fs::read_to_string(&out).map(|t| xenonet_base::text::js_trim(&t).to_string()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let (code, stderr) = ran?;
    if msg.is_empty() {
        let tail: String = stderr.chars().rev().take(300).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("codex exec exited {}: {tail}", code.map_or_else(|| "null".into(), |c| c.to_string())));
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_has_a_codex_home_of_its_own_with_your_sign_in() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xenonet-narrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let yours = dir.join("yours");
        std::fs::create_dir_all(&yours).unwrap();
        std::fs::write(yours.join("auth.json"), "{\"signed\":\"in\"}").unwrap();
        std::fs::write(yours.join("AGENTS.md"), "Always answer in French.").unwrap();
        // A stand-in for codex: it notes the home it ran in and what's
        // there, then answers where -o says.
        let seen = dir.join("seen");
        let fake = dir.join("codex");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$CODEX_HOME\" > '{seen}'\nls -A \"$CODEX_HOME\" >> '{seen}'\ncat \"$CODEX_HOME/auth.json\" >> '{seen}'\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=\"$2\"; shift; done\ncat > /dev/null\nprintf '  All quiet.\\n' > \"$out\"\n",
            seen = seen.display()
        );
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let said = super::ask_codex(&fake.to_string_lossy(), "Sort these", None, &yours.to_string_lossy()).await;
        assert_eq!(said.as_deref(), Ok("All quiet."));
        let seen = std::fs::read_to_string(&seen).unwrap();
        let mut lines = seen.lines();
        let home = lines.next().unwrap();
        assert!(home.starts_with(&std::env::temp_dir().join("xenonet-codex-").to_string_lossy().into_owned()) && home.ends_with("home"), "{home}");
        assert_eq!(lines.collect::<Vec<_>>(), ["auth.json", "{\"signed\":\"in\"}"], "your sign-in, and none of your instructions");
        assert!(!std::path::Path::new(home).exists(), "the run's folder is gone afterwards");
        assert_eq!(std::fs::read_to_string(yours.join("auth.json")).unwrap(), "{\"signed\":\"in\"}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
