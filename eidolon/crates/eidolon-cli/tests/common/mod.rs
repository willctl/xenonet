//! Helpers shared by the CLI tests: two machines' homes, a folder of
//! cells, and a way to run the built `eidolon` and read what it says.

use serde_json::Value;
use std::collections::BTreeMap;
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

pub const BIN: &str = env!("CARGO_BIN_EXE_eidolon");

pub struct Sandbox {
    pub root: PathBuf,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

impl Sandbox {
    pub fn new() -> Sandbox {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("eidolon-cli-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cells")).unwrap();
        Sandbox { root }
    }

    pub fn home(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub struct Machine {
    pub home: PathBuf,
    /// The folder of cells (`--cell-root`); none when the machine uses
    /// Adytum's real cells.
    pub cells: Option<PathBuf>,
    pub name: String,
    /// Added to the (otherwise empty) environment of every run.
    pub env: Vec<(String, String)>,
}

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> Value {
        let parsed = serde_json::from_str(self.stdout.trim());
        assert!(parsed.is_ok(), "not JSON: {:?} / {:?}", self.stdout, self.stderr);
        parsed.unwrap()
    }

    pub fn code_of_error(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }
}

impl Machine {
    /// Runs `eidolon --json <args> --cell-root <cells>` as this machine.
    pub fn run(&self, args: &[&str]) -> Out {
        let mut cmd = Command::new(BIN);
        cmd.env_clear().env("PATH", "/usr/bin:/bin").env("EIDOLON_HOME", &self.home).envs(self.env.iter().cloned()).arg("--json");
        if let Some(cells) = &self.cells {
            cmd.arg("--cell-root").arg(cells);
        }
        cmd.args(args);
        let out = cmd.stdin(Stdio::null()).output().unwrap();
        Out { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&out.stdout).into_owned(), stderr: String::from_utf8_lossy(&out.stderr).into_owned() }
    }

    /// The holder entry other machines list this one by, in a file.
    pub fn holder_file(&self, dir: &Path) -> String {
        let out = self.run(&["machine", "show"]);
        assert_eq!(out.code, 0, "{}", out.stderr);
        let path = dir.join(format!("{}.holder.json", self.name));
        std::fs::write(&path, out.stdout.trim()).unwrap();
        path.display().to_string()
    }

    /// `move <eid> -- env EIDOLON_HOME=<to> eidolon receive`.
    pub fn move_to(&self, eid: &str, to: &Machine) -> Out {
        let env = format!("EIDOLON_HOME={}", to.home.display());
        self.run(&["move", eid, "--", "env", &env, BIN, "receive"])
    }
}

pub fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// A few hundred nested files, a 5 MiB one, a script, a symlink, and a
/// cache folder the facet leaves out.
pub fn write_tree(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for d in 0..10u32 {
        for f in 0..30u32 {
            let path = dir.join(format!("d{d}/sub{}/f{f}.txt", f % 3));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes(u64::from(d * 100 + f), 200 + usize::try_from(f).unwrap() * 37)).unwrap();
        }
    }
    std::fs::write(dir.join("big.bin"), bytes(7, 5 * 1024 * 1024)).unwrap();
    std::fs::write(dir.join("run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink("d0/sub0/f0.txt", dir.join("link")).unwrap();
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("cache/junk"), bytes(9, 4096)).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
pub enum Node {
    Dir(u32),
    File(u32, Vec<u8>),
    Link(PathBuf),
}

/// Every path under `dir` with its kind, mode and bytes (or link target),
/// leaving out anything under a folder named `skip`.
pub fn tree(dir: &Path, skip: &str) -> BTreeMap<String, Node> {
    fn walk(base: &Path, dir: &Path, skip: &str, out: &mut BTreeMap<String, Node>) {
        for child in std::fs::read_dir(dir).unwrap() {
            let child = child.unwrap();
            let path = child.path();
            let rel = path.strip_prefix(base).unwrap().display().to_string();
            if rel == skip {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                out.insert(rel, Node::Link(std::fs::read_link(&path).unwrap()));
            } else if meta.is_dir() {
                out.insert(rel, Node::Dir(meta.mode() & 0o7777));
                walk(base, &path, skip, out);
            } else {
                out.insert(rel, Node::File(meta.mode() & 0o7777, std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, skip, &mut out);
    out
}
