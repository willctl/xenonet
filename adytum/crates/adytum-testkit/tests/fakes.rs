use adytum_exec::{Cancel, Chunk, Command, ExecError, Runner, SystemRunner};
use adytum_testkit::{FakeRunner, RecordingRunner, Transcript};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Res = Result<(), Box<dyn std::error::Error>>;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/lima_list.json");

fn fake() -> Result<FakeRunner, Box<dyn std::error::Error>> {
    Ok(FakeRunner::from_file(FIXTURE)?.with_var("home", "/h"))
}

#[tokio::test]
async fn replays_calls_in_order_with_placeholders() -> Res {
    let fake = fake()?;
    let cancel = Cancel::new();
    let out = fake.run(&Command::new("/opt/bin/limactl").args(["list", "--json"]), &cancel).await?;
    assert!(out.success());
    assert_eq!(out.stdout_lossy(), "{\"name\":\"adytum-test-a\",\"dir\":\"/h/.lima/adytum-test-a\"}\n");

    // {any} matches one argument, {home} is the variable's value.
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let out = fake.stream(&Command::new("limactl").args(["start", "--name", "whatever-name", "/h/tpl.yaml"]), &cancel, tx).await?;
    assert_eq!(out.status, Some(0));
    assert_eq!(rx.recv().await, Some(Chunk::Stderr(b"started\n".to_vec())));

    let mut duplex = fake.duplex(&Command::new("limactl").args(["shell", "adytum-test-a", "--", "cat"])).await?;
    duplex.write_all(b"ping").await?;
    duplex.shutdown().await?;
    let mut back = String::new();
    duplex.read_to_string(&mut back).await?;
    assert_eq!(back, "pong\n");
    assert!(duplex.finish().await?.success());
    assert_eq!(fake.duplex_writes(), vec![b"ping".to_vec()]);
    fake.assert_done();
    Ok(())
}

#[tokio::test]
async fn an_out_of_order_call_fails_with_a_readable_diff() -> Res {
    let fake = fake()?;
    let err = fake.run(&Command::new("limactl").args(["list", "--format", "json"]), &Cancel::new()).await.expect_err("should not match");
    let ExecError::Unexpected { message } = err else { panic!("wrong error") };
    assert!(message.contains("expected: limactl list --json"), "{message}");
    assert!(message.contains("actual:   limactl list --format json"), "{message}");
    assert!(message.contains("position 2"), "{message}");
    // The call stays expected, so the right one still works afterwards.
    fake.run(&Command::new("limactl").args(["list", "--json"]), &Cancel::new()).await?;
    Ok(())
}

#[tokio::test]
#[should_panic(expected = "did not match")]
async fn assert_done_remembers_a_mismatch_the_caller_swallowed() {
    let fake = fake().expect("fixture");
    let _ = fake.run(&Command::new("podman").arg("ps"), &Cancel::new()).await;
    fake.assert_done();
}

#[tokio::test]
#[should_panic(expected = "never happened")]
async fn assert_done_fails_when_calls_remain() {
    let fake = fake().expect("fixture");
    fake.run(&Command::new("limactl").args(["list", "--json"]), &Cancel::new()).await.expect("first call");
    fake.assert_done();
}

#[tokio::test]
async fn a_call_after_the_transcript_ends_fails() -> Res {
    let fake = FakeRunner::from_json(r#"{"tool":"podman","version":"5","calls":[]}"#)?;
    let err = fake.run(&Command::new("podman").arg("ps"), &Cancel::new()).await.expect_err("no calls left");
    assert!(err.to_string().contains("no calls left"));
    Ok(())
}

#[tokio::test]
async fn stdin_is_checked_when_the_golden_names_it() -> Res {
    let fake = FakeRunner::from_json(r#"{"tool":"t","version":"1","calls":[{"argv":["t"],"stdin":"yes","stdout":"ok","stderr":"","status":0}]}"#)?;
    let err = fake.run(&Command::new("t").stdin_bytes(b"no".to_vec()), &Cancel::new()).await.expect_err("stdin differs");
    assert!(err.to_string().contains("stdin"), "{err}");
    assert!(fake.run(&Command::new("t").stdin_bytes(b"yes".to_vec()), &Cancel::new()).await?.success());
    Ok(())
}

#[tokio::test]
async fn a_bad_golden_file_says_what_to_fix() {
    let missing = FakeRunner::from_file("/nonexistent/golden.json").err().expect("missing file");
    assert!(missing.to_string().contains("Could not read"));
    let bad = FakeRunner::from_json("{").err().expect("bad json");
    assert!(bad.to_string().contains("not a valid transcript"));
}

#[cfg(unix)]
#[tokio::test]
async fn records_a_real_echo_then_replays_it() -> Res {
    let recorder = RecordingRunner::new(SystemRunner::new(), "echo", "test");
    let home = std::env::var("HOME").unwrap_or_else(|_| "/nonexistent-home".to_owned());
    let cmd = Command::new("/bin/echo").arg("hello").arg(format!("{home}/x"));
    let real = recorder.run(&cmd, &Cancel::new()).await?;
    assert_eq!(real.stdout_lossy(), format!("hello {home}/x\n"));

    let transcript = recorder.transcript();
    assert_eq!(transcript.calls.len(), 1);
    assert_eq!(transcript.calls[0].argv[2], "{home}/x");
    assert!(!transcript.to_json()?.contains(&home), "the home folder leaked into the golden");

    // Through a file and back, then replayed somewhere else.
    let path = std::env::temp_dir().join(format!("adytum-testkit-{}.json", std::process::id()));
    transcript.write_to(&path)?;
    let fake = FakeRunner::from_transcript(Transcript::from_file(&path)?).with_var("home", "/elsewhere");
    let _ = std::fs::remove_file(&path);
    let replay = fake.run(&Command::new("echo").arg("hello").arg("/elsewhere/x"), &Cancel::new()).await?;
    assert_eq!(replay.stdout_lossy(), "hello /elsewhere/x\n");
    fake.assert_done();
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn records_a_duplex_run_with_what_was_written() -> Res {
    let recorder = RecordingRunner::new(SystemRunner::new(), "cat", "test");
    let mut duplex = recorder.duplex(&Command::new("/bin/cat")).await?;
    duplex.write_all(b"round trip").await?;
    duplex.shutdown().await?;
    let mut back = String::new();
    duplex.read_to_string(&mut back).await?;
    assert!(duplex.finish().await?.success());
    let call = &recorder.transcript().calls[0];
    assert_eq!(call.stdin.as_deref(), Some("round trip"));
    assert_eq!(call.stdout, "round trip");
    Ok(())
}
