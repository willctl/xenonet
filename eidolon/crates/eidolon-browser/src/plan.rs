//! Setting up the guest, as data.
//!
//! A [`Plan`] is a list of [`Step`]s. Each step is an argument list run in
//! the cell through `Driver::exec`, never a shell string. A file the guest
//! needs is written by sending its bytes as stdin to `sudo tee`. Because the
//! plan is data, a golden file pins every command it will run.

use crate::error::{BrowserError, Result};
use crate::units::{self, Ports};
use adytum_driver::{Driver, ExecOpts};
use adytum_model::CellName;
use serde::Serialize;
use std::fmt;
use std::time::Duration;

/// The fingerprint of the key Mozilla signs its APT repository with, as
/// published at <https://support.mozilla.org/kb/install-firefox-linux>.
pub const MOZILLA_KEY_FINGERPRINT: &str = "35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3";

const KEY_PATH: &str = "/etc/apt/keyrings/packages.mozilla.org.asc";
const STEP_DEADLINE: Duration = Duration::from_secs(20 * 60);

/// One command to run in the cell.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// What the step is for, in a few words.
    pub why: String,
    /// The program and its arguments.
    pub argv: Vec<String>,
    /// Text sent to the command's stdin (a file's contents, or a secret).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// A command that, if it succeeds, means this step is already done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if: Option<Vec<String>>,
    /// The command prints `gpg --with-colons` listings; its only `fpr`
    /// lines must be this fingerprint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expect_fingerprint: Option<String>,
}

// A step may carry a secret as stdin, so it never prints it.
impl fmt::Debug for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Step")
            .field("why", &self.why)
            .field("argv", &self.argv)
            .field("stdin_bytes", &self.stdin.as_ref().map(String::len))
            .field("skip_if", &self.skip_if)
            .finish_non_exhaustive()
    }
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

impl Step {
    /// A command run as root.
    fn sudo(why: &str, argv: &[&str]) -> Step {
        let mut full = vec!["sudo"];
        full.extend_from_slice(argv);
        Step { why: why.to_owned(), argv: strings(&full), stdin: None, skip_if: None, expect_fingerprint: None }
    }

    /// Writes `text` to `path` as root, with `sudo tee`.
    fn write(why: &str, path: &str, text: &str) -> Step {
        Step { stdin: Some(text.to_owned()), ..Step::sudo(why, &["tee", path]) }
    }

    fn skip_if(mut self, guard: &[&str]) -> Step {
        self.skip_if = Some(strings(guard));
        self
    }
}

/// A list of steps for one cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    /// The steps, in order.
    pub steps: Vec<Step>,
}

impl Plan {
    /// Runs every step in `cell`, stopping at the first that fails.
    /// Steps already done (their `skip_if` succeeds) are skipped, so a plan
    /// can be run again.
    pub async fn run(&self, driver: &dyn Driver, cell: &CellName) -> Result<()> {
        for step in &self.steps {
            run_step(step, driver, cell).await?;
        }
        Ok(())
    }
}

async fn run_step(step: &Step, driver: &dyn Driver, cell: &CellName) -> Result<()> {
    if let Some(guard) = &step.skip_if {
        let done = driver.exec(cell, guard, ExecOpts::default()).await?;
        if done.success() {
            return Ok(());
        }
    }
    let opts = ExecOpts { stdin: step.stdin.as_ref().map(|s| s.as_bytes().to_vec()), deadline: Some(STEP_DEADLINE), ..ExecOpts::default() };
    let out = driver.exec(cell, &step.argv, opts).await?;
    if !out.success() {
        // The step's stdin may be a secret, so only stderr is quoted.
        let stderr = out.stderr_lossy();
        let tail: String = stderr.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(BrowserError::Step { why: step.why.clone(), status: out.status, stderr: tail.trim().to_owned() });
    }
    if let Some(expected) = &step.expect_fingerprint {
        let found = fingerprints(&out.stdout_lossy());
        if found.len() != 1 || !found[0].eq_ignore_ascii_case(expected) {
            return Err(BrowserError::Fingerprint { expected: expected.clone(), found });
        }
    }
    Ok(())
}

/// The fingerprints in `gpg --with-colons --fingerprint` output: the tenth
/// field of each `fpr` line.
pub fn fingerprints(listing: &str) -> Vec<String> {
    listing.lines().filter(|l| l.starts_with("fpr:")).map(|l| l.split(':').nth(9).unwrap_or_default().to_owned()).collect()
}

/// The argument list that reaches the browser's automation port from the
/// host: `socat` bridges the cell command's stdin and stdout to the port on
/// the guest's own loopback. The port is never forwarded.
pub fn stream_argv(port: u16) -> Vec<String> {
    vec!["socat".to_owned(), "-".to_owned(), format!("TCP:127.0.0.1:{port}")]
}

/// Everything a fresh Ubuntu 24.04 cell needs to keep a browser: Firefox
/// from Mozilla's APT repository (not the snap), `socat`, `xvfb` and
/// `x11vnc`, the `eidolon` user, the profile's `user.js`, and the units.
/// Nothing is started.
pub fn provision_plan(ports: Ports) -> Plan {
    let apt = |why: &str, rest: &[&str]| {
        let mut argv = vec!["env", "DEBIAN_FRONTEND=noninteractive", "apt-get"];
        argv.extend_from_slice(rest);
        Step::sudo(why, &argv)
    };
    let mut steps = vec![
        apt("refresh the package lists", &["update"]),
        apt(
            "install the tools the browser and its window need",
            &["install", "-y", "--no-install-recommends", "ca-certificates", "curl", "gnupg", "socat", "xvfb", "x11vnc", "fonts-dejavu-core"],
        ),
        Step::sudo("make the keyring folder", &["install", "-d", "-m", "0755", "/etc/apt/keyrings"]),
        Step::sudo("fetch Mozilla's signing key", &["curl", "-fsSL", "--proto", "=https", "-o", KEY_PATH, "https://packages.mozilla.org/apt/repo-signing-key.gpg"]),
        Step {
            why: "check the signing key's fingerprint".to_owned(),
            argv: strings(&["gpg", "--show-keys", "--with-colons", "--fingerprint", KEY_PATH]),
            stdin: None,
            skip_if: None,
            expect_fingerprint: Some(MOZILLA_KEY_FINGERPRINT.to_owned()),
        },
        Step::write(
            "add Mozilla's repository",
            "/etc/apt/sources.list.d/mozilla.sources",
            &format!("Types: deb\nURIs: https://packages.mozilla.org/apt\nSuites: mozilla\nComponents: main\nSigned-By: {KEY_PATH}\n"),
        ),
        Step::write("prefer Mozilla's Firefox package", "/etc/apt/preferences.d/mozilla", "Package: *\nPin: origin packages.mozilla.org\nPin-Priority: 1000\n"),
        apt("refresh the package lists with Mozilla's repository", &["update"]),
        apt("install Firefox", &["install", "-y", "--no-install-recommends", "firefox"]),
        Step::sudo("make the browser's user", &["useradd", "--create-home", "--shell", "/usr/sbin/nologin", units::USER]).skip_if(&["id", "-u", units::USER]),
        // The host's copy-out runs as the cell's own user, which needs to
        // walk into the home folder.
        Step::sudo("let the home folder be entered", &["chmod", "0755", "/home/eidolon"]),
        Step::sudo("make the profile folder", &["install", "-d", "-o", units::USER, "-g", units::USER, "-m", "0700", units::PROFILE_PATH]),
        Step::sudo("make the files folders", &["install", "-d", "-o", units::USER, "-g", units::USER, "-m", "0755", "/home/eidolon/files", units::DOWNLOADS_PATH]),
        // Uploads are copied in as the cell's login user, a different user
        // from the browser's, so the folder takes files from anyone in the
        // cell (sticky, so nobody removes another's file). The browser only
        // reads from it. The cell holds no other users.
        Step::sudo("make the uploads folder", &["install", "-d", "-o", units::USER, "-g", units::USER, "-m", "1777", units::UPLOADS_PATH]),
        Step { stdin: Some(units::USER_JS.to_owned()), ..Step::sudo("write the profile's preferences", &["-u", units::USER, "tee", &format!("{}/user.js", units::PROFILE_PATH)]) },
    ];
    for (name, text) in units::units(ports) {
        steps.push(Step::write("install a unit", &format!("/etc/systemd/system/{name}"), &text));
    }
    steps.push(Step::sudo("load the units", &["systemctl", "daemon-reload"]));
    steps.push(Step::sudo("start the browser at boot", &["systemctl", "enable", units::BROWSER_UNIT]));
    Plan { steps }
}

/// Starts the automation browser (and so stops the sign-in window, which
/// conflicts with it).
pub fn start_browser_plan() -> Plan {
    Plan { steps: vec![Step::sudo("start the browser", &["systemctl", "start", units::BROWSER_UNIT])] }
}

/// Starts the sign-in window, which stops the automation browser. `secret`
/// is the screen share's password; it goes to the guest as stdin of one
/// command and is never an argument.
pub fn start_view_plan(secret: &str) -> Plan {
    // `dd` and not `tee`: tee would print the secret back on stdout.
    let write = Step {
        stdin: Some(secret.to_owned()),
        ..Step::sudo("give the window its password", &["-u", units::USER, "dd", &format!("of={}", units::VNC_PASSWORD_PATH), "status=none"])
    };
    Plan {
        steps: vec![
            Step::sudo("make the window's folder", &["install", "-d", "-o", units::USER, "-g", units::USER, "-m", "0700", units::RUN_PATH]),
            write,
            Step::sudo("keep the password private", &["chmod", "0600", units::VNC_PASSWORD_PATH]),
            Step::sudo("start the sign-in window", &["systemctl", "start", units::VIEW_UNIT]),
        ],
    }
}

/// The command that stops every unit of the browser.
pub fn stop_argv() -> Vec<String> {
    strings(&["sudo", "systemctl", "stop", units::BROWSER_UNIT, units::VIEW_UNIT, units::VNC_UNIT, units::XVFB_UNIT])
}
