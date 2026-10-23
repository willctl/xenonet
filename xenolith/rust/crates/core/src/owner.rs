//! The owner key: what deciding a held change or a proposal, changing the approvals setting,
//! forgetting text and leaving a note as the person need in the window's API (/api). The HTTP
//! service makes a new random key each time it starts and writes it to <state>/runtime/owner.key,
//! readable by the owner alone (private.rs; on Windows, by the installing user the service ini
//! names). The tray reads it and opens the window with it; `xenolith held approve` reads it too.
//! A process that only sends the window's X-Xenolith header can read the window's API, not decide.
//!
//! What this guarantees, and no more: a decision needs an explicit read of an owner-only file, and
//! the audit chain records each one with who made it. A sandboxed agent that can't read the state
//! folder can't approve. An agent running unsandboxed as the user can read the file, as it can
//! every other file of the user's.

use crate::paths::AppPaths;
use crate::text::{local_iso, now};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The request header that carries the key.
pub const HEADER: &str = "x-xenolith-owner";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerKey {
    pub key: String,
    /// The service that wrote it: a reader checks that this pid answers on `port` before using it.
    pub pid: u32,
    pub port: u16,
}

pub fn key_file(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("runtime").join("owner.key")
}

fn is_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A new key for the service on `port`, written for its owner (and `readers`, on Windows).
pub fn create(paths: &AppPaths, port: u16, readers: &[String]) -> std::io::Result<OwnerKey> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(format!("no random numbers for the owner key: {e}")))?;
    let key = OwnerKey { key: crate::text::hex(&bytes), pid: std::process::id(), port };
    let text = json!({ "v": 1, "key": key.key, "pid": key.pid, "port": port, "at": local_iso(&now()) });
    crate::private::write(&key_file(paths), &format!("{text}\n"), readers)?;
    Ok(key)
}

pub fn parse(text: &str) -> Option<OwnerKey> {
    let v: Value = serde_json::from_str(text).ok()?;
    let key = v["key"].as_str().filter(|k| is_key(k))?.to_string();
    Some(OwnerKey { key, pid: u32::try_from(v["pid"].as_u64()?).ok()?, port: u16::try_from(v["port"].as_u64()?).ok()? })
}

/// The key the running service wrote, if this process may read it.
pub fn read(paths: &AppPaths) -> Option<OwnerKey> {
    parse(&std::fs::read_to_string(key_file(paths)).ok()?)
}

/// Removes the key file when it still holds `key` (a newer service may have replaced it).
pub fn remove(paths: &AppPaths, key: &OwnerKey) {
    if read(paths).as_ref() == Some(key) {
        let _ = std::fs::remove_file(key_file(paths));
    }
}

/// Whether a presented key is the real one, in time that doesn't depend on where they differ.
pub fn matches(real: &str, presented: &str) -> bool {
    real.len() == presented.len() && real.bytes().zip(presented.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_key_each_start_for_the_owner_alone() {
        let home = std::env::temp_dir().join(format!("xenolith-owner-{}", crate::text::random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), home.to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        let a = create(&paths, 47999, &[]).unwrap();
        assert!(is_key(&a.key) && a.pid == std::process::id() && a.port == 47999);
        assert_eq!(read(&paths), Some(a.clone()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(key_file(&paths)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let b = create(&paths, 47999, &[]).unwrap();
        assert_ne!(a.key, b.key, "a new key each start");
        remove(&paths, &a);
        assert_eq!(read(&paths), Some(b.clone()), "an older service leaves the newer one's key alone");
        remove(&paths, &b);
        assert_eq!(read(&paths), None);
        assert!(matches(&b.key, &b.key) && !matches(&b.key, &a.key) && !matches(&b.key, "") && !matches(&b.key, &b.key[..63]));
        assert_eq!(parse(r#"{"key":"ABC","pid":1,"port":2}"#), None);
        let _ = std::fs::remove_dir_all(home);
    }
}
