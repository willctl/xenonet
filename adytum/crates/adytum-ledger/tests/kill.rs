//! Kill tests: an operation is cut at each point a crash could cut it, the
//! ledger is reopened from its file, and `reconcile` must leave the ledger
//! and the driver agreeing with nothing of the cut operation behind.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Duplex, Output};
use adytum_ledger::{create_cell, reconcile, remove_cell, Ledger, LedgerError, OpKind, Outcome};
use adytum_model::{Arch, CellName, CellSpec, CellState, DriverKind, ImagePin, RefusalCode, Resources};
use async_trait::async_trait;

type T = Result<(), Box<dyn Error>>;

fn name(n: &str) -> Result<CellName, Box<dyn Error>> {
    Ok(n.parse()?)
}

fn spec(n: &str) -> Result<CellSpec, Box<dyn Error>> {
    Ok(CellSpec {
        name: name(n)?,
        driver: DriverKind::Lima,
        image: ImagePin::new("https://example.test/a.img".into(), format!("sha256:{}", "0".repeat(64)), Arch::Aarch64)?,
        wall: "wall_a".parse()?,
        resources: Resources::default(),
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: false,
        network: None,
        clone_from: None,
        secrets: vec![],
    })
}

/// Where a driver call dies. Dying returns an error and leaves the
/// in-memory state as a `kill -9` at that point would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// `create`: the tool never ran.
    CreateBeforeTool,
    /// `create`: the tool began and left a half-made cell.
    CreateHalfMade,
    /// `create`: the tool made the cell and died before returning.
    CreateAfterMade,
    /// `remove`: the tool never ran.
    RemoveBeforeTool,
    /// `remove`: the tool began and left the cell half-removed.
    RemoveStarted,
    /// `remove`: the cell is gone and the call died before returning.
    RemoveAfterGone,
}

#[derive(Default)]
struct FlakyDriver {
    cells: Mutex<BTreeMap<CellName, CellState>>,
    cut: Mutex<Option<Cut>>,
}

impl FlakyDriver {
    fn with_foreign(n: &str) -> Result<Self, Box<dyn Error>> {
        let d = Self::default();
        d.state().insert(name(n)?, CellState::Running);
        Ok(d)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, BTreeMap<CellName, CellState>> {
        self.cells.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn die_at(&self, cut: Cut) {
        *self.cut.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cut);
    }

    fn take(&self, cut: Cut) -> bool {
        let mut g = self.cut.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *g == Some(cut) {
            *g = None;
            true
        } else {
            false
        }
    }

    fn died(what: &str) -> DriverError {
        DriverError::Tool { what: what.to_owned(), status: None, stderr: "killed".to_owned() }
    }
}

#[async_trait]
impl Driver for FlakyDriver {
    async fn create(&self, spec: &CellSpec) -> Result<(), DriverError> {
        if self.take(Cut::CreateBeforeTool) {
            return Err(Self::died("create"));
        }
        if self.take(Cut::CreateHalfMade) {
            self.state().insert(spec.name.clone(), CellState::Broken);
            return Err(Self::died("create"));
        }
        self.state().insert(spec.name.clone(), CellState::Stopped);
        if self.take(Cut::CreateAfterMade) {
            return Err(Self::died("create"));
        }
        Ok(())
    }

    async fn start(&self, n: &CellName) -> Result<(), DriverError> {
        self.state().get_mut(n).map(|s| *s = CellState::Running).ok_or(DriverError::NotFound { name: n.to_string() })
    }

    async fn exec(&self, _: &CellName, _: &[String], _: ExecOpts) -> Result<Output, DriverError> {
        Err(DriverError::Unsupported { what: "exec in the test double".to_owned() })
    }

    async fn stream(&self, _: &CellName, _: &[String]) -> Result<Duplex, DriverError> {
        Err(DriverError::Unsupported { what: "stream in the test double".to_owned() })
    }

    async fn copy_in(&self, _: &CellName, _: &Path, _: &str) -> Result<(), DriverError> {
        Ok(())
    }

    async fn copy_out(&self, _: &CellName, _: &str, _: &Path) -> Result<(), DriverError> {
        Ok(())
    }

    async fn stop(&self, n: &CellName) -> Result<(), DriverError> {
        self.state().get_mut(n).map(|s| *s = CellState::Stopped).ok_or(DriverError::NotFound { name: n.to_string() })
    }

    async fn remove(&self, n: &CellName) -> Result<(), DriverError> {
        if self.take(Cut::RemoveBeforeTool) {
            return Err(Self::died("remove"));
        }
        if self.take(Cut::RemoveStarted) {
            self.state().insert(n.clone(), CellState::Removing);
            return Err(Self::died("remove"));
        }
        if self.state().remove(n).is_none() {
            return Err(DriverError::NotFound { name: n.to_string() });
        }
        if self.take(Cut::RemoveAfterGone) {
            return Err(Self::died("remove"));
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        Ok(self.state().iter().map(|(name, state)| Listed { name: name.clone(), state: *state }).collect())
    }
}

fn scratch(tag: &str) -> Result<PathBuf, Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("adytum-ledger-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The ledger and the driver hold the same cells in the same states and
/// nothing is left open.
fn assert_agree(ledger: &Ledger, driver: &FlakyDriver, foreign: &[&str]) -> T {
    assert!(ledger.open_ops()?.is_empty(), "an operation is still open");
    let ours: Vec<(CellName, CellState)> = ledger.cells()?.into_iter().map(|c| (c.spec.name, c.state)).collect();
    let theirs: Vec<(CellName, CellState)> = driver.state().iter().filter(|(n, _)| !foreign.contains(&n.as_str())).map(|(n, s)| (n.clone(), *s)).collect();
    assert_eq!(ours, theirs);
    for f in foreign {
        assert_eq!(driver.state().get(&name(f)?), Some(&CellState::Running), "{f} was touched");
    }
    Ok(())
}

const FOREIGN: &str = "adytum-foreign";

// The op ran, the process died: the ledger handle is dropped unsettled and
// reopened from the same file, as after a restart.
fn restart(dir: &Path) -> Result<Ledger, Box<dyn Error>> {
    Ok(Ledger::open(dir)?)
}

async fn kill_create(cut: Cut, tag: &str) -> T {
    let dir = scratch(tag)?;
    let driver = FlakyDriver::with_foreign(FOREIGN)?;
    let want = spec("adytum-test-c")?;
    {
        let ledger = Ledger::open(&dir)?;
        driver.die_at(cut);
        ledger.intend_create(&want)?;
        assert!(driver.create(&want).await.is_err());
        // dropped without settling
    }
    let ledger = restart(&dir)?;
    assert_eq!(ledger.open_ops()?.len(), 1);
    let report = reconcile(&ledger, &driver).await?;
    assert_eq!(report.removed, vec![want.name.clone()]);
    assert_eq!(report.adopted_none, vec![name(FOREIGN)?]);
    assert!(ledger.cells()?.is_empty(), "the cut create left a row");
    assert!(driver.state().get(&want.name).is_none(), "the cut create left a cell");
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    // The same name can be made again.
    create_cell(&ledger, &driver, &want).await?;
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    assert_eq!(ledger.cells()?.first().map(|c| c.state), Some(CellState::Stopped));
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[tokio::test]
async fn create_cut_before_the_tool_runs() -> T {
    kill_create(Cut::CreateBeforeTool, "c1").await
}

#[tokio::test]
async fn create_cut_with_a_half_made_cell() -> T {
    kill_create(Cut::CreateHalfMade, "c2").await
}

#[tokio::test]
async fn create_cut_after_the_tool_made_the_cell() -> T {
    kill_create(Cut::CreateAfterMade, "c3").await
}

async fn kill_remove(cut: Cut, tag: &str) -> T {
    let dir = scratch(tag)?;
    let driver = FlakyDriver::with_foreign(FOREIGN)?;
    let cell = spec("adytum-test-r")?;
    {
        let ledger = Ledger::open(&dir)?;
        create_cell(&ledger, &driver, &cell).await?;
        driver.die_at(cut);
        ledger.intend(OpKind::Remove, &cell.name)?;
        assert!(driver.remove(&cell.name).await.is_err());
    }
    let ledger = restart(&dir)?;
    assert_eq!(ledger.open_ops()?.len(), 1);
    let report = reconcile(&ledger, &driver).await?;
    assert_eq!(report.finished, vec![cell.name.clone()]);
    assert!(ledger.cells()?.is_empty(), "the cut remove left a row");
    assert!(driver.state().get(&cell.name).is_none(), "the cut remove left a cell");
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    create_cell(&ledger, &driver, &cell).await?;
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[tokio::test]
async fn remove_cut_before_the_tool_runs() -> T {
    kill_remove(Cut::RemoveBeforeTool, "r1").await
}

#[tokio::test]
async fn remove_cut_after_it_started() -> T {
    kill_remove(Cut::RemoveStarted, "r2").await
}

#[tokio::test]
async fn remove_cut_after_the_cell_was_gone() -> T {
    kill_remove(Cut::RemoveAfterGone, "r3").await
}

#[tokio::test]
async fn a_failed_create_cleans_up_through_the_helper() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let want = spec("adytum-test-f")?;
    driver.die_at(Cut::CreateHalfMade);
    assert!(matches!(create_cell(&ledger, &driver, &want).await, Err(LedgerError::Driver(_))));
    assert_agree(&ledger, &driver, &[])?;
    create_cell(&ledger, &driver, &want).await?;
    assert_agree(&ledger, &driver, &[])?;
    Ok(())
}

#[tokio::test]
async fn a_failed_remove_through_the_helper_marks_the_cell_broken_and_can_be_retried() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let cell = spec("adytum-test-b")?;
    create_cell(&ledger, &driver, &cell).await?;
    driver.die_at(Cut::RemoveBeforeTool);
    assert!(remove_cell(&ledger, &driver, &cell.name).await.is_err());
    assert_eq!(ledger.cells()?.first().map(|c| c.state), Some(CellState::Broken));
    remove_cell(&ledger, &driver, &cell.name).await?;
    assert_agree(&ledger, &driver, &[])?;
    Ok(())
}

#[tokio::test]
async fn a_ledger_cell_the_driver_lost_loses_its_row_and_a_foreign_cell_is_only_reported() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::with_foreign(FOREIGN)?;
    let cell = spec("adytum-test-g")?;
    create_cell(&ledger, &driver, &cell).await?;
    driver.state().remove(&cell.name);
    let report = reconcile(&ledger, &driver).await?;
    assert_eq!(report.removed, vec![cell.name.clone()]);
    assert_eq!(report.adopted_none, vec![name(FOREIGN)?]);
    assert_eq!(ledger.cells()?.len(), 0);
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    Ok(())
}

#[tokio::test]
async fn reconcile_syncs_a_changed_state() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let cell = spec("adytum-test-s")?;
    create_cell(&ledger, &driver, &cell).await?;
    driver.start(&cell.name).await?;
    reconcile(&ledger, &driver).await?;
    assert_eq!(ledger.cells()?.first().map(|c| c.state), Some(CellState::Running));
    Ok(())
}

#[tokio::test]
async fn an_interrupted_start_is_settled_and_the_state_follows_the_driver() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let cell = spec("adytum-test-t")?;
    create_cell(&ledger, &driver, &cell).await?;
    ledger.intend(OpKind::Start, &cell.name)?;
    reconcile(&ledger, &driver).await?;
    assert_eq!(ledger.open_ops()?.len(), 0);
    assert_eq!(ledger.cells()?.first().map(|c| c.state), Some(CellState::Stopped));
    Ok(())
}

#[tokio::test]
async fn a_cell_the_ledger_never_recorded_is_never_removed_or_replaced() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::with_foreign(FOREIGN)?;
    // Same name as the foreign cell: refused, and the foreign cell stays.
    let clash = spec(FOREIGN)?;
    match create_cell(&ledger, &driver, &clash).await {
        Err(LedgerError::Refused(r)) => assert_eq!(r.code, RefusalCode::NameTaken),
        other => panic!("{other:?}"),
    }
    assert!(ledger.cells()?.is_empty() && ledger.open_ops()?.is_empty());
    assert!(matches!(remove_cell(&ledger, &driver, &name(FOREIGN)?).await, Err(LedgerError::NotRecorded(_))));
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    Ok(())
}

#[tokio::test]
async fn a_refusal_comes_before_any_intent() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let mut bad = spec("adytum-test-p")?;
    bad.sealed = true;
    bad.mounts.push(adytum_model::Mount { source: "/tmp".into(), target: "/mnt".into(), writable: false });
    assert!(matches!(create_cell(&ledger, &driver, &bad).await, Err(LedgerError::Refused(_))));
    assert!(ledger.open_ops()?.is_empty() && ledger.cells()?.is_empty() && driver.state().is_empty());
    Ok(())
}

#[tokio::test]
async fn a_cell_that_holds_an_eidolon_cannot_be_cloned() -> T {
    let ledger = Ledger::memory()?;
    let driver = FlakyDriver::default();
    let base = spec("adytum-test-h")?;
    create_cell(&ledger, &driver, &base).await?;
    ledger.hold(&base.name, "eid_one")?;
    let mut copy = spec("adytum-test-h2")?;
    copy.clone_from = Some(base.name.clone());
    match create_cell(&ledger, &driver, &copy).await {
        Err(LedgerError::Refused(r)) => assert_eq!(r.code, RefusalCode::CloneOpenEidolon),
        other => panic!("{other:?}"),
    }
    ledger.release(&base.name, "eid_one")?;
    create_cell(&ledger, &driver, &copy).await?;
    Ok(())
}

#[test]
fn a_settled_operation_cannot_be_settled_twice() -> T {
    let ledger = Ledger::memory()?;
    let op = ledger.intend_create(&spec("adytum-test-d")?)?;
    ledger.settle(op, Outcome::Done)?;
    assert!(matches!(ledger.settle(op, Outcome::Done), Err(LedgerError::AlreadySettled(_))));
    Ok(())
}

#[test]
fn a_ledger_with_another_version_is_refused_and_left_alone() -> T {
    let dir = scratch("version")?;
    drop(Ledger::open(&dir)?);
    let file = dir.join(adytum_ledger::FILE);
    {
        let c = rusqlite::Connection::open(&file)?;
        c.pragma_update(None, "user_version", 12345)?;
    }
    let before = std::fs::read(&file)?;
    match Ledger::open(&dir) {
        Err(LedgerError::SetUpAgain(why)) => assert!(why.contains("set Adytum up again"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read(&file)?, before);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn two_handles_on_one_file_share_one_order_of_writes() -> T {
    let dir = scratch("two")?;
    let a = Ledger::open(&dir)?;
    let b = Ledger::open(&dir)?;
    let specs: Vec<CellSpec> = (0..20).map(|i| spec(&format!("adytum-test-w{i}"))).collect::<Result<_, _>>()?;
    std::thread::scope(|s| -> T {
        let ha = s.spawn(|| specs.iter().step_by(2).try_for_each(|sp| a.intend_create(sp).map(drop)));
        let hb = s.spawn(|| specs.iter().skip(1).step_by(2).try_for_each(|sp| b.intend_create(sp).map(drop)));
        ha.join().map_err(|_| "thread a")??;
        hb.join().map_err(|_| "thread b")??;
        Ok(())
    })?;
    assert_eq!(a.cells()?.len(), 20);
    assert_eq!(b.open_ops()?.len(), 20);
    // The same name through the other handle is refused, not doubled.
    assert!(matches!(b.intend_create(&specs[0]), Err(LedgerError::Exists(_))));
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

const HELPER_ENV: &str = "ADYTUM_LEDGER_HELPER";

/// Runs only as the child of `a_real_process_killed_after_intend_is_reconciled`.
#[test]
fn helper() -> T {
    let Ok(dir) = std::env::var(HELPER_ENV) else { return Ok(()) };
    let ledger = Ledger::open(Path::new(&dir))?;
    ledger.intend_create(&spec("adytum-test-kill")?)?;
    std::process::abort();
}

#[tokio::test]
async fn a_real_process_killed_after_intend_is_reconciled() -> T {
    let dir = scratch("proc")?;
    let status = std::process::Command::new(std::env::current_exe()?).args(["--exact", "helper", "--nocapture"]).env(HELPER_ENV, &dir).status()?;
    assert!(!status.success(), "the helper should have aborted");
    let ledger = Ledger::open(&dir)?;
    let ops = ledger.open_ops()?;
    assert_eq!(ops.len(), 1, "the intent was written before the crash");
    // The cell the dead process may have made: the driver has it half-made.
    let driver = FlakyDriver::with_foreign(FOREIGN)?;
    driver.state().insert(name("adytum-test-kill")?, CellState::Broken);
    let report = reconcile(&ledger, &driver).await?;
    assert_eq!(report.removed, vec![name("adytum-test-kill")?]);
    assert_agree(&ledger, &driver, &[FOREIGN])?;
    create_cell(&ledger, &driver, &spec("adytum-test-kill")?).await?;
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}
