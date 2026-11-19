//! `kill -9` at every step of `eidolon seal` and `eidolon open`, with the
//! built binary and a real process death. The binary dies at the step named
//! in `EIDOLON_TEST_ABORT_AT` (see `eidolon_facet::abort_at` for the list);
//! each test then checks what the next commands say and that a clean re-run
//! ends byte for byte right. One more test kills `eidolon move` while it
//! hands the seat over and settles the result by hand.
//!
//! `open` has one step of its own beyond the facet crate's list:
//! `after_state_opening` (the state says opening, no file is placed).
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "in a test, a failure should say where")]

mod common;

use common::{bytes, tree, write_tree, Machine, Sandbox, BIN};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const WALL: &str = "wall_work";
const NOTES: &str = "/home/agent/notes";
const ABORT: &str = "EIDOLON_TEST_ABORT_AT";

fn cells_dir(sb: &Sandbox) -> PathBuf {
    sb.root.join("cells")
}

fn machine(sb: &Sandbox, name: &str) -> Machine {
    let m = Machine { home: sb.home(name), cells: Some(cells_dir(sb)), name: name.to_owned(), env: Vec::new() };
    let out = m.run(&["machine", "init", "--name", name, "--key-store", "file"]);
    assert_eq!(out.code, 0, "init {name}: {}", out.stderr);
    m
}

struct World {
    sb: Sandbox,
    desk: Machine,
    tower: Machine,
}

impl World {
    fn new() -> World {
        let sb = Sandbox::new();
        for cell in ["a", "b", "b2"] {
            std::fs::write(cells_dir(&sb).join(format!("{cell}.wall")), WALL).unwrap();
            std::fs::create_dir_all(cells_dir(&sb).join(cell)).unwrap();
        }
        let desk = machine(&sb, "desk");
        let tower = machine(&sb, "tower");
        World { sb, desk, tower }
    }

    fn folder(&self, cell: &str) -> PathBuf {
        cells_dir(&self.sb).join(cell).join(NOTES.trim_start_matches('/'))
    }

    /// A new eidolon on desk, listing desk and tower, with cell `a` filled.
    fn eidolon(&self) -> String {
        small_tree(&self.folder("a"));
        let holders = [self.desk.holder_file(&self.sb.root), self.tower.holder_file(&self.sb.root)];
        let folder = format!("notes={NOTES}");
        let out = self.desk.run(&["new", "--wall", WALL, "--holder", &holders[0], "--holder", &holders[1], "--folder", &folder, "--leave-out", "cache"]);
        assert_eq!(out.code, 0, "new: {}", out.stderr);
        out.json()["eidolon"].as_str().unwrap().to_owned()
    }

    fn empty(&self, cell: &str) {
        let _ = std::fs::remove_dir_all(self.folder(cell));
    }

    /// `m` with the process set to die at `step`.
    fn dying_at(m: &Machine, step: &str) -> Machine {
        Machine { home: m.home.clone(), cells: m.cells.clone(), name: m.name.clone(), env: vec![(ABORT.to_owned(), step.to_owned())] }
    }

    fn state(&self, eid: &str) -> String {
        let ls = self.desk.run(&["ls"]).json();
        let row = ls["eidolons"].as_array().unwrap().iter().find(|r| r["eidolon"] == eid).unwrap().clone();
        row["state"].to_string()
    }

    /// No staging folder of a dead command is left holding plain files.
    fn work_is_clean(&self) -> bool {
        std::fs::read_dir(self.desk.home.join("tmp")).map_or(true, |mut d| d.next().is_none())
    }

    /// What every file of the holding is: whole and parseable, and the
    /// holding is not half-installed.
    fn holding_is_whole(&self, eid: &str) {
        let dir = self.desk.home.join("holdings").join(eid);
        for json in ["spec.json", "seat.json", "chunks.json", "state.json"] {
            let bytes = std::fs::read(dir.join(json)).unwrap();
            assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok(), "{json} is not whole");
        }
        let head = std::fs::read(dir.join("head")).unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&head).is_ok(), "the head is not whole");
        let stray: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| std::path::Path::new(n).extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(stray.is_empty(), "temp files left: {stray:?}");
    }
}

/// Enough to cross a few chunks: some files, a 600 KiB one, a script, a link
/// and a folder the facet leaves out.
fn small_tree(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    for d in 0..3u32 {
        std::fs::create_dir_all(dir.join(format!("d{d}/sub1"))).unwrap();
        for f in 0..4u32 {
            std::fs::write(dir.join(format!("d{d}/sub1/f{f}.txt")), bytes(u64::from(d * 10 + f), 300 + 50 * f as usize)).unwrap();
        }
    }
    std::fs::write(dir.join("d1/sub1/f4.txt"), b"before").unwrap();
    std::fs::write(dir.join("big.bin"), bytes(7, 600 * 1024)).unwrap();
    std::fs::write(dir.join("run.sh"), b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("d0/sub1/f0.txt", dir.join("link")).unwrap();
    std::fs::write(dir.join("cache/junk"), bytes(9, 2048)).unwrap();
}

const SEAL_STEPS: [&str; 5] = ["after_take", "after_chunks", "before_head_recorded", "after_head_recorded", "after_state_sealed"];

/// Kills a seal at `step`. `from_open` is the usual case (open in `a`, files
/// changed, sealed again); otherwise it is the first seal of a new eidolon.
fn seal_killed_at(step: &str, from_open: bool) {
    let w = World::new();
    let eid = w.eidolon();
    if from_open {
        assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
        w.empty("a");
        assert_eq!(w.desk.run(&["open", &eid, "--cell", "a"]).code, 0);
        std::fs::write(w.folder("a").join("d1/sub1/f4.txt"), b"changed after the open").unwrap();
    }
    let want = tree(&w.folder("a"), "cache");
    let which = format!("seal at {step}, from_open {from_open}");

    let killed = World::dying_at(&w.desk, step).run(&["seal", &eid, "--cell", "a"]);
    assert_eq!(killed.code, -1, "{which}: the process should have died by a signal: {}{}", killed.stdout, killed.stderr);

    // Whatever the next command is, the holding is whole, verifies, and is
    // either still open in `a` (the seal never counted) or sealed.
    w.holding_is_whole(&eid);
    let state = w.state(&eid);
    let sealed = state.contains("sealed");
    assert!(sealed || state.contains("open"), "{which}: {state}");
    assert_eq!(sealed, !from_open || step == "after_state_sealed", "{which}: {state}");
    let verify = w.desk.run(&["verify", &eid]);
    assert_eq!(verify.code, 0, "{which}: {}", verify.stdout);

    // A clean run finishes the job, from wherever the kill left it.
    let again = if sealed { w.desk.run(&["seal", &eid, "--cell", "a"]) } else { w.desk.run(&["seal", &eid]) };
    assert_eq!(again.code, 0, "{which}: {}{}", again.stdout, again.stderr);
    assert!(w.work_is_clean(), "{which}: staging files left in the working folder");
    assert_eq!(w.state(&eid), "\"sealed\"", "{which}");
    assert_eq!(w.desk.run(&["verify", &eid]).code, 0, "{which}");
    let opened = w.desk.run(&["open", &eid, "--cell", "b"]);
    assert_eq!(opened.code, 0, "{which}: {}{}", opened.stdout, opened.stderr);
    assert_eq!(tree(&w.folder("b"), "cache"), want, "{which}: not byte for byte");
}

#[test]
fn a_seal_from_an_open_cell_killed_at_any_step_seals_cleanly_on_the_next_run() {
    for step in SEAL_STEPS {
        seal_killed_at(step, true);
    }
}

#[test]
fn the_first_seal_killed_at_any_step_seals_cleanly_on_the_next_run() {
    for step in SEAL_STEPS {
        seal_killed_at(step, false);
    }
}

/// Kills an open at `step` and checks what is recorded.
fn open_killed_at(step: &str) {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let want = tree(&w.folder("a"), "cache");
    let which = format!("open at {step}");

    let killed = World::dying_at(&w.desk, step).run(&["open", &eid, "--cell", "b"]);
    assert_eq!(killed.code, -1, "{which}: {}{}", killed.stdout, killed.stderr);
    w.holding_is_whole(&eid);
    assert_eq!(w.desk.run(&["verify", &eid]).code, 0, "{which}");

    if step == "after_state_open" {
        // Recorded as open, and the files are there.
        assert!(w.state(&eid).contains("\"open\""), "{which}");
        assert_eq!(tree(&w.folder("b"), "cache"), want, "{which}: recorded open without the files");
        let second = w.desk.run(&["open", &eid, "--cell", "b2"]);
        assert_eq!((second.code, second.code_of_error().as_str()), (1, "second_seat"), "{which}");
        // Sealing from where it is recorded works.
        assert_eq!(w.desk.run(&["seal", &eid]).code, 0, "{which}");
        assert_eq!(w.state(&eid), "\"sealed\"");
        w.empty("b");
        assert_eq!(w.desk.run(&["open", &eid, "--cell", "b"]).code, 0, "{which}");
    } else {
        // Recorded as opening in `b`, whatever was placed there. Nothing
        // seals or moves from a copy that may be partial.
        assert_eq!(w.state(&eid), "{\"opening\":{\"cell\":\"b\"}}", "{which}");
        for refused in [w.desk.run(&["seal", &eid]), w.desk.run(&["seal", &eid, "--cell", "b"])] {
            assert_eq!((refused.code, refused.code_of_error().as_str()), (1, "open_unfinished"), "{which}: {}", refused.stdout);
        }
        let moved = w.desk.move_to(&eid, &w.tower);
        assert_eq!((moved.code, moved.code_of_error().as_str()), (1, "open_unfinished"), "{which}");
        // Running open again, in the same cell, finishes it.
        let again = w.desk.run(&["open", &eid, "--cell", "b"]);
        assert_eq!(again.code, 0, "{which}: {}{}", again.stdout, again.stderr);
        assert!(again.json()["cleared"].is_null(), "{which}: the same cell is not cleared");
        assert!(w.state(&eid).contains("\"cell\":\"b\""), "{which}");
    }
    assert!(w.work_is_clean(), "{which}: staging files left");
    assert_eq!(tree(&w.folder("b"), "cache"), want, "{which}: not byte for byte");
    assert_eq!(w.desk.run(&["verify", &eid]).code, 0, "{which}");
}

#[test]
fn an_open_killed_at_any_step_leaves_it_opening_or_open_and_opens_cleanly_again() {
    for step in ["after_state_opening", "after_unpack", "before_state_open", "after_state_open"] {
        open_killed_at(step);
    }
}

#[test]
fn an_open_killed_after_placing_files_is_cleared_when_the_person_opens_elsewhere() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let want = tree(&w.folder("a"), "cache");
    assert_eq!(World::dying_at(&w.desk, "before_state_open").run(&["open", &eid, "--cell", "b"]).code, -1);
    assert!(w.folder("b").exists(), "the kill left a copy in b");
    // Opening in `b2` first clears `b`, says so, and records exactly one seat.
    let other = w.desk.run(&["open", &eid, "--cell", "b2"]);
    assert_eq!(other.code, 0, "{}{}", other.stdout, other.stderr);
    assert_eq!(other.json()["cleared"], "b");
    assert!(!w.folder("b").exists(), "the unrecorded copy in b is gone");
    let ls = w.desk.run(&["ls"]).json();
    assert_eq!(ls["eidolons"].as_array().unwrap().len(), 1);
    assert!(ls["eidolons"][0]["state"].to_string().contains("\"b2\""));
    assert_eq!(w.desk.run(&["open", &eid, "--cell", "b"]).code_of_error(), "second_seat");
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "b"]).code_of_error(), "second_seat", "only the recorded cell seals it");
    assert_eq!(w.desk.run(&["seal", &eid]).code, 0);
    assert_eq!(tree(&w.folder("b2"), "cache"), want);
}

#[test]
fn an_open_killed_before_any_file_is_placed_leaves_nothing_to_clear_and_opens_elsewhere() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let want = tree(&w.folder("a"), "cache");
    assert_eq!(World::dying_at(&w.desk, "after_state_opening").run(&["open", &eid, "--cell", "b"]).code, -1);
    assert!(!w.folder("b").exists());
    let other = w.desk.run(&["open", &eid, "--cell", "b2"]);
    assert_eq!(other.code, 0, "{}{}", other.stdout, other.stderr);
    assert_eq!(other.json()["cleared"], "b", "it says where it looked, even when nothing was there");
    assert_eq!(tree(&w.folder("b2"), "cache"), want);
}

#[test]
fn a_sweep_removes_the_staging_files_of_a_dead_command_and_leaves_a_live_ones() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(World::dying_at(&w.desk, "after_take").run(&["seal", &eid, "--cell", "a"]).code, -1);
    // The dead command left the cell's files, in the clear, in its folder.
    let left: Vec<_> = std::fs::read_dir(w.desk.home.join("tmp")).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(left.len(), 1, "{left:?}");
    // One that names a live process is not touched.
    let alive = w.desk.home.join("tmp").join(format!("run-{}-live", std::process::id()));
    std::fs::create_dir_all(&alive).unwrap();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    assert!(!left[0].exists(), "the dead command's folder is swept");
    assert!(alive.exists(), "a live process's folder stays");
}

/// Kills `eidolon move` (SIGKILL) the moment it has written its pending
/// record, which is just before it hands the seat over. Where the receiver
/// got to is a race, and the test is right either way: the sender is closed,
/// and the person's `move-settle` (confirm what arrived, or abandon what did
/// not) leaves exactly one machine that can open it, byte for byte.
#[test]
fn a_move_killed_while_handing_over_is_closed_until_settled_and_then_one_machine_opens_it() {
    let w = World::new();
    let eid = w.eidolon();
    // The full-size tree, for a move with many chunks.
    w.empty("a");
    write_tree(&w.folder("a"));
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let want = tree(&w.folder("a"), "cache");

    let pending = w.desk.home.join("holdings").join(&eid).join("pending.json");
    let env = format!("EIDOLON_HOME={}", w.tower.home.display());
    let mut sender = Command::new(BIN)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("EIDOLON_HOME", &w.desk.home)
        .args(["--json", "--cell-root"])
        .arg(cells_dir(&w.sb))
        .args(["move", &eid, "--", "env", &env, BIN, "receive"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let mut killed_in_time = false;
    while started.elapsed() < Duration::from_secs(60) {
        if pending.exists() {
            sender.kill().unwrap();
            killed_in_time = true;
            break;
        }
        if sender.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    let _ = sender.wait();
    assert!(killed_in_time, "the move finished before the pending record could be caught");
    // Let the receiver, which sees the pipe close, finish what it was doing.
    std::thread::sleep(Duration::from_millis(1500));

    // Desk is closed: a move away was begun and not confirmed.
    let ls = w.desk.run(&["ls"]).json();
    assert_eq!(ls["eidolons"][0]["pending_move"], true);
    println!("move killed at the pending record; tower holds it: {}", w.tower.run(&["ls"]).stdout.contains(&eid));
    let closed = w.desk.run(&["open", &eid, "--cell", "b"]);
    assert_eq!((closed.code, closed.code_of_error().as_str()), (1, "second_seat"));

    let tower_has_it = w.tower.run(&["ls"]).json()["eidolons"].as_array().unwrap().iter().any(|r| r["eidolon"] == eid.as_str());
    if tower_has_it {
        let seat = w.tower.home.join("holdings").join(&eid).join("seat.json");
        let confirmed = w.desk.run(&["move-settle", &eid, "--confirm", &seat.display().to_string()]);
        assert_eq!(confirmed.code, 0, "{}{}", confirmed.stdout, confirmed.stderr);
        assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["state"], "retired");
        assert_eq!(w.desk.run(&["open", &eid, "--cell", "b"]).code_of_error(), "retired");
        let opened = w.tower.run(&["open", &eid, "--cell", "b"]);
        assert_eq!(opened.code, 0, "{}{}", opened.stdout, opened.stderr);
        assert_eq!(tree(&w.folder("b"), "cache"), want);
        // And it moves on from there, back the way it came.
        assert_eq!(w.tower.run(&["seal", &eid]).code, 0);
        let back = w.tower.move_to(&eid, &w.desk);
        assert_eq!(back.code, 0, "{}{}", back.stdout, back.stderr);
    } else {
        assert_eq!(w.desk.run(&["move-settle", &eid, "--abandon"]).code, 0);
        assert_eq!(w.tower.run(&["open", &eid, "--cell", "b"]).code_of_error(), "unknown_eidolon");
        let opened = w.desk.run(&["open", &eid, "--cell", "b"]);
        assert_eq!(opened.code, 0, "{}{}", opened.stdout, opened.stderr);
        assert_eq!(tree(&w.folder("b"), "cache"), want);
        // And a clean move afterwards goes through.
        assert_eq!(w.desk.run(&["seal", &eid]).code, 0);
        let again = w.desk.move_to(&eid, &w.tower);
        assert_eq!(again.code, 0, "{}{}", again.stdout, again.stderr);
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

/// The other half of the race above, made exact: the receiver took the seat
/// and the sender died before it heard so. Desk's holding is put back as the
/// dead sender left it (sealed, a pending record, its key still there).
#[test]
fn a_move_that_arrived_but_was_never_heard_is_confirmed_by_the_seat_the_other_machine_holds() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let want = tree(&w.folder("a"), "cache");
    let holding = w.desk.home.join("holdings").join(&eid);
    let before = w.sb.root.join("desk-before");
    copy_tree(&holding, &before);
    let key_file = w.desk.home.join("keys").join(format!("eidolon__{eid}"));
    let key = std::fs::read(&key_file).unwrap();

    let moved = w.desk.move_to(&eid, &w.tower);
    assert_eq!(moved.code, 0, "{}{}", moved.stdout, moved.stderr);
    let tower_seat = w.tower.home.join("holdings").join(&eid).join("seat.json");

    copy_tree(&before, &holding);
    std::fs::write(&key_file, key).unwrap();
    let tower_key = w.tower.run(&["machine", "show"]).json()["sign"].as_str().unwrap().to_owned();
    std::fs::write(holding.join("pending.json"), format!("{{\"to\":\"{tower_key}\",\"generation\":2}}")).unwrap();
    assert_eq!(w.desk.run(&["open", &eid, "--cell", "b"]).code_of_error(), "second_seat", "closed until settled");

    let confirmed = w.desk.run(&["move-settle", &eid, "--confirm", &tower_seat.display().to_string()]);
    assert_eq!(confirmed.code, 0, "{}{}", confirmed.stdout, confirmed.stderr);
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["state"], "retired");
    assert!(!key_file.exists(), "desk let go of the key");
    assert_eq!(w.desk.run(&["open", &eid, "--cell", "b"]).code_of_error(), "retired");
    let opened = w.tower.run(&["open", &eid, "--cell", "b"]);
    assert_eq!(opened.code, 0, "{}{}", opened.stdout, opened.stderr);
    assert_eq!(tree(&w.folder("b"), "cache"), want);
}
