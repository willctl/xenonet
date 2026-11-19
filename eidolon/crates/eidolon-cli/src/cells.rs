//! Where a cell's wall and files come from. Adytum is the only place that
//! knows which wall a cell is behind: its ledger records it when the cell
//! is made. Files move through Adytum's driver.
//!
//! [`LocalCells`] is for tests (the hidden `--cell-root` option): a cell is
//! a folder under one root and its wall is written in `<cell>.wall`.

use crate::error::{CliError, Result};
use adytum_driver::{cell_name, Driver, ExecOpts};
use adytum_ledger::Ledger;
use adytum_model::{CellName, CellState};
use async_trait::async_trait;
use eidolon_facet::{CellAccess, FacetError, LocalCell, RunOutput};
use eidolon_model::{EidolonId, Wall};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[async_trait]
pub trait Cells: Send + Sync {
    /// The wall `cell` is behind.
    async fn wall(&self, cell: &str) -> Result<Wall>;
    /// Gets the cell ready (running) and returns the way to its files.
    async fn access(&self, cell: &str) -> Result<Box<dyn CellAccess>>;
    /// Records that `cell` holds `eidolon` open.
    fn hold(&self, cell: &str, eidolon: &EidolonId) -> Result<()>;
    /// Records that `cell` no longer holds `eidolon`.
    fn release(&self, cell: &str, eidolon: &EidolonId) -> Result<()>;
}

// ---- tests: folders ------------------------------------------------------

pub struct LocalCells {
    root: PathBuf,
}

impl LocalCells {
    pub fn new(root: PathBuf) -> LocalCells {
        LocalCells { root }
    }

    fn check(cell: &str) -> Result<()> {
        if cell.is_empty() || cell.contains(['/', '\\']) || cell.starts_with('.') {
            return Err(CliError::failed("unknown_cell", format!("{cell:?} is not a cell name."), "Use the name of a folder under the cell root."));
        }
        Ok(())
    }
}

#[async_trait]
impl Cells for LocalCells {
    async fn wall(&self, cell: &str) -> Result<Wall> {
        Self::check(cell)?;
        let text = std::fs::read_to_string(self.root.join(format!("{cell}.wall")))
            .map_err(|_| CliError::failed("unknown_cell", format!("There is no cell {cell}."), "Name a cell that has a wall record."))?;
        Ok(text.trim().parse()?)
    }

    async fn access(&self, cell: &str) -> Result<Box<dyn CellAccess>> {
        Self::check(cell)?;
        let dir = self.root.join(cell);
        std::fs::create_dir_all(&dir).map_err(|e| CliError::io("make the cell's folder", &e))?;
        Ok(Box::new(LocalCell::new(dir)))
    }

    fn hold(&self, _cell: &str, _eidolon: &EidolonId) -> Result<()> {
        Ok(())
    }

    fn release(&self, _cell: &str, _eidolon: &EidolonId) -> Result<()> {
        Ok(())
    }
}

// ---- Adytum --------------------------------------------------------------

pub struct AdytumCells {
    ledger: Ledger,
    driver: Arc<dyn Driver>,
}

impl AdytumCells {
    pub fn new(ledger: Ledger, driver: Arc<dyn Driver>) -> AdytumCells {
        AdytumCells { ledger, driver }
    }

    /// Adytum's folder (`$ADYTUM_HOME`, else `~/.adytum`) with its ledger
    /// and its Lima.
    pub fn system() -> Result<AdytumCells> {
        let home = adytum_lima::adytum_home().ok_or_else(|| CliError::failed("no_home", "There is no home folder for Adytum.", "Set ADYTUM_HOME to a folder."))?;
        std::fs::create_dir_all(&home).map_err(|e| CliError::io("make Adytum's folder", &e))?;
        let ledger = Ledger::open(&home)?;
        let driver = adytum_lima::Lima::system(adytum_lima::lima_home_in(&home));
        Ok(AdytumCells::new(ledger, Arc::new(driver)))
    }

    fn name(cell: &str) -> Result<CellName> {
        Ok(cell_name(cell)?)
    }

    fn recorded(&self, name: &CellName) -> Result<adytum_model::Cell> {
        self.ledger
            .cells()?
            .into_iter()
            .find(|c| c.spec.name == *name)
            .ok_or_else(|| CliError::failed("unknown_cell", format!("Adytum has no record of a cell named {name}."), "Run `adytum ls` to see the cells."))
    }
}

#[async_trait]
impl Cells for AdytumCells {
    async fn wall(&self, cell: &str) -> Result<Wall> {
        let name = Self::name(cell)?;
        Ok(self.recorded(&name)?.spec.wall.to_string().parse()?)
    }

    async fn access(&self, cell: &str) -> Result<Box<dyn CellAccess>> {
        let name = Self::name(cell)?;
        let recorded = self.recorded(&name)?;
        if recorded.state == CellState::Broken {
            return Err(CliError::failed("cell_broken", format!("{name} is broken."), "Run `adytum doctor` or remove the cell."));
        }
        let listed = self.driver.list().await?.into_iter().find(|l| l.name == name);
        match listed.map(|l| l.state) {
            Some(CellState::Running) => {}
            Some(CellState::Stopped) => self.driver.start(&name).await?,
            _ => return Err(CliError::failed("cell_missing", format!("The driver has no usable cell named {name}."), "Run `adytum ls` to see what exists.")),
        }
        Ok(Box::new(DriverAccess { driver: self.driver.clone(), name }))
    }

    fn hold(&self, cell: &str, eidolon: &EidolonId) -> Result<()> {
        Ok(self.ledger.hold(&Self::name(cell)?, &eidolon.to_string())?)
    }

    fn release(&self, cell: &str, eidolon: &EidolonId) -> Result<()> {
        Ok(self.ledger.release(&Self::name(cell)?, &eidolon.to_string())?)
    }
}

/// A cell's files through Adytum's driver. The driver copies with rsync, so
/// a folder copied into an existing folder puts its contents there, which is
/// what [`CellAccess`] asks for. Nothing is deleted on either side: a file
/// the cell has and the eidolon does not stays in the cell.
struct DriverAccess {
    driver: Arc<dyn Driver>,
    name: CellName,
}

fn cell_err(e: impl std::fmt::Display) -> FacetError {
    FacetError::Cell(e.to_string())
}

impl DriverAccess {
    async fn exec(&self, argv: &[&str]) -> std::result::Result<RunOutput, FacetError> {
        let argv: Vec<String> = argv.iter().map(|a| (*a).to_owned()).collect();
        let out = self.driver.exec(&self.name, &argv, ExecOpts::default()).await.map_err(cell_err)?;
        Ok(RunOutput { status: out.status.unwrap_or(-1), stdout: out.stdout_lossy(), stderr: out.stderr_lossy() })
    }
}

#[async_trait]
impl CellAccess for DriverAccess {
    async fn copy_out(&self, guest_path: &str, host_dir: &Path) -> std::result::Result<(), FacetError> {
        std::fs::create_dir_all(host_dir).map_err(|e| FacetError::Cell(format!("could not make a folder: {e}")))?;
        self.driver.copy_out(&self.name, guest_path, host_dir).await.map_err(cell_err)
    }

    async fn copy_in(&self, host_dir: &Path, guest_path: &str) -> std::result::Result<(), FacetError> {
        let made = self.exec(&["mkdir", "-p", guest_path]).await?;
        if made.status != 0 {
            return Err(FacetError::Cell(format!("could not make {guest_path} in {}: {}", self.name, made.stderr.trim())));
        }
        self.driver.copy_in(&self.name, host_dir, guest_path).await.map_err(cell_err)
    }

    async fn run(&self, argv: &[&str]) -> std::result::Result<RunOutput, FacetError> {
        self.exec(argv).await
    }
}
