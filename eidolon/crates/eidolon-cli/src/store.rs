//! The key store for a machine with no operating-system store this build
//! can use: one owner-only file per secret under `<home>/keys/`.

use std::path::{Path, PathBuf};
use xenonet_id::sign::KeyHome;
use xenonet_id::store::KeyStore;
use xenonet_id::{IdError, R};

pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    /// The store in `dir`, which is made owner-only when it is not there.
    pub fn open(dir: &Path) -> R<FileKeyStore> {
        make_private_dir(dir).map_err(|e| IdError::internal(format!("the key folder: {e}")))?;
        Ok(FileKeyStore { dir: dir.to_owned() })
    }

    /// A secret's name as a file name. Names are letters, digits, `-`, `_`
    /// and `/`; anything else (so also `..`) is refused, so a name can
    /// never leave the folder.
    fn file(&self, name: &str) -> R<PathBuf> {
        let ok = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'/')) && !name.starts_with('/') && !name.contains("//");
        if ok {
            Ok(self.dir.join(name.replace('/', "__")))
        } else {
            Err(IdError::internal("a secret's name has characters a file name cannot carry"))
        }
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

impl KeyStore for FileKeyStore {
    fn get(&self, name: &str) -> R<Option<Vec<u8>>> {
        match std::fs::read(self.file(name)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn put(&self, name: &str, secret: &[u8]) -> R<()> {
        Ok(xenonet_os::private_file::write(&self.file(name)?, secret, &[])?)
    }

    fn delete(&self, name: &str) -> R<()> {
        match std::fs::remove_file(self.file(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn home(&self) -> KeyHome {
        KeyHome::File
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eidolon-cli-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_secret_goes_in_comes_out_and_is_owner_only() {
        let dir = scratch("a");
        let store = FileKeyStore::open(&dir.join("keys")).unwrap();
        assert!(store.get("eidolon/eid_x").unwrap().is_none());
        store.put("eidolon/eid_x", b"secret").unwrap();
        assert_eq!(store.get("eidolon/eid_x").unwrap().unwrap(), b"secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let file = std::fs::metadata(dir.join("keys").join("eidolon__eid_x")).unwrap().permissions().mode() & 0o777;
            let folder = std::fs::metadata(dir.join("keys")).unwrap().permissions().mode() & 0o777;
            assert_eq!((file, folder), (0o600, 0o700));
        }
        store.delete("eidolon/eid_x").unwrap();
        store.delete("eidolon/eid_x").unwrap();
        assert!(store.get("eidolon/eid_x").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_cannot_leave_the_folder() {
        let dir = scratch("b");
        let store = FileKeyStore::open(&dir).unwrap();
        for bad in ["", "../x", "/etc/x", "a/../b", "a b", "a//b", "a.b"] {
            assert!(store.put(bad, b"x").is_err(), "{bad:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
