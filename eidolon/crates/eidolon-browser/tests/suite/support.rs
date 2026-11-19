//! A fake driver and a recording cell.

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Duplex, DuplexControl, ExecError, Output};
use adytum_model::{CellName, CellSpec};
use async_trait::async_trait;
use eidolon_facet::{CellAccess, LocalCell, RunOutput};
use eidolon_testkit::FakeBidi;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub fn cell() -> CellName {
    "adytum-test-browser".parse().unwrap()
}

/// One command the fake driver ran.
#[derive(Debug, Clone)]
pub struct Exec {
    pub argv: Vec<String>,
    pub stdin: Option<Vec<u8>>,
}

/// What the next call of `stream` does.
pub enum Next {
    /// A connection served by this fake.
    Serve(FakeBidi),
    /// The tool ran and failed, as `socat` does when nothing listens.
    Tool,
    /// A stream that ends at once.
    Closed,
    /// The driver refuses.
    Unsupported,
}

type Answer = Box<dyn Fn(&[String]) -> Output + Send + Sync>;

pub struct FakeDriver {
    pub execs: Mutex<Vec<Exec>>,
    pub streams: Mutex<Vec<Vec<String>>>,
    next: Mutex<VecDeque<Next>>,
    answer: Answer,
}

impl FakeDriver {
    pub fn new(next: Vec<Next>) -> Arc<FakeDriver> {
        Arc::new(FakeDriver { execs: Mutex::default(), streams: Mutex::default(), next: Mutex::new(next.into()), answer: Box::new(|_| ok(b"")) })
    }

    pub fn answering(answer: impl Fn(&[String]) -> Output + Send + Sync + 'static) -> Arc<FakeDriver> {
        Arc::new(FakeDriver { execs: Mutex::default(), streams: Mutex::default(), next: Mutex::default(), answer: Box::new(answer) })
    }

    pub fn execs(&self) -> Vec<Exec> {
        self.execs.lock().unwrap().clone()
    }

    pub fn stream_count(&self) -> usize {
        self.streams.lock().unwrap().len()
    }
}

pub fn ok(stdout: &[u8]) -> Output {
    Output { status: Some(0), stdout: stdout.to_vec(), ..Output::default() }
}

pub fn failed(status: i32, stderr: &str) -> Output {
    Output { status: Some(status), stderr: stderr.as_bytes().to_vec(), ..Output::default() }
}

struct NoControl;

#[async_trait]
impl DuplexControl for NoControl {
    async fn finish(self: Box<Self>) -> Result<Output, ExecError> {
        Ok(Output::default())
    }
    fn kill(&mut self) {}
}

fn duplex(server: Option<FakeBidi>) -> Duplex {
    let (client, other) = tokio::io::duplex(1 << 16);
    match server {
        Some(fake) => {
            tokio::spawn(fake.serve(other));
        }
        None => drop(other),
    }
    let (read, write) = tokio::io::split(client);
    Duplex::from_parts(Box::new(read), Box::new(write), Box::new(NoControl))
}

fn unsupported(what: &str) -> DriverError {
    DriverError::Unsupported { what: what.to_owned() }
}

#[async_trait]
impl Driver for FakeDriver {
    async fn create(&self, _spec: &CellSpec) -> Result<(), DriverError> {
        Err(unsupported("create"))
    }
    async fn start(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("start"))
    }
    async fn exec(&self, _name: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
        self.execs.lock().unwrap().push(Exec { argv: argv.to_vec(), stdin: opts.stdin });
        Ok((self.answer)(argv))
    }
    async fn stream(&self, _name: &CellName, argv: &[String]) -> Result<Duplex, DriverError> {
        self.streams.lock().unwrap().push(argv.to_vec());
        match self.next.lock().unwrap().pop_front() {
            Some(Next::Serve(fake)) => Ok(duplex(Some(fake))),
            Some(Next::Closed) => Ok(duplex(None)),
            Some(Next::Tool) => Err(DriverError::Tool { what: "limactl shell".into(), status: Some(1), stderr: "connection refused".into() }),
            Some(Next::Unsupported) | None => Err(unsupported("stream")),
        }
    }
    async fn copy_in(&self, _name: &CellName, _host: &Path, _guest: &str) -> Result<(), DriverError> {
        Err(unsupported("copy_in"))
    }
    async fn copy_out(&self, _name: &CellName, _guest: &str, _host: &Path) -> Result<(), DriverError> {
        Err(unsupported("copy_out"))
    }
    async fn stop(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("stop"))
    }
    async fn remove(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("remove"))
    }
    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        Ok(Vec::new())
    }
}

/// A host folder standing in for a cell, which records the commands run in
/// it and answers `pgrep` from a script.
pub struct RecordingCell {
    inner: LocalCell,
    pub runs: Mutex<Vec<Vec<String>>>,
    pgrep: Mutex<VecDeque<i32>>,
    fail_on: Option<&'static str>,
}

impl RecordingCell {
    pub fn new(root: PathBuf, pgrep: &[i32]) -> RecordingCell {
        RecordingCell { inner: LocalCell::new(root), runs: Mutex::default(), pgrep: Mutex::new(pgrep.iter().copied().collect()), fail_on: None }
    }

    /// Makes every command whose program is `program` (after `sudo`) fail.
    pub fn failing(mut self, program: &'static str) -> RecordingCell {
        self.fail_on = Some(program);
        self
    }

    pub fn commands(&self) -> Vec<String> {
        self.runs.lock().unwrap().iter().map(|a| a.join(" ")).collect()
    }
}

impl RecordingCell {
    /// Does what the two commands that rearrange the profile do, on the
    /// host folder, so a test can look at the result.
    fn act(&self, argv: &[&str]) {
        if self.fail_on.is_some_and(|p| argv.contains(&p)) {
            return;
        }
        match argv {
            ["sudo", "rm", "-rf", "--", path] => {
                if let Ok(host) = self.inner.host_path(path) {
                    let _ = std::fs::remove_dir_all(host);
                }
            }
            ["sudo", "mv", "--", from, to] => {
                if let (Ok(from), Ok(to)) = (self.inner.host_path(from), self.inner.host_path(to)) {
                    let _ = std::fs::rename(from, to);
                }
            }
            _ => {}
        }
    }
}

#[async_trait]
impl CellAccess for RecordingCell {
    async fn copy_out(&self, guest_path: &str, host_dir: &Path) -> eidolon_facet::Result<()> {
        self.inner.copy_out(guest_path, host_dir).await
    }
    async fn copy_in(&self, host_dir: &Path, guest_path: &str) -> eidolon_facet::Result<()> {
        self.inner.copy_in(host_dir, guest_path).await
    }
    async fn run(&self, argv: &[&str]) -> eidolon_facet::Result<RunOutput> {
        self.act(argv);
        self.runs.lock().unwrap().push(argv.iter().map(|a| (*a).to_owned()).collect());
        let status = if argv.first() == Some(&"pgrep") {
            // An empty script means nothing is left running.
            self.pgrep.lock().unwrap().pop_front().unwrap_or(1)
        } else {
            i32::from(self.fail_on.is_some_and(|p| argv.contains(&p)))
        };
        // The cell's login user, for `id -un`.
        let stdout = if argv == ["id", "-un"] { "builder\n".to_owned() } else { String::new() };
        Ok(RunOutput { status, stdout, stderr: if status == 0 { String::new() } else { "no".into() } })
    }
}
