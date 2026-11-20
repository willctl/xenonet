//! Runs a start in an Adytum cell: the engine's `CellHost` on a driver.
//!
//! One run makes a cell from a pinned image behind the plan's wall (sealed,
//! no mounts, no published ports), copies a staged copy of the project
//! folder in, runs the program (the driver runs it as a user without sudo,
//! behind the guest firewall that keeps the host's addresses closed) under
//! `env -i` and exactly the plan's environment, brings the result out
//! if asked, and removes the cell. Every step goes through Adytum's ledger,
//! so `adytum ls` and the doctor see the cell, and a run that dies midway is
//! cleaned up by [`CellRunner::reconcile_now`] (the service calls it at
//! start) or by the next run's own reconcile.
//!
//! The cell is named with a prefix `name_is_ours` accepts: `adytum-xn-` in
//! production, `adytum-test-` in tests (set with [`CellRunner::with_prefix`]).
//!
//! What crosses the wall, in both directions:
//! - In: a staged copy of the project under `<home>/xn-stage/<cell>/in`,
//!   made on the host first. The copy leaves out what the plan excludes
//!   (`.env`, `.env.*`, `.git/config`) and every symbolic link and special
//!   file, so nothing in it points back at the host. Its contents are copied
//!   into `/tmp/xn-in/<folder name>` in the cell, written by the cell's own
//!   user so the program can change them, and the staged copy is removed.
//! - Out: only into `<home>/xn-stage/<cell>/out`, a folder the runner makes
//!   fresh (a destination that exists is refused), never into the project or
//!   anywhere else on the host. The result is walked afterwards and is
//!   thrown away whole if it holds anything but plain files and folders.
//!   The caller looks at it and then calls [`CellRunner::discard_result`].

use adytum::{CellId, CellName, CellSpec, Cells, Driver, DriverKind, ExecOpts, ImagePin, Resources};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use xenonet_engine::confine::{check_project_unchanged, CellApp, CellHost, CellPlan, CellRun, ConfineError, CELL_HOME};

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
        let image = adytum::lima::image(adytum::lima::DEFAULT_IMAGE).ok_or_else(|| ConfineError::Host("the default cell image is not pinned".into()))?;
        let driver = adytum::lima::Lima::system(adytum::lima::lima_home_in(&adytum_home)).map_err(host)?;
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
        let id = CellId::new_random().map_err(host)?.to_string();
        let short: String = id.trim_start_matches(CellId::PREFIX).chars().take(8).collect();
        format!("{}{short}", self.prefix).parse::<CellName>().map_err(host)
    }

    /// The folder every run's staging folders are made in.
    fn stage_root(&self) -> PathBuf {
        self.home.join("xn-stage")
    }

    /// A fresh folder for one run: made here, so it did not exist, and
    /// readable by this user only. A symbolic link in the way is refused.
    fn fresh_stage(&self, cell: &CellName) -> Result<PathBuf, ConfineError> {
        let root = self.stage_root();
        let io = |what: &str, e: std::io::Error| ConfineError::Host(format!("{what}: {e}"));
        if std::fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(ConfineError::Host(format!("{} is a symbolic link; remove it so the runner can make its own folder", root.display())));
        }
        std::fs::create_dir_all(&root).map_err(|e| io("the staging folder could not be made", e))?;
        let dir = root.join(cell.as_str());
        std::fs::create_dir(&dir).map_err(|e| io("this run's staging folder could not be made (it must not exist)", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| io("this run's staging folder could not be locked to its owner", e))?;
        }
        Ok(dir)
    }

    /// Removes a result folder a run left, and its run folder. Only a folder
    /// under this runner's own staging folder is touched.
    pub fn discard_result(&self, result: &Path) -> Result<(), ConfineError> {
        let root = std::fs::canonicalize(self.stage_root()).map_err(|e| ConfineError::Host(e.to_string()))?;
        let run_dir = result.parent().and_then(|p| std::fs::canonicalize(p).ok()).filter(|p| p.parent() == Some(root.as_path()));
        let Some(run_dir) = run_dir else {
            return Err(ConfineError::Host(format!("{} is not a result this runner made", result.display())));
        };
        std::fs::remove_dir_all(run_dir).map_err(|e| ConfineError::Host(e.to_string()))
    }

    /// Settles Adytum's ledger with the driver now: a cell a run left half
    /// made, or that vanished, is removed or finished. The service calls
    /// this once at start. Returns the names it acted on.
    pub fn reconcile_now(&self) -> Result<Vec<String>, ConfineError> {
        on_own_thread(|| async {
            let report = self.cells()?.reconcile().await.map_err(host)?;
            Ok(report.removed.iter().chain(report.finished.iter()).map(ToString::to_string).collect())
        })
    }

    /// Adytum's cells under `home`: its ledger, with the driver.
    fn cells(&self) -> Result<Cells, ConfineError> {
        Cells::open(&self.home, self.driver.clone()).map_err(host)
    }

    async fn run_async(&self, plan: &CellPlan) -> Result<CellRun, ConfineError> {
        // The folder is checked again here, before anything is made: it is
        // the one the plan was made for, not one put in its place since.
        check_project_unchanged(plan)?;
        let name = self.new_name()?;
        let stage = self.fresh_stage(&name)?;
        let result = self.run_cell(plan, &name, &stage).await;
        // The staged copy of the project is not needed any more; the result
        // folder stays for the caller unless the run failed.
        let _ = std::fs::remove_dir_all(stage.join("in"));
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&stage);
        }
        result
    }

    /// The cell's whole life, with a guard that removes it however this
    /// function ends: an error, an early return or a panic.
    async fn run_cell(&self, plan: &CellPlan, name: &CellName, stage: &Path) -> Result<CellRun, ConfineError> {
        let cells = self.cells()?;
        cells.reconcile().await.map_err(host)?;
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
        };
        cells.create(&spec).await.map_err(host)?;
        let mut guard = Guard { driver: self.driver.clone(), home: self.home.clone(), name: name.clone(), armed: true };
        let ran = self.run_in(&cells, name, plan, stage).await;
        if plan.keep && ran.is_ok() {
            guard.armed = false;
            return ran;
        }
        let removal = guard.remove(&cells).await;
        match (ran, removal) {
            (Ok(mut run), Ok(())) => {
                run.removed = true;
                Ok(run)
            }
            (Ok(run), Err(why)) => Err(ConfineError::NotRemoved { cell: name.to_string(), why, result: summary(&run) }),
            (Err(e), Ok(())) => Err(e),
            (Err(e), Err(why)) => Err(ConfineError::NotRemoved { cell: name.to_string(), why, result: e.to_string() }),
        }
    }

    async fn run_in(&self, cells: &Cells, name: &CellName, plan: &CellPlan, stage: &Path) -> Result<CellRun, ConfineError> {
        cells.start(name).await.map_err(host)?;
        let folder = Self::project_folder_name(&plan.project)?;
        let guest_project = format!("{GUEST_IN}/{folder}");
        // Both as the cell's user, so the program owns its home and its copy.
        let home = ["mkdir", "-p", "--", CELL_HOME].map(String::from);
        let made = self.driver.exec(name, &home, ExecOpts { deadline: Some(Duration::from_mins(2)), ..ExecOpts::default() }).await.map_err(host)?;
        if !made.success() {
            return Err(ConfineError::Host(format!("the cell's home could not be made: {}", made.stderr_lossy().trim())));
        }
        let staged = stage.join("in");
        std::fs::create_dir(&staged).map_err(|e| ConfineError::Host(format!("the staged copy could not be made: {e}")))?;
        stage_project(&plan.project, &staged, &plan.copy_exclude).map_err(|e| ConfineError::Host(format!("the project could not be staged: {e}")))?;
        self.driver.copy_in(name, &staged, &guest_project, None).await.map_err(host)?;

        // The driver runs it as the cell's user. `env -i` empties whatever
        // the guest's login gave, then names exactly the plan's variables.
        let mut argv: Vec<String> = ["env", "-i", "-C"].map(String::from).to_vec();
        argv.push(guest_project.clone());
        argv.extend(plan.env.iter().map(|(k, v)| format!("{k}={v}")));
        argv.push(plan.program.clone());
        argv.extend(plan.args.iter().cloned());
        let opts = ExecOpts { stdin: plan.stdin.clone(), deadline: Some(plan.deadline.unwrap_or(DEFAULT_DEADLINE)), ..ExecOpts::default() };
        let out = self.driver.exec(name, &argv, opts).await.map_err(host)?;

        let mut result = None;
        if plan.copy_out {
            let dest = stage.join("out");
            std::fs::create_dir(&dest).map_err(|e| ConfineError::Host(format!("the folder for the result could not be made (it must not exist): {e}")))?;
            self.driver.copy_out(name, &guest_project, &dest).await.map_err(host)?;
            check_plain(&dest)?;
            result = Some(dest);
        }
        Ok(CellRun { status: out.status, stdout: out.stdout, stderr: out.stderr, timed_out: out.timed_out, cell: name.to_string(), removed: false, result })
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

/// Removes the cell when it goes out of scope with the cell still there:
/// an early `?`, a panic. The normal end calls [`Guard::remove`] and the
/// guard is then disarmed.
struct Guard {
    driver: Arc<dyn Driver>,
    home: PathBuf,
    name: CellName,
    armed: bool,
}

impl Guard {
    async fn remove(&mut self, cells: &Cells) -> Result<(), String> {
        let done = cells.remove(&self.name).await.map_err(|e| e.to_string());
        self.armed = done.is_err();
        done
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // A thread and runtime of their own: Drop may run inside a runtime
        // that is unwinding, which cannot start another.
        let (driver, home, name) = (self.driver.clone(), self.home.clone(), self.name.clone());
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            rt.block_on(async {
                if let Ok(cells) = Cells::open(&home, driver) {
                    let _ = cells.remove(&name).await;
                }
            });
        })
        .join();
    }
}

fn host(e: adytum::Error) -> ConfineError {
    ConfineError::Host(e.to_string())
}

fn summary(run: &CellRun) -> String {
    let said: String = String::from_utf8_lossy(&run.stdout).chars().take(2000).collect();
    format!("exit status {:?}{}, it said: {said}", run.status, if run.timed_out { " (timed out)" } else { "" })
}

/// Runs an async job on a thread of its own with its own runtime, so a call
/// from inside the service's runtime is as safe as one from outside it.
fn on_own_thread<T, F, Fut>(job: F) -> Result<T, ConfineError>
where
    T: Send,
    F: FnOnce() -> Fut + Send,
    Fut: std::future::Future<Output = Result<T, ConfineError>>,
{
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| ConfineError::Host(e.to_string()))?;
                rt.block_on(job())
            })
            .join()
            .unwrap_or_else(|_| Err(ConfineError::Host("the cell run stopped unexpectedly; its cell was removed".into())))
    })
}

/// Whether a path (relative to the project, `/`-separated) is left out of
/// the copy: a pattern with a `/` names that path from the top; a pattern
/// ending in `*` is a prefix of a name; any other is a name, at any depth.
fn excluded(rel: &str, patterns: &[String]) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    patterns.iter().any(|p| {
        if p.contains('/') {
            rel == p
        } else if let Some(prefix) = p.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            name == p
        }
    })
}

/// Copies `src` to the empty folder `dst` for the cell: plain files and
/// folders only, without what `exclude` names. A symbolic link, a socket, a
/// device or a pipe is left out, so nothing in the copy leads back to the
/// host.
fn stage_project(src: &Path, dst: &Path, exclude: &[String]) -> std::io::Result<()> {
    fn walk(src: &Path, dst: &Path, rel: &str, exclude: &[String]) -> std::io::Result<()> {
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if excluded(&rel, exclude) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                std::fs::create_dir(dst.join(&name))?;
                walk(&entry.path(), &dst.join(&name), &rel, exclude)?;
            } else if kind.is_file() {
                std::fs::copy(entry.path(), dst.join(&name))?;
            }
        }
        Ok(())
    }
    walk(src, dst, "", exclude)
}

/// Refuses a result folder that holds a symbolic link, a device, a pipe or
/// a socket, anywhere in it, and removes the whole run's folder then.
fn check_plain(dir: &Path) -> Result<(), ConfineError> {
    fn walk(dir: &Path) -> Result<(), ConfineError> {
        let io = |e: std::io::Error| ConfineError::Host(format!("the result could not be read: {e}"));
        for entry in std::fs::read_dir(dir).map_err(io)? {
            let entry = entry.map_err(io)?;
            let kind = entry.file_type().map_err(io)?;
            if kind.is_dir() {
                walk(&entry.path())?;
            } else if !kind.is_file() {
                return Err(ConfineError::ResultRefused { path: entry.file_name().to_string_lossy().into_owned() });
            }
        }
        Ok(())
    }
    walk(dir)
}

impl CellHost for CellRunner {
    fn apps(&self) -> Vec<CellApp> {
        self.apps.clone()
    }

    fn run(&self, plan: &CellPlan) -> Result<CellRun, ConfineError> {
        on_own_thread(|| self.run_async(plan))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adytum::exec::{Duplex, Output};
    use adytum::testing::{failed, ok, Exec, FakeDriver, Leaves, Op};
    use adytum::{Arch, CellState, Listed};
    use std::sync::Mutex;
    use xenonet_engine::confine::{plan, CellRequest};

    #[derive(Clone, Copy, Default)]
    struct Behave {
        fail_exec: bool,
        panic_exec: bool,
        fail_remove: bool,
        /// What the copy out puts in the result folder.
        #[cfg(unix)]
        out_symlink: bool,
    }

    /// Adytum's fake driver, and what its own copies don't do: it lists
    /// what was copied in before the staged copy goes, and writes a file
    /// (and maybe a link) into the result folder.
    struct Fake {
        inner: FakeDriver,
        copied: Mutex<Vec<String>>,
        #[cfg(unix)]
        out_symlink: bool,
    }

    fn listing(dir: &Path) -> String {
        fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).expect("readable").flatten() {
                let name = format!("{rel}{}", e.file_name().to_string_lossy());
                if e.file_type().expect("type").is_dir() {
                    walk(&e.path(), &format!("{name}/"), out);
                }
                out.push(name);
            }
        }
        let mut out = Vec::new();
        walk(dir, "", &mut out);
        out.sort();
        out.join(",")
    }

    #[async_trait::async_trait]
    impl Driver for Fake {
        async fn create(&self, spec: &CellSpec) -> adytum::Result<()> {
            self.inner.create(spec).await
        }
        async fn start(&self, name: &CellName) -> adytum::Result<()> {
            self.inner.start(name).await
        }
        async fn exec(&self, name: &CellName, argv: &[String], opts: ExecOpts) -> adytum::Result<Output> {
            self.inner.exec(name, argv, opts).await
        }
        async fn stream(&self, name: &CellName, argv: &[String]) -> adytum::Result<Duplex> {
            self.inner.stream(name, argv).await
        }
        async fn copy_in(&self, name: &CellName, host: &Path, guest: &str, owner: Option<&adytum::GuestUser>) -> adytum::Result<()> {
            self.copied.lock().expect("lock").push(listing(host));
            self.inner.copy_in(name, host, guest, owner).await
        }
        async fn copy_out(&self, name: &CellName, guest: &str, host: &Path) -> adytum::Result<()> {
            std::fs::write(host.join("made.txt"), "hi").expect("a file");
            #[cfg(unix)]
            if self.out_symlink {
                std::os::unix::fs::symlink("/etc/hosts", host.join("link")).expect("a link");
            }
            self.inner.copy_out(name, guest, host).await
        }
        async fn stop(&self, name: &CellName) -> adytum::Result<()> {
            self.inner.stop(name).await
        }
        async fn remove(&self, name: &CellName) -> adytum::Result<()> {
            self.inner.remove(name).await
        }
        async fn list(&self) -> adytum::Result<Vec<Listed>> {
            self.inner.list().await
        }
    }

    impl Fake {
        fn calls(&self) -> Vec<String> {
            self.inner.calls().iter().map(ToString::to_string).collect()
        }
        fn cells(&self) -> Vec<(CellName, CellState)> {
            self.inner.cells()
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::fs::canonicalize(std::env::temp_dir()).expect("temp").join(format!("xn-cells-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("a temp folder");
        d
    }

    /// The runner on a fake whose program says `done`, with the calls of
    /// `exec` kept.
    fn runner(behave: Behave, home: &Path) -> (CellRunner, Arc<Fake>, Arc<Mutex<Vec<Exec>>>) {
        let execs = Arc::new(Mutex::new(Vec::new()));
        let inner = FakeDriver::new();
        let kept = execs.clone();
        inner.on_exec(move |call| {
            kept.lock().expect("lock").push(call.clone());
            let program = call.argv.first().is_some_and(|a| a == "env");
            assert!(!(behave.panic_exec && program), "this test's exec panics");
            match (program, behave.fail_exec) {
                (true, _) => ok("done\n"),
                (false, true) => failed(1, "this test's exec fails"),
                (false, false) => ok(""),
            }
        });
        if behave.fail_remove {
            // The run's own removal, and the guard's second try.
            inner.fail_next(Op::Remove, Leaves::Nothing);
            inner.fail_next(Op::Remove, Leaves::Nothing);
        }
        let driver = Arc::new(Fake {
            inner,
            copied: Mutex::new(Vec::new()),
            #[cfg(unix)]
            out_symlink: behave.out_symlink,
        });
        let image = ImagePin::new("https://example.test/a.img".into(), format!("sha256:{}", "ab".repeat(32)), Arch::Aarch64).expect("a pin");
        let apps = vec![CellApp { app: "fake-agent".into(), program: "/bin/sh".into() }];
        let r = CellRunner::with_driver(driver.clone(), DriverKind::Lima, home.to_path_buf(), image, apps).with_prefix("adytum-test-").expect("prefix");
        (r, driver, execs)
    }

    fn make_plan(r: &CellRunner, project: &Path, keep: bool, copy_out: bool) -> CellPlan {
        let wall = xenonet_engine::confine::wall_for_identity(&xenonet_authz::PersonId::new("git:x").unwrap()).expect("wall").to_string();
        let args = ["agent.sh".to_owned()];
        let req = CellRequest {
            wall: &wall,
            signer_id: &xenonet_authz::PersonId::new("git:x").unwrap(),
            app: "fake-agent",
            args: &args,
            stdin: Some(b"prompt"),
            project,
            protected: &[],
            offered_env: &[],
            host_home: "/Users/nova",
            copy_out,
            keep,
            deadline: None,
        };
        plan(r, &req).expect("a plan")
    }

    fn cleanup(dirs: &[&Path]) {
        for d in dirs {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn a_run_makes_a_sealed_cell_copies_runs_and_removes_it() {
        let (home, project) = (dir("a-home"), dir("a-proj"));
        let (r, driver, execs) = runner(Behave::default(), &home);
        let run = r.run(&make_plan(&r, &project, false, false)).expect("runs");
        assert!(run.removed && run.status == Some(0) && run.stdout == b"done\n" && run.result.is_none());
        assert!(run.cell.starts_with("adytum-test-"));
        let calls = driver.calls();
        let verbs: Vec<&str> = calls.iter().map(|l| l.split(' ').next().unwrap_or("")).collect();
        assert_eq!(verbs, ["create", "start", "exec", "copy_in", "exec", "remove"], "{calls:?}");
        let name: CellName = run.cell.parse().expect("a name");
        let spec = driver.inner.spec(&name).expect("the spec");
        assert!(spec.sealed && spec.mounts.is_empty() && spec.ports.is_empty() && spec.network.is_none());
        assert_eq!(spec.wall, xenonet_engine::confine::wall_for_identity(&xenonet_authz::PersonId::new("git:x").unwrap()).expect("wall"));
        let execs = execs.lock().expect("lock");
        assert_eq!(execs[0].argv, ["mkdir", "-p", "--", "/tmp/xn-home"], "the cell's user makes its home");
        let ran = execs[1].argv.join(" ");
        assert!(ran.starts_with("env -i -C /tmp/xn-in/"), "{ran}");
        assert!(ran.ends_with("PATH=/usr/local/bin:/usr/bin:/bin HOME=/tmp/xn-home /bin/sh agent.sh"), "{ran}");
        assert_eq!(execs[1].stdin, b"prompt", "the prompt goes in on stdin");
        assert!(execs[1].env.is_empty(), "no variable reaches the cell but through env -i");
        assert!(!ran.contains("/Users"), "{ran}");
        assert!(driver.cells().is_empty(), "the cell is gone");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn secrets_and_links_are_not_copied_into_the_cell() {
        let (home, project) = (dir("s-home"), dir("s-proj"));
        for f in ["agent.sh", ".env", ".env.local", "sub/.env", ".git/config", ".git/HEAD", "src/main.rs", "envfile"] {
            let path = project.join(f);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
            std::fs::write(path, "x").expect("file");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&home, project.join("to-home")).expect("link");
        let (r, driver, _) = runner(Behave::default(), &home);
        r.run(&make_plan(&r, &project, false, false)).expect("runs");
        let copied = driver.calls().into_iter().find(|l| l.starts_with("copy_in")).expect("a copy").replace('\\', "/");
        assert!(copied.contains("xn-stage/adytum-test-") && copied.contains("/in /tmp/xn-in/"), "the copy is from the staged folder: {copied}");
        let files = driver.copied.lock().expect("lock").join(";");
        assert_eq!(files, ".git,.git/HEAD,agent.sh,envfile,src,src/main.rs,sub", "{files}");
        assert!(!std::fs::read_dir(home.join("xn-stage")).expect("stage").flatten().any(|e| e.path().join("in").exists()), "the staged copy is removed");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn what_a_cell_made_comes_back_only_into_a_fresh_folder_and_never_the_project() {
        let (home, project) = (dir("o-home"), dir("o-proj"));
        let (r, driver, _) = runner(Behave::default(), &home);
        let run = r.run(&make_plan(&r, &project, false, true)).expect("runs");
        let out = run.result.clone().expect("a result");
        assert!(out.starts_with(home.join("xn-stage")) && out.ends_with("out"), "{out:?}");
        assert_eq!(std::fs::read_to_string(out.join("made.txt")).expect("made"), "hi");
        assert!(!project.join("made.txt").exists(), "the project folder is never written to");
        assert!(driver.calls().iter().any(|l| l.starts_with("copy_out ") && l.ends_with(&format!(" {}", out.display()))));
        r.discard_result(&out).expect("discard");
        assert!(!out.exists() && !out.parent().expect("run folder").exists());
        assert!(r.discard_result(&project).is_err(), "only this runner's own results are removed");
        assert!(project.exists());
        cleanup(&[&home, &project]);
    }

    #[cfg(unix)]
    #[test]
    fn a_result_with_a_symlink_is_refused_and_removed() {
        let (home, project) = (dir("l-home"), dir("l-proj"));
        let (r, driver, _) = runner(Behave { out_symlink: true, ..Behave::default() }, &home);
        let err = r.run(&make_plan(&r, &project, false, true)).expect_err("refused");
        assert!(matches!(err, ConfineError::ResultRefused { ref path } if path == "link"), "{err}");
        assert!(std::fs::read_dir(home.join("xn-stage")).expect("stage").next().is_none(), "nothing of the result is kept");
        assert_eq!(driver.cells().len(), 0, "the cell is gone");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn a_staging_folder_that_exists_is_refused() {
        let home = dir("x-home");
        let (r, _, _) = runner(Behave::default(), &home);
        let name: CellName = "adytum-test-abc".parse().expect("name");
        let first = r.fresh_stage(&name).expect("fresh");
        assert!(first.is_dir());
        assert!(r.fresh_stage(&name).is_err(), "never merged into an existing folder");
        cleanup(&[&home]);
    }

    #[cfg(unix)]
    #[test]
    fn a_staging_root_that_is_a_symlink_is_refused() {
        let (home, other) = (dir("y-home"), dir("y-other"));
        std::os::unix::fs::symlink(&other, home.join("xn-stage")).expect("link");
        let (r, _, _) = runner(Behave::default(), &home);
        let name: CellName = "adytum-test-abc".parse().expect("name");
        assert!(r.fresh_stage(&name).is_err());
        assert!(std::fs::read_dir(&other).expect("other").next().is_none());
        cleanup(&[&home, &other]);
    }

    #[test]
    fn a_project_swapped_after_the_plan_is_refused_before_a_cell_is_made() {
        let (home, project) = (dir("w-home"), dir("w-proj"));
        let (r, driver, execs) = runner(Behave::default(), &home);
        let planned = make_plan(&r, &project, false, false);
        #[cfg(unix)]
        {
            std::fs::remove_dir_all(&project).expect("rm");
            std::os::unix::fs::symlink(&home, &project).expect("link");
        }
        #[cfg(unix)]
        {
            let err = r.run(&planned).expect_err("refused");
            assert!(matches!(err, ConfineError::ProjectChanged { .. }), "{err}");
            assert!(driver.calls().is_empty() && execs.lock().expect("lock").is_empty());
        }
        let _ = (planned, driver, execs);
        cleanup(&[&home]);
        let _ = std::fs::remove_file(&project);
    }

    #[test]
    fn the_cell_is_removed_when_the_run_fails() {
        let (home, project) = (dir("b-home"), dir("b-proj"));
        let (r, driver, _) = runner(Behave { fail_exec: true, ..Behave::default() }, &home);
        let err = r.run(&make_plan(&r, &project, false, false)).expect_err("fails");
        assert!(matches!(err, ConfineError::Host(_)));
        assert!(driver.cells().is_empty(), "a failed run leaves no cell");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn the_cell_is_removed_when_the_run_panics() {
        let (home, project) = (dir("p-home"), dir("p-proj"));
        let (r, driver, _) = runner(Behave { panic_exec: true, ..Behave::default() }, &home);
        let err = r.run(&make_plan(&r, &project, false, false)).expect_err("fails");
        assert!(matches!(err, ConfineError::Host(ref m) if m.contains("stopped unexpectedly")), "{err}");
        assert!(driver.cells().is_empty(), "the guard removed the cell as the panic unwound");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn a_cell_that_cannot_be_removed_is_an_error_that_keeps_the_runs_result() {
        let (home, project) = (dir("r-home"), dir("r-proj"));
        let (r, driver, _) = runner(Behave { fail_remove: true, ..Behave::default() }, &home);
        let err = r.run(&make_plan(&r, &project, false, false)).expect_err("not removed");
        let ConfineError::NotRemoved { cell, result, .. } = &err else { panic!("{err}") };
        assert!(cell.starts_with("adytum-test-") && result.contains("done"), "{err}");
        assert!(err.to_string().contains("adytum rm"), "{err}");
        assert_eq!(driver.cells().len(), 1, "the cell is still there, and the error says so");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn a_kept_cell_stays_and_reconcile_now_settles_what_is_gone() {
        let (home, project) = (dir("c-home"), dir("c-proj"));
        let (r, driver, _) = runner(Behave::default(), &home);
        let run = r.run(&make_plan(&r, &project, true, false)).expect("runs");
        assert!(!run.removed);
        assert_eq!(driver.cells().len(), 1);
        assert_eq!(r.reconcile_now(), Ok(vec![]), "a cell that is still there is left alone");
        driver.inner.lose(&run.cell.parse().expect("a name"));
        assert_eq!(r.reconcile_now(), Ok(vec![run.cell.clone()]), "a cell that vanished is forgotten");
        cleanup(&[&home, &project]);
    }

    #[test]
    fn a_prefix_that_is_not_ours_is_refused() {
        let home = dir("d-home");
        let (r, _, _) = runner(Behave::default(), &home);
        assert!(r.with_prefix("xenonet-").is_err());
        cleanup(&[&home]);
    }
}
