//! Replays every recorded or documented lifecycle through the fake: the
//! driver asks the tool exactly what the golden says, and reads its answers.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use adytum_model::DriverKind;
use adytum_oci::{Oci, OciTool};
use adytum_testkit::FakeRunner;

type Res = Result<(), Box<dyn std::error::Error>>;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");

fn goldens() -> Vec<(String, PathBuf)> {
    let mut found: Vec<_> = std::fs::read_dir(GOLDEN)
        .expect("the golden folder")
        .filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().join("lifecycle.json")))
        .filter(|(_, file)| file.is_file())
        .collect();
    found.sort();
    found
}

#[tokio::test]
async fn every_golden_lifecycle_replays() -> Res {
    let all = goldens();
    assert!(all.iter().any(|(n, _)| n.starts_with("podman-")), "no podman golden");
    assert!(all.iter().any(|(n, _)| n.starts_with("docker-")), "no docker golden");
    for (dir, file) in all {
        let (tool, kind) = if dir.starts_with("podman-") { (OciTool::Podman, DriverKind::Podman) } else { (OciTool::Docker, DriverKind::Docker) };
        let work = std::env::temp_dir().join(format!("adytum-oci-replay-{dir}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)?;
        std::fs::write(work.join("in.txt"), common::FILE_BYTES)?;

        let fake = Arc::new(FakeRunner::from_file(&file)?.with_var("work", &work));
        let oci = Oci::new(fake.clone(), tool, PathBuf::from(if tool == OciTool::Podman { "podman" } else { "docker" }));
        common::lifecycle(&oci, kind, &work).await?;
        fake.assert_done();
        std::fs::remove_dir_all(work)?;
    }
    Ok(())
}

#[test]
fn golden_files_hold_no_path_or_name_of_the_machine_they_were_recorded_on() -> Res {
    for (dir, file) in goldens() {
        let text = std::fs::read_to_string(&file)?;
        assert!(!text.contains("/Users/"), "{dir}: a home path");
        for var in ["HOME", "USER", "LOGNAME"] {
            if let Some(value) = std::env::var_os(var).map(|v| v.to_string_lossy().into_owned()).filter(|v| v.len() > 3) {
                assert!(!text.contains(&value), "{dir}: {var} appears in the golden file");
            }
        }
        if dir.starts_with("docker-") {
            assert!(text.contains("written from docker's documentation"), "{dir}: say it was not recorded");
        }
    }
    Ok(())
}
