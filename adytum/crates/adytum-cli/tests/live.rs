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
    out: String,
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
        Result_ { code: out.status.code().unwrap_or(-1), out: String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr) }
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let mut all = args.to_vec();
        // --json goes before any `--`.
        let at = all.iter().position(|a| *a == "--").unwrap_or(all.len());
        all.insert(at, "--json");
        let r = self.run(&all);
        let value = serde_json::from_str(r.out.trim()).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {}", r.out));
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
        let (_, listed) = self.json(&["ls"]);
        for cell in listed["cells"].as_array().into_iter().flatten() {
            if let Some(name) = cell["name"].as_str().filter(|n| n.starts_with("adytum-test-live-")) {
                let _ = self.run(&["stop", name]);
                let _ = self.run(&["rm", name]);
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
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("{:05x}", (nanos ^ std::process::id().rotate_left(13)) & 0xf_ffff)
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
    assert!(listed["cells"].as_array().unwrap().is_empty(), "{listed}");
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
    assert_eq!(default_lima_names(), before);
    println!("total: {:.1}s", total.elapsed().as_secs_f64());
}
