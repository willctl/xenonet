//! `InCell` hands a command to a driver's `exec` and `stream`.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Cancel, Command, Duplex, Output, Runner};
use adytum_model::{CellName, CellSpec};
use adytum_oci::InCell;
use async_trait::async_trait;

type Seen = Vec<(Vec<String>, ExecOpts)>;

#[derive(Default)]
struct Spy {
    execs: Mutex<Seen>,
    streams: Mutex<Vec<Vec<String>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[async_trait]
impl Driver for Spy {
    async fn create(&self, _spec: &CellSpec) -> Result<(), DriverError> {
        Ok(())
    }
    async fn start(&self, _name: &CellName) -> Result<(), DriverError> {
        Ok(())
    }
    async fn exec(&self, _name: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
        lock(&self.execs).push((argv.to_vec(), opts));
        Ok(Output { status: Some(0), stdout: b"out".to_vec(), ..Output::default() })
    }
    async fn stream(&self, _name: &CellName, argv: &[String]) -> Result<Duplex, DriverError> {
        lock(&self.streams).push(argv.to_vec());
        Err(DriverError::Unsupported { what: "a stream in this spy".to_owned() })
    }
    async fn copy_in(&self, _name: &CellName, _host: &Path, _guest: &str) -> Result<(), DriverError> {
        Ok(())
    }
    async fn copy_out(&self, _name: &CellName, _guest: &str, _host: &Path) -> Result<(), DriverError> {
        Ok(())
    }
    async fn stop(&self, _name: &CellName) -> Result<(), DriverError> {
        Ok(())
    }
    async fn remove(&self, _name: &CellName) -> Result<(), DriverError> {
        Ok(())
    }
    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        Ok(Vec::new())
    }
}

fn in_cell() -> (InCell, Arc<Spy>) {
    let spy = Arc::new(Spy::default());
    (InCell::new(spy.clone(), "adytum-test-x".parse().expect("a name")), spy)
}

#[tokio::test]
async fn a_command_runs_as_separate_arguments_with_its_stdin_deadline_and_own_variables() {
    let (cell, spy) = in_cell();
    let cmd = Command::new("podman")
        .args(["exec", "a b", "$(x)"])
        .env("LANG", "C")
        .env("PATH", "/Users/desk/bin")
        .env("HOME", "/Users/desk")
        .env("XDG_RUNTIME_DIR", "/tmp/run")
        .stdin_bytes(b"in".to_vec())
        .deadline(std::time::Duration::from_secs(5));
    let out = cell.run(&cmd, &Cancel::new()).await.unwrap();
    assert_eq!(out.stdout, b"out");
    let seen = lock(&spy.execs);
    let (argv, opts) = &seen[0];
    assert_eq!(argv, &["podman", "exec", "a b", "$(x)"]);
    // The host's own PATH, HOME and runtime folder mean nothing in the guest.
    assert_eq!(opts.env, [("LANG".to_owned(), "C".to_owned())]);
    assert_eq!(opts.stdin.as_deref(), Some(&b"in"[..]));
    assert_eq!(opts.deadline, Some(std::time::Duration::from_secs(5)));
}

#[tokio::test]
async fn a_working_folder_is_refused_and_a_duplex_carries_the_variables_as_env() {
    let (cell, spy) = in_cell();
    assert!(cell.run(&Command::new("podman").cwd("/tmp"), &Cancel::new()).await.is_err());
    assert!(lock(&spy.execs).is_empty());

    let _ = cell.duplex(&Command::new("podman").arg("exec").env("LANG", "C").env("HOME", "/h")).await;
    assert_eq!(lock(&spy.streams)[0], ["env", "LANG=C", "podman", "exec"]);
}

#[tokio::test]
async fn a_cancel_ends_the_wait() {
    struct Slow;
    #[async_trait]
    impl Driver for Slow {
        async fn create(&self, _: &CellSpec) -> Result<(), DriverError> {
            Ok(())
        }
        async fn start(&self, _: &CellName) -> Result<(), DriverError> {
            Ok(())
        }
        async fn exec(&self, _: &CellName, _: &[String], _: ExecOpts) -> Result<Output, DriverError> {
            std::future::pending().await
        }
        async fn stream(&self, _: &CellName, _: &[String]) -> Result<Duplex, DriverError> {
            Err(DriverError::Unsupported { what: "x".to_owned() })
        }
        async fn copy_in(&self, _: &CellName, _: &Path, _: &str) -> Result<(), DriverError> {
            Ok(())
        }
        async fn copy_out(&self, _: &CellName, _: &str, _: &Path) -> Result<(), DriverError> {
            Ok(())
        }
        async fn stop(&self, _: &CellName) -> Result<(), DriverError> {
            Ok(())
        }
        async fn remove(&self, _: &CellName) -> Result<(), DriverError> {
            Ok(())
        }
        async fn list(&self) -> Result<Vec<Listed>, DriverError> {
            Ok(Vec::new())
        }
    }
    let cell = InCell::new(Arc::new(Slow), "adytum-test-x".parse().expect("a name"));
    let cancel = Cancel::new();
    cancel.cancel();
    let out = cell.run(&Command::new("sleep"), &cancel).await.unwrap();
    assert!(out.cancelled && !out.success());
}
