use crate::cell::CellAccess;
use crate::error::{FacetError, Result};
use async_trait::async_trait;
use eidolon_model::{FacetKind, FacetSpec};
use std::path::Path;

/// One part of an eidolon's state, moved between a cell and a host folder.
#[async_trait]
pub trait Facet: Send + Sync {
    fn kind(&self) -> FacetKind;
    fn spec(&self) -> &FacetSpec;
    /// Gets the facet's files into a state that can be copied (a browser
    /// flushes its profile). Called before `take`.
    async fn quiet(&self, cell: &dyn CellAccess) -> Result<()>;
    /// Lets the facet run again. Called after `take`, even if `take` failed.
    async fn resume(&self, cell: &dyn CellAccess) -> Result<()>;
    /// Patterns for what the facet does not carry, relative to its root.
    fn leave_out(&self) -> Vec<String>;
    /// Copies the facet's files out of the cell into `staging`.
    async fn take(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()>;
    /// Copies the files in `staging` into the cell where the facet lives.
    async fn place(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()>;
}

/// A plain folder: its files are carried as they are.
#[derive(Debug, Clone)]
pub struct FolderFacet {
    spec: FacetSpec,
}

impl FolderFacet {
    /// A folder facet for `spec`, which must be a folder spec in its
    /// formats with leave-out patterns that stay inside the facet.
    pub fn new(spec: FacetSpec) -> Result<FolderFacet> {
        if spec.kind != FacetKind::Folder {
            return Err(FacetError::WrongKind { expected: FacetKind::Folder });
        }
        spec.validate().map_err(|e| FacetError::Cell(e.to_string()))?;
        for pattern in &spec.leave_out {
            eidolon_policy::leave_out_is_inside(pattern)?;
        }
        Ok(FolderFacet { spec })
    }
}

#[async_trait]
impl Facet for FolderFacet {
    fn kind(&self) -> FacetKind {
        FacetKind::Folder
    }

    fn spec(&self) -> &FacetSpec {
        &self.spec
    }

    async fn quiet(&self, _cell: &dyn CellAccess) -> Result<()> {
        Ok(())
    }

    async fn resume(&self, _cell: &dyn CellAccess) -> Result<()> {
        Ok(())
    }

    fn leave_out(&self) -> Vec<String> {
        self.spec.leave_out.clone()
    }

    async fn take(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()> {
        cell.copy_out(&self.spec.path, staging).await
    }

    async fn place(&self, cell: &dyn CellAccess, staging: &Path) -> Result<()> {
        cell.copy_in(staging, &self.spec.path).await
    }
}
