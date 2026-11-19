mod common;
mod lifecycle;

use std::sync::Arc;

use adytum_lima::Lima;
use adytum_testkit::FakeRunner;

type Res = Result<(), Box<dyn std::error::Error>>;

// A short path: Lima keeps unix sockets under the folder and they hold about 100 bytes.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from("/tmp").join(format!("adl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    dir
}

#[tokio::test]
async fn the_recorded_lifecycle_replays_through_the_fake() -> Res {
    let dir = scratch("replay");
    let home = dir.join("lima");
    let work = dir.join("work");
    std::fs::create_dir_all(&home)?;
    std::fs::create_dir_all(&work)?;
    std::fs::write(work.join("in.txt"), lifecycle::FILE_BYTES)?;

    let fake = Arc::new(FakeRunner::from_file(lifecycle::GOLDEN)?.with_var("lima_home", &home).with_var("work", &work));
    let lima = Lima::new(fake.clone(), "limactl".into(), home);
    lifecycle::lifecycle(&lima, &work).await?;
    fake.assert_done();
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn the_golden_file_holds_no_path_or_name_of_the_machine_it_was_recorded_on() -> Res {
    let text = std::fs::read_to_string(lifecycle::GOLDEN)?;
    assert!(!text.contains("/Users/"), "a home path");
    for var in ["HOME", "USER", "LOGNAME"] {
        if let Some(value) = std::env::var_os(var).map(|v| v.to_string_lossy().into_owned()).filter(|v| v.len() > 3) {
            assert!(!text.contains(&value), "{var} appears in the golden file");
        }
    }
    assert!(text.contains("{lima_home}"));
    Ok(())
}
