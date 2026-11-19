//! Lima driver behaviour against fakes: what is refused before `limactl`
//! runs, what it is asked, and what its failures say.

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use adytum_driver::{cell_name, Driver, DriverError, ExecOpts};
use adytum_exec::{Cancel, Chunk, Command, Duplex, ExecError, Output, Runner};
use adytum_lima::Lima;
use adytum_model::{CellState, RefusalCode};
use adytum_testkit::{FakeRunner, Transcript};
use async_trait::async_trait;
use tokio::sync::mpsc;

fn transcript(calls: &[(&[&str], &str, &str, i32)]) -> Transcript {
    let calls = calls
        .iter()
        .map(|(argv, stdout, stderr, status)| adytum_testkit::Call {
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            stdin: None,
            stdout: (*stdout).to_owned(),
            stderr: (*stderr).to_owned(),
            status: *status,
        })
        .collect();
    Transcript { tool: "limactl".into(), version: "2.1.2".into(), calls }
}

// A short path: Lima keeps unix sockets under the folder and they hold about 100 bytes.
/// A scratch folder, removed when the test ends.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(tag: &str) -> Dir {
    let dir = PathBuf::from("/tmp").join(format!("adl-f-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    Dir(dir)
}

fn lima_with(calls: &[(&[&str], &str, &str, i32)], tag: &str) -> (Lima, Arc<FakeRunner>, Dir) {
    let home = scratch(tag);
    let fake = Arc::new(FakeRunner::from_transcript(transcript(calls)).with_var("lima_home", &home.0));
    (Lima::new(fake.clone(), "limactl".into(), home.0.clone()), fake, home)
}

fn list_line(name: &str, status: &str) -> String {
    format!("{{\"name\":\"{name}\",\"status\":\"{status}\"}}\n")
}

#[tokio::test]
async fn a_name_outside_ours_is_refused_before_any_limactl_call() {
    for bad in ["pane", "xenonet-vm1", "xenonet-vm2", "xenonet-base-fedora", "xenonet-base-ubuntu", "default"] {
        let Err(DriverError::Refused(r)) = cell_name(bad) else { panic!("{bad} was not refused") };
        assert_eq!(r.code, RefusalCode::NameNotOurs);
    }
    // The driver takes names that passed, so the fake sees nothing at all.
    let (_lima, fake, _home) = lima_with(&[], "refuse");
    fake.assert_done();
}

#[tokio::test]
async fn list_leaves_out_instances_that_are_not_ours() {
    let out = format!("{}{}{}{}", list_line("pane", "Running"), list_line("adytum-test-a", "Running"), list_line("xenonet-vm1", "Stopped"), list_line("adytum-test-b", "Stopped"));
    let (lima, fake, _home) = lima_with(&[(&["limactl", "list", "--json"], &out, "", 0)], "list");
    let cells = lima.list().await.unwrap();
    let names: Vec<_> = cells.iter().map(|c| c.name.to_string()).collect();
    assert_eq!(names, ["adytum-test-a", "adytum-test-b"]);
    assert_eq!(cells[0].state, CellState::Running);
    assert_eq!(cells[1].state, CellState::Stopped);
    fake.assert_done();
}

#[tokio::test]
async fn a_failing_limactl_surfaces_its_stderr() {
    let (lima, fake, home) = lima_with(
        &[(&["limactl", "create", "--name=adytum-test-golden", "--tty=false", "{lima_home}/_templates/adytum-test-golden.yaml"], "", "level=fatal msg=\"disk is full\"\n", 1)],
        "fail",
    );
    let err = lima.create(&common::spec()).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("disk is full"), "{text}");
    assert!(text.contains("limactl create exited with status 1"), "{text}");
    assert!(matches!(err, DriverError::Tool { status: Some(1), .. }));
    // The template is removed even when limactl failed.
    assert!(!home.0.join("_templates/adytum-test-golden.yaml").exists());
    fake.assert_done();
}

#[tokio::test]
async fn starting_a_cell_that_does_not_exist_never_runs_limactl_start() {
    // limactl start would make a default VM of an unknown name.
    let (lima, fake, _home) = lima_with(&[(&["limactl", "list", "--json"], "", "", 0)], "nostart");
    let err = lima.start(&common::name()).await.unwrap_err();
    assert!(matches!(err, DriverError::NotFound { .. }), "{err:?}");
    fake.assert_done();
}

#[tokio::test]
async fn stopping_a_cell_that_does_not_exist_is_not_found() {
    let (lima, fake, _home) = lima_with(&[(&["limactl", "list", "--json"], "", "", 0)], "nostop");
    assert!(matches!(lima.stop(&common::name()).await, Err(DriverError::NotFound { .. })));
    fake.assert_done();
}

#[tokio::test]
async fn exec_hands_each_argument_over_whole() {
    let (lima, fake, _home) = lima_with(&[(&["limactl", "shell", "--workdir=/", "adytum-test-golden", "--", "printf", "%s\n", "$(id)", "a b", ""], "ok", "", 0)], "args");
    let argv: Vec<String> = ["printf", "%s\n", "$(id)", "a b", ""].iter().map(|s| (*s).to_owned()).collect();
    let out = lima.exec(&common::name(), &argv, ExecOpts::default()).await.unwrap();
    assert_eq!(out.stdout_lossy(), "ok");
    fake.assert_done();
}

#[tokio::test]
async fn exec_variables_go_through_env_and_a_bad_name_is_refused() {
    let (lima, fake, _home) = lima_with(&[(&["limactl", "shell", "--workdir=/", "adytum-test-golden", "--", "env", "A=1 2", "sh", "-c", "true"], "", "", 0)], "env");
    let argv: Vec<String> = ["sh", "-c", "true"].iter().map(|s| (*s).to_owned()).collect();
    let opts = ExecOpts { env: vec![("A".into(), "1 2".into())], ..ExecOpts::default() };
    lima.exec(&common::name(), &argv, opts).await.unwrap();
    let bad = ExecOpts { env: vec![("A B".into(), "x".into())], ..ExecOpts::default() };
    assert!(matches!(lima.exec(&common::name(), &argv, bad).await, Err(DriverError::Unsupported { .. })));
    let eq = ExecOpts { env: vec![("A".into(), "x".into())], ..ExecOpts::default() };
    assert!(matches!(lima.exec(&common::name(), &["B=1".to_owned()], eq).await, Err(DriverError::Unsupported { .. })));
    assert!(matches!(lima.exec(&common::name(), &[], ExecOpts::default()).await, Err(DriverError::Unsupported { .. })));
    fake.assert_done();
}

#[tokio::test]
async fn a_missing_cell_is_not_found_from_exec_and_copy() {
    let msg = "time=\"x\" level=fatal msg=\"instance `adytum-test-golden` does not exist, run `limactl create adytum-test-golden` to create a new instance\"\n";
    let (lima, fake, _home) = lima_with(
        &[
            (&["limactl", "shell", "--workdir=/", "adytum-test-golden", "--", "true"], "", msg, 1),
            (&["limactl", "copy", "--tty=false", "/etc/hostname", "adytum-test-golden:/tmp/x"], "", msg, 1),
        ],
        "missing",
    );
    let name = common::name();
    assert!(matches!(lima.exec(&name, &["true".to_owned()], ExecOpts::default()).await, Err(DriverError::NotFound { .. })));
    assert!(matches!(lima.copy_in(&name, Path::new("/etc/hostname"), "/tmp/x").await, Err(DriverError::NotFound { .. })));
    fake.assert_done();
}

#[tokio::test]
async fn copy_paths_are_checked_before_any_call() {
    let (lima, fake, _home) = lima_with(&[], "paths");
    let name = common::name();
    for guest in ["tmp/x", "/tmp/../etc/passwd", "/a:b", ""] {
        assert!(matches!(lima.copy_in(&name, Path::new("/etc/hostname"), guest).await, Err(DriverError::BadPath { .. })), "{guest}");
        assert!(matches!(lima.copy_out(&name, guest, Path::new("/tmp/out")).await, Err(DriverError::BadPath { .. })), "{guest}");
    }
    for host in ["relative.txt", "/tmp/a:b"] {
        assert!(matches!(lima.copy_in(&name, Path::new(host), "/tmp/x").await, Err(DriverError::BadPath { .. })), "{host}");
    }
    fake.assert_done();
}

#[tokio::test]
async fn a_sealed_spec_with_a_mount_never_reaches_limactl() {
    let (lima, fake, _home) = lima_with(&[], "sealed");
    let mut spec = common::spec();
    spec.mounts.push(adytum_model::Mount { source: "/Users".into(), target: "/mnt".into(), writable: true });
    let Err(DriverError::Refused(r)) = lima.create(&spec).await else { panic!("not refused") };
    assert_eq!(r.code, RefusalCode::SealedCellMount);
    fake.assert_done();
}

#[tokio::test]
async fn a_home_that_is_too_long_for_lima_sockets_is_refused() {
    let long = PathBuf::from(format!("/{}", "a".repeat(90)));
    let fake = Arc::new(FakeRunner::from_transcript(transcript(&[])));
    let lima = Lima::new(fake.clone(), "limactl".into(), long);
    assert!(matches!(lima.create(&common::spec()).await, Err(DriverError::BadPath { .. })));
    let relative = Lima::new(fake.clone(), "limactl".into(), "relative/lima".into());
    assert!(matches!(relative.list().await, Err(DriverError::BadPath { .. })));
    fake.assert_done();
}

/// Records each command so a test can look at what the child would get.
#[derive(Default)]
struct Spy(Mutex<Vec<Command>>);

#[async_trait]
impl Runner for Spy {
    async fn run(&self, cmd: &Command, _cancel: &Cancel) -> Result<Output, ExecError> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(cmd.clone());
        let stdout = if cmd.arg_list().first().is_some_and(|a| a == "list") { list_line("adytum-test-golden", "Stopped").into_bytes() } else { Vec::new() };
        Ok(Output { status: Some(0), stdout, ..Output::default() })
    }

    async fn stream(&self, cmd: &Command, cancel: &Cancel, _sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        self.run(cmd, cancel).await
    }

    async fn duplex(&self, _cmd: &Command) -> Result<Duplex, ExecError> {
        Err(ExecError::Unexpected { message: "no duplex".into() })
    }
}

#[tokio::test]
async fn limactl_gets_only_lima_home_path_and_home() {
    let spy = Arc::new(Spy::default());
    let home = scratch("env");
    let lima = Lima::new(spy.clone(), "limactl".into(), home.0.clone());
    lima.start(&common::name()).await.unwrap();
    lima.stop(&common::name()).await.unwrap();
    lima.remove(&common::name()).await.unwrap();
    let calls = spy.0.lock().unwrap().clone();
    assert!(calls.len() >= 5);
    for cmd in calls {
        let keys: Vec<String> = cmd.env_vars().keys().map(|k| k.to_string_lossy().into_owned()).collect();
        assert!(keys.iter().all(|k| ["LIMA_HOME", "PATH", "HOME"].contains(&k.as_str())), "{keys:?}");
        assert!(keys.contains(&"LIMA_HOME".to_owned()));
        assert_eq!(cmd.env_vars().get(std::ffi::OsStr::new("LIMA_HOME")).map(|v| v.to_string_lossy().into_owned()), Some(home.0.to_string_lossy().into_owned()));
    }
}
