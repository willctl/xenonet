//! What the recording and its replay share: the steps.

use std::path::Path;

use adytum_driver::{Driver, DriverError, ExecOpts};
use adytum_lima::Lima;
use adytum_model::CellState;

use crate::common;

pub const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/lima-2.1.2/lifecycle.json");
pub const CLONE_GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/lima-2.1.2/clone.json");
pub const FILE_BYTES: &[u8] = b"golden bytes\n";

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// One cell's whole life: create, start, run, copy both ways, stop, remove,
/// with a list between the steps. `work` holds `in.txt` and receives
/// `out.txt`.
pub async fn lifecycle(lima: &Lima, work: &Path) -> Result<(), DriverError> {
    let name = common::name();
    assert_eq!(lima.list().await?, Vec::new());

    lima.create(&common::spec()).await?;
    assert_eq!(lima.list().await?[0].state, CellState::Stopped);

    lima.start(&name).await?;
    assert_eq!(lima.list().await?[0].state, CellState::Running);

    let out = lima.exec(&name, &argv(&["uname", "-a"]), ExecOpts::default()).await?;
    assert!(out.success() && out.stdout_lossy().starts_with("Linux"), "{out:?}");

    // Arguments arrive literally: nothing is expanded, split or run.
    let out = lima.exec(&name, &argv(&["printf", "%s\\n", "$(touch /tmp/x)", "a b", ";", "*", "it's"]), ExecOpts::default()).await?;
    assert_eq!(out.stdout_lossy(), "$(touch /tmp/x)\na b\n;\n*\nit's\n");

    lima.copy_in(&name, &work.join("in.txt"), "/tmp/in.txt").await?;
    lima.copy_out(&name, "/tmp/in.txt", &work.join("out.txt")).await?;

    lima.stop(&name).await?;
    assert_eq!(lima.list().await?[0].state, CellState::Stopped);

    lima.remove(&name).await?;
    assert_eq!(lima.list().await?, Vec::new());
    Ok(())
}

/// A cell cloned while stopped: the clone is its own cell, starts, runs a
/// command, and both are removed.
pub async fn clone_lifecycle(lima: &Lima) -> Result<(), DriverError> {
    let (source, copy) = (common::name(), common::clone_name());
    assert_eq!(lima.list().await?, Vec::new());
    lima.create(&common::spec()).await?;

    lima.clone(&source, &common::clone_spec()).await?;
    let listed = lima.list().await?;
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|c| c.state == CellState::Stopped));

    lima.start(&copy).await?;
    let out = lima.exec(&copy, &argv(&["uname", "-a"]), ExecOpts::default()).await?;
    assert!(out.success() && out.stdout_lossy().starts_with("Linux"), "{out:?}");
    lima.stop(&copy).await?;

    lima.remove(&copy).await?;
    lima.remove(&source).await?;
    assert_eq!(lima.list().await?, Vec::new());
    Ok(())
}
