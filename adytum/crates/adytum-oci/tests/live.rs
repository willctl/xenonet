//! The real thing: a Lima cell with podman installed in it, and `Oci`
//! driving that podman through `InCell`. Skipped unless `ADYTUM_LIVE=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 npm run cargo -- test -p adytum-oci --test live -- --nocapture
//! ```
//!
//! Add `ADYTUM_RECORD_GOLDEN=1` to write the podman golden from this run
//! (`tests/golden/podman-<version>/lifecycle.json`).
//!
//! The Lima cell is `adytum-test-oci-<random>` under Adytum's own folder
//! (`ADYTUM_HOME`). It is removed at the end, also when a step fails. Nothing
//! is installed on the host and the VMs `pane` and `xenonet-*` are never
//! named.

mod common;

use std::path::Path;
use std::sync::Arc;

use adytum_driver::{Driver, ExecOpts};
use adytum_exec::{Cancel, Command, Runner};
use adytum_lima::{adytum_home, lima_home_in, Lima};
use adytum_model::{CellId, CellName, CellSpec, DriverKind, Resources};
use adytum_oci::{InCell, Oci, OciTool};
use adytum_testkit::{RecordingRunner, Transcript};
use serde_json::Value;

type Res = Result<(), Box<dyn std::error::Error>>;

const WORK: &str = "/tmp/adytum-oci-work";

fn live() -> bool {
    std::env::var("ADYTUM_LIVE").as_deref() == Ok("1")
}

/// Removes the Lima cell on drop, from a thread of its own, so it works
/// while the test's runtime is busy unwinding.
struct Cleanup {
    home: std::path::PathBuf,
    name: CellName,
}

impl Cleanup {
    fn run(&self) {
        let (home, name) = (self.home.clone(), self.name.clone());
        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                let _ = rt.block_on(Lima::system(home).remove(&name));
            }
        })
        .join();
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.run();
    }
}

async fn sh(lima: &Lima, name: &CellName, script: &str) -> Result<adytum_exec::Output, adytum_driver::DriverError> {
    let argv = vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()];
    let deadline = std::time::Duration::from_secs(20 * 60);
    lima.exec(name, &argv, ExecOpts { deadline: Some(deadline), ..ExecOpts::default() }).await
}

/// A list answer without the details of the machine: only the fields the
/// driver reads.
fn sanitize_list(stdout: &str) -> String {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(stdout) else { return stdout.to_owned() };
    let kept: Vec<Value> = items
        .into_iter()
        .map(|item| {
            let mut out = serde_json::Map::new();
            for key in ["Names", "State", "Labels", "Image"] {
                if let Some(v) = item.get(key) {
                    out.insert(key.to_owned(), v.clone());
                }
            }
            Value::Object(out)
        })
        .collect();
    Value::Array(kept).to_string()
}

fn sanitize(mut t: Transcript) -> Transcript {
    for call in &mut t.calls {
        if call.argv.get(1).map(String::as_str) == Some("ps") {
            call.stdout = sanitize_list(&call.stdout);
        }
        // Progress lines and pull messages carry timing and layer ids; a failing call keeps its message.
        if call.status == 0 {
            call.stderr.clear();
        }
    }
    t
}

async fn podman(cell: &InCell, args: &[&str]) -> Result<adytum_exec::Output, adytum_exec::ExecError> {
    cell.run(&Command::new("podman").args(args), &Cancel::new()).await
}

#[tokio::test]
async fn podman_in_a_lima_cell() -> Res {
    if !live() {
        eprintln!("skipped: set ADYTUM_LIVE=1 to run against Lima");
        return Ok(());
    }
    let home = lima_home_in(&adytum_home().ok_or("no ADYTUM_HOME or HOME")?);
    let suffix: String = CellId::new_random()?.to_string().chars().skip(5).take(8).collect();
    let name: CellName = format!("adytum-test-oci-{suffix}").parse()?;
    let _guard = Cleanup { home: home.clone(), name: name.clone() };

    // The VM.
    let lima = Arc::new(Lima::system(home));
    let vm = CellSpec {
        name: name.clone(),
        driver: DriverKind::Lima,
        image: adytum_lima::image(adytum_lima::DEFAULT_IMAGE).ok_or("no default Lima image")?,
        wall: "wall_test".parse()?,
        resources: Resources { cpus: 2, memory_mib: 2048, disk_gib: 10 },
        ports: vec![],
        automation_port: None,
        mounts: vec![],
        sealed: true,
        network: None,
        clone_from: None,
        secrets: vec![],
    };
    lima.create(&vm).await?;
    lima.start(&name).await?;
    let out = sh(&lima, &name, "sudo apt-get update -qq && sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq podman").await?;
    assert!(out.success(), "installing podman: {}", out.stderr_lossy());

    // Podman, driven through the cell.
    let cell = InCell::new(lima.clone(), name.clone());
    let version = podman(&cell, &["--version"]).await?.stdout_lossy();
    let version = version.trim().rsplit(' ').next().unwrap_or("unknown").to_owned();
    eprintln!("podman {version}");

    let out = sh(&lima, &name, &format!("mkdir -p {WORK} && printf 'golden bytes\\n' > {WORK}/in.txt")).await?;
    assert!(out.success(), "{}", out.stderr_lossy());

    let recorder = Arc::new(RecordingRunner::new(InCell::new(lima.clone(), name.clone()), "podman", &version).with_var("work", WORK));
    let oci = Oci::new(recorder.clone(), OciTool::Podman, "podman".into());
    common::lifecycle(&oci, DriverKind::Podman, Path::new(WORK)).await?;
    let out = sh(&lima, &name, &format!("cat {WORK}/out.txt")).await?;
    assert_eq!(out.stdout, common::FILE_BYTES);

    // A sealed container has no mount, no network and no capability.
    let oci = Oci::new(Arc::new(InCell::new(lima.clone(), name.clone())), OciTool::Podman, "podman".into());
    let mut spec = common::spec(DriverKind::Podman);
    spec.name = "adytum-test-oci-seal".parse()?;
    oci.create(&spec).await?;
    oci.start(&spec.name).await?;
    let inspect = |format: &'static str| {
        let (cell, n) = (&cell, spec.name.to_string());
        async move { podman(cell, &["inspect", "--format", format, &n]).await.map(|o| o.stdout_lossy().trim().to_owned()) }
    };
    assert_eq!(inspect("{{len .Mounts}}").await?, "0", "mounts");
    assert_eq!(inspect("{{.HostConfig.NetworkMode}}").await?, "none");
    // No capability is left in the container's first process.
    let caps = oci.exec(&spec.name, &["grep".to_owned(), "CapEff".to_owned(), "/proc/self/status".to_owned()], ExecOpts::default()).await?;
    assert!(caps.stdout_lossy().trim().ends_with("0000000000000000"), "{}", caps.stdout_lossy());
    oci.remove(&spec.name).await?;

    // A container Adytum did not make is not listed and not removed.
    let image = format!("docker.io/library/alpine@{}", spec.image.digest);
    let made = podman(&cell, &["create", "--name", "adytum-test-oci-foreign", "--network=none", &image, "sleep", "infinity"]).await?;
    assert!(made.success(), "{}", made.stderr_lossy());
    let listed = oci.list().await?;
    assert!(listed.iter().all(|c| c.name.as_str() != "adytum-test-oci-foreign"), "{listed:?}");
    let refused = oci.remove(&"adytum-test-oci-foreign".parse()?).await;
    assert!(matches!(refused, Err(adytum_driver::DriverError::Refused(_))), "{refused:?}");
    let still = podman(&cell, &["container", "exists", "adytum-test-oci-foreign"]).await?;
    assert!(still.success(), "the foreign container was removed");

    if std::env::var("ADYTUM_RECORD_GOLDEN").as_deref() == Ok("1") {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("podman-{version}"));
        std::fs::create_dir_all(&dir)?;
        sanitize(recorder.transcript()).write_to(&dir.join("lifecycle.json"))?;
    }
    Ok(())
}
