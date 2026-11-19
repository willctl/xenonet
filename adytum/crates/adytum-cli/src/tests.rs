//! The commands against a fake driver that remembers cells in memory and
//! logs every call.

use std::path::Path;
use std::sync::{Arc, Mutex};

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Duplex, ExecError, Output};
use adytum_model::{CellName, CellSpec, CellState};
use async_trait::async_trait;
use serde_json::Value;

use super::{run, Context};

#[derive(Default)]
struct Mem {
    cells: Mutex<Vec<(CellName, CellState, CellSpec)>>,
    calls: Mutex<Vec<String>>,
}

impl Mem {
    fn log(&self, what: String) {
        self.calls.lock().unwrap().push(what);
    }
}

#[async_trait]
impl Driver for Mem {
    async fn create(&self, spec: &CellSpec) -> Result<(), DriverError> {
        self.log(format!("create {}", spec.name));
        self.cells.lock().unwrap().push((spec.name.clone(), CellState::Stopped, spec.clone()));
        Ok(())
    }
    async fn start(&self, name: &CellName) -> Result<(), DriverError> {
        self.log(format!("start {name}"));
        self.cells.lock().unwrap().iter_mut().filter(|c| &c.0 == name).for_each(|c| c.1 = CellState::Running);
        Ok(())
    }
    async fn exec(&self, name: &CellName, argv: &[String], _opts: ExecOpts) -> Result<Output, DriverError> {
        self.log(format!("exec {name} {argv:?}"));
        if argv[0] == "fail" {
            return Ok(Output { status: Some(3), stderr: b"bad\n".to_vec(), ..Output::default() });
        }
        Ok(Output { status: Some(0), stdout: argv.join(" ").into_bytes(), ..Output::default() })
    }
    async fn stream(&self, _name: &CellName, _argv: &[String]) -> Result<Duplex, DriverError> {
        Err(DriverError::Exec(ExecError::Unexpected { message: "no stream".into() }))
    }
    async fn copy_in(&self, name: &CellName, host: &Path, guest: &str) -> Result<(), DriverError> {
        self.log(format!("copy_in {name} {} {guest}", host.display()));
        Ok(())
    }
    async fn copy_out(&self, name: &CellName, guest: &str, host: &Path) -> Result<(), DriverError> {
        self.log(format!("copy_out {name} {guest} {}", host.display()));
        Ok(())
    }
    async fn stop(&self, name: &CellName) -> Result<(), DriverError> {
        self.log(format!("stop {name}"));
        Ok(())
    }
    async fn remove(&self, name: &CellName) -> Result<(), DriverError> {
        self.log(format!("remove {name}"));
        self.cells.lock().unwrap().retain(|c| &c.0 != name);
        Ok(())
    }
    async fn clone(&self, from: &CellName, to: &CellSpec) -> Result<(), DriverError> {
        self.log(format!("clone {from} {}", to.name));
        self.cells.lock().unwrap().push((to.name.clone(), CellState::Stopped, to.clone()));
        Ok(())
    }
    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        self.log("list".to_owned());
        Ok(self.cells.lock().unwrap().iter().map(|c| Listed { name: c.0.clone(), state: c.1 }).collect())
    }
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(self.out.trim()).unwrap_or_else(|e| panic!("stdout is not one JSON object: {e}: {:?}", self.out))
    }
}

struct Rig {
    mem: Arc<Mem>,
    ctx: Context,
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let home = std::env::temp_dir().join(format!("adytum-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let mem = Arc::new(Mem::default());
        Rig { ctx: Context { driver: mem.clone(), home }, mem }
    }

    async fn go(&self, line: &str) -> Run {
        let tokens: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        self.go_tokens(&tokens).await
    }

    async fn go_tokens(&self, tokens: &[String]) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(tokens, &self.ctx, &mut out, &mut err).await;
        Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
    }

    fn calls(&self) -> Vec<String> {
        self.mem.calls.lock().unwrap().clone()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.ctx.home);
    }
}

#[tokio::test]
async fn a_name_outside_ours_is_refused_before_the_driver_is_touched() {
    let rig = Rig::new("refuse");
    for cmd in ["start pane", "stop xenonet-vm1", "rm xenonet-base-ubuntu", "exec pane", "create default", "cp ./a pane:/tmp/a", "cp xenonet-vm2:/etc/hosts ./h"] {
        let tail = if cmd.starts_with("exec") { " --json -- true" } else { " --json" };
        let r = rig.go(&format!("{cmd}{tail}")).await;
        assert_eq!(r.code, 1, "{cmd}");
        assert_eq!(r.json()["error"]["code"], "name_not_ours", "{cmd}");
        assert!(r.err.is_empty(), "{cmd}: {:?}", r.err);
    }
    assert!(rig.calls().is_empty(), "{:?}", rig.calls());
}

#[tokio::test]
async fn a_human_error_reads_adytum_message_fix_on_stderr() {
    let rig = Rig::new("human");
    let r = rig.go("rm pane").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.out, "");
    assert!(r.err.starts_with("adytum: \"pane\" is not a cell Adytum made. Adytum only touches cells named adytum-"), "{}", r.err);
}

#[tokio::test]
async fn create_start_ls_stop_rm_in_json() {
    let rig = Rig::new("life");
    let r = rig.go("create adytum-test-a --sealed --wall wall_work --json").await;
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!(r.json()["name"], "adytum-test-a");
    assert_eq!(r.json()["state"], "stopped");
    assert_eq!(r.json()["sealed"], true);

    let r = rig.go("ls --json").await;
    assert_eq!(r.json()["cells"][0]["name"], "adytum-test-a");
    assert_eq!(r.json()["cells"][0]["wall"], "wall_work");

    assert_eq!(rig.go("start adytum-test-a --json").await.json()["state"], "running");
    assert_eq!(rig.go("ls --json").await.json()["cells"][0]["state"], "running");
    assert_eq!(rig.go("stop adytum-test-a --json").await.json()["state"], "stopped");
    assert_eq!(rig.go("rm adytum-test-a --json").await.json()["removed"], true);
    assert_eq!(rig.go("ls --json").await.json()["cells"].as_array().unwrap().len(), 0);
    assert_eq!(rig.go("ls").await.out, "no cells\n");
}

#[tokio::test]
async fn creating_a_name_that_exists_is_refused_by_policy() {
    let rig = Rig::new("taken");
    assert_eq!(rig.go("create adytum-test-a").await.code, 0);
    let r = rig.go("create adytum-test-a --json").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "name_taken");
    assert!(r.json()["error"]["fix"].as_str().unwrap().contains("remove"));
    assert_eq!(rig.calls().iter().filter(|c| c.starts_with("create")).count(), 1);
}

#[tokio::test]
async fn the_spec_a_create_builds_has_localhost_ports_and_no_mounts() {
    let rig = Rig::new("spec");
    let r = rig.go("create adytum-test-p --sealed --cpus 3 --memory 3072 --port 18080:80 --automation-port 9222").await;
    assert_eq!(r.code, 0, "{}", r.err);
    let cells = rig.mem.cells.lock().unwrap();
    let spec = &cells[0].2;
    assert_eq!((spec.resources.cpus, spec.resources.memory_mib), (3, 3072));
    assert_eq!(spec.ports.len(), 1);
    assert_eq!((spec.ports[0].host, spec.ports[0].guest, spec.ports[0].bind.to_string().as_str()), (18080, 80, "127.0.0.1"));
    assert_eq!(spec.automation_port, Some(9222));
    assert!(spec.mounts.is_empty() && spec.sealed);
    assert_eq!(spec.wall.as_str(), "wall_default");
}

#[tokio::test]
async fn publishing_the_automation_port_is_refused() {
    let rig = Rig::new("auto");
    let r = rig.go("create adytum-test-p --port 9222:9222 --automation-port 9222 --json").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "automation_port_published");
    assert!(rig.calls().iter().all(|c| !c.starts_with("create")));
}

#[tokio::test]
async fn usage_errors_exit_2_and_json_ones_are_shaped_like_other_errors() {
    let rig = Rig::new("usage");
    let r = rig.go("frobnicate").await;
    assert_eq!(r.code, 2);
    assert!(r.err.starts_with("adytum: There is no command"), "{}", r.err);
    let r = rig.go("start --json").await;
    assert_eq!(r.code, 2);
    assert_eq!(r.json()["error"]["code"], "usage");
    let r = rig.go("create adytum-test-a --image nonesuch --json").await;
    assert_eq!(r.code, 2);
    assert!(r.json()["error"]["fix"].as_str().unwrap().contains("ubuntu-24.04"));
    let r = rig.go("create adytum-test-a --wall Nope").await;
    assert_eq!(r.code, 2);
    assert_eq!(rig.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn exec_passes_arguments_whole_and_reports_the_status() {
    let rig = Rig::new("exec");
    assert_eq!(rig.go("create adytum-test-a").await.code, 0);
    let tokens: Vec<String> = ["exec", "adytum-test-a", "--json", "--", "echo", "$(id)", "a b"].iter().map(|s| (*s).to_owned()).collect();
    let r = rig.go_tokens(&tokens).await;
    assert_eq!(r.code, 0);
    assert_eq!(r.json()["status"], 0);
    assert_eq!(r.json()["stdout"], "echo $(id) a b");
    assert_eq!(rig.calls().iter().filter(|c| c.starts_with("exec")).collect::<Vec<_>>(), [r#"exec adytum-test-a ["echo", "$(id)", "a b"]"#]);

    let r = rig.go("exec adytum-test-a -- fail").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.err, "bad\nadytum: the command exited with status 3.\n");
    let r = rig.go("exec adytum-test-a --json -- fail").await;
    assert_eq!((r.code, r.json()["status"].as_i64()), (1, Some(3)));
}

#[tokio::test]
async fn cp_works_out_which_side_is_the_cell() {
    let rig = Rig::new("cp");
    assert_eq!(rig.go("create adytum-test-a").await.code, 0);
    assert_eq!(rig.go("cp /tmp/a adytum-test-a:/home/a").await.code, 0);
    assert_eq!(rig.go("cp adytum-test-a:/home/a /tmp/b").await.code, 0);
    let rel = rig.go("cp ./rel.txt adytum-test-a:/tmp/rel.txt").await;
    assert_eq!(rel.code, 0);
    let calls: Vec<String> = rig.calls().into_iter().filter(|c| c.starts_with("copy_")).collect();
    assert_eq!(calls[0], "copy_in adytum-test-a /tmp/a /home/a");
    assert_eq!(calls[1], "copy_out adytum-test-a /home/a /tmp/b");
    assert!(calls[2].starts_with("copy_in adytum-test-a /") && calls[2].ends_with("rel.txt /tmp/rel.txt"), "{}", calls[2]);
    assert_eq!(rig.go("cp /tmp/a /tmp/b").await.code, 2);
    assert_eq!(rig.go("cp adytum-test-a:/a adytum-test-b:/b").await.code, 2);
}

#[tokio::test]
async fn help_prints_usage_on_stdout() {
    let rig = Rig::new("help");
    let r = rig.go("--help").await;
    assert_eq!(r.code, 0);
    assert!(r.out.contains("adytum create <name>") && r.out.contains("Exit codes"));
    assert_eq!(r.err, "");
}

fn field<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(v, |v, k| &v[*k])
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_owned()).collect()
}

fn spec_named(rig: &Rig, name: &str) -> CellSpec {
    let json = rig.mem.cells.lock().unwrap().iter().find(|c| c.0.as_str() == name).map(|c| c.2.clone());
    json.unwrap()
}

#[tokio::test]
async fn reconcile_at_start_removes_the_half_made_cell_of_an_interrupted_create() {
    let rig = Rig::new("interrupted");
    // What a create killed half-way leaves: an unsettled intent in the
    // ledger, and the driver already holding a cell by that name.
    let spec = rig.go("create adytum-test-seed --sealed").await;
    assert_eq!(spec.code, 0);
    let mut half = spec_named(&rig, "adytum-test-seed");
    half.name = "adytum-test-half".parse().unwrap();
    {
        let ledger = adytum_ledger::Ledger::open(&rig.ctx.home).unwrap();
        ledger.intend_create(&half).unwrap();
        assert_eq!(ledger.open_ops().unwrap().len(), 1);
    }
    rig.mem.cells.lock().unwrap().push((half.name.clone(), CellState::Stopped, half));

    let r = rig.go("ls --json").await;
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(names(&r.json()["reconciled"]["removed"]), ["adytum-test-half"]);
    assert!(r.err.contains("adytum-test-half"), "{}", r.err);
    assert_eq!(r.json()["cells"].as_array().unwrap().len(), 1);
    assert!(rig.calls().iter().any(|c| c == "remove adytum-test-half"));
    assert!(rig.mem.cells.lock().unwrap().iter().all(|c| c.0.as_str() != "adytum-test-half"));
    assert_eq!(adytum_ledger::Ledger::open(&rig.ctx.home).unwrap().open_ops().unwrap().len(), 0);
    // The name is free again.
    assert_eq!(rig.go("create adytum-test-half").await.code, 0);
}

#[tokio::test]
async fn a_cell_the_ledger_never_recorded_is_reported_and_never_touched() {
    let rig = Rig::new("unrecorded");
    let mut spec = {
        assert_eq!(rig.go("create adytum-test-seed").await.code, 0);
        spec_named(&rig, "adytum-test-seed")
    };
    spec.name = "adytum-test-stranger".parse().unwrap();
    rig.mem.cells.lock().unwrap().push((spec.name.clone(), CellState::Running, spec));
    let before = rig.calls().len();

    let r = rig.go("ls --json").await;
    assert_eq!(names(&r.json()["reconciled"]["unrecorded"]), ["adytum-test-stranger"]);
    assert_eq!(r.json()["cells"].as_array().unwrap().len(), 1);
    assert!(rig.go("ls").await.out.contains("adytum-test-stranger  (not recorded by adytum; left alone)"));
    for cmd in [
        "start adytum-test-stranger --json",
        "stop adytum-test-stranger --json",
        "rm adytum-test-stranger --json",
        "exec adytum-test-stranger --json -- true",
        "cp /tmp/a adytum-test-stranger:/tmp/a --json",
    ] {
        let r = rig.go(cmd).await;
        assert_eq!((r.code, r.json()["error"]["code"].as_str()), (1, Some("not_recorded")), "{cmd}");
    }
    let touched: Vec<_> = rig.calls().into_iter().skip(before).filter(|c| c != "list").collect();
    assert_eq!(touched, Vec::<String>::new(), "only listings may reach the driver");
    assert!(rig.mem.cells.lock().unwrap().iter().any(|c| c.0.as_str() == "adytum-test-stranger" && c.1 == CellState::Running));
}

#[tokio::test]
async fn a_clone_across_walls_is_refused_as_wall_clone_before_the_driver_clones() {
    let rig = Rig::new("wallclone");
    assert_eq!(rig.go("create adytum-test-a --wall wall_a").await.code, 0);
    let r = rig.go("clone adytum-test-a adytum-test-b --wall wall_b --json").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "wall_clone");
    assert!(rig.calls().iter().all(|c| !c.starts_with("clone")), "{:?}", rig.calls());
    // Nothing was recorded for the refused clone.
    assert_eq!(rig.go("ls --json").await.json()["cells"].as_array().unwrap().len(), 1);
    assert_eq!(adytum_ledger::Ledger::open(&rig.ctx.home).unwrap().open_ops().unwrap().len(), 0);
}

#[tokio::test]
async fn a_clone_of_a_cell_holding_an_eidolon_is_refused_until_it_is_released() {
    let rig = Rig::new("holdclone");
    assert_eq!(rig.go("create adytum-test-a --wall wall_a").await.code, 0);
    let r = rig.go("hold adytum-test-a eid_1 --json").await;
    assert_eq!((r.code, r.json()["held"].as_bool()), (0, Some(true)), "{}", r.err);

    let r = rig.go("clone adytum-test-a adytum-test-b --json").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.json()["error"]["code"], "clone_open_eidolon");
    assert!(rig.calls().iter().all(|c| !c.starts_with("clone")));

    assert_eq!(rig.go("release adytum-test-a eid_1").await.code, 0);
    let r = rig.go("clone adytum-test-a adytum-test-b --json").await;
    assert_eq!(r.code, 0, "{}", r.out);
    assert_eq!((field(&r.json(), &["state"]).as_str(), field(&r.json(), &["wall"]).as_str()), (Some("stopped"), Some("wall_a")));
    assert!(rig.calls().iter().any(|c| c == "clone adytum-test-a adytum-test-b"));
    let listed = rig.go("ls --json").await.json();
    assert_eq!(listed["cells"].as_array().unwrap().len(), 2);
    // The copy's spec says where it came from and publishes nothing of the source's.
    let spec = spec_named(&rig, "adytum-test-b");
    assert_eq!(spec.clone_from.as_ref().map(CellName::as_str), Some("adytum-test-a"));
    assert_eq!(spec.ports.len(), 0);
}

#[tokio::test]
async fn hold_and_release_need_a_recorded_cell_and_a_plain_id() {
    let rig = Rig::new("hold");
    let r = rig.go("hold adytum-test-nope eid_1 --json").await;
    assert_eq!(r.json()["error"]["code"], "not_recorded");
    assert_eq!(rig.go("hold adytum-test-nope bad/id").await.code, 2);
    let r = rig.go("hold pane eid_1 --json").await;
    assert_eq!(r.json()["error"]["code"], "name_not_ours");
}

#[tokio::test]
async fn start_and_stop_are_recorded_and_settled() {
    let rig = Rig::new("ops");
    assert_eq!(rig.go("create adytum-test-a").await.code, 0);
    assert_eq!(rig.go("start adytum-test-a").await.code, 0);
    let ledger = adytum_ledger::Ledger::open(&rig.ctx.home).unwrap();
    assert_eq!(ledger.open_ops().unwrap().len(), 0);
    assert_eq!(ledger.cells().unwrap()[0].state, CellState::Running);
    assert_eq!(rig.go("stop adytum-test-a").await.code, 0);
    assert_eq!(ledger.cells().unwrap()[0].state, CellState::Stopped);
}

#[tokio::test]
async fn cell_wall_reads_the_ledger() {
    let rig = Rig::new("wall");
    assert_eq!(rig.go("create adytum-test-a --wall wall_work").await.code, 0);
    let name: CellName = "adytum-test-a".parse().unwrap();
    assert_eq!(crate::cell_wall(&rig.ctx.home, &name).unwrap().as_str(), "wall_work");
    let other: CellName = "adytum-test-b".parse().unwrap();
    assert!(matches!(crate::cell_wall(&rig.ctx.home, &other), Err(crate::LedgerError::NotRecorded(_))));
}
