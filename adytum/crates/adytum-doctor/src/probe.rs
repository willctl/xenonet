//! What the checks need to know about the machine, behind a trait so a test
//! can describe any machine.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use adytum_exec::{Cancel, Command, Runner, SystemRunner};
use adytum_ledger::{Ledger, LedgerError};
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::nearest_existing;

/// What opening the ledger found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerStatus {
    /// There is no ledger file yet.
    Missing,
    /// It opens; `unsettled` operations were started and never settled.
    Open {
        /// How many.
        unsettled: usize,
    },
    /// The file is a ledger this build does not read, or not a ledger.
    SetUpAgain(String),
    /// It could not be opened for another reason.
    Unreadable(String),
}

/// The machine, as the checks see it.
#[async_trait]
pub trait Probe: Send + Sync {
    /// The version `limactl --version` reports, such as `2.1.2`; `None` if
    /// there is no `limactl`.
    async fn limactl_version(&self) -> Option<String>;

    /// Adytum's folder.
    fn adytum_home(&self) -> Option<PathBuf>;

    /// Whether a file or folder exists.
    fn exists(&self, path: &Path) -> bool;

    /// Bytes free on the disk that holds `path` (or the nearest folder of it
    /// that exists); `None` if that cannot be read.
    async fn free_bytes(&self, path: &Path) -> Option<u64>;

    /// Whether Lima has already downloaded the image at `location`.
    fn lima_cache_has(&self, location: &str) -> bool;

    /// Opens the ledger in `home` and counts what is unsettled, without
    /// making one if there is none.
    fn ledger(&self, home: &Path) -> LedgerStatus;

    /// The version of a container tool on the host, or `None` if it is not
    /// installed.
    async fn tool_version(&self, tool: &str) -> Option<String>;
}

/// The real machine.
pub struct SystemProbe {
    runner: SystemRunner,
}

impl Default for SystemProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProbe {
    /// A probe of this machine.
    pub fn new() -> SystemProbe {
        SystemProbe { runner: SystemRunner::new() }
    }

    async fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let cmd = Command::new(program).args(args).pass_env(&["PATH", "HOME"]).deadline(Duration::from_secs(15));
        let out = self.runner.run(&cmd, &Cancel::new()).await.ok()?;
        out.success().then(|| out.stdout_lossy())
    }
}

/// The last word of a version line, without a leading `v`:
/// `limactl version 2.1.2` is `2.1.2`.
fn last_word(line: &str) -> Option<String> {
    line.split_whitespace().last().map(|w| w.trim_start_matches('v').to_owned())
}

/// The free bytes in the answer of `df -Pk`. The column before the first one
/// ending in `%` is "Available" in 1024-byte blocks; the file system's name
/// may hold spaces, so columns are not counted from the left.
fn parse_df(text: &str) -> Option<u64> {
    let tokens: Vec<&str> = text.lines().nth(1)?.split_whitespace().collect();
    let at = tokens.iter().position(|t| t.ends_with('%'))?;
    let blocks: u64 = tokens.get(at.checked_sub(1)?)?.parse().ok()?;
    blocks.checked_mul(1024)
}

fn lima_cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Caches/lima"));
    }
    let base = std::env::var_os("XDG_CACHE_HOME").filter(|h| !h.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".cache")));
    base.map(|b| b.join("lima"))
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

#[async_trait]
impl Probe for SystemProbe {
    async fn limactl_version(&self) -> Option<String> {
        let text = self.run("limactl", &["--version"]).await?;
        last_word(text.lines().next()?)
    }

    fn adytum_home(&self) -> Option<PathBuf> {
        adytum_lima::adytum_home()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    async fn free_bytes(&self, path: &Path) -> Option<u64> {
        let at = nearest_existing(path, Path::exists);
        let text = self.run("df", &["-Pk", &at.to_string_lossy()]).await?;
        parse_df(&text)
    }

    fn lima_cache_has(&self, location: &str) -> bool {
        lima_cache_dir().is_some_and(|dir| dir.join("download/by-url-sha256").join(sha256_hex(location)).join("data").is_file())
    }

    fn ledger(&self, home: &Path) -> LedgerStatus {
        // Opening a ledger makes one, and a doctor changes nothing.
        if !home.join(adytum_ledger::FILE).exists() {
            return LedgerStatus::Missing;
        }
        match Ledger::open(home).and_then(|l| l.open_ops()) {
            Ok(ops) => LedgerStatus::Open { unsettled: ops.len() },
            Err(LedgerError::SetUpAgain(why)) => LedgerStatus::SetUpAgain(why),
            Err(e) => LedgerStatus::Unreadable(e.to_string()),
        }
    }

    async fn tool_version(&self, tool: &str) -> Option<String> {
        let text = self.run(tool, &["--version"]).await?;
        // "podman version 5.3.1" and "Docker version 27.3.1, build ce12230".
        last_word(text.lines().next()?.split(',').next()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_lines_read_to_the_version() {
        assert_eq!(last_word("limactl version 2.1.2").as_deref(), Some("2.1.2"));
        assert_eq!(last_word("limactl version v2.1.2").as_deref(), Some("2.1.2"));
        assert_eq!(last_word("").as_deref(), None);
    }

    #[test]
    fn df_reads_available_blocks_even_when_the_file_system_has_spaces() {
        let mac = "Filesystem   1024-blocks      Used Available Capacity  Mounted on\n/dev/disk3s5  971350180 500000000 100000000    84%    /System/Volumes/Data\n";
        assert_eq!(parse_df(mac), Some(100_000_000 * 1024));
        let spaced = "Filesystem 1024-blocks Used Available Capacity Mounted on\nmap auto_home 0 0 0 100% /System/Volumes/Data/home\n";
        assert_eq!(parse_df(spaced), Some(0));
        assert_eq!(parse_df("nothing"), None);
        assert_eq!(parse_df("Filesystem\nbroken line"), None);
    }

    #[test]
    fn a_cache_entry_is_named_by_the_hash_of_the_url() {
        // Pinned against the folder Lima 2.1.2 itself made for this URL.
        assert_eq!(
            sha256_hex("https://cloud-images.ubuntu.com/releases/noble/release-20260518/ubuntu-24.04-server-cloudimg-arm64.img"),
            "2307d0ba402620cbfbfd84cb192964b33bc4f4c491640945aab32c0787ed31ef"
        );
    }
}
