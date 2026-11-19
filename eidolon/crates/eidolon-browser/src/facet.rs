//! The browser as a facet: its profile folder, made quiet before it is
//! copied and left running afterwards.

use crate::plan::stop_argv;
use crate::units::{self, USER};
use async_trait::async_trait;
use eidolon_facet::{CellAccess, Facet, FacetError, Result, RunOutput};
use eidolon_model::{FacetKind, FacetSpec};
use std::path::Path;
use std::time::Duration;

/// What a profile rebuilds by itself, so it is not carried.
/// `sessionstore-backups/recovery.baklz4` is kept on purpose: it holds the
/// open tabs.
pub const PROFILE_LEAVE_OUT: [&str; 10] =
    ["cache2/**", "startupCache/**", "thumbnails/**", "crashes/**", "minidumps/**", "datareporting/**", "saved-telemetry-pings/**", "*.lock", "lock", ".parentlock"];

/// How many times `quiet` asks whether the browser's processes are gone.
const QUIET_ATTEMPTS: u32 = 40;

/// The browser facet. `spec.path` is the profile folder in the cell.
#[derive(Debug, Clone)]
pub struct BrowserFacet {
    spec: FacetSpec,
    retry_gap: Duration,
}

impl BrowserFacet {
    /// A browser facet for `spec`, which must be a browser spec whose
    /// leave-out patterns stay inside the facet.
    pub fn new(spec: FacetSpec) -> Result<BrowserFacet> {
        if spec.kind != FacetKind::Browser {
            return Err(FacetError::WrongKind { expected: FacetKind::Browser });
        }
        spec.validate().map_err(|e| FacetError::Cell(e.to_string()))?;
        for pattern in &spec.leave_out {
            eidolon_policy::leave_out_is_inside(pattern)?;
        }
        Ok(BrowserFacet { spec, retry_gap: Duration::from_millis(250) })
    }

    /// How long `quiet` waits between two asks. Tests set zero.
    #[must_use]
    pub fn with_retry_gap(mut self, gap: Duration) -> BrowserFacet {
        self.retry_gap = gap;
        self
    }
}

fn run_failed(what: &str, out: &RunOutput) -> FacetError {
    FacetError::Cell(format!("{what} exited with status {}: {}", out.status, out.stderr.trim()))
}

async fn run_ok(cell: &dyn CellAccess, what: &str, argv: &[&str]) -> Result<()> {
    let out = cell.run(argv).await?;
    if out.status == 0 {
        Ok(())
    } else {
        Err(run_failed(what, &out))
    }
}

#[async_trait]
impl Facet for BrowserFacet {
    fn kind(&self) -> FacetKind {
        FacetKind::Browser
    }

    fn spec(&self) -> &FacetSpec {
        &self.spec
    }

    /// Stops both browser units, then asks until the `eidolon` user has no
    /// process left (`pgrep` answers 1 when there is none). Firefox writes
    /// its profile while it exits, so copying earlier would tear it.
    async fn quiet(&self, cell: &dyn CellAccess) -> Result<()> {
        let stop = stop_argv();
        let stop: Vec<&str> = stop.iter().map(String::as_str).collect();
        run_ok(cell, "Stopping the browser", &stop).await?;
        for attempt in 1..=QUIET_ATTEMPTS {
            let out = cell.run(&["pgrep", "-u", USER]).await?;
            match out.status {
                1 => return Ok(()),
                0 if attempt < QUIET_ATTEMPTS => {
                    if !self.retry_gap.is_zero() {
                        tokio::time::sleep(self.retry_gap).await;
                    }
                }
                0 => break,
                _ => return Err(run_failed("Looking for browser processes", &out)),
            }
        }
        Err(FacetError::Cell("The browser's processes did not stop. Run `systemctl status eidolon-browser` in the cell and stop what is left, then try again.".to_owned()))
    }

    async fn resume(&self, cell: &dyn CellAccess) -> Result<()> {
        run_ok(cell, "Starting the browser", &["sudo", "systemctl", "start", units::BROWSER_UNIT]).await
    }

    fn leave_out(&self) -> Vec<String> {
        let mut all: Vec<String> = PROFILE_LEAVE_OUT.iter().map(|p| (*p).to_owned()).collect();
        for pattern in &self.spec.leave_out {
            if !all.contains(pattern) {
                all.push(pattern.clone());
            }
        }
        all
    }

    /// Opens the profile for reading, copies it, and closes it again even
    /// when the copy failed. The profile is private to `eidolon`, and the
    /// copy runs as the cell's own user.
    async fn take(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()> {
        let path = self.spec.path.as_str();
        run_ok(cell, "Opening the profile for copying", &["sudo", "chmod", "-R", "a+rX", path]).await?;
        let copied = cell.copy_out(path, staging).await;
        let closed = run_ok(cell, "Closing the profile again", &["sudo", "chmod", "-R", "u+rwX,go-rwx", path]).await;
        copied.and(closed)
    }

    /// Copies the profile in and gives it to `eidolon`: the copy arrives
    /// owned by the cell's own user.
    async fn place(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()> {
        let path = self.spec.path.as_str();
        cell.copy_in(staging, path).await?;
        run_ok(cell, "Giving the profile to its user", &["sudo", "chown", "-R", "eidolon:eidolon", path]).await?;
        run_ok(cell, "Closing the profile", &["sudo", "chmod", "-R", "u+rwX,go-rwx", path]).await
    }
}
