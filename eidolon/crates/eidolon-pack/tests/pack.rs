//! Packing and unpacking against a real vault in a scratch folder.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a failing test should say where; helpers here are not #[test] fns")]

use eidolon_model::{EidolonId, FacetId, FacetKind, HolderKey};
use eidolon_pack::chunker::chunk_slice;
use eidolon_pack::{diff, load, pack_folder, seal, unpack_facet, Entry, FacetManifest, Manifest, Node, PackError, FORMAT};
use eidolon_vault::{EidolonKey, Keys, Vault, VaultError};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use xenonet_id::codec::{hex, random};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!("eidolon-pack-test-{}", hex(&random::<8>().unwrap())));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Test folders may hold read-only folders.
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn noise(len: usize, mut seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.extend_from_slice(&seed.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn keys(n: u8) -> Keys {
    Keys::new(&EidolonKey::from_bytes([n; 32])).unwrap()
}

fn facet() -> FacetId {
    FacetId::from_bytes([9; 16])
}

fn pack(dir: &Path, leave_out: &[&str], keys: &Keys, vault: &Vault) -> eidolon_pack::Packed {
    let leave: Vec<String> = leave_out.iter().map(|s| (*s).to_owned()).collect();
    pack_folder(facet(), FacetKind::Folder, dir, &leave, keys, vault).unwrap()
}

fn manifest_of(fm: FacetManifest) -> Manifest {
    Manifest { format: FORMAT, eidolon: EidolonId::from_bytes([1; 16]), generation: 1, facets: vec![fm] }
}

fn tree(root: &Path) {
    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::create_dir_all(root.join("empty-dir")).unwrap();
    fs::write(root.join("empty.txt"), b"").unwrap();
    fs::write(root.join("a/hello.txt"), b"hello").unwrap();
    fs::write(root.join("a/b/c/deep.bin"), noise(50_000, 5)).unwrap();
    fs::write(root.join("big.bin"), noise(3 << 20, 11)).unwrap();
    fs::write(root.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(root.join("secret.txt"), b"s").unwrap();
    fs::set_permissions(root.join("secret.txt"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(root.join("héllo wörld ✓.txt"), "unicode").unwrap();
    symlink("a/hello.txt", root.join("link")).unwrap();
    symlink("/nowhere/at/all", root.join("dangling")).unwrap();
}

/// Every path, kind, mode, bytes and link target under `root`.
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
                out.push(format!("{rel} {mode:o} {}", hex(&xenonet_id::codec::sha256(&fs::read(&p).unwrap()))));
            }
        }
    }
    let mut out = Vec::new();
    go(root, root, &mut out);
    out
}

#[test]
fn a_tree_round_trips_byte_for_byte() {
    let s = Scratch::new();
    let src = s.path("src");
    tree(&src);
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let packed = pack(&src, &[], &k, &vault);
    assert_eq!(packed.skipped.len(), 0);
    let signer = SeedSigner::from_seed(&[5; 32], KeyHome::File).unwrap();
    let head = seal(&manifest_of(packed.manifest), &k, &vault, &signer).unwrap();
    let loaded = load(&head, &k, &vault).unwrap();
    let out = s.path("out");
    unpack_facet(&loaded.facets[0], &k, &vault, &out).unwrap();
    assert_eq!(snapshot(&src), snapshot(&out));
    assert!(snapshot(&out).iter().any(|l| l.starts_with("run.sh 755")));
    assert!(snapshot(&out).iter().any(|l| l.starts_with("secret.txt 600")));
    assert!(snapshot(&out).iter().any(|l| l.starts_with("empty-dir/")));
}

#[test]
fn the_same_folder_twice_gives_the_same_manifest_and_no_new_chunks() {
    let s = Scratch::new();
    let src = s.path("src");
    tree(&src);
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let first = pack(&src, &[], &k, &vault);
    let chunks = vault.list().unwrap();
    let second = pack(&src, &[], &k, &vault);
    assert_eq!(first.manifest, second.manifest);
    assert_eq!(chunks, vault.list().unwrap());
    let paths: Vec<_> = first.manifest.entries.iter().map(|e| e.path.clone()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "entries are sorted by path");
}

#[test]
fn one_changed_file_makes_only_its_changed_chunks_new() {
    let s = Scratch::new();
    let src = s.path("src");
    tree(&src);
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let before = manifest_of(pack(&src, &[], &k, &vault).manifest);
    let mut big = fs::read(src.join("big.bin")).unwrap();
    big.splice(1 << 20..1 << 20, *b"inserted!!");
    fs::write(src.join("big.bin"), &big).unwrap();
    let after = manifest_of(pack(&src, &[], &k, &vault).manifest);
    let d = diff(&before, &after);
    assert!(!d.added.is_empty() && d.added.len() <= 3, "added {}", d.added.len());
    assert!(d.removed.len() <= 3);
    assert!(d.shared_bytes > 2 << 20, "shared {}", d.shared_bytes);
    assert!(d.new_bytes < 600 * 1024, "new {}", d.new_bytes);
    let untouched = |m: &Manifest| m.facets[0].entries.iter().filter(|e| e.path != "big.bin").cloned().collect::<Vec<Entry>>();
    assert_eq!(untouched(&before), untouched(&after));
}

#[test]
fn an_insert_in_the_middle_changes_at_most_three_chunk_names() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let original = noise(1 << 20, 99);
    let mut edited = original.clone();
    edited.splice(500 * 1024..500 * 1024, *b"0123456789");
    let names = |data: &[u8]| -> std::collections::BTreeSet<_> { chunk_slice(data).into_iter().map(|c| k.name_of(c)).collect() };
    let _ = &vault;
    let (a, b) = (names(&original), names(&edited));
    assert!(a.len() > 8, "a 1 MiB file is several chunks");
    assert!(b.difference(&a).count() <= 3, "{} new names", b.difference(&a).count());
    assert!(a.difference(&b).count() <= 3);
}

#[test]
fn leave_out_globs_keep_files_out() {
    let s = Scratch::new();
    let src = s.path("src");
    fs::create_dir_all(src.join("cache2/entries")).unwrap();
    fs::create_dir_all(src.join("keep")).unwrap();
    fs::write(src.join("cache2/entries/x"), b"x").unwrap();
    fs::write(src.join("keep/a.lock"), b"lock").unwrap();
    fs::write(src.join("b.lock"), b"lock").unwrap();
    fs::write(src.join("keep/a.txt"), b"a").unwrap();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let packed = pack(&src, &["cache2/**", "*.lock"], &keys(1), &vault);
    let paths: Vec<_> = packed.manifest.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["keep", "keep/a.txt"]);
}

#[test]
fn sockets_are_skipped_and_reported() {
    let s = Scratch::new();
    let src = s.path("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("f"), b"f").unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(src.join("sock")).unwrap();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let packed = pack(&src, &[], &keys(1), &vault);
    assert_eq!(packed.manifest.entries.len(), 1);
    assert_eq!(packed.skipped.len(), 1);
    assert_eq!(packed.skipped[0].path, "sock");
}

fn hostile(entries: Vec<Entry>) -> FacetManifest {
    FacetManifest { facet: facet(), kind: FacetKind::Folder, entries }
}

fn file(path: &str, k: &Keys, vault: &Vault) -> Entry {
    let (name, sealed) = k.seal(b"owned");
    vault.put(&name, &sealed).unwrap();
    Entry { path: path.to_owned(), node: Node::File { mode: 0o644, size: 5, chunks: vec![name] } }
}

#[test]
fn hostile_paths_are_refused_and_nothing_lands_outside() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let outside = s.path("outside");
    fs::create_dir_all(&outside).unwrap();
    let into = s.path("work/into");
    let abs = outside.join("abs").display().to_string();
    for bad in ["../x", "a/../../x", "/etc/x", abs.as_str(), "", "a//b", "./x", "a/./b", "a/.."] {
        let m = hostile(vec![file(bad, &k, &vault)]);
        let err = unpack_facet(&m, &k, &vault, &into).unwrap_err();
        assert!(matches!(err, PackError::UnsafePath(_)), "{bad:?}: {err}");
        assert!(!into.exists(), "{bad:?} left a folder");
        assert!(!s.path("work/x").exists() && !s.path("x").exists());
    }
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let leftovers = fs::read_dir(s.path("work")).map_or(0, Iterator::count);
    assert_eq!(leftovers, 0, "no staging folder left behind");
}

#[test]
fn a_file_through_an_earlier_symlink_is_refused() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let outside = s.path("outside");
    fs::create_dir_all(&outside).unwrap();
    let into = s.path("work/into");
    let m = hostile(vec![Entry { path: "link".into(), node: Node::Symlink { target: outside.display().to_string() } }, file("link/pwned", &k, &vault)]);
    let err = unpack_facet(&m, &k, &vault, &into).unwrap_err();
    assert!(matches!(err, PackError::UnsafePath(_)), "{err}");
    assert!(!into.exists());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0, "nothing written through the link");
    // A symlink named like a file about to be written is not followed either.
    let m = hostile(vec![Entry { path: "f".into(), node: Node::Symlink { target: outside.join("target").display().to_string() } }, file("f", &k, &vault)]);
    assert!(unpack_facet(&m, &k, &vault, &into).is_err());
    assert!(!outside.join("target").exists());
}

#[test]
fn a_tampered_chunk_fails_the_unpack_and_leaves_the_folder_absent() {
    let s = Scratch::new();
    let src = s.path("src");
    tree(&src);
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let packed = pack(&src, &[], &k, &vault);
    let victim = packed.manifest.entries.iter().find_map(|e| if let Node::File { chunks, .. } = &e.node { chunks.last().copied() } else { None }).unwrap();
    let mut sealed = vault.get(&victim).unwrap();
    let last = sealed.len() - 1;
    sealed[last] ^= 1;
    vault.put(&victim, &sealed).unwrap();
    let out = s.path("out");
    let err = unpack_facet(&packed.manifest, &k, &vault, &out).unwrap_err();
    assert!(matches!(err, PackError::Vault(VaultError::ChangedChunk)), "{err}");
    assert!(!out.exists());
    let strays = fs::read_dir(&s.0).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains("staging")).count();
    assert_eq!(strays, 0);
}

#[test]
fn the_target_must_be_absent_or_empty() {
    let s = Scratch::new();
    let src = s.path("src");
    tree(&src);
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let packed = pack(&src, &[], &k, &vault);
    let full = s.path("full");
    fs::create_dir_all(&full).unwrap();
    fs::write(full.join("keep"), b"mine").unwrap();
    assert!(matches!(unpack_facet(&packed.manifest, &k, &vault, &full), Err(PackError::NotEmpty(_))));
    assert_eq!(fs::read(full.join("keep")).unwrap(), b"mine");
    let empty = s.path("empty");
    fs::create_dir_all(&empty).unwrap();
    unpack_facet(&packed.manifest, &k, &vault, &empty).unwrap();
    assert_eq!(snapshot(&src), snapshot(&empty));
}

#[test]
fn a_head_signed_by_another_key_fails_verify() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let desk = SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap();
    let tower = SeedSigner::from_seed(&[2; 32], KeyHome::File).unwrap();
    let key_of = |x: &SeedSigner| HolderKey::from_verifying_key(&x.public()).unwrap();
    let head = seal(&manifest_of(hostile(vec![])), &k, &vault, &desk).unwrap();
    assert!(head.verify(&key_of(&desk)));
    assert!(!head.verify(&key_of(&tower)));
    let mut moved = head.clone();
    moved.head.generation = 2;
    assert!(!moved.verify(&key_of(&desk)), "another generation");
    let mut junk = head;
    junk.signature = "not a signature".into();
    assert!(!junk.verify(&key_of(&desk)));
}

#[test]
fn the_signed_bytes_are_fixed_json_and_the_manifest_is_sealed() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let signer = SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap();
    let head = seal(&manifest_of(hostile(vec![file("secret-name.txt", &k, &vault)])), &k, &vault, &signer).unwrap();
    let text = String::from_utf8(head.head.canonical_bytes()).unwrap();
    assert!(text.starts_with("{\"format\":1,\"eidolon\":\"eid_"));
    assert!(text.contains(",\"generation\":1,\"manifest\":[\""));
    assert!(!text.contains(' '));
    for name in vault.list().unwrap() {
        let raw = vault.get(&name).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("secret-name"), "file names are not in clear");
    }
    // Another eidolon key cannot read the manifest.
    assert!(load(&head, &keys(2), &vault).is_err());
}

#[test]
fn a_large_manifest_splits_into_several_chunks_and_loads_back() {
    let s = Scratch::new();
    let vault = Vault::open(&s.path("vault")).unwrap();
    let k = keys(1);
    let signer = SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap();
    let entries = (0..6000).map(|i| Entry { path: format!("dir/with/a/long/enough/path/file-{i:06}.txt"), node: Node::Symlink { target: format!("target-number-{i}") } }).collect();
    let m = manifest_of(hostile(entries));
    let head = seal(&m, &k, &vault, &signer).unwrap();
    assert!(head.head.manifest.len() > 1);
    assert_eq!(load(&head, &k, &vault).unwrap(), m);
}
