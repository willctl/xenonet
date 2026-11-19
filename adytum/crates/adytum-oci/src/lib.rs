//! Podman and docker cells: the same [`Driver`] trait as Lima, by running the
//! tool's command line through `adytum-exec`.
//!
//! A container is a weaker wall than a VM: it shares the host's kernel. Use
//! it where that is enough, or run podman inside a Lima cell with [`InCell`].
//!
//! - A container's name is the cell's name. Every container Adytum makes
//!   carries the labels `adytum.cell` (an id derived from the name) and
//!   `adytum.wall`. [`Driver::list`] shows only containers with the cell
//!   label, and every other call looks at the label first and refuses a
//!   container without it, so a container Adytum did not make is never
//!   seen, started, stopped, copied from or removed.
//! - An image is named by digest only: the pin's location without its
//!   `https://`, then `@sha256:...`. The runtime cannot be handed a moving
//!   tag.
//! - A cell gets no network unless its spec asks for one (a `network` or a
//!   port). It then joins `adytum-wall-<wall>`, made on first use, so two
//!   walls never share one.
//! - A sealed cell gets no `-v` or `--mount`. Ports are published on the
//!   spec's loopback address and nowhere else, and the automation port is
//!   never published.
//! - Arguments reach the container as separate arguments, never through a
//!   shell.

mod incell;
mod parse;
mod pins;

pub use incell::InCell;
pub use pins::{image, DEFAULT_IMAGE};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Cancel, Command, Duplex, Output, Runner, SystemRunner};
use adytum_model::{CellId, CellName, CellSpec, DriverKind, Port, Refusal, RefusalCode, Wall};
use async_trait::async_trait;
use sha2::{Digest, Sha256};

/// The label every Adytum container carries; its value is the cell's id.
pub const CELL_LABEL: &str = "adytum.cell";
/// The label that holds the cell's wall.
pub const WALL_LABEL: &str = "adytum.wall";

const CREATE_DEADLINE: Duration = Duration::from_secs(30 * 60);
const COPY_DEADLINE: Duration = Duration::from_secs(10 * 60);
const QUICK_DEADLINE: Duration = Duration::from_secs(2 * 60);
const STDERR_KEPT: usize = 2000;
/// Seconds a stop waits before it kills; the container's first process
/// (`sleep`) does not answer SIGTERM.
const STOP_SECONDS: &str = "2";

/// Which command line to speak.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OciTool {
    /// `podman`.
    Podman,
    /// `docker`.
    Docker,
}

impl OciTool {
    fn name(self) -> &'static str {
        match self {
            Self::Podman => "podman",
            Self::Docker => "docker",
        }
    }

    fn kind(self) -> DriverKind {
        match self {
            Self::Podman => DriverKind::Podman,
            Self::Docker => DriverKind::Docker,
        }
    }
}

/// When the runtime may download an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pull {
    /// Only if the image is not there yet (the default).
    Missing,
    /// Never: the image must already be in the runtime's store.
    Never,
}

/// The container driver.
pub struct Oci {
    runner: Arc<dyn Runner>,
    tool: OciTool,
    program: PathBuf,
    pull: Pull,
}

impl Oci {
    /// A driver that runs `program` (the tool's executable) through `runner`.
    pub fn new(runner: Arc<dyn Runner>, tool: OciTool, program: PathBuf) -> Oci {
        Oci { runner, tool, program, pull: Pull::Missing }
    }

    /// The tool on this machine, found on `PATH`.
    pub fn system(tool: OciTool) -> Oci {
        Oci::new(Arc::new(SystemRunner::new()), tool, PathBuf::from(tool.name()))
    }

    /// Changes when images may be downloaded.
    #[must_use]
    pub fn with_pull(mut self, pull: Pull) -> Oci {
        self.pull = pull;
        self
    }

    /// The tool's command with only what it needs from the environment.
    fn tool_cmd(&self) -> Command {
        Command::new(&self.program).pass_env(&["PATH", "HOME", "XDG_RUNTIME_DIR"])
    }

    async fn run(&self, cmd: Command, what: &str) -> Result<Output, DriverError> {
        let out = self.runner.run(&cmd, &Cancel::new()).await?;
        if out.timed_out {
            return Err(DriverError::Tool { what: what.to_owned(), status: None, stderr: "It did not finish before its deadline and was stopped.".to_owned() });
        }
        Ok(out)
    }

    async fn run_ok(&self, cmd: Command, what: &str) -> Result<Output, DriverError> {
        let out = self.run(cmd, what).await?;
        if out.success() {
            Ok(out)
        } else {
            Err(DriverError::Tool { what: what.to_owned(), status: out.status, stderr: tail(&out.stderr_lossy()) })
        }
    }

    fn what(&self, verb: &str) -> String {
        format!("{} {verb}", self.tool.name())
    }

    /// Reads the cell label of a container: `Ok(None)` if there is no such
    /// container, `Ok(Some(label))` otherwise (empty if it has none).
    async fn label_of(&self, name: &CellName) -> Result<Option<String>, DriverError> {
        let format = format!("{{{{index .Config.Labels \"{CELL_LABEL}\"}}}}");
        let cmd = self.tool_cmd().args(["container", "inspect", "--format"]).arg(format).arg(name.as_str()).deadline(QUICK_DEADLINE);
        let out = self.run(cmd, &self.what("container inspect")).await?;
        if out.success() {
            let label = out.stdout_lossy().trim().to_owned();
            return Ok(Some(if label == "<no value>" { String::new() } else { label }));
        }
        let stderr = out.stderr_lossy();
        if stderr.to_lowercase().contains("no such") {
            return Ok(None);
        }
        Err(DriverError::Tool { what: self.what("container inspect"), status: out.status, stderr: tail(&stderr) })
    }

    /// Passes only a container Adytum made. A container that exists without
    /// the label is refused; one that does not exist is `NotFound`.
    async fn require_ours(&self, name: &CellName) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        match self.label_of(name).await? {
            None => Err(DriverError::NotFound { name: name.to_string() }),
            Some(label) if label.is_empty() => Err(foreign(name)),
            Some(_) => Ok(()),
        }
    }

    /// Makes the wall's network if it is not there.
    async fn ensure_network(&self, wall: &Wall) -> Result<String, DriverError> {
        let net = wall_network(wall);
        let probe = self.tool_cmd().args(["network", "inspect", "--format", "{{.Name}}"]).arg(&net).deadline(QUICK_DEADLINE);
        if self.run(probe, &self.what("network inspect")).await?.success() {
            return Ok(net);
        }
        let create = self.tool_cmd().args(["network", "create", "--label"]).arg(format!("{WALL_LABEL}={wall}")).arg(&net).deadline(QUICK_DEADLINE);
        self.run_ok(create, &self.what("network create")).await?;
        Ok(net)
    }

    fn exec_command(&self, name: &CellName, argv: &[String], env: &[(String, String)], interactive: bool) -> Result<Command, DriverError> {
        if argv.is_empty() {
            return Err(DriverError::Unsupported { what: "an empty command".to_owned() });
        }
        if argv.iter().chain(env.iter().flat_map(|(k, v)| [k, v])).any(|a| a.contains('\0')) {
            return Err(DriverError::Unsupported { what: "a NUL byte in a command".to_owned() });
        }
        let mut cmd = self.tool_cmd().arg("exec");
        if interactive {
            cmd = cmd.arg("-i");
        }
        for (key, value) in env {
            let ok = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !ok {
                return Err(DriverError::Unsupported { what: format!("the variable name {key:?}") });
            }
            cmd = cmd.arg("-e").arg(format!("{key}={value}"));
        }
        Ok(cmd.arg(name.as_str()).args(argv))
    }
}

fn foreign(name: &CellName) -> DriverError {
    Refusal::new(
        RefusalCode::NameNotOurs,
        format!("A container named {name} exists but Adytum did not make it (it has no {CELL_LABEL} label)."),
        "Adytum leaves it alone. Pick another name, or remove that container yourself.",
    )
    .into()
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

/// The id a container's `adytum.cell` label holds: derived from the name, so
/// the same cell always has the same label.
pub fn cell_label(name: &CellName) -> String {
    let digest = Sha256::digest(format!("adytum-cell:{name}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    CellId::from_bytes(bytes).to_string()
}

/// The network a wall's cells join.
pub fn wall_network(wall: &Wall) -> String {
    format!("adytum-wall-{}", wall.as_str().strip_prefix("wall_").unwrap_or(wall.as_str()))
}

/// The image reference for a pin: its location without `https://`, then
/// `@sha256:...`. A pin that is a file path, or already names a tag or
/// digest, cannot be a container image.
fn image_ref(spec: &CellSpec) -> Result<String, DriverError> {
    let rest = spec.image.location.strip_prefix("https://").filter(|r| !r.contains('@'));
    match rest {
        Some(r) => Ok(format!("{r}@{}", spec.image.digest)),
        None => Err(DriverError::Unsupported { what: "an image location that is not an https:// registry path without a digest".to_owned() }),
    }
}

fn publish(p: &Port) -> String {
    match p.bind {
        std::net::IpAddr::V4(v4) => format!("{v4}:{}:{}", p.host, p.guest),
        std::net::IpAddr::V6(v6) => format!("[{v6}]:{}:{}", p.host, p.guest),
    }
}

fn bad_path(path: &str, rule: &'static str) -> DriverError {
    DriverError::BadPath { path: path.to_owned(), rule }
}

fn check_guest_path(path: &str) -> Result<(), DriverError> {
    if !path.starts_with('/') {
        return Err(bad_path(path, "a guest path must be absolute"));
    }
    if path.chars().any(char::is_control) {
        return Err(bad_path(path, "it has a control character"));
    }
    if path.contains(':') {
        return Err(bad_path(path, "the tool reads a colon as the end of a container name"));
    }
    if path.split('/').any(|part| part == "..") {
        return Err(bad_path(path, "it climbs out with .."));
    }
    Ok(())
}

fn check_host_path(host: &Path) -> Result<(), DriverError> {
    let text = host.to_string_lossy();
    if !host.is_absolute() {
        return Err(bad_path(&text, "a host path must be absolute"));
    }
    if text.contains(':') {
        return Err(bad_path(&text, "the tool reads a colon as the end of a container name"));
    }
    Ok(())
}

#[async_trait]
impl Driver for Oci {
    async fn create(&self, spec: &CellSpec) -> Result<(), DriverError> {
        // The rules that guard the host run first, so a driver called
        // without the policy check still cannot publish or mount wrongly.
        adytum_policy::name_is_ours(spec.name.as_str())?;
        adytum_policy::ports_on_localhost(spec)?;
        adytum_policy::automation_port_kept_inside(spec)?;
        adytum_policy::sealed_has_no_mounts(spec)?;
        if spec.driver != self.tool.kind() {
            return Err(DriverError::Unsupported { what: format!("a cell for another driver (this one is {})", self.tool.name()) });
        }
        if spec.clone_from.is_some() {
            return Err(DriverError::Unsupported { what: "cloning a cell".to_owned() });
        }
        if !spec.secrets.is_empty() {
            return Err(DriverError::Unsupported { what: "secrets".to_owned() });
        }
        let image = image_ref(spec)?;
        let mut mounts = Vec::new();
        if !spec.sealed {
            for m in &spec.mounts {
                let (source, target) = (m.source.to_string_lossy().into_owned(), m.target.clone());
                if !m.source.is_absolute() || [&source, &target].iter().any(|t| t.contains(',') || t.chars().any(char::is_control)) {
                    return Err(bad_path(&source, "a mount needs an absolute source, and no commas or control characters in either path"));
                }
                if !target.starts_with('/') {
                    return Err(bad_path(&target, "a mount target must be absolute"));
                }
                mounts.push(format!("type=bind,source={source},target={target}{}", if m.writable { "" } else { ",readonly" }));
            }
        }

        let network = if spec.network.is_some() || !spec.ports.is_empty() { self.ensure_network(&spec.wall).await? } else { "none".to_owned() };

        let mut cmd = self.tool_cmd().arg("create").arg("--name").arg(spec.name.as_str());
        cmd = cmd.arg("--label").arg(format!("{CELL_LABEL}={}", cell_label(&spec.name)));
        cmd = cmd.arg("--label").arg(format!("{WALL_LABEL}={}", spec.wall));
        cmd = cmd.arg(match self.pull {
            Pull::Missing => "--pull=missing",
            Pull::Never => "--pull=never",
        });
        cmd = cmd.arg(format!("--network={network}")).arg("--cap-drop=ALL").arg("--security-opt=no-new-privileges");
        cmd = cmd.arg(format!("--cpus={}", spec.resources.cpus)).arg(format!("--memory={}m", spec.resources.memory_mib));
        for p in &spec.ports {
            cmd = cmd.arg("-p").arg(publish(p));
        }
        for m in &mounts {
            cmd = cmd.arg("--mount").arg(m);
        }
        let cmd = cmd.arg(image).args(["sleep", "infinity"]).deadline(CREATE_DEADLINE);
        let out = self.run(cmd, &self.what("create")).await?;
        if out.success() {
            return Ok(());
        }
        let stderr = out.stderr_lossy();
        if stderr.contains("already in use") {
            return Err(
                Refusal::new(RefusalCode::NameTaken, format!("A container named {} already exists.", spec.name), "Pick another name, or remove that container first.").into()
            );
        }
        Err(DriverError::Tool { what: self.what("create"), status: out.status, stderr: tail(&stderr) })
    }

    async fn start(&self, name: &CellName) -> Result<(), DriverError> {
        self.require_ours(name).await?;
        let cmd = self.tool_cmd().arg("start").arg(name.as_str()).deadline(QUICK_DEADLINE);
        self.run_ok(cmd, &self.what("start")).await.map(|_| ())
    }

    async fn exec(&self, name: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
        self.require_ours(name).await?;
        let mut cmd = self.exec_command(name, argv, &opts.env, opts.stdin.is_some())?;
        if let Some(bytes) = opts.stdin {
            cmd = cmd.stdin_bytes(bytes);
        }
        if let Some(deadline) = opts.deadline {
            cmd = cmd.deadline(deadline);
        }
        Ok(self.runner.run(&cmd, &Cancel::new()).await?)
    }

    async fn stream(&self, name: &CellName, argv: &[String]) -> Result<Duplex, DriverError> {
        self.require_ours(name).await?;
        let cmd = self.exec_command(name, argv, &[], true)?;
        Ok(self.runner.duplex(&cmd).await?)
    }

    async fn copy_in(&self, name: &CellName, host: &Path, guest: &str) -> Result<(), DriverError> {
        self.require_ours(name).await?;
        check_guest_path(guest)?;
        check_host_path(host)?;
        let cmd = self.tool_cmd().arg("cp").arg(host).arg(format!("{name}:{guest}")).deadline(COPY_DEADLINE);
        self.run_ok(cmd, &self.what("cp")).await.map(|_| ())
    }

    async fn copy_out(&self, name: &CellName, guest: &str, host: &Path) -> Result<(), DriverError> {
        self.require_ours(name).await?;
        check_guest_path(guest)?;
        check_host_path(host)?;
        let cmd = self.tool_cmd().arg("cp").arg(format!("{name}:{guest}")).arg(host).deadline(COPY_DEADLINE);
        self.run_ok(cmd, &self.what("cp")).await.map(|_| ())
    }

    async fn stop(&self, name: &CellName) -> Result<(), DriverError> {
        self.require_ours(name).await?;
        let cmd = self.tool_cmd().args(["stop", "--time", STOP_SECONDS]).arg(name.as_str()).deadline(COPY_DEADLINE);
        self.run_ok(cmd, &self.what("stop")).await.map(|_| ())
    }

    async fn remove(&self, name: &CellName) -> Result<(), DriverError> {
        adytum_policy::name_is_ours(name.as_str())?;
        match self.label_of(name).await? {
            // Already gone counts as removed, so removing twice is fine.
            None => return Ok(()),
            Some(label) if label.is_empty() => return Err(foreign(name)),
            Some(_) => {}
        }
        let mut cmd = self.tool_cmd().args(["rm", "--force", "--volumes"]);
        if self.tool == OciTool::Podman {
            // Podman waits for a graceful stop unless told not to; docker's kills at once.
            cmd = cmd.args(["--time", "0"]);
        }
        let cmd = cmd.arg(name.as_str()).deadline(COPY_DEADLINE);
        self.run_ok(cmd, &self.what("rm")).await.map(|_| ())
    }

    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        let cmd = self.tool_cmd().args(["ps", "-a", "--filter"]).arg(format!("label={CELL_LABEL}")).args(["--format", "json"]).deadline(QUICK_DEADLINE);
        let out = self.run_ok(cmd, &self.what("ps")).await?;
        parse::ps(&out.stdout_lossy()).map_err(|e| DriverError::Tool { what: self.what("ps"), status: out.status, stderr: format!("its output was not JSON: {e}") })
    }
}
