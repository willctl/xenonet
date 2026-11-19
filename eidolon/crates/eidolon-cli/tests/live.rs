//! The real thing: an eidolon moved between two Lima cells behind one wall,
//! through two machines' homes, and the four refusals against live cells.
//!
//! Skipped unless `ADYTUM_LIVE=1`. Run it as
//! `ADYTUM_LIVE=1 npm run cargo -- test -p eidolon-cli --test live -- --nocapture`.
//!
//! The cells are `adytum-test-a-<n>` and `adytum-test-b-<n>` (booted, wall
//! `wall_live`) and `adytum-test-c-<n>`, which is only a row in Adytum's
//! ledger behind another wall: the wall check reads the ledger and never
//! reaches the cell, so a third VM would prove nothing more and cost a
//! boot. They live in a Lima home of their own (`ADYTUM_HOME`), and a drop
//! guard deletes them, also when the test fails.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "in a test, a failure should say where")]

mod common;

use adytum_driver::{Driver, ExecOpts};
use adytum_ledger::{create_cell, Ledger, Outcome};
use adytum_lima::{lima_home_in, Lima};
use adytum_model::{CellName, CellSpec, DriverKind, Resources};
use common::{tree, write_tree, Machine, Node, Sandbox};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const WALL: &str = "wall_live";
const FOLDER: &str = "/tmp/eidolon-live/notes";

/// Deletes the cells and Adytum's folder, whatever happens.
struct Cleanup {
    home: PathBuf,
    names: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for name in &self.names {
            // Only our own names, only in our own Lima home.
            assert!(name.starts_with("adytum-test-"));
            let _ = Command::new("limactl").env("LIMA_HOME", lima_home_in(&self.home)).args(["delete", "--force", "--tty=false", name]).output();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn spec(name: &str, wall: &str) -> CellSpec {
    CellSpec {
        name: name.parse().unwrap(),
        driver: DriverKind::Lima,
        image: adytum_lima::image(adytum_lima::DEFAULT_IMAGE).unwrap(),
        wall: wall.parse().unwrap(),
        resources: Resources::default(),
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: true,
        network: None,
        clone_from: None,
        secrets: vec![],
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

async fn sh(lima: &Lima, cell: &CellName, script: &str) -> String {
    let out = lima.exec(cell, &argv(&["sh", "-c", script]), ExecOpts::default()).await.unwrap();
    assert!(out.success(), "{script}: {}", out.stderr_lossy());
    out.stdout_lossy()
}

/// Mode, kind, link target and checksum of everything under the folder.
async fn listing(lima: &Lima, cell: &CellName) -> String {
    // `cache` is what the facet leaves out, so it is not compared.
    let all = sh(lima, cell, &format!("cd {FOLDER} && find . -printf '%y %m %p %l\\n' | LC_ALL=C sort && find . -type f -exec sha256sum {{}} + | LC_ALL=C sort -k2")).await;
    all.lines().filter(|l| !l.contains("./cache")).collect::<Vec<_>>().join("\n")
}

/// The lines the two listings do not share, so a failure names the files.
fn differences(left: &str, right: &str) -> Vec<String> {
    let (l, r): (std::collections::BTreeSet<&str>, std::collections::BTreeSet<&str>) = (left.lines().collect(), right.lines().collect());
    l.symmetric_difference(&r).take(12).map(|x| (*x).to_owned()).collect()
}

struct Steps(Vec<(String, f64)>);

impl Steps {
    fn time<T>(&mut self, name: &str, started: Instant, value: T) -> T {
        self.0.push((name.to_owned(), started.elapsed().as_secs_f64()));
        value
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_eidolon_moves_between_two_real_cells_and_the_refusals_hold() {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: set ADYTUM_LIVE=1 to run against Lima");
        return;
    }
    let tag = format!("{:05x}", std::process::id() & 0xf_ffff);
    let (a, b, c) = (format!("adytum-test-a-{tag}"), format!("adytum-test-b-{tag}"), format!("adytum-test-c-{tag}"));
    // Short, because Lima keeps unix sockets under it.
    let home = PathBuf::from(format!("/tmp/eidl-{tag}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let _cleanup = Cleanup { home: home.clone(), names: vec![a.clone(), b.clone(), c.clone()] };
    let mut steps = Steps(Vec::new());

    // ---- the cells -------------------------------------------------------
    let lima = Lima::system(lima_home_in(&home));
    let ledger = Ledger::open(&home).unwrap();
    let t = Instant::now();
    let (spec_a, spec_b) = (spec(&a, WALL), spec(&b, WALL));
    let (made_a, made_b) = tokio::join!(create_cell(&ledger, &lima, &spec_a), create_cell(&ledger, &lima, &spec_b));
    made_a.unwrap();
    made_b.unwrap();
    let (cell_a, cell_b): (CellName, CellName) = (a.parse().unwrap(), b.parse().unwrap());
    let (started_a, started_b) = tokio::join!(lima.start(&cell_a), lima.start(&cell_b));
    started_a.unwrap();
    started_b.unwrap();
    steps.time("create and start cells A and B", t, ());
    // C: a record behind another wall, no VM.
    let op = ledger.intend_create(&spec(&c, "wall_elsewhere")).unwrap();
    ledger.settle(op, Outcome::Done).unwrap();

    // ---- what is written in A, through the driver -------------------------
    let sb = Sandbox::new();
    let seed = sb.root.join("seed/notes");
    write_tree(&seed);
    let t = Instant::now();
    sh(&lima, &cell_a, "rm -rf /tmp/eidolon-seed && mkdir -p /tmp/eidolon-seed /tmp/eidolon-live").await;
    lima.copy_in(&cell_a, &seed, "/tmp/eidolon-seed").await.unwrap();
    sh(&lima, &cell_a, &format!("rm -rf {FOLDER} && mkdir -p {FOLDER} && cp -a /tmp/eidolon-seed/. {FOLDER}/")).await;
    steps.time("write the folder in A", t, ());
    let written = listing(&lima, &cell_a).await;
    assert!(written.lines().count() > 600, "{}", written.lines().count());

    // ---- two machines ------------------------------------------------------
    let env = vec![
        ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
        ("HOME".to_owned(), std::env::var("HOME").unwrap_or_default()),
        ("ADYTUM_HOME".to_owned(), home.display().to_string()),
    ];
    let machine = |name: &str| {
        let m = Machine { home: sb.home(name), cells: None, name: name.to_owned(), env: env.clone() };
        assert_eq!(m.run(&["machine", "init", "--name", name, "--key-store", "file"]).code, 0);
        m
    };
    let (desk, tower, stranger) = (machine("desk"), machine("tower"), machine("stranger"));
    let holders = [desk.holder_file(&sb.root), tower.holder_file(&sb.root)];
    let new = desk.run(&["new", "--wall", WALL, "--holder", &holders[0], "--holder", &holders[1], "--folder", &format!("notes={FOLDER}"), "--leave-out", "cache"]);
    assert_eq!(new.code, 0, "{}{}", new.stderr, new.stdout);
    let eid = new.json()["eidolon"].as_str().unwrap().to_owned();

    // ---- seal from A, move, open in B -------------------------------------
    let t = Instant::now();
    let sealed = desk.run(&["seal", &eid, "--cell", &a]);
    assert_eq!(sealed.code, 0, "{}{}", sealed.stderr, sealed.stdout);
    steps.time("desk: seal from A", t, ());

    let t = Instant::now();
    let first = desk.move_to(&eid, &tower);
    assert_eq!(first.code, 0, "{}{}", first.stderr, first.stdout);
    let first = first.json();
    steps.time("move desk to tower", t, ());

    let retired = desk.run(&["open", &eid, "--cell", &a]);
    assert_eq!((retired.code, retired.code_of_error().as_str()), (1, "retired"), "desk's open is refused");
    let wrong = tower.run(&["open", &eid, "--cell", &c]);
    assert_eq!((wrong.code, wrong.code_of_error().as_str()), (1, "wrong_wall"), "a cell behind another wall");

    let t = Instant::now();
    let opened = tower.run(&["open", &eid, "--cell", &b]);
    assert_eq!(opened.code, 0, "{}{}", opened.stderr, opened.stdout);
    steps.time("tower: open in B", t, ());
    let second_open = tower.run(&["open", &eid, "--cell", &a]);
    assert_eq!((second_open.code, second_open.code_of_error().as_str()), (1, "second_seat"), "a second open copy");

    // Every byte and mode, in the guest and on the host.
    let in_b = listing(&lima, &cell_b).await;
    assert_eq!(differences(&in_b, &written), Vec::<String>::new(), "B matches A in the guest");
    let t = Instant::now();
    sh(&lima, &cell_b, &format!("tar -C {FOLDER} -cf /tmp/eidolon-b.tar .")).await;
    let out_dir = sb.root.join("from-b");
    std::fs::create_dir_all(&out_dir).unwrap();
    lima.copy_out(&cell_b, "/tmp/eidolon-b.tar", &out_dir.join("b.tar")).await.unwrap();
    let extracted = out_dir.join("files");
    std::fs::create_dir_all(&extracted).unwrap();
    assert!(Command::new("tar").arg("-xf").arg(out_dir.join("b.tar")).arg("-C").arg(&extracted).status().unwrap().success());
    steps.time("copy B's folder out", t, ());
    let got = tree(&extracted, "");
    let want = tree(&seed, "cache");
    assert!(matches!(want.get("link"), Some(Node::Link(_))) && matches!(want.get("run.sh"), Some(Node::File(0o755, _))));
    assert_eq!(got.len(), want.len());
    for (path, node) in &want {
        // Folders can differ in mode between host and guest; files and links may not.
        if !matches!(node, Node::Dir(_)) {
            assert_eq!(got.get(path), Some(node), "{path}");
        }
    }

    // ---- change B, seal on tower, move back, open in A ---------------------
    let t = Instant::now();
    sh(&lima, &cell_b, &format!("cd {FOLDER} && printf 'changed\\n' > d1/sub1/f4.txt && head -c 50000 /dev/urandom >> big.bin")).await;
    let changed = listing(&lima, &cell_b).await;
    steps.time("change a small file and append to the big one in B", t, ());

    let t = Instant::now();
    let resealed = tower.run(&["seal", &eid]);
    assert_eq!(resealed.code, 0, "{}{}", resealed.stderr, resealed.stdout);
    steps.time("tower: seal from B", t, ());

    let t = Instant::now();
    let second = tower.move_to(&eid, &desk);
    assert_eq!(second.code, 0, "{}{}", second.stderr, second.stdout);
    let second = second.json();
    steps.time("move tower to desk", t, ());

    let t = Instant::now();
    let back = desk.run(&["open", &eid, "--cell", &a]);
    assert_eq!(back.code, 0, "{}{}", back.stderr, back.stdout);
    steps.time("desk: open in A", t, ());
    assert_eq!(differences(&listing(&lima, &cell_a).await, &changed), Vec::<String>::new(), "A now matches B");

    // ---- a changed chunk on desk -------------------------------------------
    assert_eq!(desk.run(&["seal", &eid]).code, 0);
    assert_eq!(desk.run(&["verify", &eid]).code, 0);
    let chunk = std::fs::read_dir(desk.home.join("vault/chunks"))
        .unwrap()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .map(|f| f.unwrap().path())
        .find(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .unwrap();
    flip(&chunk);
    let verify = desk.run(&["verify", &eid]);
    assert_eq!((verify.code, verify.code_of_error().as_str()), (1, "changed_chunk"));
    let open = desk.run(&["open", &eid, "--cell", &b]);
    assert_eq!((open.code, open.code_of_error().as_str()), (1, "changed_chunk"));

    // ---- a machine off the list ---------------------------------------------
    let refused = tower.move_to(&eid, &stranger);
    assert_eq!(refused.code, 1, "tower retired it, so the move is refused before any stranger is asked");
    let off = desk.move_to(&eid, &stranger);
    assert_eq!((off.code, off.code_of_error().as_str()), (1, "not_a_holder"));

    // ---- the summary -----------------------------------------------------------
    let mut table = String::from("\nstep                                                     seconds\n");
    for (name, secs) in &steps.0 {
        let _ = writeln!(table, "{name:<55} {secs:>8.2}");
    }
    for (label, m) in [("first move (desk to tower)", &first), ("second move (tower to desk)", &second)] {
        let _ = writeln!(table, "{label}: {} of {} chunks sent, {} bytes sent, {} seconds", m["chunks_sent"], m["chunks_offered"], m["bytes_sent"], m["seconds"]);
    }
    println!("{table}");
}

fn flip(file: &Path) {
    let mut data = std::fs::read(file).unwrap();
    let at = data.len() / 2;
    data[at] ^= 1;
    std::fs::write(file, data).unwrap();
}
