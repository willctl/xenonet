//! core/secret-store.js's two stores (crates/xenonet-actors/src/secret_store.rs):
//! your computer's own credential store, whose program runs here with the
//! secret only ever on its standard input, and the memory store the demo
//! and test instances keep theirs in, so nothing reaches yours from them.

use crate::fx::Locked;
use std::io::{Read, Write};
use std::process::{Command as Proc, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xenonet_actors::secret_store::{Command, MemorySecrets, Ran, Run, SecretStore};

pub enum Secrets {
    Store { platform: String, tool: Option<String> },
    Memory(Mutex<MemorySecrets>),
}

/// runFile(file, args, { input }): { code, stdout }, never failing for a
/// non-zero exit; -1 when it couldn't run, or ran past 20 seconds.
struct Runner;

impl Run for Runner {
    fn run(&mut self, c: Command) -> Result<Ran, String> {
        Ok(run_blocking(&c.file, &c.args, c.input.as_deref().unwrap_or(""), Duration::from_secs(20)))
    }
}

fn run_blocking(file: &str, args: &[String], input: &str, timeout: Duration) -> Ran {
    let mut cmd = Proc::new(file);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let Ok(mut child) = cmd.spawn() else { return Ran { code: -1.0, stdout: String::new() } };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut out);
        }
        out
    });
    let started = Instant::now();
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().map_or(-1.0, f64::from),
            Ok(None) if started.elapsed() < timeout => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break -1.0;
            }
        }
    };
    let out = reader.join().unwrap_or_default();
    Ran { code, stdout: String::from_utf8_lossy(&out).into_owned() }
}

impl Secrets {
    /// Your credential store: the keychain, Windows Credential Manager or
    /// secret-tool on PATH.
    pub fn real() -> Secrets {
        let platform = crate::fx::platform().to_string();
        let tool = if platform != "darwin" && platform != "win32" { crate::fx::on_path("secret-tool") } else { None };
        Secrets::Store { platform, tool }
    }

    pub fn memory() -> Secrets {
        Secrets::Memory(Mutex::new(MemorySecrets::default()))
    }

    fn store(platform: &str, tool: &Option<String>) -> SecretStore {
        SecretStore::new(platform, tool.clone())
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Secrets::Store { platform, tool } => Self::store(platform, tool).kind(),
            Secrets::Memory(_) => MemorySecrets::KIND,
        }
    }

    pub fn available(&self) -> bool {
        match self {
            Secrets::Store { platform, tool } => Self::store(platform, tool).available(),
            Secrets::Memory(_) => true,
        }
    }

    // The store's program runs on a blocking thread: it can take a while
    // (the keychain may ask you first).
    async fn blocking<T: Send + 'static>(
        platform: &str,
        tool: &Option<String>,
        f: impl FnOnce(&SecretStore, &mut Runner) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let store = Self::store(platform, tool);
        tokio::task::spawn_blocking(move || f(&store, &mut Runner)).await.map_err(|e| e.to_string())?
    }

    pub async fn has(&self, target: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().has(target)),
            Secrets::Store { platform, tool } => {
                let t = target.to_string();
                Self::blocking(platform, tool, move |s, r| s.has(r, &t)).await
            }
        }
    }

    pub async fn get(&self, target: &str) -> Result<Option<String>, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().get(target)),
            Secrets::Store { platform, tool } => {
                let t = target.to_string();
                Self::blocking(platform, tool, move |s, r| s.get(r, &t)).await
            }
        }
    }

    pub async fn set(&self, target: &str, secret: &str, comment: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => m.locked().set(target, secret),
            Secrets::Store { platform, tool } => {
                let (t, s, c) = (target.to_string(), secret.to_string(), comment.to_string());
                Self::blocking(platform, tool, move |st, r| st.set(r, &t, &s, &c)).await
            }
        }
    }

    pub async fn remove(&self, target: &str) -> Result<bool, String> {
        match self {
            Secrets::Memory(m) => Ok(m.locked().remove(target)),
            Secrets::Store { platform, tool } => {
                let t = target.to_string();
                Self::blocking(platform, tool, move |s, r| s.remove(r, &t)).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_keeps_what_its_given_and_checks_it() {
        let s = Secrets::memory();
        assert!(!s.has("XenoNet/Test/one").await.unwrap());
        assert!(s.set("XenoNet/Test/one", "made-up-secret-1234", "a test's").await.unwrap());
        assert_eq!(s.get("XenoNet/Test/one").await.unwrap().as_deref(), Some("made-up-secret-1234"));
        assert!(s.set("not a target", "x", "").await.is_err());
        assert!(s.remove("XenoNet/Test/one").await.unwrap());
        assert_eq!(s.get("XenoNet/Test/one").await.unwrap(), None);
        assert_eq!(s.kind(), MemorySecrets::KIND);
    }

    #[cfg(unix)]
    #[test]
    fn runs_with_input_and_a_time_limit() {
        let r = run_blocking("/bin/cat", &[], "fed", Duration::from_secs(5));
        assert_eq!((r.code, r.stdout.as_str()), (0.0, "fed"));
        let r = run_blocking("/bin/sh", &["-c".into(), "sleep 5".into()], "", Duration::from_millis(50));
        assert_eq!(r.code, -1.0);
        assert_eq!(run_blocking("/no/such/program", &[], "", Duration::from_secs(1)).code, -1.0);
    }
}
