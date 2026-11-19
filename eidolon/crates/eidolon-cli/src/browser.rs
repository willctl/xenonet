//! What the browser facet needs of the CLI: setting a cell up for it once,
//! starting it, and keeping it stopped after a seal.

use crate::cells::Cells;
use crate::error::Result;
use adytum_driver::{Driver, ExecOpts};
use adytum_model::CellName;
use async_trait::async_trait;
use eidolon_browser::plan::stop_argv;
use eidolon_browser::units::{Ports, BROWSER_UNIT};
use eidolon_browser::{provision_plan, start_browser_plan};
use eidolon_facet::{CellAccess, Facet, FacetError, Result as FacetResult, RunOutput};
use eidolon_model::{EidolonSpec, FacetKind, FacetSpec};
use std::path::Path;

/// Whether the spec has a browser facet.
pub fn has_browser(spec: &EidolonSpec) -> bool {
    spec.facets.iter().any(|f| f.kind == FacetKind::Browser)
}

/// A browser facet as the CLI uses it.
///
/// - While sealing, it is not started again after it was taken: a sealed
///   eidolon is open nowhere, so its browser stays stopped. The next `open`
///   starts it.
/// - Before a profile is placed, the cell's profile folder is emptied and
///   made over to the cell's login user, who copies the files in. A browser
///   that ran in the cell has left files of its own (a database's journal
///   beside the wrong database would corrupt it), and the copy cannot set
///   the times of folders another user owns. The facet gives the folder
///   back to the browser's user once the files are in. An eidolon sealed
///   with no profile yet leaves the folder as provisioning made it, which
///   keeps the browser's preferences file.
pub struct CellBrowser {
    inner: Box<dyn Facet>,
    keep_stopped: bool,
}

impl CellBrowser {
    pub fn new(inner: Box<dyn Facet>, keep_stopped: bool) -> CellBrowser {
        CellBrowser { inner, keep_stopped }
    }
}

async fn run_ok(cell: &dyn CellAccess, what: &str, argv: &[&str]) -> FacetResult<RunOutput> {
    let out = cell.run(argv).await?;
    if out.status != 0 {
        return Err(FacetError::Cell(format!("{what} exited with status {}: {}", out.status, out.stderr.trim())));
    }
    Ok(out)
}

#[async_trait]
impl Facet for CellBrowser {
    fn kind(&self) -> FacetKind {
        self.inner.kind()
    }

    fn spec(&self) -> &FacetSpec {
        self.inner.spec()
    }

    async fn quiet(&self, cell: &dyn CellAccess) -> FacetResult<()> {
        self.inner.quiet(cell).await
    }

    async fn resume(&self, cell: &dyn CellAccess) -> FacetResult<()> {
        if self.keep_stopped {
            return Ok(());
        }
        self.inner.resume(cell).await
    }

    fn leave_out(&self) -> Vec<String> {
        self.inner.leave_out()
    }

    async fn take(&self, cell: &dyn CellAccess, staging: &Path) -> FacetResult<()> {
        self.inner.take(cell, staging).await
    }

    async fn place(&self, cell: &dyn CellAccess, staging: &Path) -> FacetResult<()> {
        let path = self.inner.spec().path.as_str();
        let has_files = std::fs::read_dir(staging).map_err(|e| FacetError::Cell(format!("Could not read the staged profile: {e}.")))?.next().is_some();
        // Two folders deep at least: never a top-level folder of the cell.
        if has_files && path.split('/').filter(|p| !p.is_empty()).count() >= 2 {
            let user = run_ok(cell, "Asking who the cell's login user is", &["id", "-un"]).await?.stdout;
            run_ok(cell, "Clearing the profile folder", &["sudo", "rm", "-rf", "--", path]).await?;
            run_ok(cell, "Making the profile folder", &["sudo", "install", "-d", "-o", user.trim(), "-m", "0700", path]).await?;
        }
        self.inner.place(cell, staging).await
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

/// Whether the cell has the browser's units. The plan installs them last,
/// so their presence means the plan finished.
async fn provisioned(driver: &dyn Driver, cell: &CellName) -> Result<bool> {
    Ok(driver.exec(cell, &argv(&["systemctl", "cat", BROWSER_UNIT]), ExecOpts::default()).await?.success())
}

/// Sets the cell up for a browser if it is not yet. Returns whether it ran.
pub async fn provision_if_needed(cells: &dyn Cells, cell: &str) -> Result<bool> {
    let (driver, name) = cells.driver(cell).await?;
    if provisioned(driver.as_ref(), &name).await? {
        return Ok(false);
    }
    provision_plan(Ports::default()).run(driver.as_ref(), &name).await?;
    Ok(true)
}

/// Stops the browser's units, so a profile can be placed or cleared. A cell
/// that was never set up has no units, which is as stopped as it gets, so
/// what `systemctl` says is not looked at.
pub async fn stop(access: &dyn CellAccess) -> Result<()> {
    let stop = stop_argv();
    let stop: Vec<&str> = stop.iter().map(String::as_str).collect();
    access.run(&stop).await?;
    Ok(())
}

/// Starts the browser for automation.
pub async fn start(cells: &dyn Cells, cell: &str) -> Result<()> {
    let (driver, name) = cells.driver(cell).await?;
    start_browser_plan().run(driver.as_ref(), &name).await?;
    Ok(())
}
