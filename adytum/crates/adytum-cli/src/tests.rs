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
    let tokens: Vec<String> = ["exec", "adytum-test-a", "--json", "--", "echo", "$(id)", "a b"].iter().map(|s| (*s).to_owned()).collect();
    let r = rig.go_tokens(&tokens).await;
    assert_eq!(r.code, 0);
    assert_eq!(r.json()["status"], 0);
    assert_eq!(r.json()["stdout"], "echo $(id) a b");
    assert_eq!(rig.calls(), [r#"exec adytum-test-a ["echo", "$(id)", "a b"]"#]);

    let r = rig.go("exec adytum-test-a -- fail").await;
    assert_eq!(r.code, 1);
    assert_eq!(r.err, "bad\nadytum: the command exited with status 3.\n");
    let r = rig.go("exec adytum-test-a --json -- fail").await;
    assert_eq!((r.code, r.json()["status"].as_i64()), (1, Some(3)));
}

#[tokio::test]
async fn cp_works_out_which_side_is_the_cell() {
    let rig = Rig::new("cp");
    assert_eq!(rig.go("cp /tmp/a adytum-test-a:/home/a").await.code, 0);
    assert_eq!(rig.go("cp adytum-test-a:/home/a /tmp/b").await.code, 0);
    let rel = rig.go("cp ./rel.txt adytum-test-a:/tmp/rel.txt").await;
    assert_eq!(rel.code, 0);
    let calls = rig.calls();
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
