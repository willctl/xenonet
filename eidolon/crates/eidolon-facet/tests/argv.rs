//! What a seal and an open ask of a cell: every path and command the facet
//! hands to `CellAccess`, recorded. None may carry the eidolon's key, in
//! raw, hex or base64url form.
#![expect(clippy::unwrap_used, reason = "in a test, a failure should say where")]

use async_trait::async_trait;
use eidolon_facet::{open_facets, seal_facets, CellAccess, Facet, FolderFacet, LocalCell, RunOutput};
use eidolon_model::{EidolonId, FacetId, FacetKind, FacetSpec};
use eidolon_vault::{EidolonKey, Keys, Vault};
use std::path::Path;
use std::sync::Mutex;
use xenonet_id::codec::{b64, hex};
use xenonet_id::sign::{KeyHome, SeedSigner};

/// A cell that writes down every argument it is given.
struct Recording {
    inner: LocalCell,
    calls: Mutex<Vec<String>>,
}

impl Recording {
    fn note(&self, line: String) {
        self.calls.lock().unwrap().push(line);
    }
}

#[async_trait]
impl CellAccess for Recording {
    async fn copy_out(&self, guest_path: &str, host_dir: &Path) -> eidolon_facet::Result<()> {
        self.note(format!("copy_out {guest_path} {}", host_dir.display()));
        self.inner.copy_out(guest_path, host_dir).await
    }

    async fn copy_in(&self, host_dir: &Path, guest_path: &str) -> eidolon_facet::Result<()> {
        self.note(format!("copy_in {} {guest_path}", host_dir.display()));
        self.inner.copy_in(host_dir, guest_path).await
    }

    async fn run(&self, argv: &[&str]) -> eidolon_facet::Result<RunOutput> {
        self.note(format!("run {}", argv.join(" ")));
        self.inner.run(argv).await
    }
}

#[tokio::test]
async fn a_seal_and_an_open_ask_the_cell_for_paths_only_never_a_key() {
    let root = std::env::temp_dir().join(format!("eidolon-facet-argv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let cell_dir = root.join("cell");
    std::fs::create_dir_all(cell_dir.join("home/notes")).unwrap();
    std::fs::write(cell_dir.join("home/notes/a.txt"), b"hello").unwrap();
    let spec = FacetSpec { id: FacetId::new_random().unwrap(), kind: FacetKind::Folder, name: "notes".into(), path: "/home/notes".into(), leave_out: Vec::new() };
    let facets: Vec<Box<dyn Facet>> = vec![Box::new(FolderFacet::new(spec).unwrap())];
    let key = EidolonKey::generate().unwrap();
    let keys = Keys::new(&key).unwrap();
    let vault = Vault::open(&root.join("vault")).unwrap();
    let signer = SeedSigner::from_seed(&[7; 32], KeyHome::File).unwrap();
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let cell = Recording { inner: LocalCell::new(&cell_dir), calls: Mutex::new(Vec::new()) };

    let eid = EidolonId::new_random().unwrap();
    let head = seal_facets(&facets, &cell, &keys, &vault, &signer, eid, 1, &work).await.unwrap();
    std::fs::remove_dir_all(cell_dir.join("home/notes")).unwrap();
    open_facets(&head, &facets, &cell, &keys, &vault, &work).await.unwrap();
    assert_eq!(std::fs::read(cell_dir.join("home/notes/a.txt")).unwrap(), b"hello");

    let calls = cell.calls.lock().unwrap().join("\n");
    assert!(calls.contains("copy_out /home/notes") && calls.contains("copy_in"), "{calls}");
    let secret = key.as_bytes();
    for needle in [hex(secret).into_bytes(), b64(secret).into_bytes(), secret.to_vec()] {
        assert!(!calls.as_bytes().windows(needle.len()).any(|w| w == needle), "the key reached the cell in {calls}");
    }
    // Nothing is left in the working folder either.
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&root);
}
