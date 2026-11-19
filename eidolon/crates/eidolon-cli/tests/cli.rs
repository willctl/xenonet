//! The `eidolon` binary, end to end, with two machines (desk and tower),
//! each with its own `EIDOLON_HOME`, and folders standing in for cells
//! through the hidden `--cell-root` option.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "in a test, a failure should say where")]

mod common;

use common::{bytes, tree, write_tree, Machine, Sandbox};
use std::path::PathBuf;

const WALL: &str = "wall_work";

fn cells_dir(sb: &Sandbox) -> PathBuf {
    let dir = sb.root.join("cells");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A machine with its own home, set up and ready.
fn machine(sb: &Sandbox, name: &str) -> Machine {
    let m = Machine { home: sb.home(name), cells: Some(cells_dir(sb)), name: name.to_owned(), env: Vec::new() };
    let out = m.run(&["machine", "init", "--name", name, "--key-store", "file"]);
    assert_eq!(out.code, 0, "init {name}: {}", out.stderr);
    m
}

/// A cell behind `wall`: a folder and its wall record.
fn cell(sb: &Sandbox, name: &str, wall: &str) {
    std::fs::write(cells_dir(sb).join(format!("{name}.wall")), wall).unwrap();
    std::fs::create_dir_all(cells_dir(sb).join(name)).unwrap();
}

struct World {
    sb: Sandbox,
    desk: Machine,
    tower: Machine,
    notes: &'static str,
}

impl World {
    fn new() -> World {
        let sb = Sandbox::new();
        cell(&sb, "a", WALL);
        cell(&sb, "b", WALL);
        cell(&sb, "c", "wall_other");
        let desk = machine(&sb, "desk");
        let tower = machine(&sb, "tower");
        World { sb, desk, tower, notes: "/home/agent/notes" }
    }

    fn folder(&self, cell: &str) -> PathBuf {
        cells_dir(&self.sb).join(cell).join(self.notes.trim_start_matches('/'))
    }

    /// A cell with nothing in it, as a fresh one is (the folder cell the
    /// tests use cannot copy over a symlink the way a real cell's `cp -a`
    /// does).
    fn empty(&self, cell: &str) {
        let _ = std::fs::remove_dir_all(self.folder(cell));
    }

    /// A new eidolon on desk, listing desk and tower, with `a`'s folder filled.
    fn eidolon(&self) -> String {
        write_tree(&self.folder("a"));
        let holders = [self.desk.holder_file(&self.sb.root), self.tower.holder_file(&self.sb.root)];
        let folder = format!("notes={}", self.notes);
        let out = self.desk.run(&["new", "--wall", WALL, "--holder", &holders[0], "--holder", &holders[1], "--folder", &folder, "--leave-out", "cache"]);
        assert_eq!(out.code, 0, "new: {}", out.stderr);
        out.json()["eidolon"].as_str().unwrap().to_owned()
    }
}

#[test]
fn an_eidolon_goes_desk_to_tower_and_back_sending_only_what_changed() {
    let w = World::new();
    let eid = w.eidolon();

    let sealed = w.desk.run(&["seal", &eid, "--cell", "a"]);
    assert_eq!(sealed.code, 0, "{}", sealed.stderr);
    assert!(sealed.json()["files"].as_u64().unwrap() > 300);
    assert_eq!(w.desk.run(&["verify", &eid]).code, 0);

    let first = w.desk.move_to(&eid, &w.tower);
    assert_eq!(first.code, 0, "{}{}", first.stderr, first.stdout);
    let first = first.json();
    assert_eq!(first["chunks_sent"], first["chunks_offered"], "tower had nothing yet");
    assert_eq!(first["generation"], 2);

    // Desk is done with it.
    let again = w.desk.run(&["open", &eid, "--cell", "a"]);
    assert_eq!((again.code, again.code_of_error().as_str()), (1, "retired"));
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["state"], "retired");

    // Tower opens it in another cell, byte for byte.
    let opened = w.tower.run(&["open", &eid, "--cell", "b"]);
    assert_eq!(opened.code, 0, "{}{}", opened.stderr, opened.stdout);
    assert_eq!(tree(&w.folder("a"), "cache"), tree(&w.folder("b"), "cache"));
    assert!(!w.folder("b").join("cache").exists(), "what the facet leaves out is not carried");

    // One small file changes and the big one grows.
    std::fs::write(w.folder("b").join("d1/sub1/f4.txt"), b"changed").unwrap();
    let mut big = std::fs::read(w.folder("b").join("big.bin")).unwrap();
    big.extend(bytes(99, 50_000));
    std::fs::write(w.folder("b").join("big.bin"), big).unwrap();
    let resealed = w.tower.run(&["seal", &eid]);
    assert_eq!(resealed.code, 0, "{}{}", resealed.stderr, resealed.stdout);

    let second = w.tower.move_to(&eid, &w.desk);
    assert_eq!(second.code, 0, "{}{}", second.stderr, second.stdout);
    let second = second.json();
    println!("first move: {first}\nsecond move: {second}");
    assert_eq!(second["generation"], 3);
    assert!(second["chunks_sent"].as_u64().unwrap() <= 8, "only the changed chunks: {second}");
    assert!(second["bytes_sent"].as_u64().unwrap() * 5 < first["bytes_sent"].as_u64().unwrap(), "{second} against {first}");

    w.empty("a");
    let back = w.desk.run(&["open", &eid, "--cell", "a"]);
    assert_eq!(back.code, 0, "{}{}", back.stderr, back.stdout);
    assert_eq!(tree(&w.folder("a"), "cache"), tree(&w.folder("b"), "cache"));
    assert_eq!(std::fs::read(w.folder("a").join("d1/sub1/f4.txt")).unwrap(), b"changed");
}

#[test]
fn a_machine_off_the_holder_list_cannot_receive() {
    let w = World::new();
    let stranger = machine(&w.sb, "stranger");
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let out = w.desk.move_to(&eid, &stranger);
    assert_eq!(out.code, 1);
    assert_eq!(out.code_of_error(), "not_a_holder", "{}", out.stdout);
    // Nothing was handed over: desk still holds it and can open it.
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["pending_move"], false);
    w.empty("a");
    assert_eq!(w.desk.run(&["open", &eid, "--cell", "a"]).code, 0);
}

#[test]
fn an_eidolon_open_in_one_cell_does_not_open_in_a_second_or_move() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    w.empty("a");
    let first = w.desk.run(&["open", &eid, "--cell", "a"]);
    assert_eq!(first.code, 0, "{}{}", first.stderr, first.stdout);
    let second = w.desk.run(&["open", &eid, "--cell", "b"]);
    assert_eq!((second.code, second.code_of_error().as_str()), (1, "second_seat"));
    let moved = w.desk.move_to(&eid, &w.tower);
    assert_eq!((moved.code, moved.code_of_error().as_str()), (1, "second_seat"), "seal first");
    assert!(moved.json()["fix"].as_str().unwrap().contains("seal"));
    // Tower never held it, and cannot open one it was not given.
    assert_eq!(w.tower.run(&["open", &eid, "--cell", "b"]).code_of_error(), "unknown_eidolon");
}

#[test]
fn a_cell_behind_another_wall_does_not_open_or_seal_it() {
    let w = World::new();
    let eid = w.eidolon();
    let sealed = w.desk.run(&["seal", &eid, "--cell", "c"]);
    assert_eq!((sealed.code, sealed.code_of_error().as_str()), (1, "wrong_wall"));
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    let opened = w.desk.run(&["open", &eid, "--cell", "c"]);
    assert_eq!((opened.code, opened.code_of_error().as_str()), (1, "wrong_wall"));
    assert!(!cells_dir(&w.sb).join("c/home").exists(), "nothing reached the cell");
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["state"], "sealed", "and it is still sealed");
}

#[test]
fn a_changed_chunk_in_the_vault_is_refused_by_verify_and_open() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    assert_eq!(w.desk.run(&["verify", &eid]).code, 0);
    let chunk = std::fs::read_dir(w.desk.home.join("vault/chunks"))
        .unwrap()
        .flat_map(|d| std::fs::read_dir(d.unwrap().path()).unwrap())
        .map(|f| f.unwrap().path())
        .find(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .unwrap();
    let mut data = std::fs::read(&chunk).unwrap();
    let at = data.len() / 2;
    data[at] ^= 1;
    std::fs::write(&chunk, data).unwrap();

    let verify = w.desk.run(&["verify", &eid]);
    assert_eq!((verify.code, verify.code_of_error().as_str()), (1, "changed_chunk"));
    let open = w.desk.run(&["open", &eid, "--cell", "b"]);
    assert_eq!((open.code, open.code_of_error().as_str()), (1, "changed_chunk"));
    assert!(!cells_dir(&w.sb).join("b/home").exists(), "nothing reached the cell");
}

#[test]
fn an_unfinished_move_closes_the_copy_until_the_person_settles_it() {
    let w = World::new();
    let eid = w.eidolon();
    assert_eq!(w.desk.run(&["seal", &eid, "--cell", "a"]).code, 0);
    // A move that was started and never answered, as the sender leaves it.
    let tower_key = w.tower.run(&["machine", "show"]).json()["sign"].as_str().unwrap().to_owned();
    let pending = w.desk.home.join("holdings").join(&eid).join("pending.json");
    std::fs::write(&pending, format!("{{\"to\":\"{tower_key}\",\"generation\":2}}")).unwrap();
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["pending_move"], true);
    let closed = w.desk.run(&["open", &eid, "--cell", "a"]);
    assert_eq!((closed.code, closed.code_of_error().as_str()), (1, "second_seat"));
    // A seat that is not the one handed over does not settle it.
    let seat = w.desk.home.join("holdings").join(&eid).join("seat.json");
    let wrong = w.desk.run(&["move-settle", &eid, "--confirm", &seat.display().to_string()]);
    assert_eq!((wrong.code, wrong.code_of_error().as_str()), (1, "stale_generation"));
    // The person says it never arrived: the copy opens again.
    assert_eq!(w.desk.run(&["move-settle", &eid, "--abandon"]).code, 0);
    assert_eq!(w.desk.run(&["move-settle", &eid, "--abandon"]).code, 1, "nothing left to settle");
    w.empty("a");
    assert_eq!(w.desk.run(&["open", &eid, "--cell", "a"]).code, 0);
}

#[test]
fn mistakes_in_the_command_are_exit_2_and_a_machine_without_an_identity_says_so() {
    let sb = Sandbox::new();
    let lone = Machine { home: sb.home("lone"), cells: Some(cells_dir(&sb)), name: "lone".into(), env: Vec::new() };
    let bad = lone.run(&["frobnicate"]);
    assert_eq!(bad.code, 2);
    assert_eq!(bad.code_of_error(), "usage");
    let none = lone.run(&["ls"]);
    assert_eq!(none.code, 0);
    let not_set_up = lone.run(&["new", "--wall", WALL, "--holder", "x"]);
    assert_eq!((not_set_up.code, not_set_up.code_of_error().as_str()), (1, "not_initialised"));
    let init = lone.run(&["machine", "init", "--name", "lone", "--key-store", "file"]);
    assert!(init.json()["warning"].as_str().unwrap().contains("weaker"));
    assert_eq!(lone.run(&["machine", "init", "--name", "again"]).code_of_error(), "already_initialised");
    assert_eq!(lone.run(&["machine", "init", "--name", "x", "--key-store", "os"]).code_of_error(), "no_os_key_store");
    // Plain text errors read `eidolon: <what> <fix>` on standard error.
    let out = std::process::Command::new(common::BIN).env_clear().env("EIDOLON_HOME", &lone.home).arg("verify").arg("eid_aeaqcaibaeaqcaibaeaqcaibae").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("eidolon: This machine holds no eidolon"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout.len(), 0);
}

/// A new eidolon on desk with a folder facet and a browser facet.
fn browser_eidolon(w: &World) -> String {
    let holders = [w.desk.holder_file(&w.sb.root), w.tower.holder_file(&w.sb.root)];
    let out = w.desk.run(&[
        "new",
        "--wall",
        WALL,
        "--holder",
        &holders[0],
        "--holder",
        &holders[1],
        "--folder",
        "notes=/home/agent/notes",
        "--browser",
        "web",
        "--leave-out",
        "extra/**",
    ]);
    assert_eq!(out.code, 0, "new: {}", out.stderr);
    out.json()["eidolon"].as_str().unwrap().to_owned()
}

#[test]
fn a_browser_facet_is_made_at_the_profile_path_with_its_own_leave_outs() {
    let w = World::new();
    let eid = browser_eidolon(&w);
    let facets = w.desk.run(&["facets", &eid]).json();
    let facets = facets["facets"].as_array().unwrap();
    assert_eq!(facets.len(), 2);
    assert_eq!((facets[0]["kind"].as_str(), facets[0]["path"].as_str()), (Some("folder"), Some("/home/agent/notes")));
    assert_eq!((facets[1]["kind"].as_str(), facets[1]["name"].as_str(), facets[1]["path"].as_str()), (Some("browser"), Some("web"), Some("/home/eidolon/profile")));
    assert_eq!(facets[1]["leave_out"], serde_json::json!(["extra/**"]));
    // A leave-out that reaches outside the profile is refused, as for a folder.
    let holders = w.desk.holder_file(&w.sb.root);
    let outside = w.desk.run(&["new", "--wall", WALL, "--holder", &holders, "--browser", "web", "--leave-out", "../escape"]);
    assert_eq!(outside.code, 1, "{}", outside.stdout);
}

#[test]
fn a_folder_cell_has_no_browser_so_opening_a_browser_eidolon_there_is_refused_before_anything_is_recorded() {
    let w = World::new();
    let eid = browser_eidolon(&w);
    let opened = w.desk.run(&["open", &eid, "--cell", "a"]);
    assert_eq!((opened.code, opened.code_of_error().as_str()), (1, "no_driver"), "{}", opened.stdout);
    assert_eq!(w.desk.run(&["ls"]).json()["eidolons"][0]["state"], "sealed");
    assert!(!w.folder("a").exists(), "nothing was placed");
}

/// The error code of a failed command. `mcp` owns standard output, so its
/// errors are on standard error.
fn failed_with(command: &str, out: &common::Out) -> String {
    let text = if command == "mcp" { &out.stderr } else { &out.stdout };
    serde_json::from_str::<serde_json::Value>(text.trim()).unwrap()["code"].as_str().unwrap_or_default().to_owned()
}

/// Pretends the person's window is open: the marker holds a live pid, as
/// `eidolon view` writes it, and the holding says open in cell `a`.
fn window_is_open(w: &World, eid: &str) {
    let dir = w.desk.home.join("holdings").join(eid);
    std::fs::write(dir.join("state.json"), r#"{"open":{"cell":"a"}}"#).unwrap();
    std::fs::write(dir.join("view.open"), format!("{}\n", std::process::id())).unwrap();
}

#[test]
fn the_tools_and_a_second_window_are_refused_while_the_sign_in_window_is_open() {
    let w = World::new();
    let eid = browser_eidolon(&w);
    window_is_open(&w, &eid);
    for command in ["mcp", "view"] {
        let refused = w.desk.run(&[command, &eid]);
        assert_eq!((refused.code, failed_with(command, &refused).as_str()), (1, "view_open"), "{command}: {}{}", refused.stdout, refused.stderr);
    }
    // A marker whose process is gone counts as closed: the next refusal is
    // about the folder cell, not the window.
    let marker = w.desk.home.join("holdings").join(&eid).join("view.open");
    std::fs::write(&marker, "2147483646\n").unwrap();
    let after = w.desk.run(&["mcp", &eid]);
    assert_eq!(failed_with("mcp", &after), "no_driver");
    assert!(!marker.exists(), "a stale marker is removed");
}

#[test]
fn view_and_mcp_need_an_open_eidolon_with_a_browser() {
    let w = World::new();
    let browserless = w.eidolon();
    let sealed = browser_eidolon(&w);
    for (eid, command, want) in [(&sealed, "view", "not_open"), (&sealed, "mcp", "not_open"), (&browserless, "view", "not_open")] {
        assert_eq!(failed_with(command, &w.desk.run(&[command, eid])), want, "{command}");
    }
    assert_eq!(w.desk.run(&["seal", &browserless, "--cell", "a"]).code, 0);
    w.empty("a");
    assert_eq!(w.desk.run(&["open", &browserless, "--cell", "a"]).code, 0);
    for command in ["view", "mcp"] {
        assert_eq!(failed_with(command, &w.desk.run(&[command, &browserless])), "no_browser", "{command}");
    }
}

#[test]
fn mcp_keeps_standard_output_for_the_protocol_even_when_it_fails() {
    let w = World::new();
    let eid = browser_eidolon(&w);
    let out = std::process::Command::new(common::BIN)
        .env_clear()
        .env("EIDOLON_HOME", &w.desk.home)
        .args(["--json", "--cell-root"])
        .arg(cells_dir(&w.sb))
        .args(["mcp", &eid])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout.len(), 0, "nothing but protocol may reach standard output");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not_open"));
    let http = w.desk.run(&["mcp", &eid, "--http", "127.0.0.1:0"]);
    assert_eq!((http.stdout.len(), failed_with("mcp", &http).as_str()), (0, "no_http"));
}
