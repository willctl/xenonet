//! What the servers read of their own process, as Node gives it: the
//! environment, the parent's pid (process.ppid), whether a pid is running
//! (process.kill(pid, 0)) and a process's command line. The parent is the
//! agent that started the server (Claude Code, Codex or an actor), which
//! XenoNet watches, so this works on every platform the binary is built
//! for.
#![expect(unsafe_code, reason = "kill(2) and the Windows process calls, which std doesn't wrap")]

/// A variable as `process.env.X` reads it: None when it isn't set (an
/// empty one is "", as in JS).
pub fn var(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

/// A variable that JS tests for truth: set and not empty.
pub fn set(name: &str) -> Option<String> {
    var(name).filter(|v| !v.is_empty())
}

/// process.cwd().
pub fn cwd() -> String {
    std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

/// process.ppid.
#[cfg(unix)]
pub fn ppid() -> u32 {
    std::os::unix::process::parent_id()
}

/// process.ppid: Windows has no call for it, so it's read from a snapshot
/// of the processes, as libuv does.
#[cfg(windows)]
pub fn ppid() -> u32 {
    windows::parent_of(std::process::id()).unwrap_or(0)
}

/// Whether `pid` is a running process this account may signal, as
/// process.kill(pid, 0) says without throwing. Node takes only a pid that's
/// a 32-bit integer.
pub fn alive(pid: f64) -> bool {
    if pid.fract() != 0.0 || pid < f64::from(i32::MIN) || pid > f64::from(i32::MAX) {
        return false;
    }
    alive_pid(pid as i32)
}

#[cfg(unix)]
fn alive_pid(pid: i32) -> bool {
    // SAFETY: kill with signal 0 sends nothing; it only checks.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(windows)]
fn alive_pid(pid: i32) -> bool {
    windows::alive(pid as u32)
}

/// A process's command line, as mcp/xenonet.js reads it: /proc on Linux,
/// ps on macOS, and nothing elsewhere (Windows, where XENONET_CHANNEL says
/// instead).
pub fn cmdline(pid: f64) -> String {
    #[cfg(target_os = "linux")]
    {
        std::fs::read(format!("/proc/{}/cmdline", xenonet_base::json::number_string(pid))).map(|b| String::from_utf8_lossy(&b).replace('\0', " ")).unwrap_or_default()
    }
    #[cfg(target_os = "macos")]
    {
        ps_args(pid).unwrap_or_default()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        String::new()
    }
}

// `ps -o args= -p <pid>`, given two seconds, as execFileSync with a
// timeout would.
#[cfg(target_os = "macos")]
fn ps_args(pid: f64) -> Option<String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child =
        Command::new("ps").args(["-o", "args=", "-p", &xenonet_base::json::number_string(pid)]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if Instant::now() >= until => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out)
}

#[cfg(windows)]
mod windows {
    // The few kernel32 calls this needs, declared here rather than through
    // a crate of bindings.
    #[repr(C)]
    #[expect(non_snake_case, reason = "the fields keep the names PROCESSENTRY32W has in Windows's headers")]
    struct ProcessEntry32W {
        dwSize: u32,
        cntUsage: u32,
        th32ProcessID: u32,
        th32DefaultHeapID: usize,
        th32ModuleID: u32,
        cntThreads: u32,
        th32ParentProcessID: u32,
        pcPriClassBase: i32,
        dwFlags: u32,
        szExeFile: [u16; 260],
    }

    const TH32CS_SNAPPROCESS: u32 = 0x2;
    const INVALID_HANDLE_VALUE: isize = -1;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const STILL_ACTIVE: u32 = 259;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    pub fn parent_of(pid: u32) -> Option<u32> {
        // SAFETY: the snapshot is closed on every path, and the entry is
        // sized as the calls require.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
                return None;
            }
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut found = None;
            let mut more = Process32FirstW(snapshot, &mut entry) != 0;
            while more {
                if entry.th32ProcessID == pid {
                    found = Some(entry.th32ParentProcessID);
                    break;
                }
                more = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            found
        }
    }

    // As libuv's uv_kill(pid, 0): a process it can open that hasn't exited.
    pub fn alive(pid: u32) -> bool {
        // SAFETY: the handle is closed once it's been asked.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION | SYNCHRONIZE, 0, pid);
            if h == 0 {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
            CloseHandle(h);
            ok
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_and_its_parent() {
        assert!(ppid() > 0);
        assert!(alive(f64::from(std::process::id())));
        assert!(!alive(2f64.powi(30)), "a pid nobody has");
        assert!(!alive(1.5) && !alive(2f64.powi(40)));
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        assert_ne!(cmdline(f64::from(std::process::id())), "");
    }
}
