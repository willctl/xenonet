//! What the recording and its replay share: the cell and the steps.

use std::path::Path;

use adytum_driver::{Driver, DriverError, ExecOpts};
use adytum_model::{CellName, CellSpec, CellState, DriverKind, Resources};
use adytum_oci::Oci;

pub const NAME: &str = "adytum-test-golden";
pub const FILE_BYTES: &[u8] = b"golden bytes\n";

pub fn name() -> CellName {
    NAME.parse().expect("a valid name")
}

pub fn spec(driver: DriverKind) -> CellSpec {
    CellSpec {
        name: name(),
        driver,
        image: adytum_oci::image(adytum_oci::DEFAULT_IMAGE).expect("the default image is pinned"),
        wall: "wall_test".parse().expect("a valid wall"),
        resources: Resources { cpus: 1, memory_mib: 256, disk_gib: 1 },
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: true,
        network: None,
        clone_from: None,
        secrets: vec![],
    }
}

pub fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_owned()).collect()
}

/// One container's whole life: create, start, run, copy both ways, stop,
/// remove, with a list between the steps. `work` holds `in.txt` and receives
/// `out.txt`.
pub async fn lifecycle(oci: &Oci, driver: DriverKind, work: &Path) -> Result<(), DriverError> {
    let name = name();
    assert_eq!(oci.list().await?, Vec::new());

    oci.create(&spec(driver)).await?;
    assert_eq!(oci.list().await?[0].state, CellState::Stopped);

    oci.start(&name).await?;
    assert_eq!(oci.list().await?[0].state, CellState::Running);

    let out = oci.exec(&name, &argv(&["cat", "/etc/os-release"]), ExecOpts::default()).await?;
    assert!(out.success() && out.stdout_lossy().contains("Alpine"), "{out:?}");

    // Arguments arrive literally: nothing is expanded, split or run.
    let out = oci.exec(&name, &argv(&["printf", "%s\\n", "$(touch /tmp/x)", "a b", ";", "*", "it's", "-n"]), ExecOpts::default()).await?;
    assert_eq!(out.stdout_lossy(), "$(touch /tmp/x)\na b\n;\n*\nit's\n-n\n");

    oci.copy_in(&name, &work.join("in.txt"), "/tmp/in.txt").await?;
    oci.copy_out(&name, "/tmp/in.txt", &work.join("out.txt")).await?;

    oci.stop(&name).await?;
    assert_eq!(oci.list().await?[0].state, CellState::Stopped);

    oci.remove(&name).await?;
    assert_eq!(oci.list().await?, Vec::new());
    Ok(())
}
