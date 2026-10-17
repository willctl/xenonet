// xenonet hook, as Claude Code and Codex run it: the event goes to XenoNet
// by itself, quickly, and XenoNet's answer is what the agent hears; with
// no XenoNet there, or one that doesn't answer, it prints nothing and
// exits 0. The same cases as test/launcher.test.js's, against the binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct Ran {
    code: i32,
    stdout: String,
    stderr: String,
    took: Duration,
}

fn run(args: &[&str], input: &str, port_env: Option<u16>) -> Ran {
    let started = Instant::now();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xenonet"));
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).env_remove("XENONET_PORT");
    if let Some(p) = port_env {
        cmd.env("XENONET_PORT", p.to_string());
    }
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    Ran { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8(out.stdout).unwrap(), stderr: String::from_utf8(out.stderr).unwrap(), took: started.elapsed() }
}

// A XenoNet that hears requests and answers each with `status` and `body`:
// what it heard goes to the channel, as (path, Host, body).
fn xenonet(status: u16, body: &'static str) -> (u16, mpsc::Receiver<(String, String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for s in listener.incoming() {
            let mut s = s.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split(' ').nth(1).unwrap_or("").to_string();
            let (mut length, mut host) = (0, String::new());
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                let h = h.trim_end().to_string();
                if h.is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "host" => host = v.trim().to_string(),
                    _ => {}
                }
            }
            let mut got = vec![0; length];
            r.read_exact(&mut got).unwrap();
            tx.send((path, host, String::from_utf8(got).unwrap())).unwrap();
            write!(s, "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (port, rx)
}

#[test]
fn the_event_goes_to_xenonet_and_its_answer_is_what_the_agent_hears() {
    let (port, heard) = xenonet(200, r#"{"systemMessage":"from XenoNet"}"#);
    let out = run(&["hook", "codex"], r#"{"hook_event_name":"Stop"}"#, Some(port));
    assert_eq!((out.code, out.stdout.as_str(), out.stderr.as_str()), (0, r#"{"systemMessage":"from XenoNet"}"#, ""));
    assert_eq!(heard.recv().unwrap(), ("/api/hook/codex".into(), format!("127.0.0.1:{port}"), r#"{"hook_event_name":"Stop"}"#.into()));
    let claude = run(&["hook", "claude", "--port", &port.to_string()], "", None);
    assert_eq!(claude.code, 0);
    let (path, _, body) = heard.recv().unwrap();
    assert_eq!((path.as_str(), body.as_str()), ("/api/hook/claude", "{}"), "no event is an empty one");
}

#[test]
fn an_answer_other_than_200_says_nothing() {
    let (port, _heard) = xenonet(204, "");
    let out = run(&["hook", "claude"], "{}", Some(port));
    assert_eq!((out.code, out.stdout.as_str()), (0, ""));
}

#[test]
fn with_no_xenonet_there_it_prints_nothing_and_exits_0_quickly() {
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let out = run(&["hook", "codex"], "{}", Some(free));
    assert_eq!((out.code, out.stdout.as_str(), out.stderr.as_str()), (0, "", ""));
    assert!(out.took < Duration::from_secs(4), "quickly ({:?})", out.took);
}

#[test]
fn a_xenonet_that_doesnt_answer_is_given_up_on_after_4_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // It takes the connection and says nothing.
    let held = thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            thread::sleep(Duration::from_secs(6));
            drop(s);
        }
    });
    let out = run(&["hook", "codex"], "{}", Some(port));
    assert_eq!((out.code, out.stdout.as_str()), (0, ""));
    assert!(out.took >= Duration::from_millis(3900) && out.took < Duration::from_millis(5500), "{:?}", out.took);
    drop(held);
}
