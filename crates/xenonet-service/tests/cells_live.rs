//! A start confined to a cell, against a real Lima VM. Skipped unless
//! `ADYTUM_LIVE=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 cargo test -p xenonet-service --test cells_live -- --nocapture
//! ```
//!
//! It uses an Adytum home of its own under /tmp (short, for Lima's unix
//! sockets), names its cell `adytum-test-*`, and removes the cell and the
//! folder at the end, also when a step fails. Needs about 6 GB free.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use xenonet_engine::confine::{run_confined, wall_for_identity, CellApp, CellRequest, Confinement};
use xenonet_service::cells::CellRunner;

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        // Whatever the ledger lost track of: delete any instance left in
        // this home's own Lima folder, by Lima's own command.
        if let Ok(entries) = std::fs::read_dir(self.0.join("lima")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("adytum-test-") {
                    let _ = Command::new("limactl")
                        .args(["delete", "--force", "--tty=false", &name])
                        .env_clear()
                        .env("LIMA_HOME", self.0.join("lima"))
                        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
                        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                        .output();
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn instances_in(home: &Path) -> Vec<String> {
    let out = Command::new("limactl")
        .args(["list", "--format", "{{.Name}}"])
        .env_clear()
        .env("LIMA_HOME", home.join("lima"))
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .expect("limactl runs");
    String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect()
}

#[test]
fn a_start_in_a_cell_is_walled_in() {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") {
        println!("skipped: set ADYTUM_LIVE=1 to run against a real Lima VM");
        return;
    }
    let tag = format!("{:x}", std::process::id() ^ 0x5a5a);
    let home = Home(PathBuf::from("/tmp").join(format!("axo-{tag}")));
    let project = home.0.join("proj");
    let out_dir = home.0.join("out");
    std::fs::create_dir_all(&project).unwrap();

    // Something on the host that the cell must not reach: a file, and a
    // port on the host's loopback.
    let secret = home.0.join("host-secret.txt");
    std::fs::write(&secret, "host only\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // The fake agent: a script the cell runs with /bin/sh.
    let script = format!(
        r#"echo hi > inside.txt && echo inside_write_ok
if (echo x > /etc/xn-outside) 2>/dev/null; then echo OUTSIDE_WRITE_OK; else echo outside_write_denied; fi
if cat '{secret}' >/dev/null 2>&1; then echo HOST_READ_OK; else echo host_read_denied; fi
if ls /Users >/dev/null 2>&1; then echo HOST_USERS_VISIBLE; else echo users_absent; fi
if timeout 5 bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}' 2>/dev/null; then echo LOOPBACK_REACHABLE; else echo loopback_unreachable; fi
gw=$(getent hosts host.lima.internal | cut -d' ' -f1)
if [ -n "$gw" ] && timeout 5 bash -c "exec 3<>/dev/tcp/$gw/{port}" 2>/dev/null; then echo HOSTNAME_REACHABLE; else echo hostname_unreachable; fi
echo "home=$HOME pwd=$(pwd)"
env | sort | cut -d= -f1 | tr '\n' ' '; echo
"#,
        secret = secret.display(),
    );
    std::fs::write(project.join("agent.sh"), script).unwrap();

    let identity = "git:live-test";
    let confinement = Confinement::Cell { wall: wall_for_identity(identity).unwrap() };
    let runner = CellRunner::lima(home.0.clone(), vec![CellApp { app: "fake-agent".into(), program: "/bin/sh".into() }]).unwrap().with_prefix("adytum-test-").unwrap();
    let args = ["agent.sh".to_owned()];
    let host_home = std::env::var("HOME").unwrap_or_default();
    let started = std::time::Instant::now();
    let run = run_confined(
        &runner,
        &CellRequest {
            confinement: &confinement,
            identity_id: identity,
            app: "fake-agent",
            args: &args,
            stdin: None,
            project: &project,
            offered_env: &[("LANG".to_owned(), "C.UTF-8".to_owned())],
            host_home: &host_home,
            copy_out: Some(&out_dir),
            keep: false,
            deadline: Some(std::time::Duration::from_secs(120)),
        },
    )
    .expect("the start runs in a cell");
    println!("run took {:.1}s in {}", started.elapsed().as_secs_f64(), run.cell);
    let said = String::from_utf8_lossy(&run.stdout).into_owned();
    println!("{said}\nstderr: {}", String::from_utf8_lossy(&run.stderr));

    assert_eq!(run.status, Some(0), "{said}");
    assert!(said.contains("inside_write_ok"), "{said}");
    assert!(said.contains("outside_write_denied") && !said.contains("OUTSIDE_WRITE_OK"), "{said}");
    assert!(said.contains("host_read_denied") && !said.contains("HOST_READ_OK"), "{said}");
    assert!(said.contains("users_absent") && !said.contains("HOST_USERS_VISIBLE"), "{said}");
    assert!(said.contains("loopback_unreachable") && !said.contains("LOOPBACK_REACHABLE"), "{said}");
    assert!(said.contains("hostname_unreachable") && !said.contains("HOSTNAME_REACHABLE"), "{said}");
    assert!(said.contains("home=/tmp/xn-home pwd=/tmp/xn-in/proj"), "{said}");
    assert!(!said.contains(&host_home), "the host's home folder reached the cell: {said}");
    let names = said.lines().last().unwrap_or_default();
    for banned in ["XENONET", "TOKEN", "SSH_AUTH_SOCK", "USER"] {
        assert!(!names.contains(banned), "{banned} reached the cell: {names}");
    }

    // The write landed in the cell's copy and came out; the host's own
    // project folder is untouched.
    assert_eq!(std::fs::read_to_string(out_dir.join("proj/inside.txt")).unwrap(), "hi\n");
    assert!(!project.join("inside.txt").exists(), "the cell wrote into the host's project folder");

    // The cell is gone, from Lima and from the run's report.
    assert!(run.removed);
    assert!(instances_in(&home.0).iter().all(|n| !n.starts_with("adytum-test-")), "{:?}", instances_in(&home.0));
    drop(listener);
}
