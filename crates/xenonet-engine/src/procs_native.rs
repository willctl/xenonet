//! The calls procs makes on Linux and Windows (`procs::PROCESSES`,
//! `TCP_PEER` and `COMMAND_LINE`), answered from the system itself rather
//! than by a command: on Linux from /proc, on Windows from a ToolHelp32
//! snapshot, each process's image path and creation time, and its command
//! line. Each answer is text as procs reads it (`procs::parse_table`), so
//! a test answers the same calls with a made-up table.

use crate::procs::{Call, Failure, COMMAND_LINE, PROCESSES, TCP_PEER};

/// The answer to one of procs' own calls, or None for any other call,
/// which is a command to run.
pub fn answer(call: &Call) -> Option<Result<String, Failure>> {
    let file = call.file.as_str();
    if ![PROCESSES, TCP_PEER, COMMAND_LINE].contains(&file) {
        return None;
    }
    Some(imp::answer(file, &call.args).ok_or_else(|| Failure { message: format!("{file} couldn't be read on this system"), stdout: None }))
}

// A field of a table line: no tabs or line breaks of its own.
#[cfg(any(target_os = "linux", windows))]
fn field(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

#[cfg(target_os = "linux")]
mod imp {
    use crate::procs::{parse_proc_net_tcp, COMMAND_LINE, PROCESSES, TCP_PEER};
    use std::fmt::Write;
    use std::fs;

    pub fn answer(file: &str, args: &[String]) -> Option<String> {
        match file {
            PROCESSES => Some(table()),
            TCP_PEER => {
                let (local, remote) = (args.first()?.parse().ok()?, args.get(1)?.parse().ok()?);
                Some(tcp_peer(local, remote).map(|p| p.to_string()).unwrap_or_default())
            }
            COMMAND_LINE => command_line(args.first()?),
            _ => None,
        }
    }

    // When the machine booted, in ms (UTC): /proc/stat's btime.
    fn boot_ms() -> f64 {
        let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
        stat.lines().find_map(|l| l.strip_prefix("btime ")).and_then(|v| v.trim().parse::<f64>().ok()).map_or(0.0, |s| s * 1000.0)
    }

    fn pids() -> Vec<u32> {
        fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
    }

    fn argv(pid: &str) -> Vec<String> {
        let bytes = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        bytes.split(|b| *b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
    }

    /// Every process: its parent and start time from /proc/<pid>/stat
    /// (start in clock ticks since boot, which Linux shows userspace at 100
    /// a second), the program it says it is (`argv[0]`, or its name), and
    /// its executable (/proc/<pid>/exe, which another user's process
    /// doesn't show).
    fn table() -> String {
        let boot = boot_ms();
        let mut out = String::new();
        for pid in pids() {
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
            // The name is in parentheses and may hold anything, spaces and
            // parentheses too: the fields come after the last ")".
            let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
            let name = &stat[open + 1..close];
            let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
            let (Some(ppid), Some(ticks)) = (rest.get(1), rest.get(19).and_then(|t| t.parse::<f64>().ok())) else { continue };
            let p = pid.to_string();
            let program = argv(&p).into_iter().next().unwrap_or_else(|| name.to_string());
            let exe = fs::read_link(format!("/proc/{pid}/exe")).map(|e| e.to_string_lossy().trim_end_matches(" (deleted)").to_string()).unwrap_or_default();
            let start = boot + ticks * 10.0;
            let _ = writeln!(out, "{pid}\t{ppid}\t{start}\t{}\t{}", super::field(&program), super::field(&exe));
        }
        out
    }

    /// The process holding the client's end of a connection to 127.0.0.1:
    /// its socket's inode from /proc/net/tcp, then the process with that
    /// socket open, among this user's.
    fn tcp_peer(local: u16, remote: u16) -> Option<u32> {
        let inode = parse_proc_net_tcp(&fs::read_to_string("/proc/net/tcp").ok()?, local, remote)?;
        let target = format!("socket:[{inode}]");
        pids()
            .into_iter()
            .find(|pid| fs::read_dir(format!("/proc/{pid}/fd")).into_iter().flatten().flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|l| l.to_string_lossy() == target)))
    }

    fn command_line(pid: &str) -> Option<String> {
        pid.parse::<u32>().ok()?;
        Some(argv(pid).join(" "))
    }
}

#[cfg(windows)]
mod imp {
    use crate::procs::{COMMAND_LINE, PROCESSES};
    use std::fmt::Write;

    const SNAPPROCESS: u32 = 0x2;
    const QUERY_LIMITED: u32 = 0x1000;
    const COMMAND_LINE_INFORMATION: u32 = 60;
    // FILETIME's 100 ns ticks from 1601 to 1970.
    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

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

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum: u16,
        buffer: *const u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn QueryFullProcessImageNameW(process: isize, flags: u32, name: *mut u16, size: *mut u32) -> i32;
        fn GetProcessTimes(process: isize, created: *mut FileTime, exited: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(process: isize, class: u32, info: *mut core::ffi::c_void, length: u32, returned: *mut u32) -> i32;
    }

    pub fn answer(file: &str, args: &[String]) -> Option<String> {
        match file {
            PROCESSES => table(),
            COMMAND_LINE => command_line(args.first()?.parse().ok()?),
            // Windows asks netstat -ano which process holds a connection.
            _ => None,
        }
    }

    fn text(wide: &[u16]) -> String {
        String::from_utf16_lossy(&wide[..wide.iter().position(|c| *c == 0).unwrap_or(wide.len())])
    }

    /// A process's image path and creation time (ms, UTC), where it can be
    /// opened: not the system's own, nor another user's elevated ones.
    #[expect(unsafe_code, reason = "OpenProcess, QueryFullProcessImageNameW and GetProcessTimes, which std doesn't wrap")]
    fn details(pid: u32) -> (Option<String>, f64) {
        // SAFETY: the handle is asked two questions, through buffers and
        // locals that outlive the calls, then closed.
        unsafe {
            let h = OpenProcess(QUERY_LIMITED, 0, pid);
            if h == 0 {
                return (None, 0.0);
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let exe = (QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &raw mut len) != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]));
            let (mut created, mut exited, mut kernel, mut user) = (FileTime::default(), FileTime::default(), FileTime::default(), FileTime::default());
            let start = if GetProcessTimes(h, &raw mut created, &raw mut exited, &raw mut kernel, &raw mut user) != 0 {
                let ticks = (u64::from(created.high) << 32) | u64::from(created.low);
                ticks.saturating_sub(UNIX_EPOCH_TICKS) as f64 / 10_000.0
            } else {
                0.0
            };
            CloseHandle(h);
            (exe, start)
        }
    }

    /// Every process, from a ToolHelp32 snapshot: its parent, and its image
    /// path and start time where it can be opened (else its image's name).
    #[expect(unsafe_code, reason = "ToolHelp32, which std doesn't wrap")]
    fn table() -> Option<String> {
        let mut rows = Vec::new();
        // SAFETY: the snapshot is walked through an entry that outlives each
        // call, its size set as ToolHelp32 asks, then closed.
        unsafe {
            let snap = CreateToolhelp32Snapshot(SNAPPROCESS, 0);
            if snap == -1 {
                return None;
            }
            let mut e: ProcessEntry = std::mem::zeroed();
            e.size = size_of::<ProcessEntry>() as u32;
            let mut more = Process32FirstW(snap, &raw mut e) != 0;
            while more {
                rows.push((e.pid, e.ppid, text(&e.exe)));
                more = Process32NextW(snap, &raw mut e) != 0;
            }
            CloseHandle(snap);
        }
        let mut out = String::new();
        for (pid, ppid, name) in rows {
            let (exe, start) = details(pid);
            let program = exe.clone().unwrap_or(name);
            let _ = writeln!(out, "{pid}\t{ppid}\t{start}\t{}\t{}", super::field(&program), super::field(&exe.unwrap_or_default()));
        }
        Some(out)
    }

    /// A process's command line, as NtQueryInformationProcess gives it
    /// (Windows 8.1 and later): a UNICODE_STRING, its text right after it
    /// in the same buffer.
    #[expect(unsafe_code, reason = "NtQueryInformationProcess, which std doesn't wrap")]
    fn command_line(pid: u32) -> Option<String> {
        // SAFETY: the handle is asked once for the size and once into a
        // buffer of that size, aligned for the UNICODE_STRING at its start,
        // whose text lies within the same buffer; then closed.
        unsafe {
            let h = OpenProcess(QUERY_LIMITED, 0, pid);
            if h == 0 {
                return None;
            }
            let mut needed = 0u32;
            NtQueryInformationProcess(h, COMMAND_LINE_INFORMATION, std::ptr::null_mut(), 0, &raw mut needed);
            let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(4)];
            let size = (buf.len() * 8) as u32;
            let status = NtQueryInformationProcess(h, COMMAND_LINE_INFORMATION, buf.as_mut_ptr().cast(), size, &raw mut needed);
            CloseHandle(h);
            if status < 0 {
                return None;
            }
            let s = &*(buf.as_ptr().cast::<UnicodeString>());
            if s.buffer.is_null() {
                return Some(String::new());
            }
            Some(String::from_utf16_lossy(std::slice::from_raw_parts(s.buffer, usize::from(s.length) / 2)))
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    pub fn answer(_file: &str, _args: &[String]) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_procs_own_calls_are_answered_here() {
        assert!(answer(&Call::new("ps", &["-axo", "pid="])).is_none(), "a command is run");
        assert!(answer(&Call::new(PROCESSES, &[])).is_some());
    }

    // This process and its parent, read from the system itself.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn this_process_is_in_the_table_with_its_executable() {
        let text = answer(&Call::new(PROCESSES, &[])).unwrap().unwrap();
        let (table, exes) = crate::procs::parse_table(&text);
        let me = f64::from(std::process::id());
        let row = table.get(me).unwrap_or_else(|| panic!("{me} isn't in the table"));
        assert!(row.start > 1.6e12, "a start time in ms: {}", row.start);
        assert!(table.get(row.ppid).is_some(), "its parent is there too");
        let exe = exes.get(me).unwrap();
        let real = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(std::fs::canonicalize(exe).unwrap(), real);
        let line = answer(&Call::new(COMMAND_LINE, &[&std::process::id().to_string()])).unwrap().unwrap();
        assert!(!line.is_empty(), "its command line");
    }
}
