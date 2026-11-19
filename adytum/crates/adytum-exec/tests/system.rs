//! Real processes. Unix-gated where a test leans on /usr/bin tools or
//! process groups.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use adytum_exec::{Cancel, Chunk, Command, ExecError, Runner, SystemRunner};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Res = Result<(), Box<dyn std::error::Error>>;

fn scratch(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("adytum-exec-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

const SAFE_PATH: &str = "/usr/bin:/bin";

fn gone(pid: i32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid), None) == Err(Errno::ESRCH)
}

async fn wait_gone(pid: i32) -> bool {
    for _ in 0..50 {
        if gone(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn arguments_arrive_literally_and_no_shell_runs() -> Res {
    let dir = scratch("args");
    let cmd = Command::new("printf").env("PATH", SAFE_PATH).cwd(&dir).args(["%s\n", "a b", "it's \"q\"", "$(touch x)", "`touch x`", ";", "*", "a;touch x"]);
    let out = SystemRunner::new().run(&cmd, &Cancel::new()).await?;
    assert!(out.success());
    assert_eq!(out.stdout_lossy(), "a b\nit's \"q\"\n$(touch x)\n`touch x`\n;\n*\na;touch x\n");
    assert!(!dir.join("x").exists(), "a shell ran one of the arguments");
    Ok(())
}

#[tokio::test]
async fn environment_holds_only_named_variables() -> Res {
    let cmd = Command::new("/usr/bin/env").env("ONLY_THIS", "1");
    let out = SystemRunner::new().run(&cmd, &Cancel::new()).await?;
    assert_eq!(out.stdout_lossy(), "ONLY_THIS=1\n");
    Ok(())
}

#[tokio::test]
async fn pass_env_copies_named_variables_only() -> Res {
    let cmd = Command::new("/usr/bin/env").pass_env(&["PATH"]);
    let out = SystemRunner::new().run(&cmd, &Cancel::new()).await?;
    let lines: Vec<String> = out.stdout_lossy().lines().map(str::to_owned).collect();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with("PATH="));
    Ok(())
}

#[tokio::test]
async fn deadline_stops_a_grandchild_too() -> Res {
    let dir = scratch("deadline");
    let pidfile = dir.join("pid");
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "sh"]).arg(&pidfile).deadline(Duration::from_millis(700));
    let runner = SystemRunner::new().with_grace(Duration::from_millis(300));
    let started = Instant::now();
    let out = runner.run(&cmd, &Cancel::new()).await?;
    assert!(out.timed_out);
    assert!(!out.success());
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid: i32 = std::fs::read_to_string(&pidfile)?.trim().parse()?;
    assert!(wait_gone(pid).await, "grandchild {pid} is still running");
    Ok(())
}

#[tokio::test]
async fn sigterm_ignorers_are_killed_after_the_grace() -> Res {
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "trap '' TERM; sleep 60"]).deadline(Duration::from_millis(300));
    let runner = SystemRunner::new().with_grace(Duration::from_millis(300));
    let started = Instant::now();
    let out = runner.run(&cmd, &Cancel::new()).await?;
    assert!(out.timed_out);
    assert!(started.elapsed() < Duration::from_secs(10));
    Ok(())
}

#[tokio::test]
async fn cancel_stops_the_whole_group() -> Res {
    let dir = scratch("cancel");
    let pidfile = dir.join("pid");
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "sleep 60 & echo $! > \"$1\"; wait", "sh"]).arg(&pidfile);
    let cancel = Cancel::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        trigger.cancel();
    });
    let out = SystemRunner::new().with_grace(Duration::from_millis(300)).run(&cmd, &cancel).await?;
    assert!(out.cancelled);
    assert!(!out.timed_out);
    let pid: i32 = std::fs::read_to_string(&pidfile)?.trim().parse()?;
    assert!(wait_gone(pid).await, "grandchild {pid} is still running");
    Ok(())
}

#[tokio::test]
async fn stream_delivers_output_before_exit() -> Res {
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "echo first; sleep 1.5; echo second"]);
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let task = tokio::spawn(async move { SystemRunner::new().stream(&cmd, &Cancel::new(), tx).await });
    let first = rx.recv().await.expect("a chunk");
    assert_eq!(first, Chunk::Stdout(b"first\n".to_vec()));
    assert!(!task.is_finished(), "the first chunk should arrive while the command still runs");
    let second = rx.recv().await.expect("another chunk");
    assert_eq!(second, Chunk::Stdout(b"second\n".to_vec()));
    let out = task.await??;
    assert!(out.success());
    Ok(())
}

#[tokio::test]
async fn stream_labels_stderr() -> Res {
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "echo oops >&2"]);
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let out = SystemRunner::new().stream(&cmd, &Cancel::new(), tx).await?;
    assert!(out.success());
    assert_eq!(rx.recv().await, Some(Chunk::Stderr(b"oops\n".to_vec())));
    Ok(())
}

#[tokio::test]
async fn duplex_round_trips_a_mebibyte_through_cat() -> Res {
    let cmd = Command::new("cat").env("PATH", SAFE_PATH);
    let duplex = SystemRunner::new().duplex(&cmd).await?;
    let sent: Vec<u8> = (0..1024 * 1024u32).map(|i| (i.wrapping_mul(31) >> 3) as u8).collect();
    let (mut rd, mut wr) = tokio::io::split(duplex);
    let to_send = sent.clone();
    let write = async move {
        wr.write_all(&to_send).await?;
        wr.shutdown().await?;
        Ok::<_, std::io::Error>(wr)
    };
    let read = async move {
        let mut back = Vec::new();
        rd.read_to_end(&mut back).await?;
        Ok::<_, std::io::Error>((rd, back))
    };
    let (wr, read) = tokio::join!(write, read);
    let (rd, back) = read?;
    assert_eq!(back.len(), sent.len());
    assert!(back == sent, "bytes came back different");
    let out = rd.unsplit(wr?).finish().await?;
    assert!(out.success());
    Ok(())
}

#[tokio::test]
async fn duplex_kill_stops_the_child() -> Res {
    let cmd = Command::new("sleep").env("PATH", SAFE_PATH).arg("60");
    let mut duplex = SystemRunner::new().duplex(&cmd).await?;
    duplex.kill();
    let out = tokio::time::timeout(Duration::from_secs(10), duplex.finish()).await??;
    assert_eq!(out.status, None);
    Ok(())
}

#[tokio::test]
async fn stdin_bytes_reach_the_child() -> Res {
    let cmd = Command::new("cat").env("PATH", SAFE_PATH).stdin_bytes(b"hello\nthere".to_vec());
    let out = SystemRunner::new().run(&cmd, &Cancel::new()).await?;
    assert_eq!(out.stdout, b"hello\nthere");
    Ok(())
}

#[tokio::test]
async fn a_missing_program_is_a_clear_error() {
    let cmd = Command::new("adytum-no-such-program").env("PATH", SAFE_PATH);
    let err = SystemRunner::new().run(&cmd, &Cancel::new()).await.expect_err("should not run");
    assert!(matches!(err, ExecError::NotFound { .. }));
    let message = err.to_string();
    assert!(message.contains("adytum-no-such-program") && message.contains("not found"), "{message}");

    let by_path = Command::new("/nonexistent/dir/tool");
    assert!(matches!(SystemRunner::new().run(&by_path, &Cancel::new()).await, Err(ExecError::NotFound { .. })));
    assert!(matches!(SystemRunner::new().run(&Command::new(""), &Cancel::new()).await, Err(ExecError::EmptyProgram)));
}

#[tokio::test]
async fn a_bare_name_is_looked_up_on_the_commands_path_not_ours() {
    // /usr/bin has printf; an empty directory on the command's PATH doesn't.
    let dir = scratch("emptypath");
    let cmd = Command::new("printf").env("PATH", dir.display().to_string()).arg("x");
    assert!(matches!(SystemRunner::new().run(&cmd, &Cancel::new()).await, Err(ExecError::NotFound { .. })));
}

#[tokio::test]
async fn a_failing_command_is_an_output_not_an_error() -> Res {
    let cmd = Command::new("sh").env("PATH", SAFE_PATH).args(["-c", "echo bad >&2; exit 3"]);
    let out = SystemRunner::new().run(&cmd, &Cancel::new()).await?;
    assert_eq!(out.status, Some(3));
    assert_eq!(out.stderr_lossy(), "bad\n");
    assert!(!out.success());
    Ok(())
}
