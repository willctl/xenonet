//! Records the golden lifecycle against the real `limactl`. Skipped unless
//! `ADYTUM_LIVE=1` and `ADYTUM_RECORD_GOLDEN=1`:
//!
//! ```text
//! ADYTUM_LIVE=1 ADYTUM_RECORD_GOLDEN=1 npm run cargo -- test -p adytum-lima --test record -- --nocapture
//! ```
//!
//! It uses a Lima folder of its own under the home folder and removes the
//! cell afterwards, also when a step fails.

mod common;
mod lifecycle;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adytum_driver::Driver;
use adytum_exec::SystemRunner;
use adytum_lima::Lima;
use adytum_testkit::{RecordingRunner, Transcript};
use serde_json::Value;

type Res = Result<(), Box<dyn std::error::Error>>;

/// Removes the cell and the folder on drop, from a thread of its own, so it
/// works while the test's runtime is busy unwinding.
struct Cleanup(PathBuf);

impl Cleanup {
    fn run(&self) {
        let home = self.0.clone();
        let _ = std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
                let _ = rt.block_on(Lima::system(home.clone()).remove(&common::name()));
            }
            let _ = std::fs::remove_dir_all(home.parent().unwrap_or(&home));
        })
        .join();
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.run();
    }
}

/// A list line without the machine's details: ports and process ids are 0,
/// and the long `config` is dropped.
fn sanitize_list(stdout: &str) -> String {
    let mut out = String::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(line) else { continue };
        obj.remove("config");
        for key in ["sshLocalPort", "hostAgentPID", "driverPID"] {
            if obj.contains_key(key) {
                obj.insert(key.to_owned(), Value::from(0));
            }
        }
        out.push_str(&Value::Object(obj).to_string());
        out.push('\n');
    }
    out
}

fn sanitize(mut t: Transcript) -> Transcript {
    for call in &mut t.calls {
        if call.argv.get(1).map(String::as_str) == Some("list") {
            call.stdout = sanitize_list(&call.stdout);
        }
        // limactl's progress lines carry timestamps and paths; a failing call keeps its message.
        if call.status == 0 {
            call.stderr.clear();
        }
    }
    t
}

#[tokio::test]
async fn record_the_lifecycle() -> Res {
    if std::env::var("ADYTUM_LIVE").as_deref() != Ok("1") || std::env::var("ADYTUM_RECORD_GOLDEN").as_deref() != Ok("1") {
        return Ok(());
    }
    let base = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".adytum-golden");
    let home = base.join("lima");
    let guard = Cleanup(home.clone());
    guard.run();
    std::fs::create_dir_all(&home)?;
    let work = base.join("work");
    std::fs::create_dir_all(&work)?;
    std::fs::write(work.join("in.txt"), lifecycle::FILE_BYTES)?;

    let recorder = Arc::new(RecordingRunner::new(SystemRunner::new(), "limactl", "2.1.2").with_var("lima_home", &home).with_var("work", &work));
    let lima = Lima::new(recorder.clone(), "limactl".into(), home);
    lifecycle::lifecycle(&lima, &work).await?;
    assert_eq!(std::fs::read(work.join("out.txt"))?, lifecycle::FILE_BYTES);

    sanitize(recorder.transcript()).write_to(Path::new(lifecycle::GOLDEN))?;
    Ok(())
}
