//! The whole thing against a real VM, through the built binary. Skipped
//! unless `ADYTUM_LIVE=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 npm run cargo -- test -p adytum-cli --test live -- --nocapture
//! ```
//!
//! It uses an `ADYTUM_HOME` of its own under /tmp (short, for Lima's unix
//! sockets), names its cells `adytum-test-live-*`, and removes them and the
//! folder at the end, also when a step fails.
//!
//! Three scenarios: a sealed cell from create to rm, a cell with a published
//! port, and a clone (with the eidolon-hold guard) and a kill test (`adytum
//! create` killed with `kill -9` while Lima is making the instance, then the
//! next command reconciles it away).

#![expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a live gate: helpers outside #[test] fail loudly where they stand, and the timings are printed on purpose"
)]

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const BIN: &str = env!("CARGO_BIN_EXE_adytum");

struct Env {
    home: PathBuf,
}

struct Result_ {
    code: i32,
    /// stdout and stderr together, for messages.
    out: String,
    /// stdout alone: the JSON.
    stdout: String,
}

impl Env {
    fn run(&self, args: &[&str]) -> Result_ {
        let out = Command::new(BIN)
            .args(args)
            .env_clear()
            .env("ADYTUM_HOME", &self.home)
            .env("HOME", std::env::var_os("HOME").unwrap_or_default())
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .output()
            .expect("the binary starts");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        Result_ { code: out.status.code().unwrap_or(-1), out: stdout.clone() + &String::from_utf8_lossy(&out.stderr), stdout }
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let mut all = args.to_vec();
        // --json goes before any `--`.
        let at = all.iter().position(|a| *a == "--").unwrap_or(all.len());
        all.insert(at, "--json");
        let r = self.run(&all);
        let value = serde_json::from_str(r.stdout.trim()).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {}", r.out));
        (r.code, value)
    }

    /// Runs a command in the cell and returns its JSON result.
    fn exec(&self, name: &str, argv: &[&str]) -> Value {
        let mut args = vec!["exec", name, "--"];
        args.extend_from_slice(argv);
        self.json(&args).1
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // Remove every cell this home holds, then the folder.
        let listed = self.run(&["ls", "--json"]);
        if let Ok(listed) = serde_json::from_str::<Value>(listed.stdout.trim()) {
            for cell in listed["cells"].as_array().into_iter().flatten() {
                if let Some(name) = cell["name"].as_str().filter(|n| n.starts_with("adytum-test-live-")) {
                    let _ = self.run(&["stop", name]);
                    let _ = self.run(&["rm", name]);
                }
            }
        }
        // Whatever the ledger lost track of: delete any instance left in
        // this home's Lima folder, in Lima's own words, never by guessing.
        if let Ok(entries) = std::fs::read_dir(self.home.join("lima")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("adytum-test-live-") {
                    let _ = Command::new("limactl")
                        .args(["delete", "--force", "--tty=false", &name])
                        .env_clear()
                        .env("LIMA_HOME", self.home.join("lima"))
                        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
                        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                        .output();
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn step<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let v = f();
    println!("  {label}: {:.2}s", t.elapsed().as_secs_f64());
    v
}

fn random_tag() -> String {
    // The counter keeps two tags made in the same instant apart.
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let count = COUNT.fetch_add(0x1_1111, std::sync::atomic::Ordering::Relaxed);
    format!("{:05x}", (nanos ^ std::process::id().rotate_left(13) ^ count) & 0xf_ffff)
}

fn default_lima_names() -> Vec<String> {
    // Read-only: the names in Lima's own home, which Adytum must not touch.
    let out = Command::new("limactl")
        .args(["list", "--format", "{{.Name}}"])
        .env_clear()
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output();
    let mut names: Vec<String> = out.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).collect()).unwrap_or_default();
    names.sort();
    names
}

fn listener_reachable(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok()
}

/// Starts a web server in the guest and waits until the guest itself can reach it.
fn serve_in_guest(env: &Env, name: &str, port: u16) {
    let start = format!("nohup python3 -m http.server {port} --bind 127.0.0.1 >/dev/null 2>&1 &");
    assert_eq!(env.exec(name, &["sh", "-c", &start])["status"], 0);
    let wait = format!("for i in $(seq 1 50); do curl -s -o /dev/null http://127.0.0.1:{port}/ && exit 0; sleep 0.2; done; exit 1");
    assert_eq!(env.exec(name, &["sh", "-c", &wait])["status"], 0, "the guest's server never came up");
}

fn hostagent_log(env: &Env, name: &str) -> String {
    std::fs::read_to_string(env.home.join("lima").join(name).join("ha.stderr.log")).unwrap_or_default()
}

/// Sealed cell, no published ports: the full scenario.
fn sealed_cell(before: &[String]) {
    let env = Env { home: PathBuf::from("/tmp").join(format!("adl-live-{}", random_tag())) };
    let name = format!("adytum-test-live-{}", random_tag());
    println!("sealed cell {name}, ADYTUM_HOME {}", env.home.display());

    let (code, v) = step("create --sealed", || env.json(&["create", &name, "--sealed", "--wall", "wall_live"]));
    assert_eq!((code, v["state"].as_str()), (0, Some("stopped")), "{v}");
    let (code, v) = step("start", || env.json(&["start", &name]));
    assert_eq!((code, v["state"].as_str()), (0, Some("running")), "{v}");

    let uname = step("exec uname -a", || env.exec(&name, &["uname", "-a"]));
    assert!(uname["stdout"].as_str().unwrap_or_default().starts_with("Linux"), "{uname}");

    // Arguments arrive literally.
    let args = step("exec literal args", || env.exec(&name, &["printf", "%s\\n", "$(touch /tmp/x)", "a b", ";", "*", "it's", ""]));
    assert_eq!(args["stdout"], "$(touch /tmp/x)\na b\n;\n*\nit's\n\n", "{args}");
    assert_ne!(env.exec(&name, &["test", "-e", "/tmp/x"])["status"], 0, "the guest ran an argument as a command");

    // Files in and out, byte for byte, including bytes that are not text.
    let host_dir = env.home.join("files");
    std::fs::create_dir_all(&host_dir).unwrap();
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i * 7 + (i >> 8)) as u8).collect();
    let (src, back) = (host_dir.join("in.bin"), host_dir.join("out.bin"));
    std::fs::write(&src, &bytes).unwrap();
    let guest = format!("{name}:/tmp/in.bin");
    let (code, v) = step("cp in", || env.json(&["cp", src.to_str().unwrap(), &guest]));
    assert_eq!(code, 0, "{v}");
    let (code, v) = step("cp out", || env.json(&["cp", &guest, back.to_str().unwrap()]));
    assert_eq!(code, 0, "{v}");
    assert_eq!(std::fs::read(&back).unwrap(), bytes, "the file changed on the way");
    // A folder too.
    std::fs::create_dir_all(host_dir.join("tree/sub")).unwrap();
    std::fs::write(host_dir.join("tree/sub/leaf.txt"), "leaf\n").unwrap();
    let (code, v) = env.json(&["cp", host_dir.join("tree").to_str().unwrap(), &format!("{name}:/tmp/tree")]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(env.exec(&name, &["cat", "/tmp/tree/sub/leaf.txt"])["stdout"], "leaf\n");

    // Nothing from the host is mounted.
    let mounts = step("check mounts", || env.exec(&name, &["cat", "/proc/mounts"]));
    let mounts = mounts["stdout"].as_str().unwrap_or_default();
    for fs in ["virtiofs", "9p", "sshfs", "fuse.sshfs"] {
        assert!(!mounts.split_whitespace().any(|w| w == fs), "{fs} is mounted:\n{mounts}");
    }
    assert!(!mounts.contains("/Users"), "a host path is mounted:\n{mounts}");
    assert_ne!(env.exec(&name, &["test", "-e", "/Users"])["status"], 0);
    let findmnt = env.exec(&name, &["findmnt", "-n", "-t", "virtiofs,9p,fuse.sshfs"]);
    assert_eq!(findmnt["stdout"], "", "{findmnt}");

    // The user inside is not the host user.
    let whoami = env.exec(&name, &["id", "-un"]);
    assert_eq!(whoami["stdout"], "adytum\n");

    // A guest listener nobody published is not on the host.
    step("unpublished port", || {
        serve_in_guest(&env, &name, 18111);
        assert!(!listener_reachable(18111), "the guest's port 18111 is reachable from the host");
    });
    let log = hostagent_log(&env, &name);
    assert!(
        !log.lines().any(|l| l.contains("18111") && l.to_lowercase().contains("forward") && !l.to_lowercase().contains("not forward")),
        "the hostagent forwarded 18111:\n{log}"
    );
    assert!(!listener_reachable(18111), "reachable after the log check");

    // A stream carries bytes both ways.
    step("stream cat", || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            use adytum_driver::Driver;
            let lima = adytum_lima::Lima::system(adytum_lima::lima_home_in(&env.home));
            let cell: adytum_model::CellName = name.parse().unwrap();
            let mut duplex = lima.stream(&cell, &["cat".to_owned()]).await.unwrap();
            duplex.write_all(b"through the stream\n").await.unwrap();
            duplex.shutdown().await.unwrap();
            let mut got = String::new();
            duplex.read_to_string(&mut got).await.unwrap();
            assert_eq!(got, "through the stream\n");
            assert!(duplex.finish().await.unwrap().success());
        });
    });

    let (code, v) = step("stop", || env.json(&["stop", &name]));
    assert_eq!((code, v["state"].as_str()), (0, Some("stopped")), "{v}");
    let (code, v) = step("rm", || env.json(&["rm", &name]));
    assert_eq!((code, v["removed"].as_bool()), (0, Some(true)), "{v}");
    let (_, listed) = env.json(&["ls"]);
    assert_eq!(listed["cells"].as_array().unwrap().len(), 0, "{listed}");
    assert!(!env.home.join("lima").join(&name).exists());
    assert_eq!(default_lima_names(), before, "Lima's own home changed");
}

/// A cell with one published port: that port reaches the host on 127.0.0.1
/// and another one does not.
fn published_cell() {
    let env = Env { home: PathBuf::from("/tmp").join(format!("adl-live-{}", random_tag())) };
    let name = format!("adytum-test-live-{}", random_tag());
    println!("cell with a published port {name}");
    let (code, v) = step("create --port", || env.json(&["create", &name, "--sealed", "--port", "18112:18112"]));
    assert_eq!(code, 0, "{v}");
    assert_eq!(env.json(&["start", &name]).0, 0);
    step("published and unpublished", || {
        serve_in_guest(&env, &name, 18112);
        serve_in_guest(&env, &name, 18113);
        assert!(listener_reachable(18112), "the published port did not reach the host");
        assert!(!listener_reachable(18113), "an unpublished port reached the host");
    });
    let (code, _) = env.json(&["stop", &name]);
    assert_eq!(code, 0);
    assert_eq!(env.json(&["rm", &name]).0, 0);
}

fn new_env() -> Env {
    Env { home: PathBuf::from("/tmp").join(format!("adl-live-{}", random_tag())) }
}

fn listed_names(env: &Env) -> Vec<String> {
    let (_, listed) = env.json(&["ls"]);
    listed["cells"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_owned()).collect()
}

/// create, stop, clone into a second cell, start and use the clone; a held
/// source refuses the next clone; release; remove both.
fn clone_cells() {
    let env = new_env();
    let (src, copy, third) = (format!("adytum-test-live-{}", random_tag()), format!("adytum-test-live-{}", random_tag()), format!("adytum-test-live-{}", random_tag()));
    println!("clone: {src} -> {copy}, ADYTUM_HOME {}", env.home.display());

    let (code, v) = step("create source", || env.json(&["create", &src, "--sealed", "--wall", "wall_live"]));
    assert_eq!(code, 0, "{v}");
    assert_eq!(env.json(&["start", &src]).0, 0);
    let marker = step("exec in source", || env.exec(&src, &["sh", "-c", "echo from-the-source > /var/tmp/marker && cat /var/tmp/marker"]));
    assert_eq!(marker["stdout"], "from-the-source\n", "{marker}");
    let (code, v) = step("stop source", || env.json(&["stop", &src]));
    assert_eq!((code, v["state"].as_str()), (0, Some("stopped")), "{v}");

    let (code, v) = step("clone", || env.json(&["clone", &src, &copy]));
    assert_eq!((code, v["state"].as_str(), v["wall"].as_str()), (0, Some("stopped"), Some("wall_live")), "{v}");
    assert_eq!(listed_names(&env).len(), 2);

    let (code, v) = step("start the clone", || env.json(&["start", &copy]));
    assert_eq!((code, v["state"].as_str()), (0, Some("running")), "{v}");
    let seen = step("exec in the clone", || env.exec(&copy, &["cat", "/var/tmp/marker"]));
    assert_eq!(seen["stdout"], "from-the-source\n", "the clone does not hold the source's disk: {seen}");
    assert_eq!(env.exec(&copy, &["id", "-un"])["stdout"], "adytum\n");
    // A clone is its own cell: a change in it does not reach the source.
    assert_eq!(env.exec(&copy, &["rm", "/var/tmp/marker"])["status"], 0);
    assert_eq!(env.json(&["stop", &copy]).0, 0);

    // A held source is not cloned, however many times it is asked.
    assert_eq!(env.json(&["hold", &src, "eid_live"]).0, 0);
    let (code, v) = step("clone while held", || env.json(&["clone", &src, &third]));
    assert_eq!((code, v["error"]["code"].as_str()), (1, Some("clone_open_eidolon")), "{v}");
    assert_eq!(listed_names(&env).len(), 2, "a refused clone left a cell");
    assert!(!env.home.join("lima").join(&third).exists());
    let (code, v) = env.json(&["release", &src, "eid_live"]);
    assert_eq!((code, v["held"].as_bool()), (0, Some(false)), "{v}");

    // A clone across walls is refused as well.
    let (code, v) = env.json(&["clone", &src, &third, "--wall", "wall_other"]);
    assert_eq!((code, v["error"]["code"].as_str()), (1, Some("wall_clone")), "{v}");

    for name in [&copy, &src] {
        let (code, v) = step("rm", || env.json(&["rm", name]));
        assert_eq!((code, v["removed"].as_bool()), (0, Some(true)), "{v}");
    }
    assert_eq!(listed_names(&env).len(), 0);
}

/// `adytum create` killed with SIGKILL while Lima is making the instance:
/// the next command removes the half-made cell and closes the operation.
fn kill_during_create() {
    let env = new_env();
    let name = format!("adytum-test-live-{}", random_tag());
    println!("kill test: {name}, ADYTUM_HOME {}", env.home.display());
    let instance = env.home.join("lima").join(&name);

    let mut child = Command::new(BIN)
        .args(["create", &name, "--sealed", "--wall", "wall_live"])
        .env_clear()
        .env("ADYTUM_HOME", &env.home)
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the binary starts");
    let waited = Instant::now();
    while !instance.exists() {
        assert!(waited.elapsed() < Duration::from_secs(120), "Lima never began making the instance");
        assert!(child.try_wait().unwrap().is_none(), "adytum create ended before Lima began");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().expect("SIGKILL");
    child.wait().unwrap();
    println!("  killed adytum create {:.2}s after it started waiting", waited.elapsed().as_secs_f64());
    // Lima's own process would go on without it; a crash takes the machine's
    // processes with it, so stop whatever still carries this cell's name.
    let _ = Command::new("pkill").args(["-9", "-f", &name]).output();
    std::thread::sleep(Duration::from_millis(500));

    let ledger = adytum_ledger::Ledger::open(&env.home).unwrap();
    let open = ledger.open_ops().unwrap();
    assert_eq!(open.len(), 1, "the killed create left no open operation: {open:?}");
    assert_eq!(open[0].name.as_str(), name);
    drop(ledger);

    let (code, v) = step("ls after the kill (reconcile)", || env.json(&["ls"]));
    assert_eq!(code, 0, "{v}");
    let removed: Vec<_> = v["reconciled"]["removed"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(removed, [name.as_str()], "{v}");
    assert_eq!(v["cells"], serde_json::json!([]), "{v}");
    assert!(!instance.exists(), "the half-made instance folder is still there");
    let ledger = adytum_ledger::Ledger::open(&env.home).unwrap();
    assert_eq!(ledger.open_ops().unwrap().len(), 0, "an operation is still open");
    assert_eq!(ledger.cells().unwrap().len(), 0);
    drop(ledger);

    // The same name creates cleanly.
    let (code, v) = step("create the same name", || env.json(&["create", &name, "--sealed", "--wall", "wall_live"]));
    assert_eq!((code, v["state"].as_str()), (0, Some("stopped")), "{v}");
    assert_eq!(env.json(&["rm", &name]).0, 0);
    assert_eq!(listed_names(&env).len(), 0);
}

#[test]
fn a_sealed_cell_from_create_to_rm() {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") {
        println!("skipped: set ADYTUM_LIVE=1 to run against a real VM");
        return;
    }
    let before = default_lima_names();
    let total = Instant::now();
    sealed_cell(&before);
    published_cell();
    clone_cells();
    kill_during_create();
    assert_eq!(default_lima_names(), before);
    println!("total: {:.1}s", total.elapsed().as_secs_f64());
}
