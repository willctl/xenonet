//! A program XenoNet starts in the background shows no window on Windows
//! (`xenonet_base::process::Background::no_window`). What's checked is
//! the window, not the console host: CREATE_NO_WINDOW still gives each
//! console program a conhost.exe of its own, only one without a window, so
//! counting conhosts says nothing. The check runs where the service and
//! the supervisor do, in a process with no console to share: this test
//! starts itself again detached from its console, and that copy starts a
//! console program (ping) with no_window and looks at every visible
//! top-level window for one of the program's or its console host's.
#![cfg(windows)]

use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use xenonet_base::process::Background;

/// Set in the copy started without a console.
const HELPER: &str = "XENONET_NO_WINDOW_HELPER";
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    pid: u32,
    heap: usize,
    module: u32,
    threads: u32,
    ppid: u32,
    priority: i32,
    flags: u32,
    exe: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
    fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn EnumWindows(each: extern "system" fn(isize, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(window: isize, pid: *mut u32) -> u32;
    fn IsWindowVisible(window: isize) -> i32;
}

/// The processes whose parent is `pid`: a console program's conhost.exe.
#[expect(unsafe_code, reason = "ToolHelp32, which std doesn't wrap")]
fn children_of(pid: u32) -> Vec<u32> {
    const SNAPPROCESS: u32 = 0x2;
    let mut out = Vec::new();
    // SAFETY: the snapshot is read through an entry that outlives each
    // call, with its size set as ToolHelp32 asks, then closed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(SNAPPROCESS, 0);
        if snap == -1 {
            return out;
        }
        let mut e: ProcessEntry = std::mem::zeroed();
        e.size = u32::try_from(size_of::<ProcessEntry>()).unwrap_or(0);
        let mut more = Process32FirstW(snap, &raw mut e) != 0;
        while more {
            if e.ppid == pid {
                out.push(e.pid);
            }
            more = Process32NextW(snap, &raw mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[expect(unsafe_code, reason = "EnumWindows' callback, with the list it was given")]
extern "system" fn note_window(window: isize, data: isize) -> i32 {
    // SAFETY: `data` is the list visible_windows passed, which outlives
    // EnumWindows; the window is asked about while it enumerates.
    unsafe {
        let seen = &mut *(data as *mut Vec<u32>);
        let mut pid = 0u32;
        if IsWindowVisible(window) != 0 {
            GetWindowThreadProcessId(window, &raw mut pid);
            seen.push(pid);
        }
    }
    1
}

/// The processes that own a visible top-level window.
#[expect(unsafe_code, reason = "EnumWindows, which std doesn't wrap")]
fn visible_windows() -> Vec<u32> {
    let mut seen: Vec<u32> = Vec::new();
    // SAFETY: the callback gets a pointer to `seen`, which outlives the call.
    unsafe {
        EnumWindows(note_window, (&raw mut seen) as isize);
    }
    seen
}

/// ping for ten seconds, a console program, with or without no_window.
fn console_program(hidden: bool) -> Child {
    let mut c = Command::new("ping.exe");
    if hidden {
        c.no_window();
    }
    c.args(["-n", "10", "127.0.0.1"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    c.spawn().expect("ping.exe starts")
}

/// The most visible windows the program or its console host had at once
/// within two seconds of starting (stopping at the first, `first`).
fn windows_of(mut child: Child, first: bool) -> usize {
    let until = Instant::now() + Duration::from_secs(2);
    let mut most = 0;
    while Instant::now() < until {
        let mut ours = children_of(child.id());
        ours.push(child.id());
        let n = visible_windows().iter().filter(|p| ours.contains(p)).count();
        most = most.max(n);
        if first && n > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    most
}

#[test]
#[ignore = "started by a_console_program_started_with_no_window_shows_none, in a process with no console"]
fn report_the_windows_a_console_program_shows() {
    if std::env::var_os(HELPER).is_none() {
        return;
    }
    let quiet = windows_of(console_program(true), false);
    let shown = windows_of(console_program(false), true);
    println!("RESULT {quiet} {shown}");
}

#[test]
fn a_console_program_started_with_no_window_shows_none() {
    let me = std::env::current_exe().unwrap();
    let mut c = Command::new(me);
    c.args(["--ignored", "--exact", "report_the_windows_a_console_program_shows", "--nocapture", "--test-threads=1"])
        .env(HELPER, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(DETACHED_PROCESS);
    let out = c.output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    // The test harness prints "test <name> ... " before the test's own
    // output, on the same line, so the result can start mid-line.
    let line = text
        .lines()
        .find_map(|l| l.split_once("RESULT ").map(|(_, rest)| rest))
        .unwrap_or_else(|| panic!("the detached copy said nothing:\n{text}\n{}", String::from_utf8_lossy(&out.stderr)));
    let counts: Vec<usize> = line.split_whitespace().take(2).map(|n| n.parse().unwrap()).collect();
    // Without no_window, the same program from the same detached process
    // gets a console window of its own, where the session has a desktop to
    // show one on: that's what no_window is for.
    eprintln!("without no_window, ping showed {} visible window(s) here", counts[1]);
    assert_eq!(counts[0], 0, "a console program started with no_window showed a visible window");
}
