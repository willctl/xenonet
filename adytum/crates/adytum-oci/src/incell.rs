//! A `Runner` that runs each command inside a cell, so `Oci` can drive
//! podman in a VM instead of on the host.

use std::sync::Arc;

use adytum_driver::{Driver, ExecOpts};
use adytum_exec::{Cancel, Chunk, Command, Duplex, ExecError, Output, Runner};
use adytum_model::CellName;
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Runs commands in `cell` through `driver`'s `exec` and `stream`.
///
/// The command's program and arguments reach the guest as separate
/// arguments. Its variables go in as `env` arguments, so keep secrets out of
/// them, except `PATH`, `HOME` and `XDG_RUNTIME_DIR`, which the guest
/// has of its own. A working folder is not supported (the guest's login folder is
/// used), and `stream` delivers the output when the command ends, not as it
/// arrives. A cancel stops the wait, and the guest command ends when its
/// connection closes.
pub struct InCell {
    driver: Arc<dyn Driver>,
    cell: CellName,
}

impl InCell {
    /// Commands run in `cell`, which `driver` must already hold and have
    /// started.
    pub fn new(driver: Arc<dyn Driver>, cell: CellName) -> InCell {
        InCell { driver, cell }
    }

    /// The command's variables, except the three drivers copy from the host
    /// (`PATH`, `HOME`, `XDG_RUNTIME_DIR`): the guest has its own.
    fn vars(cmd: &Command) -> impl Iterator<Item = (String, String)> + '_ {
        cmd.env_vars()
            .iter()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
            .filter(|(k, _)| !matches!(k.as_str(), "PATH" | "HOME" | "XDG_RUNTIME_DIR"))
    }

    fn argv(cmd: &Command) -> Vec<String> {
        cmd.argv()
    }

    fn refuse_cwd(cmd: &Command) -> Result<(), ExecError> {
        match cmd.working_dir() {
            None => Ok(()),
            Some(_) => Err(ExecError::Unexpected { message: "A command run in a cell cannot name a working folder. Run it without one.".to_owned() }),
        }
    }
}

fn failed(e: &adytum_driver::DriverError) -> ExecError {
    ExecError::Unexpected { message: e.to_string() }
}

#[async_trait]
impl Runner for InCell {
    async fn run(&self, cmd: &Command, cancel: &Cancel) -> Result<Output, ExecError> {
        Self::refuse_cwd(cmd)?;
        let argv = Self::argv(cmd);
        let env = Self::vars(cmd).collect();
        let opts = ExecOpts { env, stdin: cmd.stdin().map(<[u8]>::to_vec), deadline: cmd.deadline_limit() };
        tokio::select! {
            out = self.driver.exec(&self.cell, &argv, opts) => out.map_err(|e| failed(&e)),
            () = cancel.cancelled() => Ok(Output { cancelled: true, ..Output::default() }),
        }
    }

    async fn stream(&self, cmd: &Command, cancel: &Cancel, sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        let out = self.run(cmd, cancel).await?;
        if !out.stdout.is_empty() {
            let _ = sink.send(Chunk::Stdout(out.stdout.clone())).await;
        }
        if !out.stderr.is_empty() {
            let _ = sink.send(Chunk::Stderr(out.stderr.clone())).await;
        }
        Ok(Output { stdout: Vec::new(), stderr: Vec::new(), ..out })
    }

    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError> {
        Self::refuse_cwd(cmd)?;
        let mut argv = Vec::new();
        let vars: Vec<_> = Self::vars(cmd).collect();
        if !vars.is_empty() {
            argv.push("env".to_owned());
            argv.extend(vars.iter().map(|(k, v)| format!("{k}={v}")));
        }
        argv.extend(Self::argv(cmd));
        self.driver.stream(&self.cell, &argv).await.map_err(|e| failed(&e))
    }
}
