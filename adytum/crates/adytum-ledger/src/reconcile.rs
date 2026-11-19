use adytum_driver::{Driver, DriverError};
use adytum_model::{CellId, CellName, CellSpec, Refusal, RefusalCode};

use crate::{Ledger, LedgerError, OpKind, Outcome};

/// What [`reconcile`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Cells taken out: half-made ones from an interrupted create, and
    /// ledger rows whose cell the driver no longer has.
    pub removed: Vec<CellName>,
    /// Removals that were interrupted and are now done.
    pub finished: Vec<CellName>,
    /// Cells the driver reports that the ledger never recorded. Reported and
    /// never touched: Adytum deletes only what it wrote down.
    pub adopted_none: Vec<CellName>,
}

// A cell that is already gone is the state a remove wants.
async fn remove_tolerant(driver: &dyn Driver, name: &CellName) -> Result<(), LedgerError> {
    match driver.remove(name).await {
        Ok(()) | Err(DriverError::NotFound { .. }) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Run at every start. Settles each unsettled operation and then makes the
/// ledger and the driver agree:
///
/// - an unsettled create may have left a half-made cell: it is removed if
///   listed, its row deleted, the operation settled as interrupted;
/// - an unsettled remove is run again and its row deleted;
/// - an unsettled start or stop is settled as interrupted;
/// - a recorded cell the driver no longer lists loses its row;
/// - a listed cell the ledger never recorded is reported, never touched.
///
/// If the driver fails, the operation stays open and the next start tries
/// again.
pub async fn reconcile(ledger: &Ledger, driver: &dyn Driver) -> Result<Report, LedgerError> {
    let mut report = Report::default();
    for op in ledger.open_ops()? {
        match op.kind {
            OpKind::Create => {
                remove_tolerant(driver, &op.name).await?;
                ledger.settle(op.id, Outcome::Failed("interrupted".to_owned()))?;
                report.removed.push(op.name);
            }
            OpKind::Remove => {
                remove_tolerant(driver, &op.name).await?;
                ledger.settle(op.id, Outcome::Done)?;
                report.finished.push(op.name);
            }
            OpKind::Start | OpKind::Stop => ledger.settle(op.id, Outcome::Failed("interrupted".to_owned()))?,
        }
    }
    let listed = driver.list().await?;
    for cell in ledger.cells()? {
        let name = &cell.spec.name;
        match listed.iter().find(|l| &l.name == name) {
            None => {
                ledger.forget(name)?;
                report.removed.push(name.clone());
            }
            Some(l) if l.state != cell.state => ledger.set_state(name, l.state)?,
            Some(_) => {}
        }
    }
    let known: Vec<CellName> = ledger.cells()?.into_iter().map(|c| c.spec.name).collect();
    report.adopted_none = listed.into_iter().map(|l| l.name).filter(|n| !known.contains(n)).collect();
    Ok(report)
}

/// Makes a cell: checks the rules against the ledger's world (a refusal
/// happens before anything is written), writes the intent, runs the driver,
/// settles. If the driver fails, the half-made cell is removed first; if
/// that also fails the operation stays open for the next [`reconcile`].
pub async fn create_cell(ledger: &Ledger, driver: &dyn Driver, spec: &CellSpec) -> Result<CellId, LedgerError> {
    adytum_policy::check_create(spec, &ledger.world()?)?;
    // A cell the driver has under this name and the ledger does not know is
    // not ours to replace or to clean up after a failed create.
    if driver.list().await?.iter().any(|l| l.name == spec.name) {
        return Err(Refusal::new(
            RefusalCode::NameTaken,
            format!("The driver already has a cell named {} that Adytum did not make.", spec.name),
            "Pick another name; Adytum does not touch cells it did not record.",
        )
        .into());
    }
    let op = ledger.intend_create(spec)?;
    match driver.create(spec).await {
        Ok(()) => {
            ledger.settle(op, Outcome::Done)?;
            ledger.cells()?.into_iter().find(|c| c.spec.name == spec.name).map(|c| c.id).ok_or_else(|| LedgerError::NotRecorded(spec.name.clone()))
        }
        Err(e) => {
            if remove_tolerant(driver, &spec.name).await.is_ok() {
                ledger.settle(op, Outcome::Failed(e.to_string()))?;
            }
            Err(e.into())
        }
    }
}

/// Removes a recorded cell: writes the intent, runs the driver, settles. A
/// cell that is already gone counts as removed. A cell the ledger never
/// recorded is refused. If the driver fails the cell is marked `Broken`.
pub async fn remove_cell(ledger: &Ledger, driver: &dyn Driver, name: &CellName) -> Result<(), LedgerError> {
    let op = ledger.intend(OpKind::Remove, name)?;
    match remove_tolerant(driver, name).await {
        Ok(()) => ledger.settle(op, Outcome::Done),
        Err(e) => {
            ledger.settle(op, Outcome::Failed(e.to_string()))?;
            Err(e)
        }
    }
}
