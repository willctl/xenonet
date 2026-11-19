//! The built binary, with nothing but its own arguments: no VM is touched.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static RUNS: AtomicUsize = AtomicUsize::new(0);

fn adytum(args: &[&str]) -> (i32, String, String, PathBuf) {
    let home = PathBuf::from("/tmp").join(format!("adl-cli-{}-{}", std::process::id(), RUNS.fetch_add(1, Ordering::Relaxed)));
    let empty_path = home.join("no-programs-here");
    let out = Command::new(env!("CARGO_BIN_EXE_adytum"))
        .args(args)
        .env_clear()
        .env("ADYTUM_HOME", &home)
        .env("HOME", &home)
        .env("PATH", &empty_path)
        .output()
        .expect("the binary starts");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned(), home)
}

#[test]
fn a_name_outside_ours_is_refused_with_no_limactl_on_the_path() {
    // With no limactl to find, getting `name_not_ours` back proves the
    // refusal came before any attempt to run it.
    for name in ["pane", "xenonet-vm1", "xenonet-base-fedora"] {
        let (code, out, err, home) = adytum(&["rm", name, "--json"]);
        assert_eq!(code, 1, "{name}");
        let value: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON object");
        assert_eq!(value["error"]["code"], "name_not_ours");
        assert_eq!(err, "");
        assert!(!home.exists(), "nothing was made for a refused name");
    }
}

#[test]
fn ls_with_no_cells_needs_no_limactl() {
    let (code, out, _, _) = adytum(&["ls", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"cells":[]}"#);
}

#[test]
fn starting_a_missing_cell_is_not_found_and_needs_no_limactl() {
    let (code, _, err, home) = adytum(&["start", "adytum-test-nonesuch"]);
    // start lists first; with no cells it never needs limactl and says the cell is missing.
    assert_eq!(code, 1);
    assert!(err.starts_with("adytum: There is no cell named adytum-test-nonesuch."), "{err}");
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn usage_errors_exit_2() {
    let (code, _, err, _) = adytum(&["frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.starts_with("adytum: "));
    let (code, out, _, _) = adytum(&["--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("Exit codes"));
}
