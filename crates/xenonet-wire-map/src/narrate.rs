//! digest.js's askCodex, run: one `codex exec` turn on a prompt (the
//! words for the day or for while you were away, or Codex's pass over the
//! projects folder), resolving to its last message. Its arguments and the
//! prompt are xenonet-state's digest.rs and classify.rs.

use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(120);

/// `codex exec` on `prompt`; with a JSON Schema in `schema`, Codex answers
/// in that shape (--output-schema).
pub async fn ask_codex(bin: &str, prompt: &str, schema: Option<Value>) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("xenonet-codex-{}-{}", std::process::id(), crate::util::now_ms() as u64));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join("last.txt");
    let schema_file = match schema {
        Some(s) => {
            let f = dir.join("schema.json");
            std::fs::write(&f, xenonet_base::json::stringify(&s, 0)).map_err(|e| e.to_string())?;
            Some(f.to_string_lossy().into_owned())
        }
        None => None,
    };
    let args = xenonet_state::digest::codex_exec_args(&dir.to_string_lossy(), &out.to_string_lossy(), schema_file.as_deref());
    let ran = async {
        let mut child = Command::new(bin)
            .args(&args)
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
