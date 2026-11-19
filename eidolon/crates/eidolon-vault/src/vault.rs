//! Sealed chunks on disk: `chunks/<first two hex>/<rest of hex>`.
//!
//! The vault holds sealed bytes and never needs a key. A chunk is written to
//! a hidden temp file beside its place, flushed, and renamed, so a kill
//! mid-write leaves no half chunk under a real name.

use crate::keys::ChunkName;
use crate::VaultError;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A temp file older than this is left from a write that died.
const STALE_TEMP: Duration = Duration::from_secs(60 * 60);

pub struct Vault {
    chunks: PathBuf,
}

impl Vault {
    /// The vault in `dir`, made (owner-only) if it is not there yet.
    pub fn open(dir: &Path) -> Result<Vault, VaultError> {
        let chunks = dir.join("chunks");
        make_private_dir(&chunks)?;
        Ok(Vault { chunks })
    }

    fn shard(&self, name: &ChunkName) -> (PathBuf, PathBuf) {
        let hex = name.to_hex();
        let dir = self.chunks.join(&hex[..2]);
        let file = dir.join(&hex[2..]);
        (dir, file)
    }

    pub fn has(&self, name: &ChunkName) -> bool {
        self.shard(name).1.is_file()
    }

    /// Keeps `sealed` under `name`. The vault does not look inside: whoever
    /// opens it checks it against the name.
    pub fn put(&self, name: &ChunkName, sealed: &[u8]) -> Result<(), VaultError> {
        let (dir, file) = self.shard(name);
        make_private_dir(&dir)?;
        xenonet_os::private_file::write(&file, sealed, &[]).map_err(VaultError::io("write a chunk"))
    }

    pub fn get(&self, name: &ChunkName) -> Result<Vec<u8>, VaultError> {
        std::fs::read(self.shard(name).1).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => VaultError::MissingChunk(*name),
            _ => VaultError::Io { what: "read a chunk", source: e },
        })
    }

    /// Every chunk in the vault, in name order. Temp files and anything else
    /// that is not a chunk are ignored.
    pub fn list(&self) -> Result<Vec<ChunkName>, VaultError> {
        let mut names = Vec::new();
        self.walk(|entry| {
            if let Entry::Chunk(name, _) = entry {
                names.push(name);
            }
            Ok(())
        })?;
        names.sort();
        Ok(names)
    }

    /// Removes a chunk; false when it was not there.
    pub fn remove(&self, name: &ChunkName) -> Result<bool, VaultError> {
        match std::fs::remove_file(self.shard(name).1) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(VaultError::Io { what: "remove a chunk", source: e }),
        }
    }

    /// Removes every chunk not in `keep`, and temp files left by writes that
    /// died more than an hour ago (a younger one may be a write in progress).
    /// Returns how many files it removed.
    pub fn gc(&self, keep: &BTreeSet<ChunkName>) -> Result<usize, VaultError> {
        let mut removed = 0;
        self.walk(|entry| {
            let (path, doomed) = match entry {
                Entry::Chunk(name, path) => (path, !keep.contains(&name)),
                Entry::Temp(path) => {
                    let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
                    (path, age.is_some_and(|a| a >= STALE_TEMP))
                }
            };
            if doomed {
                match std::fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(VaultError::Io { what: "remove a chunk", source: e }),
                }
            }
            Ok(())
        })?;
        Ok(removed)
    }

    /// The bytes the chunks take on disk.
    pub fn size_bytes(&self) -> Result<u64, VaultError> {
        let mut total = 0;
        self.walk(|entry| {
            if let Entry::Chunk(_, path) = entry {
                total += std::fs::metadata(path).map_err(VaultError::io("measure a chunk"))?.len();
            }
            Ok(())
        })?;
        Ok(total)
    }

    fn walk(&self, mut each: impl FnMut(Entry) -> Result<(), VaultError>) -> Result<(), VaultError> {
        for shard in std::fs::read_dir(&self.chunks).map_err(VaultError::io("read the vault"))? {
            let shard = shard.map_err(VaultError::io("read the vault"))?;
            let shard_name = shard.file_name().to_string_lossy().into_owned();
            if shard_name.len() != 2 || !shard.path().is_dir() {
                continue;
            }
            for file in std::fs::read_dir(shard.path()).map_err(VaultError::io("read the vault"))? {
                let file = file.map_err(VaultError::io("read the vault"))?;
                let file_name = file.file_name().to_string_lossy().into_owned();
                let path = file.path();
                if file_name.starts_with('.') && Path::new(&file_name).extension().is_some_and(|e| e == "tmp") {
                    each(Entry::Temp(path))?;
                } else if let Ok(name) = format!("{shard_name}{file_name}").parse::<ChunkName>() {
                    each(Entry::Chunk(name, path))?;
                }
            }
        }
        Ok(())
    }
}

enum Entry {
    Chunk(ChunkName, PathBuf),
    Temp(PathBuf),
}

fn make_private_dir(dir: &Path) -> Result<(), VaultError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(VaultError::io("create the vault"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{EidolonKey, Keys};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let id = xenonet_id::codec::hex(&xenonet_id::codec::random::<8>().unwrap());
            Tmp(std::env::temp_dir().join(format!("eidolon-vault-test-{id}")))
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn keys() -> Keys {
        Keys::new(&EidolonKey::from_bytes([9; 32])).unwrap()
    }

    #[test]
    fn put_get_has_list_and_remove() {
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        let keys = keys();
        let (a, sealed_a) = keys.seal(b"one");
        let (b, sealed_b) = keys.seal(b"two");
        assert!(!vault.has(&a));
        assert!(matches!(vault.get(&a), Err(VaultError::MissingChunk(_))));
        vault.put(&a, &sealed_a).unwrap();
        vault.put(&b, &sealed_b).unwrap();
        vault.put(&b, &sealed_b).unwrap();
        assert!(vault.has(&a));
        assert_eq!(vault.get(&a).unwrap(), sealed_a);
        assert_eq!(keys.open(&b, &vault.get(&b).unwrap()).unwrap(), b"two");
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(vault.list().unwrap(), want);
        assert_eq!(vault.size_bytes().unwrap(), (sealed_a.len() + sealed_b.len()) as u64);
        assert!(vault.remove(&a).unwrap());
        assert!(!vault.remove(&a).unwrap());
        assert!(!vault.has(&a));
    }

    #[test]
    fn a_chunk_lives_at_its_two_character_shard() {
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        let (name, sealed) = keys().seal(b"x");
        vault.put(&name, &sealed).unwrap();
        let hex = name.to_hex();
        assert!(tmp.0.join("chunks").join(&hex[..2]).join(&hex[2..]).is_file());
    }

    #[test]
    fn gc_removes_chunks_nothing_names() {
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        let keys = keys();
        let (a, sa) = keys.seal(b"keep");
        let (b, sb) = keys.seal(b"drop");
        vault.put(&a, &sa).unwrap();
        vault.put(&b, &sb).unwrap();
        assert_eq!(vault.gc(&BTreeSet::from([a])).unwrap(), 1);
        assert_eq!(vault.list().unwrap(), vec![a]);
    }

    #[test]
    fn a_temp_file_from_a_killed_write_is_ignored_by_list_and_removed_by_gc() {
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        let (name, sealed) = keys().seal(b"real");
        vault.put(&name, &sealed).unwrap();
        let shard = tmp.0.join("chunks").join(&name.to_hex()[..2]);
        let temp = shard.join(".deadbeef.123.abc.tmp");
        std::fs::write(&temp, b"half a chu").unwrap();
        assert_eq!(vault.list().unwrap(), vec![name]);
        assert_eq!(vault.size_bytes().unwrap(), sealed.len() as u64);
        // Fresh, it might be a write in progress.
        assert_eq!(vault.gc(&BTreeSet::from([name])).unwrap(), 0);
        assert!(temp.exists());
        // Old, the write died.
        let old = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        std::fs::File::options().write(true).open(&temp).unwrap().set_modified(old).unwrap();
        assert_eq!(vault.gc(&BTreeSet::from([name])).unwrap(), 1);
        assert!(!temp.exists());
        assert!(vault.has(&name));
    }

    #[test]
    fn strangers_in_the_vault_are_not_chunks() {
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        std::fs::write(tmp.0.join("chunks").join("README"), b"x").unwrap();
        std::fs::create_dir_all(tmp.0.join("chunks").join("zz")).unwrap();
        std::fs::write(tmp.0.join("chunks").join("zz").join("notachunk"), b"x").unwrap();
        assert_eq!(vault.list().unwrap().len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn directories_and_chunks_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = Tmp::new();
        let vault = Vault::open(&tmp.0).unwrap();
        let (name, sealed) = keys().seal(b"secret");
        vault.put(&name, &sealed).unwrap();
        let hex = name.to_hex();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let chunks = tmp.0.join("chunks");
        assert_eq!(mode(&chunks), 0o700);
        assert_eq!(mode(&chunks.join(&hex[..2])), 0o700);
        assert_eq!(mode(&chunks.join(&hex[..2]).join(&hex[2..])), 0o600);
    }
}
