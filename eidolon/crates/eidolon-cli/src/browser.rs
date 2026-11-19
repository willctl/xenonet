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
use eidolon_facet::{CellAccess, Facet, Result as FacetResult};
use eidolon_model::{EidolonSpec, FacetKind, FacetSpec};
use std::path::Path;

/// Whether the spec has a browser facet.
pub fn has_browser(spec: &EidolonSpec) -> bool {
    spec.facets.iter().any(|f| f.kind == FacetKind::Browser)
}

/// A browser facet as the CLI uses it.
///
/// While sealing, it is not started again after it was taken: a sealed
/// eidolon is open nowhere, so its browser stays stopped. The next `open`
/// starts it.
pub struct CellBrowser {
    inner: Box<dyn Facet>,
    keep_stopped: bool,
}

impl CellBrowser {
    pub fn new(inner: Box<dyn Facet>, keep_stopped: bool) -> CellBrowser {
        CellBrowser { inner, keep_stopped }
    }
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
