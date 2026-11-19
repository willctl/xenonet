//! `$EIDOLON_HOME` (default `~/.eidolon`): the vault, the holdings, this
//! machine's keys and its name.

use crate::error::{CliError, Result};
use crate::store::FileKeyStore;
use eidolon_model::{EidolonId, Holder, HolderKey};
use eidolon_vault::Vault;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};
use xenonet_id::codec::random;
use xenonet_id::sign::Signer;
use xenonet_id::store::{KeyStore, StoredSigner};
use zeroize::Zeroizing;

const SIGN_KEY: &str = "machine-sign";
const SEAL_KEY: &str = "machine-seal";

/// The weaker store's warning, shown when a machine is set up on it.
pub const FILE_STORE_WARNING: &str = "The keys are kept in owner-only files under the keys folder of the eidolon home. That is weaker than the operating system's key store, which this build does not use yet: anyone who can read your files as you can read the keys.";

#[derive(Debug, Serialize, Deserialize)]
struct MachineFile {
    name: String,
    key_store: String,
}

#[derive(Debug, Clone)]
pub struct Home {
    pub dir: PathBuf,
}

impl Home {
    pub fn from_env() -> Result<Home> {
        if let Some(dir) = std::env::var_os("EIDOLON_HOME").filter(|d| !d.is_empty()) {
            return Ok(Home { dir: PathBuf::from(dir) });
        }
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .ok_or_else(|| CliError::failed("no_home", "There is no home folder to keep eidolons in.", "Set EIDOLON_HOME to a folder."))?;
        Ok(Home { dir: PathBuf::from(home).join(".eidolon") })
    }

    pub fn holdings(&self) -> PathBuf {
        self.dir.join("holdings")
    }

    pub fn work(&self) -> PathBuf {
        self.dir.join("tmp")
    }

    /// A private staging folder for one seal or open, under [`Home::work`].
    /// The name carries the process id so a later command can tell the
    /// folders of dead processes from live ones; see [`Home::sweep_work`].
    pub fn run_dir(&self) -> Result<PathBuf> {
        let tag = xenonet_id::codec::hex(&random::<6>().map_err(|e| CliError::failed("random", e.to_string(), "Try again."))?);
        let dir = self.work().join(format!("run-{}-{tag}", std::process::id()));
        make_private_dir(&dir).map_err(|e| CliError::io("make the working folder", &e))?;
        Ok(dir)
    }

    /// Removes the staging folders a killed command left behind: they hold
    /// plain files from the cell. Folders of processes still running stay.
    pub fn sweep_work(&self) {
        let Ok(entries) = std::fs::read_dir(self.work()) else { return };
        for entry in entries.filter_map(std::result::Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let pid = name.strip_prefix("run-").and_then(|r| r.split('-').next()).and_then(|p| p.parse::<u32>().ok());
            if pid.is_some_and(|p| p != std::process::id() && !process_is_running(p)) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// The one host folder an eidolon's browser exchanges files through:
    /// uploads come from it, fetched downloads go to it. Made owner-only.
    pub fn files_dir(&self, eid: &EidolonId) -> Result<PathBuf> {
        let dir = self.dir.join("files").join(eid.to_string());
        make_private_dir(&dir).map_err(|e| CliError::io("make the files folder", &e))?;
        // An earlier run, or a person, may have made it with looser rights.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| CliError::io("make the files folder private", &e))?;
        }
        Ok(dir)
    }

    fn machine_file(&self) -> PathBuf {
        self.dir.join("machine.json")
    }

    pub fn vault(&self) -> Result<Vault> {
        Ok(Vault::open(&self.dir.join("vault"))?)
    }

    pub fn store(&self) -> Result<FileKeyStore> {
        FileKeyStore::open(&self.dir.join("keys")).map_err(|e| CliError::failed("key_store", e.to_string(), "Check that the keys folder can be written."))
    }

    /// Makes this machine's keys and name. Refused when it already has them:
    /// a second identity would orphan every seat that names the first.
    pub fn init(&self, name: &str, store: &dyn KeyStore) -> Result<Machine> {
        if self.machine_file().exists() {
            return Err(CliError::failed("already_initialised", "This machine already has an identity.", "Use `eidolon machine show` to read it."));
        }
        let ok = (1..=48).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(CliError::usage("A machine name is 1 to 48 of a-z, 0-9 or -.", "Pick a name such as `desk`."));
        }
        let bad = |e: xenonet_id::IdError| CliError::failed("key_store", e.to_string(), "Check that the keys folder can be written.");
        StoredSigner::load_or_create(store, SIGN_KEY).map_err(bad)?;
        let seal = Zeroizing::new(random::<32>().map_err(bad)?);
        store.put(SEAL_KEY, &*seal).map_err(bad)?;
        let file = MachineFile { name: name.to_owned(), key_store: "file".to_owned() };
        xenonet_os::private_file::write(&self.machine_file(), &serde_json::to_vec_pretty(&file)?, &[]).map_err(|e| CliError::io("write the machine's name", &e))?;
        self.machine(store)
    }

    /// This machine's identity, as `init` made it.
    pub fn machine(&self, store: &dyn KeyStore) -> Result<Machine> {
        let not_set_up = || CliError::failed("not_initialised", "This machine has no identity yet.", "Run `eidolon machine init --name <name>` first.");
        let bytes = match std::fs::read(self.machine_file()) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(not_set_up()),
            Err(e) => return Err(CliError::io("read the machine's name", &e)),
        };
        let file: MachineFile = serde_json::from_slice(&bytes)?;
        let damaged = || CliError::failed("key_store", "This machine's keys are missing or damaged.", "Restore the keys folder, or set up a new identity in a new EIDOLON_HOME.");
        let signer = StoredSigner::load(store, SIGN_KEY).map_err(|_| damaged())?.ok_or_else(damaged)?;
        let seal: [u8; 32] = store.get(SEAL_KEY).map_err(|_| damaged())?.and_then(|b| b.try_into().ok()).ok_or_else(damaged)?;
        let seal_secret = StaticSecret::from(seal);
        let key = HolderKey::from_verifying_key(&signer.public()).ok_or_else(damaged)?;
        let holder = Holder { name: file.name, sign: key, seal: PublicKey::from(&seal_secret).to_bytes() };
        Ok(Machine { signer, key, seal_secret, holder, key_store: file.key_store })
    }
}

fn make_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Whether process `pid` is alive, asked of the system's `kill -0`. When it
/// cannot be asked, the answer is yes, so nothing live is ever swept.
fn process_is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_or(true, |s| s.success())
}

pub struct Machine {
    pub signer: StoredSigner,
    pub key: HolderKey,
    pub seal_secret: StaticSecret,
    pub holder: Holder,
    pub key_store: String,
}

/// Reads a holder list entry from a file, or standard input for `-`.
pub fn read_holder(from: &str, stdin: &mut dyn FnMut() -> std::io::Result<Vec<u8>>) -> Result<Holder> {
    let bytes = if from == "-" {
        stdin().map_err(|e| CliError::io("read standard input", &e))?
    } else {
        std::fs::read(Path::new(from)).map_err(|e| CliError::io(&format!("read {from}"), &e))?
    };
    Ok(serde_json::from_slice(&bytes)?)
}
