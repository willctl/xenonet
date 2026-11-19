//! Driver behaviour against fakes: what is refused before the tool runs, what
//! it is asked, and what it never touches.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use adytum_driver::{cell_name, Driver, DriverError, ExecOpts};
use adytum_exec::{Cancel, Chunk, Command, Duplex, ExecError, Output, Runner};
use adytum_model::{CellName, CellSpec, CellState, DriverKind, Mount, Port, RefusalCode, Resources};
use adytum_oci::{cell_label, image, Oci, OciTool, DEFAULT_IMAGE};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Answers each call from a script (success with no output when it runs
/// out) and keeps every argv it saw.
#[derive(Default)]
struct Script {
    answers: Mutex<Vec<(i32, String, String)>>,
    seen: Mutex<Vec<Vec<String>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Script {
    fn new(answers: &[(i32, &str, &str)]) -> Arc<Script> {
        let list = answers.iter().rev().map(|(s, o, e)| (*s, (*o).to_owned(), (*e).to_owned())).collect();
        Arc::new(Script { answers: Mutex::new(list), seen: Mutex::new(Vec::new()) })
    }

    fn calls(&self) -> Vec<Vec<String>> {
        lock(&self.seen).clone()
    }

    fn joined(&self) -> Vec<String> {
        self.calls().iter().map(|c| c.join(" ")).collect()
    }
}

#[async_trait]
impl Runner for Script {
    async fn run(&self, cmd: &Command, _cancel: &Cancel) -> Result<Output, ExecError> {
        lock(&self.seen).push(cmd.argv());
        let (status, stdout, stderr) = lock(&self.answers).pop().unwrap_or((0, String::new(), String::new()));
        Ok(Output { status: Some(status), stdout: stdout.into_bytes(), stderr: stderr.into_bytes(), ..Output::default() })
    }

    async fn stream(&self, cmd: &Command, cancel: &Cancel, _sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        self.run(cmd, cancel).await
    }

    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError> {
        lock(&self.seen).push(cmd.argv());
        Err(ExecError::Unexpected { message: "no duplex in this fake".to_owned() })
    }
}

fn oci(script: &Arc<Script>, tool: OciTool) -> Oci {
    Oci::new(script.clone(), tool, PathBuf::from(if tool == OciTool::Podman { "podman" } else { "docker" }))
}

fn name(text: &str) -> CellName {
    text.parse().expect("a valid name")
}

fn spec(driver: DriverKind) -> CellSpec {
    CellSpec {
        name: name("adytum-test-a"),
        driver,
        image: image(DEFAULT_IMAGE).expect("pinned"),
        wall: "wall_test".parse().expect("a wall"),
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

fn loopback(host: u16, guest: u16) -> Port {
    Port { guest, host, bind: IpAddr::V4(Ipv4Addr::LOCALHOST) }
}

fn refused(result: Result<(), DriverError>) -> Option<RefusalCode> {
    match result {
        Err(DriverError::Refused(r)) => Some(r.code),
        _ => None,
    }
}

#[tokio::test]
async fn a_name_outside_ours_is_refused_before_any_call() {
    for bad in ["pane", "xenonet-vm1", "xenonet-base-ubuntu", "default", "web"] {
        let Err(DriverError::Refused(r)) = cell_name(bad) else { panic!("{bad} was not refused") };
        assert_eq!(r.code, RefusalCode::NameNotOurs);
    }
    // Drivers take names that passed, so the tool sees nothing at all.
    let script = Script::new(&[]);
    let _oci = oci(&script, OciTool::Podman);
    assert_eq!(script.joined(), Vec::<String>::new());
}

#[tokio::test]
async fn a_sealed_create_has_no_mount_and_every_label() {
    let script = Script::new(&[]);
    let mut s = spec(DriverKind::Podman);
    s.mounts = vec![Mount { source: "/data".into(), target: "/data".into(), writable: true }];
    // A sealed spec with a mount is refused by the rule, before any call.
    assert_eq!(refused(oci(&script, OciTool::Podman).create(&s).await), Some(RefusalCode::SealedCellMount));
    assert_eq!(script.joined(), Vec::<String>::new());

    s.mounts.clear();
    oci(&script, OciTool::Podman).create(&s).await.unwrap();
    let calls = script.calls();
    assert_eq!(calls.len(), 1, "no network is made for a cell without one");
    let argv = &calls[0];
    assert!(!argv.iter().any(|a| a == "-v" || a.starts_with("--volume") || a.starts_with("--mount")), "{argv:?}");
    assert!(argv.contains(&format!("adytum.cell={}", cell_label(&s.name))));
    assert!(argv.contains(&"adytum.wall=wall_test".to_owned()));
    assert!(argv.contains(&"--network=none".to_owned()));
    assert!(argv.contains(&"--cap-drop=ALL".to_owned()));
    assert!(argv.contains(&"--security-opt=no-new-privileges".to_owned()));
    // The image is by digest, and the container's first process is fixed.
    let pin = image(DEFAULT_IMAGE).unwrap();
    let want = format!("docker.io/library/alpine@{}", pin.digest);
    assert_eq!(argv[argv.len() - 3..], [want, "sleep".to_owned(), "infinity".to_owned()]);
}

#[tokio::test]
async fn every_published_port_is_on_loopback_and_the_automation_port_is_not_published() {
    let script = Script::new(&[(1, "", "no such network"), (0, "", ""), (0, "", "")]);
    let mut s = spec(DriverKind::Docker);
    s.ports = vec![loopback(8080, 80), loopback(8443, 443)];
    s.automation_port = Some(9222);
    oci(&script, OciTool::Docker).create(&s).await.unwrap();
    let calls = script.calls();
    let create = calls.last().unwrap();
    let publishes: Vec<&String> = create.iter().enumerate().filter(|(_, a)| *a == "-p").map(|(i, _)| &create[i + 1]).collect();
    assert_eq!(publishes, ["127.0.0.1:8080:80", "127.0.0.1:8443:443"]);
    assert!(!create.iter().any(|a| a.contains("9222")));
    // The wall's own network was made first, labelled with the wall.
    assert_eq!(calls[1], ["docker", "network", "create", "--label", "adytum.wall=wall_test", "adytum-wall-test"]);
    assert!(create.contains(&"--network=adytum-wall-test".to_owned()));
}

#[tokio::test]
async fn what_the_rules_refuse_is_refused_before_any_call() {
    let script = Script::new(&[]);
    let driver = oci(&script, OciTool::Podman);

    let mut s = spec(DriverKind::Podman);
    s.ports = vec![Port { guest: 80, host: 80, bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED) }];
    assert_eq!(refused(driver.create(&s).await), Some(RefusalCode::PortNotLocalhost));

    let mut s = spec(DriverKind::Podman);
    s.ports = vec![loopback(9222, 9222)];
    s.automation_port = Some(9222);
    assert_eq!(refused(driver.create(&s).await), Some(RefusalCode::AutomationPortPublished));

    assert!(matches!(driver.create(&spec(DriverKind::Lima)).await, Err(DriverError::Unsupported { .. })));
    let mut s = spec(DriverKind::Podman);
    s.clone_from = Some(name("adytum-test-b"));
    assert!(matches!(driver.create(&s).await, Err(DriverError::Unsupported { .. })));
    let mut s = spec(DriverKind::Podman);
    s.image.location = "/var/images/a.tar".to_owned();
    assert!(matches!(driver.create(&s).await, Err(DriverError::Unsupported { .. })));

    assert!(script.calls().is_empty(), "{:?}", script.calls());
}

#[tokio::test]
async fn an_unsealed_cell_gets_only_the_mounts_it_asked_for() {
    let script = Script::new(&[]);
    let mut s = spec(DriverKind::Podman);
    s.sealed = false;
    s.mounts = vec![Mount { source: "/data".into(), target: "/in".into(), writable: false }];
    oci(&script, OciTool::Podman).create(&s).await.unwrap();
    let argv = &script.calls()[0];
    let at = argv.iter().position(|a| a == "--mount").unwrap();
    assert_eq!(argv[at + 1], "type=bind,source=/data,target=/in,readonly");
}

#[tokio::test]
async fn a_container_without_our_label_is_never_touched() {
    for tool in [OciTool::Podman, OciTool::Docker] {
        let n = name("adytum-someone-elses");
        let host = Path::new("/tmp/x");
        for step in ["start", "exec", "stream", "copy_in", "copy_out", "stop", "remove"] {
            // The inspect answers with an empty label: the container exists, we did not make it.
            let script = Script::new(&[(0, "\n", "")]);
            let driver = oci(&script, tool);
            let result = match step {
                "start" => driver.start(&n).await,
                "exec" => driver.exec(&n, &["true".to_owned()], ExecOpts::default()).await.map(|_| ()),
                "stream" => driver.stream(&n, &["true".to_owned()]).await.map(|_| ()),
                "copy_in" => driver.copy_in(&n, host, "/tmp/y").await,
                "copy_out" => driver.copy_out(&n, "/tmp/y", host).await,
                "stop" => driver.stop(&n).await,
                _ => driver.remove(&n).await,
            };
            assert_eq!(refused(result), Some(RefusalCode::NameNotOurs), "{step}");
            // Only the label look-up ran: nothing was started, stopped, copied or removed.
            assert_eq!(script.calls().len(), 1, "{step}: {:?}", script.joined());
            assert_eq!(script.calls()[0][1..3], ["container", "inspect"]);
        }
    }
}

#[tokio::test]
async fn docker_shows_no_value_for_a_missing_label_and_that_counts_as_none() {
    let script = Script::new(&[(0, "<no value>\n", "")]);
    assert_eq!(refused(oci(&script, OciTool::Docker).remove(&name("adytum-x")).await), Some(RefusalCode::NameNotOurs));
    assert_eq!(script.calls().len(), 1);
}

#[tokio::test]
async fn listing_asks_only_for_labelled_containers_and_drops_the_rest() {
    let reply = r#"[{"Names":["adytum-a"],"State":"running","Labels":{"adytum.cell":"cell_x"}},{"Names":["adytum-b"],"State":"running","Labels":{}},{"Names":["web"],"State":"running","Labels":{"adytum.cell":"cell_x"}}]"#;
    let script = Script::new(&[(0, reply, "")]);
    let cells = oci(&script, OciTool::Podman).list().await.unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!((cells[0].name.as_str(), cells[0].state), ("adytum-a", CellState::Running));
    assert_eq!(script.joined(), ["podman ps -a --filter label=adytum.cell --format json"]);
}

#[tokio::test]
async fn removing_a_container_that_is_gone_is_fine_and_runs_nothing_else() {
    let script = Script::new(&[(1, "", "Error: no such container adytum-a")]);
    oci(&script, OciTool::Podman).remove(&name("adytum-a")).await.unwrap();
    assert_eq!(script.calls().len(), 1);
}

#[tokio::test]
async fn a_name_in_use_by_another_container_is_a_name_taken_refusal() {
    let script = Script::new(&[(125, "", "Error: creating container storage: the container name \"adytum-test-a\" is already in use by abc")]);
    assert_eq!(refused(oci(&script, OciTool::Podman).create(&spec(DriverKind::Podman)).await), Some(RefusalCode::NameTaken));
}

#[tokio::test]
async fn exec_passes_each_argument_as_one_argument_with_no_shell() {
    let script = Script::new(&[(0, "adytum.cell-id\n", "")]);
    let argv = ["sh".to_owned(), "-c".to_owned(), "echo $(id); rm -rf /".to_owned()];
    let opts = ExecOpts { env: vec![("LANG".to_owned(), "C".to_owned())], stdin: Some(b"in".to_vec()), deadline: None };
    oci(&script, OciTool::Podman).exec(&name("adytum-a"), &argv, opts).await.unwrap();
    let calls = script.calls();
    assert_eq!(calls[1], ["podman", "exec", "-i", "-e", "LANG=C", "adytum-a", "sh", "-c", "echo $(id); rm -rf /"]);

    let script = Script::new(&[(0, "x\n", ""), (0, "x\n", "")]);
    let bad = oci(&script, OciTool::Podman).exec(&name("adytum-a"), &["a\0b".to_owned()], ExecOpts::default()).await;
    assert!(matches!(bad, Err(DriverError::Unsupported { .. })));
    let bad = oci(&script, OciTool::Podman).exec(&name("adytum-a"), &[], ExecOpts::default()).await;
    assert!(matches!(bad, Err(DriverError::Unsupported { .. })));
}

#[tokio::test]
async fn copy_paths_must_be_absolute_and_stay_put() {
    let script = Script::new(&[(0, "x\n", ""), (0, "x\n", ""), (0, "x\n", "")]);
    let driver = oci(&script, OciTool::Podman);
    let n = name("adytum-a");
    assert!(driver.copy_in(&n, Path::new("/tmp/f"), "tmp/rel").await.is_err());
    assert!(driver.copy_in(&n, Path::new("/tmp/f"), "/tmp/../etc/x").await.is_err());
    assert!(driver.copy_in(&n, Path::new("rel"), "/tmp/x").await.is_err());
    assert!(driver.copy_out(&n, "/tmp/x:y", Path::new("/tmp/f")).await.is_err());
    assert!(script.joined().iter().all(|c| !c.contains(" cp ")), "{:?}", script.joined());
}

#[tokio::test]
async fn cloning_is_unsupported_and_runs_nothing() {
    let script = Script::new(&[]);
    let oci = oci(&script, OciTool::Podman);
    let mut to = spec(DriverKind::Podman);
    to.name = name("adytum-test-b");
    to.clone_from = Some(name("adytum-test-a"));
    let result = Driver::clone(&oci, &name("adytum-test-a"), &to).await;
    assert!(matches!(result, Err(DriverError::Unsupported { .. })), "{result:?}");
    assert_eq!(script.joined(), Vec::<String>::new());
}
