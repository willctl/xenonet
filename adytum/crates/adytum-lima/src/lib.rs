//! Lima cells: create, start, run, copy, stop and remove a VM from a pinned
//! image, by running `limactl` through `adytum-exec`.
//!
//! Adytum runs `limactl` with a `LIMA_HOME` of its own (`$ADYTUM_HOME/lima`),
//! so the VMs a person keeps in Lima's default home are not listed, and
//! cannot be reached by a name typo. Every call also checks the cell name
//! with `adytum_policy::name_is_ours` first.
//!
//! What `limactl` does that a driver must not trust, found against Lima
//! 2.1.2:
//!
//! - `limactl start <name>` for a name that does not exist creates a default
//!   VM (home folder mounted, another image) and downloads it. `start` and
//!   `stop` therefore list first and refuse a missing cell.
//! - `limactl shell <name> -- <argv>` passes each argument to the guest
//!   literally, so no quoting is done here; a live test pins that.
//! - Lima forwards every port a guest listens on, unless an `ignore` rule
//!   matches it. The template (see [`template`]) ends with those rules.

mod guest;
mod pins;
pub mod template;

pub use guest::check_guest_path;
pub use pins::{image, image_names, DEFAULT_IMAGE};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Cancel, Command, Duplex, Output, Runner, SystemRunner};
use adytum_model::{CellName, CellSpec, CellState};
use async_trait::async_trait;

/// Unix socket paths hold about 104 bytes on macOS; Lima keeps sockets
/// inside each instance's folder, the longest being `default_ep.sock`.
const SOCKET_BUDGET: usize = 103;
const LONGEST_SOCKET: &str = "default_ep.sock";

const CREATE_DEADLINE: Duration = Duration::from_secs(30 * 60);
const START_DEADLINE: Duration = Duration::from_secs(10 * 60);
const STOP_DEADLINE: Duration = Duration::from_secs(5 * 60);
const COPY_DEADLINE: Duration = Duration::from_secs(10 * 60);
const QUICK_DEADLINE: Duration = Duration::from_secs(2 * 60);

/// How much of a failed tool's stderr an error keeps.
const STDERR_KEPT: usize = 2000;

/// Adytum's folder: `$ADYTUM_HOME` if set, else `~/.adytum`.
pub fn adytum_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("ADYTUM_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".adytum"))
}

/// Adytum's `LIMA_HOME`, inside its folder.
pub fn lima_home_in(adytum_home: &Path) -> PathBuf {
    adytum_home.join("lima")
}

/// The Lima driver.
pub struct Lima {
    runner: Arc<dyn Runner>,
    limactl: PathBuf,
    home: PathBuf,
}

impl Lima {
    /// A driver that runs `limactl` through `runner` with `home` as its
    /// `LIMA_HOME`.
    pub fn new(runner: Arc<dyn Runner>, limactl: PathBuf, home: PathBuf) -> Lima {
        Lima { runner, limactl, home }
    }

    /// The real thing: `limactl` found on `PATH`, run on this machine.
    pub fn system(home: PathBuf) -> Lima {
        Lima::new(Arc::new(SystemRunner::new()), PathBuf::from("limactl"), home)
    }

    /// The `LIMA_HOME` this driver uses.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// `limactl` with only `LIMA_HOME`, `PATH` and `HOME` in its environment.
    fn limactl(&self) -> Command {
        Command::new(&self.limactl).env("LIMA_HOME", &self.home).pass_env(&["PATH", "HOME"])
    }

    fn check_home(&self) -> Result<(), DriverError> {
        if self.home.is_absolute() {
            Ok(())
        } else {
            Err(DriverError::BadPath { path: self.home.display().to_string(), rule: "Adytum's Lima folder must be an absolute path" })
        }
    }

    /// Lima keeps unix sockets in the instance's folder; a path that is too
    /// long for one makes the VM fail to start with a message about sockets.
    fn check_socket_path(&self, name: &CellName) -> Result<(), DriverError> {
        let longest = self.home.join(name.as_str()).join(LONGEST_SOCKET);
        if longest.as_os_str().len() > SOCKET_BUDGET {
            return Err(DriverError::BadPath { path: self.home.display().to_string(), rule: "it is too long for the sockets Lima keeps inside it (about 100 bytes in all)" });
        }
        Ok(())
    }

    fn template_path(&self, name: &CellName) -> PathBuf {
        self.home.join("_templates").join(format!("{name}.yaml"))
    }

    async fn run(&self, cmd: Command, what: &str) -> Result<Output, DriverError> {
        let out = self.runner.run(&cmd, &Cancel::new()).await?;
        if out.timed_out {
            return Err(DriverError::Tool { what: what.to_owned(), status: None, stderr: "It did not finish before its deadline and was stopped.".to_owned() });
        }
        Ok(out)
    }

    /// Runs a `limactl` command that must succeed.
    async fn run_ok(&self, cmd: Command, what: &str, name: &CellName) -> Result<Output, DriverError> {
        let out = self.run(cmd, what).await?;
        if out.success() {
            return Ok(out);
        }
        Err(tool_failure(what, name.as_str(), &out))
    }

    /// The cell's state, or `NotFound`.
    async fn require(&self, name: &CellName) -> Result<CellState, DriverError> {
        self.list().await?.into_iter().find(|c| &c.name == name).map(|c| c.state).ok_or_else(|| DriverError::NotFound { name: name.to_string() })
    }

    fn shell_command(&self, name: &CellName, argv: &[String], env: &[(String, String)]) -> Result<Command, DriverError> {
        let Some(program) = argv.first() else {
            return Err(DriverError::Unsupported { what: "an empty command".to_owned() });
        };
        if argv.iter().chain(env.iter().flat_map(|(k, v)| [k, v])).any(|a| a.contains('\0')) {
            return Err(DriverError::Unsupported { what: "a NUL byte in a command".to_owned() });
        }
        let mut cmd = self.limactl().args(["shell", "--workdir=/"]).arg(name.as_str()).arg("--");
        if !env.is_empty() {
            for (key, _) in env {
                let ok = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !ok {
                    return Err(DriverError::Unsupported { what: format!("the variable name {key:?}") });
                }
            }
            if program.contains('=') {
                return Err(DriverError::Unsupported { what: "a program name with = in it together with variables".to_owned() });
            }
            cmd = cmd.arg("env");
            for (key, value) in env {
                cmd = cmd.arg(format!("{key}={value}"));
            }
        }
        Ok(cmd.args(argv))
    }
}

/// The error for a `limactl` call that exited badly. A missing instance has
/// its own error, so callers can tell it from a failing tool.
fn tool_failure(what: &str, name: &str, out: &Output) -> DriverError {
    let stderr = out.stderr_lossy();
    if stderr.contains("does not exist") || (stderr.contains("no such file or directory") && stderr.contains("lima.yaml")) {
        return DriverError::NotFound { name: name.to_owned() };
    }
    DriverError::Tool { what: what.to_owned(), status: out.status, stderr: tail(&stderr) }
}

fn tail(text: &str) -> String {
    let text = text.trim();
    if text.len() <= STDERR_KEPT {
        return text.to_owned();
    }
    let mut start = text.len() - STDERR_KEPT;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("...{}", &text[start..])
}

fn state_of(status: &str) -> CellState {
    match status {
        "Running" => CellState::Running,
        "Stopped" => CellState::Stopped,
        _ => CellState::Broken,
    }
}

fn guest_arg(name: &CellName, path: &str) -> String {
    format!("{name}:{path}")
}

fn host_arg(host: &Path) -> Result<&Path, DriverError> {
    let text = host.to_string_lossy();
    if !host.is_absolute() {
        return Err(DriverError::BadPath { path: text.into_owned(), rule: "a host path must be absolute" });
    }
    if text.contains(':') {
        return Err(DriverError::BadPath { path: text.into_owned(), rule: "limactl reads a colon as the end of an instance name" });
    }
    Ok(host)
}

#[async_trait]
impl Driver for Lima {
    async fn create(&self, spec: &CellSpec) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(spec.name.as_str())?;
        self.check_home()?;
        self.check_socket_path(&spec.name)?;
        // Rendering re-checks the rules that guard the host, before anything is written.
        let text = template::render(spec)?;
        let file = self.template_path(&spec.name);
        let dir = file.parent().unwrap_or(&self.home);
        std::fs::create_dir_all(dir).map_err(|source| DriverError::Io { what: "make the folder for templates", source })?;
        std::fs::write(&file, text).map_err(|source| DriverError::Io { what: "write the template", source })?;
        let cmd = self.limactl().arg("create").arg(format!("--name={}", spec.name)).arg("--tty=false").arg(&file).deadline(CREATE_DEADLINE);
        let result = self.run_ok(cmd, "limactl create", &spec.name).await;
        // The template is only an input; the instance keeps its own copy.
        let _ = std::fs::remove_file(&file);
        result.map(|_| ())
    }

    async fn start(&self, name: &CellName) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        // limactl would make a default VM of an unknown name.
        self.require(name).await?;
        let cmd = self.limactl().arg("start").arg("--tty=false").arg(name.as_str()).deadline(START_DEADLINE);
        self.run_ok(cmd, "limactl start", name).await.map(|_| ())
    }

    async fn exec(&self, name: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        let mut cmd = self.shell_command(name, argv, &opts.env)?;
        if let Some(bytes) = opts.stdin {
            cmd = cmd.stdin_bytes(bytes);
        }
        if let Some(deadline) = opts.deadline {
            cmd = cmd.deadline(deadline);
        }
        let out = self.runner.run(&cmd, &Cancel::new()).await?;
        // limactl itself failed (no such instance), as opposed to the command.
        if !out.success() && out.stderr_lossy().contains("level=fatal") && out.stderr_lossy().contains("does not exist") {
            return Err(DriverError::NotFound { name: name.to_string() });
        }
        Ok(out)
    }

    async fn stream(&self, name: &CellName, argv: &[String]) -> Result<Duplex, DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        let cmd = self.shell_command(name, argv, &[])?;
        Ok(self.runner.duplex(&cmd).await?)
    }

    async fn copy_in(&self, name: &CellName, host: &Path, guest: &str) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        check_guest_path(guest)?;
        let host = host_arg(host)?;
        let mut cmd = self.limactl().arg("copy").arg("--tty=false");
        if host.is_dir() {
            cmd = cmd.arg("-r");
        }
        let cmd = cmd.arg(host).arg(guest_arg(name, guest)).deadline(COPY_DEADLINE);
        self.run_ok(cmd, "limactl copy", name).await.map(|_| ())
    }

    async fn copy_out(&self, name: &CellName, guest: &str, host: &Path) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        check_guest_path(guest)?;
        let host = host_arg(host)?;
        // rsync fails on a file given -r, and a folder needs it, so ask the guest.
        let probe = self.shell_command(name, &["test".to_owned(), "-d".to_owned(), guest.to_owned()], &[])?.deadline(QUICK_DEADLINE);
        let probe = self.run(probe, "limactl shell").await?;
        let is_dir = match probe.status {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(tool_failure("limactl shell", name.as_str(), &probe)),
        };
        let mut cmd = self.limactl().arg("copy").arg("--tty=false");
        if is_dir {
            cmd = cmd.arg("-r");
        }
        let cmd = cmd.arg(guest_arg(name, guest)).arg(host).deadline(COPY_DEADLINE);
        self.run_ok(cmd, "limactl copy", name).await.map(|_| ())
    }

    async fn stop(&self, name: &CellName) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        self.require(name).await?;
        let cmd = self.limactl().arg("stop").arg("--tty=false").arg(name.as_str()).deadline(STOP_DEADLINE);
        self.run_ok(cmd, "limactl stop", name).await.map(|_| ())
    }

    async fn remove(&self, name: &CellName) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        self.check_home()?;
        // Lima ignores a name that does not exist, so removing twice is fine.
        let cmd = self.limactl().args(["delete", "--force", "--tty=false"]).arg(name.as_str()).deadline(STOP_DEADLINE);
        self.run_ok(cmd, "limactl delete", name).await.map(|_| ())
    }

    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        self.check_home()?;
        if !self.home.exists() {
            return Ok(Vec::new());
        }
        let cmd = self.limactl().args(["list", "--json"]).deadline(QUICK_DEADLINE);
        let out = self.run(cmd, "limactl list").await?;
        if !out.success() {
            return Err(DriverError::Tool { what: "limactl list".to_owned(), status: out.status, stderr: tail(&out.stderr_lossy()) });
        }
        let mut cells = Vec::new();
        for line in out.stdout_lossy().lines().filter(|l| !l.trim().is_empty()) {
            let value: serde_json::Value = serde_json::from_str(line).map_err(|e| DriverError::Tool {
                what: "limactl list".to_owned(),
                status: out.status,
                stderr: format!("its output was not JSON: {e}"),
            })?;
            let (Some(name), Some(status)) = (value.get("name").and_then(|v| v.as_str()), value.get("status").and_then(|v| v.as_str())) else { continue };
            // Anything not named by Adytum is none of its business, even
            // inside Adytum's own folder.
            if let Ok(name) = name.parse::<CellName>() {
                cells.push(Listed { name, state: state_of(status) });
            }
        }
        Ok(cells)
    }
}
