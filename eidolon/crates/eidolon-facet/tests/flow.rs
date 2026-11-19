//! A folder sealed from one cell and opened in another.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a failing test should say where; helpers here are not #[test] fns")]

use async_trait::async_trait;
use eidolon_facet::{open_facets, seal_facets, CellAccess, Facet, FacetError, FolderFacet, LocalCell};
use eidolon_model::{EidolonId, FacetId, FacetKind, FacetSpec};
use eidolon_vault::{EidolonKey, Keys, Vault};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use xenonet_id::codec::{hex, random, sha256};
use xenonet_id::sign::{KeyHome, SeedSigner};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!("eidolon-facet-test-{}", hex(&random::<8>().unwrap())));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn spec(path: &str, leave_out: &[&str]) -> FacetSpec {
    FacetSpec { id: FacetId::from_bytes([4; 16]), kind: FacetKind::Folder, name: "notes".into(), path: path.into(), leave_out: leave_out.iter().map(|s| (*s).to_owned()).collect() }
}

fn keys() -> Keys {
    Keys::new(&EidolonKey::from_bytes([3; 32])).unwrap()
}

fn snapshot(root: &Path) -> Vec<String> {
    fn go(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let mut kids: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        kids.sort();
        for p in kids {
            let rel = p.strip_prefix(base).unwrap().display().to_string();
            let meta = fs::symlink_metadata(&p).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            if meta.file_type().is_symlink() {
                out.push(format!("{rel} -> {}", fs::read_link(&p).unwrap().display()));
            } else if meta.is_dir() {
                out.push(format!("{rel}/ {mode:o}"));
                go(&p, base, out);
            } else {
                out.push(format!("{rel} {mode:o} {}", hex(&sha256(&fs::read(&p).unwrap()))));
            }
        }
    }
    let mut out = Vec::new();
    go(root, root, &mut out);
    out
}

fn fill(root: &Path) {
    fs::create_dir_all(root.join("sub/deeper")).unwrap();
    fs::create_dir_all(root.join("cache")).unwrap();
    fs::write(root.join("a.txt"), b"alpha").unwrap();
    fs::write(root.join("sub/deeper/b.bin"), vec![7u8; 300_000]).unwrap();
    fs::write(root.join("cache/junk"), b"junk").unwrap();
    fs::write(root.join("x.lock"), b"lock").unwrap();
    fs::write(root.join("tool.sh"), b"#!/bin/sh").unwrap();
    fs::set_permissions(root.join("tool.sh"), fs::Permissions::from_mode(0o750)).unwrap();
    symlink("a.txt", root.join("link")).unwrap();
}

#[tokio::test]
async fn a_folder_sealed_in_one_cell_opens_identical_in_another() {
    let s = Scratch::new();
    let (cell_a, cell_b) = (LocalCell::new(s.path("cell-a")), LocalCell::new(s.path("cell-b")));
    fill(&s.path("cell-a/home/will/notes"));
    let vault = Vault::open(&s.path("vault")).unwrap();
    let signer = SeedSigner::from_seed(&[8; 32], KeyHome::File).unwrap();
    let k = keys();
    let facets: Vec<Box<dyn Facet>> = vec![Box::new(FolderFacet::new(spec("/home/will/notes", &["cache/**", "*.lock"])).unwrap())];
    let head = seal_facets(&facets, &cell_a, &k, &vault, &signer, EidolonId::from_bytes([1; 16]), 4, &s.path("work")).await.unwrap();
    assert_eq!(head.head.generation, 4);
    assert_eq!(fs::read_dir(s.path("work")).unwrap().count(), 0, "staging is cleaned up");
    open_facets(&head, &facets, &cell_b, &k, &vault, &s.path("work")).await.unwrap();
    assert_eq!(fs::read_dir(s.path("work")).unwrap().count(), 0);
    let (a, b) = (snapshot(&s.path("cell-a/home/will/notes")), snapshot(&s.path("cell-b/home/will/notes")));
    let carried: Vec<_> = a.into_iter().filter(|l| !l.starts_with("cache") && !l.starts_with("x.lock")).collect();
    assert_eq!(carried, b);
    assert!(b.iter().any(|l| l.starts_with("tool.sh 750")));
    assert!(!s.path("cell-b/home/will/notes/cache").exists());
    assert!(!s.path("cell-b/home/will/notes/x.lock").exists());
}

#[test]
fn a_facet_refuses_a_leave_out_that_escapes_and_a_wrong_kind() {
    let escaping = FolderFacet::new(spec("/home/will/notes", &["../secrets/**"]));
    assert!(matches!(escaping, Err(FacetError::Refused(_))));
    assert!(matches!(FolderFacet::new(spec("/p", &["/etc/**"])), Err(FacetError::Refused(_))));
    let mut browser = spec("/p", &[]);
    browser.kind = FacetKind::Browser;
    assert!(matches!(FolderFacet::new(browser), Err(FacetError::WrongKind { .. })));
}

/// A facet that records whether it was resumed and fails to take.
struct Failing {
    spec: FacetSpec,
    resumed: Arc<AtomicBool>,
}

#[async_trait]
impl Facet for Failing {
    fn kind(&self) -> FacetKind {
        FacetKind::Folder
    }
    fn spec(&self) -> &FacetSpec {
        &self.spec
    }
    async fn quiet(&self, _cell: &dyn CellAccess) -> eidolon_facet::Result<()> {
        Ok(())
    }
    async fn resume(&self, _cell: &dyn CellAccess) -> eidolon_facet::Result<()> {
        self.resumed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn leave_out(&self) -> Vec<String> {
        Vec::new()
    }
    async fn take(&self, _cell: &dyn CellAccess, _staging: &Path) -> eidolon_facet::Result<()> {
        Err(FacetError::Cell("the cell went away".into()))
    }
    async fn place(&self, _cell: &dyn CellAccess, _staging: &Path) -> eidolon_facet::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_facet_that_fails_to_take_is_still_resumed() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let signer = SeedSigner::from_seed(&[8; 32], KeyHome::File).unwrap();
    let resumed = Arc::new(AtomicBool::new(false));
    let facets: Vec<Box<dyn Facet>> = vec![Box::new(Failing { spec: spec("/p", &[]), resumed: resumed.clone() })];
    let err = seal_facets(&facets, &LocalCell::new(s.path("cell")), &keys(), &vault, &signer, EidolonId::from_bytes([1; 16]), 1, &s.path("work")).await.unwrap_err();
    assert!(matches!(err, FacetError::Cell(_)));
    assert!(resumed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn opening_a_manifest_without_the_facet_places_nothing() {
    let s = Scratch::new();
    let cell_a = LocalCell::new(s.path("cell-a"));
    fill(&s.path("cell-a/home/will/notes"));
    let vault = Vault::open(&s.path("vault")).unwrap();
    let signer = SeedSigner::from_seed(&[8; 32], KeyHome::File).unwrap();
    let k = keys();
    let sealed: Vec<Box<dyn Facet>> = vec![Box::new(FolderFacet::new(spec("/home/will/notes", &[])).unwrap())];
    let head = seal_facets(&sealed, &cell_a, &k, &vault, &signer, EidolonId::from_bytes([1; 16]), 1, &s.path("work")).await.unwrap();
    let mut other = spec("/home/will/other", &[]);
    other.id = FacetId::from_bytes([5; 16]);
    other.name = "other".into();
    let wanted: Vec<Box<dyn Facet>> = vec![Box::new(FolderFacet::new(spec("/home/will/notes", &[])).unwrap()), Box::new(FolderFacet::new(other).unwrap())];
    let cell_b = LocalCell::new(s.path("cell-b"));
    let err = open_facets(&head, &wanted, &cell_b, &k, &vault, &s.path("work")).await.unwrap_err();
    assert!(matches!(err, FacetError::MissingFacet(_)));
    assert!(!s.path("cell-b").exists(), "the cell saw nothing");
    assert_eq!(fs::read_dir(s.path("work")).unwrap().count(), 0);
}

#[tokio::test]
async fn a_local_cell_will_not_leave_its_root() {
    let s = Scratch::new();
    let cell = LocalCell::new(s.path("cell"));
    for bad in ["../x", "relative", "/a/../../x"] {
        assert!(matches!(cell.copy_out(bad, &s.path("out")).await, Err(FacetError::BadGuestPath(_))), "{bad}");
    }
    assert_eq!(cell.run(&["true"]).await.unwrap().status, 0);
}
