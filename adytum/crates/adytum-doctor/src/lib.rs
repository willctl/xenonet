//! What is wrong with Adytum's setup on this machine, each problem with a fix
//! a person can act on.
//!
//! [`checks`] looks at the real machine; [`run`] takes a [`Probe`], so a test
//! can describe any machine. [`render_human`] and [`render_json`] print the
//! answer.

mod probe;
mod render;

pub use probe::{LedgerStatus, Probe, SystemProbe};
pub use render::{has_failure, render_human, render_json};

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Lima versions whose `limactl` output Adytum has recorded golden files for
/// (`adytum-lima/tests/golden/lima-<version>`).
pub const KNOWN_LIMA_VERSIONS: &[&str] = &["2.1.2"];

/// Most bytes a unix socket path may hold on macOS, with its closing NUL.
const SOCKET_BUDGET: usize = 104;
/// The longest socket Lima makes inside an instance's folder.
const LONGEST_SOCKET: &str = "default_ep.sock";
/// The longest cell name (`adytum-` and 47 more characters).
const LONGEST_CELL_NAME: usize = 54;
/// Below this many characters a cell name has no room to say anything.
const SHORTEST_USEFUL_NAME: usize = 16;
/// Less free space than this and a VM cannot be trusted to start.
const FREE_DISK_FAIL: u64 = 20 * 1024 * 1024 * 1024;

/// How much a finding matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Something to know. Nothing is broken.
    Info,
    /// Something will go wrong or work worse.
    Warn,
    /// Adytum cannot do its job until this is fixed.
    Fail,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// A stable name, such as `limactl`.
    pub id: String,
    /// True when there is nothing to do.
    pub ok: bool,
    /// How much it matters when it is not ok; `Info` when it is.
    pub level: Level,
    /// What was found, as a sentence.
    pub message: String,
    /// What to do about it. Present on every check that is not ok.
    pub fix: Option<String>,
}

impl Check {
    fn ok(id: &str, message: impl Into<String>) -> Check {
        Check { id: id.to_owned(), ok: true, level: Level::Info, message: message.into(), fix: None }
    }

    fn not_ok(id: &str, level: Level, message: impl Into<String>, fix: impl Into<String>) -> Check {
        Check { id: id.to_owned(), ok: false, level, message: message.into(), fix: Some(fix.into()) }
    }
}

/// Every check, against this machine.
pub async fn checks() -> Vec<Check> {
    run(&SystemProbe::new()).await
}

/// Every check, against whatever `probe` describes.
pub async fn run(probe: &dyn Probe) -> Vec<Check> {
    let home = probe.adytum_home();
    let mut out = vec![limactl(probe).await];
    match &home {
        Some(home) => {
            out.push(lima_home(probe, home));
            out.push(socket_budget(home));
            out.push(image_cache(probe));
            out.push(disk(probe, home).await);
            out.extend(ledger(probe, home));
        }
        None => out.push(Check::not_ok(
            "adytum-home",
            Level::Fail,
            "Adytum's folder cannot be found: neither ADYTUM_HOME nor HOME is set.",
            "Set ADYTUM_HOME to a folder Adytum may keep its cells in, such as ~/.adytum.",
        )),
    }
    out.push(host_containers(probe).await);
    out.push(Check::ok(
        "container-wall",
        "A container is a weaker wall than a VM: it shares this machine's kernel, so a flaw in the kernel reaches the host. Use a VM cell for what must stay apart, and a container where that is enough.",
    ));
    out
}

async fn limactl(probe: &dyn Probe) -> Check {
    let known = KNOWN_LIMA_VERSIONS.join(", ");
    match probe.limactl_version().await {
        None => Check::not_ok(
            "limactl",
            Level::Fail,
            "limactl was not found, so Adytum cannot make VM cells.",
            "Install Lima (https://lima-vm.io) so that `limactl` is on PATH, then run the doctor again.",
        ),
        Some(v) if KNOWN_LIMA_VERSIONS.contains(&v.as_str()) => Check::ok("limactl", format!("limactl {v}, a version Adytum has recorded behaviour for.")),
        Some(v) => Check::not_ok(
            "limactl",
            Level::Warn,
            format!("limactl {v} is not a version Adytum has recorded behaviour for (it knows {known}); its answers may differ."),
            format!("Install Lima {known}, or make and remove a test cell before relying on this version."),
        ),
    }
}

fn lima_home(probe: &dyn Probe, home: &Path) -> Check {
    let lima = adytum_lima::lima_home_in(home);
    if probe.exists(&lima) {
        Check::ok("lima-home", format!("Adytum's Lima folder exists at {}.", lima.display()))
    } else {
        Check {
            id: "lima-home".to_owned(),
            ok: false,
            level: Level::Info,
            message: format!("Adytum's Lima folder {} does not exist yet.", lima.display()),
            fix: Some("Nothing to do: the first `adytum create` makes it.".to_owned()),
        }
    }
}

/// Lima keeps unix sockets in each instance's folder, and a socket path holds
/// 104 bytes on macOS. How long a cell name can be is what is left.
fn socket_budget(home: &Path) -> Check {
    let lima = adytum_lima::lima_home_in(home);
    let used = lima.as_os_str().len() + 1 + 1 + LONGEST_SOCKET.len() + 1; // "/", the name, "/", the socket, NUL
    let room = SOCKET_BUDGET.saturating_sub(used);
    if room >= LONGEST_CELL_NAME {
        Check::ok("socket-path", format!("Lima's longest socket path fits for any cell name ({} of {SOCKET_BUDGET} bytes at most).", used + LONGEST_CELL_NAME))
    } else if room >= SHORTEST_USEFUL_NAME {
        Check::not_ok(
            "socket-path",
            Level::Warn,
            format!("Adytum's folder is long: cell names over {room} characters would make a socket path Lima cannot use, and Adytum will refuse them."),
            "Keep cell names short, or set ADYTUM_HOME to a shorter folder (such as ~/.adytum).",
        )
    } else {
        Check::not_ok(
            "socket-path",
            Level::Fail,
            format!("Adytum's folder {} is too long: Lima's sockets would not fit in it (room for a {room}-character name).", lima.display()),
            "Set ADYTUM_HOME to a shorter folder, such as ~/.adytum.",
        )
    }
}

fn image_cache(probe: &dyn Probe) -> Check {
    let name = adytum_lima::DEFAULT_IMAGE;
    let Some(pin) = adytum_lima::image(name) else {
        return Check::not_ok("image-cache", Level::Warn, format!("The default image {name} is not pinned in this build."), "Report this: the build is missing its pin file.");
    };
    if probe.lima_cache_has(&pin.location) {
        Check::ok("image-cache", format!("The pinned image {name} is in Lima's download cache."))
    } else {
        Check {
            id: "image-cache".to_owned(),
            ok: false,
            level: Level::Info,
            message: format!("The pinned image {name} is not in Lima's download cache yet, so the first create downloads it."),
            fix: Some("Nothing to do: the first `adytum create` downloads it, and later ones reuse it.".to_owned()),
        }
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

async fn disk(probe: &dyn Probe, home: &Path) -> Check {
    match probe.free_bytes(home).await {
        None => Check::not_ok(
            "disk",
            Level::Warn,
            "Could not read how much disk is free where Adytum keeps its cells.",
            "Run `df -h` on Adytum's folder yourself and make sure at least 20 GiB is free.",
        ),
        Some(free) if free < FREE_DISK_FAIL => Check::not_ok(
            "disk",
            Level::Fail,
            format!("Only {} is free where Adytum keeps its cells; a VM needs at least {}.", gib(free), gib(FREE_DISK_FAIL)),
            "Free disk space (empty the trash, remove unused cells with `adytum rm`), or move ADYTUM_HOME to a bigger disk.",
        ),
        Some(free) => Check::ok("disk", format!("{} is free where Adytum keeps its cells.", gib(free))),
    }
}

fn ledger(probe: &dyn Probe, home: &Path) -> Vec<Check> {
    match probe.ledger(home) {
        LedgerStatus::Missing => vec![Check::ok("ledger", "There is no ledger yet; the first command makes it.")],
        LedgerStatus::Open { unsettled: 0 } => vec![Check::ok("ledger", "The ledger opens."), Check::ok("ledger-settled", "Every operation in the ledger is settled.")],
        LedgerStatus::Open { unsettled } => vec![
            Check::ok("ledger", "The ledger opens."),
            Check::not_ok(
                "ledger-settled",
                Level::Warn,
                format!("{unsettled} operation(s) started and never settled, probably because Adytum was stopped part-way."),
                "Run any adytum command to finish them.",
            ),
        ],
        LedgerStatus::SetUpAgain(why) => vec![Check::not_ok(
            "ledger",
            Level::Fail,
            format!("The ledger cannot be used: {why}"),
            "Set Adytum up again on this machine: move adytum.sqlite out of Adytum's folder, remove by hand any cells Adytum made (`limactl list`, names starting adytum-), then run any adytum command.",
        )],
        LedgerStatus::Unreadable(why) => vec![Check::not_ok(
            "ledger",
            Level::Fail,
            format!("The ledger could not be opened: {why}"),
            "Check that Adytum's folder can be read and written and the disk is not full, then run the doctor again.",
        )],
    }
}

async fn host_containers(probe: &dyn Probe) -> Check {
    let mut found = Vec::new();
    for tool in ["podman", "docker"] {
        if let Some(v) = probe.tool_version(tool).await {
            found.push(format!("{tool} {v}"));
        }
    }
    if found.is_empty() {
        Check::ok("host-containers", "No podman or docker on this machine. Not needed: containers can run inside a cell.")
    } else {
        Check::ok(
            "host-containers",
            format!("Found {} on this machine. Not needed: containers can also run inside a cell, which keeps them off this machine's kernel.", found.join(" and ")),
        )
    }
}

/// The folder a `Path` lives in, or itself: used by probes that ask the
/// disk about a folder that may not exist yet.
pub(crate) fn nearest_existing(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    let mut at = path.to_path_buf();
    while !exists(&at) {
        match at.parent() {
            Some(p) => at = p.to_path_buf(),
            None => break,
        }
    }
    at
}
