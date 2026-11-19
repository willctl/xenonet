//! No secret in an argument, a file or a log. One full life of an eidolon
//! (new, seal, move to tower, open, seal, move back, open) runs through the
//! built binary under two `EIDOLON_HOME`s. Every secret either machine keeps
//! (the eidolon's key, each machine's signing seed and sealing key) is then
//! looked for, in raw bytes, hex, base64url and base64, in everything the
//! binary printed, in both directions of the move's wire, and in every file
//! under both homes and the cells except the key stores themselves. The key
//! stores must be owner-only.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "in a test, a failure should say where")]

mod common;

use common::{tree, write_tree, Machine, Out, Sandbox, BIN};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use xenonet_id::codec::{b64, hex};

const WALL: &str = "wall_work";
const NOTES: &str = "/home/agent/notes";

fn machine(sb: &Sandbox, name: &str) -> Machine {
    let m = Machine { home: sb.home(name), cells: Some(sb.root.join("cells")), name: name.to_owned(), env: Vec::new() };
    let out = m.run(&["machine", "init", "--name", name, "--key-store", "file"]);
    assert_eq!(out.code, 0, "init {name}: {}", out.stderr);
    m
}

/// Every form a secret could be written in.
fn forms(secret: &[u8]) -> Vec<(String, Vec<u8>)> {
    let url = b64(secret);
    let mut std_b64 = url.replace('-', "+").replace('_', "/");
    let unpadded = std_b64.clone();
    while !std_b64.len().is_multiple_of(4) {
        std_b64.push('=');
    }
    vec![
        ("raw".into(), secret.to_vec()),
        ("hex".into(), hex(secret).into_bytes()),
        ("base64url".into(), url.into_bytes()),
        ("base64".into(), unpadded.into_bytes()),
        ("base64 padded".into(), std_b64.into_bytes()),
    ]
}

fn files_under(dir: &Path, skip: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        if path == skip {
            continue;
        }
        let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
        if kind.is_dir() {
            files_under(&path, skip, out);
        } else if kind.is_file() {
            out.push(path);
        }
    }
}

fn found(haystack: &[u8], needle: &[u8]) -> bool {
    let Some((&first, _)) = needle.split_first() else { return false };
    haystack.iter().enumerate().filter(|(_, &b)| b == first).any(|(i, _)| haystack[i..].starts_with(needle))
}

/// Adds every secret in `home`'s key store to `secrets`, by file and content,
/// so a key that is later deleted (a moved eidolon's) is still looked for.
fn collect(m: &Machine, secrets: &mut BTreeMap<String, Vec<u8>>) {
    for e in std::fs::read_dir(m.home.join("keys")).unwrap() {
        let e = e.unwrap();
        let content = std::fs::read(e.path()).unwrap();
        assert!(content.len() >= 16, "{} holds {} bytes", e.file_name().display(), content.len());
        secrets.insert(format!("{}:{}", m.name, e.file_name().to_string_lossy()), content);
    }
}

#[test]
fn no_key_reaches_an_argument_a_file_a_log_or_the_wire() {
    let sb = Sandbox::new();
    let cells = sb.root.join("cells");
    std::fs::write(cells.join("a.wall"), WALL).unwrap();
    std::fs::write(cells.join("b.wall"), WALL).unwrap();
    let folder = |cell: &str| cells.join(cell).join(NOTES.trim_start_matches('/'));
    std::fs::create_dir_all(folder("a")).unwrap();
    std::fs::create_dir_all(folder("b")).unwrap();
    write_tree(&folder("a"));
    let desk = machine(&sb, "desk");
    let tower = machine(&sb, "tower");
    let holders = [desk.holder_file(&sb.root), tower.holder_file(&sb.root)];

    let mut said: Vec<(String, Out)> = Vec::new();
    let mut secrets: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut step = |label: &str, out: Out| {
        assert_eq!(out.code, 0, "{label}: {}{}", out.stdout, out.stderr);
        said.push((label.to_owned(), out));
        // After every step, so a key that is deleted later is still looked for.
        collect(&desk, &mut secrets);
        collect(&tower, &mut secrets);
    };
    let folder_arg = format!("notes={NOTES}");
    let new = desk.run(&["new", "--wall", WALL, "--holder", &holders[0], "--holder", &holders[1], "--folder", &folder_arg]);
    let eid = new.json()["eidolon"].as_str().unwrap().to_owned();
    step("new", new);
    step("seal", desk.run(&["seal", &eid, "--cell", "a"]));
    step("verify", desk.run(&["verify", &eid]));

    // The first move runs through `tee`, so both directions of the wire are
    // kept for the scan.
    let (wire_in, wire_out) = (sb.root.join("wire-in.bin"), sb.root.join("wire-out.bin"));
    let script = format!("tee '{}' | env EIDOLON_HOME='{}' '{}' receive | tee '{}'", wire_in.display(), tower.home.display(), BIN, wire_out.display());
    step("move", desk.run(&["move", &eid, "--", "sh", "-c", &script]));
    step("open", tower.run(&["open", &eid, "--cell", "b"]));
    step("reseal", tower.run(&["seal", &eid]));
    step("move back", tower.move_to(&eid, &desk));
    step("open again", desk.run(&["open", &eid, "--cell", "a"]));
    step("ls", desk.run(&["ls"]));
    step("tower ls", tower.run(&["ls"]));
    step("facets", desk.run(&["facets", &eid]));
    // Refusals print too: a wrong-wall seal, an unknown eidolon.
    let refused = desk.run(&["verify", "eid_aeaqcaibaeaqcaibaeaqcaibae"]);
    assert_eq!((refused.code, refused.code_of_error().as_str()), (1, "unknown_eidolon"));
    said.push(("refusal".into(), refused));

    // Each machine has a signing key and a sealing key, and held the
    // eidolon's key for a while.
    assert_eq!(secrets.len(), 6, "{:?}", secrets.keys());
    // The scan itself finds a planted secret in each form.
    for (form, needle) in forms(&secrets["desk:machine-seal"]) {
        let planted = [b"junk ".as_slice(), &needle, b" junk"].concat();
        assert!(found(&planted, &needle), "{form}");
    }
    assert!(secrets.keys().filter(|k| k.contains("eidolon__")).count() == 2, "{:?}", secrets.keys());
    println!("secrets checked: {:?}", secrets.keys().collect::<Vec<_>>());

    let mut places: Vec<(String, Vec<u8>)> = Vec::new();
    for (label, out) in &said {
        places.push((format!("stdout of {label}"), out.stdout.clone().into_bytes()));
        places.push((format!("stderr of {label}"), out.stderr.clone().into_bytes()));
    }
    let mut files = Vec::new();
    for m in [&desk, &tower] {
        files_under(&m.home, &m.home.join("keys"), &mut files);
    }
    files_under(&cells, &sb.root.join("nothing"), &mut files);
    files.extend([wire_in.clone(), wire_out.clone()]);
    files.extend(holders.iter().map(PathBuf::from));
    assert!(files.len() > 40, "{} files scanned", files.len());
    for f in files {
        let name = f.strip_prefix(&sb.root).unwrap().display().to_string();
        places.push((name, std::fs::read(&f).unwrap()));
    }
    assert!(std::fs::metadata(&wire_in).unwrap().len() > 100_000 && std::fs::metadata(&wire_out).unwrap().len() > 0, "the wire was kept");

    let mut leaks = Vec::new();
    for (secret_name, secret) in &secrets {
        for (form, needle) in forms(secret) {
            for (place, content) in &places {
                if found(content, &needle) {
                    leaks.push(format!("{secret_name} ({form}) is in {place}"));
                }
            }
        }
    }
    assert!(leaks.is_empty(), "{leaks:#?}");

    assert_eq!(tree(&folder("a"), "cache"), tree(&folder("b"), "cache"));

    // The key stores, and the machine's name file, are owner-only.
    for m in [&desk, &tower] {
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&m.home.join("keys")), 0o700, "{} keys folder", m.name);
        for e in std::fs::read_dir(m.home.join("keys")).unwrap() {
            let p = e.unwrap().path();
            assert_eq!(mode(&p), 0o600, "{}", p.display());
        }
        assert_eq!(mode(&m.home.join("machine.json")), 0o600);
    }
}
