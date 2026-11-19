//! Runs a start in an Adytum cell: the engine's `CellHost` on a driver.
//!
//! One run makes a cell from a pinned image behind the plan's wall (sealed,
//! no mounts, no published ports), copies the project folder in, runs the
//! program with `env -i` and exactly the plan's environment, copies the
//! result out if asked, and removes the cell. Every step goes through
//! Adytum's ledger, so `adytum ls` and the doctor see the cell, and a run
//! that dies midway is cleaned up by the next reconcile.
//!
//! The cell is named with a prefix `name_is_ours` accepts: `adytum-xn-` in
//! production, `adytum-test-` in tests (set with [`CellRunner::with_prefix`]).
//!
//! The project is copied, never mounted. It lands in the cell at
//! `/tmp/xn-in/<folder name>`, and a copy-out lands at
//! `<copy_out>/<folder name>`.

use adytum_driver::{Driver, DriverError, ExecOpts};
use adytum_ledger::{create_cell, reconcile, remove_cell, Ledger, OpKind, Outcome};
use adytum_model::{CellId, CellName, CellSpec, DriverKind, ImagePin, Resources};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use xenonet_engine::confine::{CellApp, CellHost, CellPlan, CellRun, ConfineError, CELL_HOME};

/// Where the project copy goes in the cell: a folder the copy lands in.
const GUEST_IN: &str = "/tmp/xn-in";

/// A run with no deadline of its own ends after this long.
const DEFAULT_DEADLINE: Duration = Duration::from_mins(30);

/// The cell name prefix in production.
pub const PRODUCTION_PREFIX: &str = "adytum-xn-";

/// Runs plans in cells made by `driver`, recorded in the ledger under `home`.
pub struct CellRunner {
    driver: Arc<dyn Driver>,
    driver_kind: DriverKind,
    home: PathBuf,
    prefix: String,
    image: ImagePin,
    apps: Vec<CellApp>,
}

impl CellRunner {
    /// Lima cells under `adytum_home` (its ledger and `lima/` folder), from
    /// the pinned default image. `apps` is what that image holds.
    pub fn lima(adytum_home: PathBuf, apps: Vec<CellApp>) -> Result<Self, ConfineError> {
        let image = adytum_lima::image(adytum_lima::DEFAULT_IMAGE).ok_or_else(|| ConfineError::Host("the default cell image is not pinned".into()))?;
        let driver = adytum_lima::Lima::system(adytum_lima::lima_home_in(&adytum_home));
        Ok(Self::with_driver(Arc::new(driver), DriverKind::Lima, adytum_home, image, apps))
    }

    /// Any driver, for another kind of cell or a test's fake.
    pub fn with_driver(driver: Arc<dyn Driver>, driver_kind: DriverKind, home: PathBuf, image: ImagePin, apps: Vec<CellApp>) -> Self {
        Self { driver, driver_kind, home, prefix: PRODUCTION_PREFIX.to_owned(), image, apps }
    }

    /// Names cells with this prefix instead (`adytum-test-` in tests).
    pub fn with_prefix(mut self, prefix: &str) -> Result<Self, ConfineError> {
        format!("{prefix}x").parse::<CellName>().map_err(|e| ConfineError::Host(format!("{prefix:?} is not a cell name prefix: {e}")))?;
        prefix.clone_into(&mut self.prefix);
        Ok(self)
    }

    fn new_name(&self) -> Result<CellName, ConfineError> {
        let id = CellId::new_random().map_err(|e| ConfineError::Host(e.to_string()))?.to_string();
        let short: String = id.trim_start_matches(CellId::PREFIX).chars().take(8).collect();
        format!("{}{short}", self.prefix).parse::<CellName>().map_err(|e| ConfineError::Host(e.to_string()))
    }

    async fn run_async(&self, plan: &CellPlan) -> Result<CellRun, ConfineError> {
        let host = |e: &dyn std::fmt::Display| ConfineError::Host(e.to_string());
        let name = self.new_name()?;
        let ledger = Ledger::open(&self.home).map_err(|e| host(&e))?;
        reconcile(&ledger, &*self.driver).await.map_err(|e| host(&e))?;
        let spec = CellSpec {
            name: name.clone(),
            driver: self.driver_kind,
            image: self.image.clone(),
            wall: plan.wall.clone(),
            resources: Resources::default(),
            ports: Vec::new(),
            automation_port: None,
            // The project is copied in; the host's folders are never mounted.
            mounts: Vec::new(),
            sealed: true,
            network: None,
            clone_from: None,
            secrets: Vec::new(),
        };
        create_cell(&ledger, &*self.driver, &spec).await.map_err(|e| host(&e))?;
        let result = self.run_in(&ledger, &name, plan).await;
        let removed = if plan.keep && result.is_ok() { false } else { remove_cell(&ledger, &*self.driver, &name).await.is_ok() };
        match result {
            Ok(mut run) => {
                run.removed = removed;
                Ok(run)
            }
            Err(e) => Err(e),
        }
    }

    async fn run_in(&self, ledger: &Ledger, name: &CellName, plan: &CellPlan) -> Result<CellRun, ConfineError> {
        let host = |e: DriverError| ConfineError::Host(e.to_string());
        let op = ledger.intend(OpKind::Start, name).map_err(|e| ConfineError::Host(e.to_string()))?;
        let started = self.driver.start(name).await;
        let outcome = started.as_ref().map_or_else(|e| Outcome::Failed(e.to_string()), |()| Outcome::Done);
        ledger.settle(op, outcome).map_err(|e| ConfineError::Host(e.to_string()))?;
        started.map_err(host)?;

        let folder = Self::project_folder_name(&plan.project)?;
        let setup = self.driver.exec(name, &["mkdir".into(), "-p".into(), CELL_HOME.into(), GUEST_IN.into()], ExecOpts::default()).await.map_err(host)?;
        if !setup.success() {
            return Err(ConfineError::Host(format!("the cell's folders could not be made: {}", setup.stderr_lossy().trim())));
        }
        self.driver.copy_in(name, &plan.project, GUEST_IN).await.map_err(host)?;
        let guest_project = format!("{GUEST_IN}/{folder}");

        // `env -i` empties whatever the guest's login gave, then names
        // exactly the plan's variables.
        let mut argv: Vec<String> = vec!["env".into(), "-i".into(), "-C".into(), guest_project.clone()];
        argv.extend(plan.env.iter().map(|(k, v)| format!("{k}={v}")));
        argv.push(plan.program.clone());
        argv.extend(plan.args.iter().cloned());
        let opts = ExecOpts { env: Vec::new(), stdin: plan.stdin.clone(), deadline: Some(plan.deadline.unwrap_or(DEFAULT_DEADLINE)) };
        let out = self.driver.exec(name, &argv, opts).await.map_err(host)?;

        if let Some(dest) = &plan.copy_out {
            std::fs::create_dir_all(dest).map_err(|e| ConfineError::Host(format!("the folder for the result could not be made: {e}")))?;
            self.driver.copy_out(name, &guest_project, dest).await.map_err(host)?;
        }
        Ok(CellRun { status: out.status, stdout: out.stdout, stderr: out.stderr, timed_out: out.timed_out, cell: name.to_string(), removed: false })
    }

    fn project_folder_name(project: &Path) -> Result<String, ConfineError> {
        let refuse = |why| ConfineError::ProjectRefused { project: project.display().to_string(), why };
        if !project.is_dir() {
            return Err(refuse("it is not a folder"));
        }
        let name = project.file_name().and_then(|n| n.to_str()).ok_or_else(|| refuse("its name is not plain text"))?;
        if name.is_empty() || name.chars().any(|c| c.is_control() || c == '/' || c == ':') || name == ".." {
            return Err(refuse("its name has a character a cell path cannot hold"));
        }
        Ok(name.to_owned())
    }
}

impl CellHost for CellRunner {
    fn apps(&self) -> Vec<CellApp> {
        self.apps.clone()
    }

    fn run(&self, plan: &CellPlan) -> Result<CellRun, ConfineError> {
        // A thread of its own with its own runtime, so this is safe to call
        // from inside the service's runtime as well as outside it.
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| ConfineError::Host(e.to_string()))?;
                    rt.block_on(self.run_async(plan))
                })
                .join()
                .unwrap_or_else(|_| Err(ConfineError::Host("the cell run stopped unexpectedly".into())))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adytum_driver::Listed;
    use adytum_exec::{Duplex, Output};
    use adytum_model::{Arch, CellState};
    use std::sync::Mutex;
    use xenonet_engine::confine::{plan, CellRequest, Confinement};

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    /// A driver that records what it is asked and keeps a list of cells.
    struct Recording {
        log: Arc<Log>,
        cells: Mutex<Vec<CellName>>,
        specs: Mutex<Vec<CellSpec>>,
        fail_exec: bool,
    }

    #[async_trait::async_trait]
    impl Driver for Recording {
        async fn create(&self, spec: &CellSpec) -> Result<(), DriverError> {
            self.log.0.lock().expect("lock").push(format!("create {}", spec.name));
            self.cells.lock().expect("lock").push(spec.name.clone());
            self.specs.lock().expect("lock").push(spec.clone());
            Ok(())
        }
        async fn start(&self, name: &CellName) -> Result<(), DriverError> {
            self.log.0.lock().expect("lock").push(format!("start {name}"));
            Ok(())
        }
        async fn exec(&self, _: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError> {
            self.log.0.lock().expect("lock").push(format!("exec {} stdin={}", argv.join(" "), opts.stdin.is_some()));
            if self.fail_exec && argv.first().is_some_and(|a| a == "env") {
                return Err(DriverError::Unsupported { what: "this test's exec".into() });
            }
            Ok(Output { status: Some(0), stdout: b"done\n".to_vec(), ..Output::default() })
        }
        async fn stream(&self, _: &CellName, _: &[String]) -> Result<Duplex, DriverError> {
            Err(DriverError::Unsupported { what: "stream".into() })
        }
        async fn copy_in(&self, _: &CellName, host: &Path, guest: &str) -> Result<(), DriverError> {
            self.log.0.lock().expect("lock").push(format!("copy_in {} {guest}", host.display()));
            Ok(())
        }
        async fn copy_out(&self, _: &CellName, guest: &str, host: &Path) -> Result<(), DriverError> {
            self.log.0.lock().expect("lock").push(format!("copy_out {guest} {}", host.display()));
            Ok(())
        }
        async fn stop(&self, _: &CellName) -> Result<(), DriverError> {
            Ok(())
        }
        async fn remove(&self, name: &CellName) -> Result<(), DriverError> {
            self.log.0.lock().expect("lock").push(format!("remove {name}"));
            self.cells.lock().expect("lock").retain(|c| c != name);
            Ok(())
        }
        async fn list(&self) -> Result<Vec<Listed>, DriverError> {
            Ok(self.cells.lock().expect("lock").iter().map(|name| Listed { name: name.clone(), state: CellState::Running }).collect())
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xn-cells-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("a temp folder");
        d
    }

    fn runner(fail_exec: bool, home: &Path) -> (CellRunner, Arc<Recording>, Arc<Log>) {
        let log = Arc::new(Log::default());
        let driver = Arc::new(Recording { log: log.clone(), cells: Mutex::new(Vec::new()), specs: Mutex::new(Vec::new()), fail_exec });
        let image = ImagePin::new("https://example.test/a.img".into(), format!("sha256:{}", "ab".repeat(32)), Arch::Aarch64).expect("a pin");
        let apps = vec![CellApp { app: "fake-agent".into(), program: "/bin/sh".into() }];
        let r = CellRunner::with_driver(driver.clone(), DriverKind::Lima, home.to_path_buf(), image, apps).with_prefix("adytum-test-").expect("prefix");
        (r, driver, log)
    }

    fn make_plan(r: &CellRunner, project: &Path, keep: bool) -> CellPlan {
        let wall = xenonet_engine::confine::wall_for_identity("git:x").expect("wall");
        let c = Confinement::Cell { wall };
        let args = ["agent.sh".to_owned()];
        let req = CellRequest {
            confinement: &c,
            identity_id: "git:x",
            app: "fake-agent",
            args: &args,
            stdin: Some(b"prompt"),
            project,
            offered_env: &[],
            host_home: "/Users/nova",
            copy_out: None,
            keep,
            deadline: None,
        };
        plan(r, &req).expect("a plan")
    }

    #[test]
    fn a_run_makes_a_sealed_cell_copies_runs_and_removes_it() {
        let (home, project) = (dir("a-home"), dir("a-proj"));
        let (r, driver, log) = runner(false, &home);
        let run = r.run(&make_plan(&r, &project, false)).expect("runs");
        assert!(run.removed && run.status == Some(0) && run.stdout == b"done\n");
        assert!(run.cell.starts_with("adytum-test-"));
        let log = log.0.lock().expect("lock");
        let verbs: Vec<&str> = log.iter().map(|l| l.split(' ').next().unwrap_or("")).collect();
        assert_eq!(verbs, ["create", "start", "exec", "copy_in", "exec", "remove"], "{log:?}");
        let spec = driver.specs.lock().expect("lock")[0].clone();
        assert!(spec.sealed && spec.mounts.is_empty() && spec.ports.is_empty() && spec.secrets.is_empty() && spec.network.is_none());
        assert_eq!(spec.wall, xenonet_engine::confine::wall_for_identity("git:x").expect("wall"));
        let ran = log.iter().find(|l| l.starts_with("exec env")).expect("the program ran");
        assert!(ran.contains("env -i -C /tmp/xn-in/"), "{ran}");
        assert!(ran.contains("PATH=/usr/local/bin:/usr/bin:/bin HOME=/tmp/xn-home /bin/sh agent.sh stdin=true"), "{ran}");
        assert!(!ran.contains("/Users"), "{ran}");
        assert!(driver.cells.lock().expect("lock").is_empty(), "the cell is gone");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn the_cell_is_removed_when_the_run_fails() {
        let (home, project) = (dir("b-home"), dir("b-proj"));
        let (r, driver, _) = runner(true, &home);
        let err = r.run(&make_plan(&r, &project, false)).expect_err("fails");
        assert!(matches!(err, ConfineError::Host(_)));
        assert!(driver.cells.lock().expect("lock").is_empty(), "a failed run leaves no cell");
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn a_kept_cell_stays() {
        let (home, project) = (dir("c-home"), dir("c-proj"));
        let (r, driver, _) = runner(false, &home);
        let run = r.run(&make_plan(&r, &project, true)).expect("runs");
        assert!(!run.removed);
        assert_eq!(driver.cells.lock().expect("lock").len(), 1);
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }

    #[test]
    fn a_prefix_that_is_not_ours_is_refused() {
        let home = dir("d-home");
        let (r, _, _) = runner(false, &home);
        assert!(r.with_prefix("xenonet-").is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_project_that_is_not_a_folder_is_refused() {
        let (home, project) = (dir("e-home"), dir("e-proj"));
        let (r, driver, _) = runner(false, &home);
        let missing = project.join("nope");
        let err = r.run(&make_plan(&r, &missing, false)).expect_err("refused");
        assert!(matches!(err, ConfineError::ProjectRefused { .. }), "{err}");
        assert!(driver.cells.lock().expect("lock").is_empty());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&project);
    }
}
