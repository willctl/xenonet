//! Whether a process is running, on every platform the binary is built for,
//! and the commands XenoNet starts in the background, which show no window.

/// Windows gives each console program a window of its own when the program
/// starting it has no console to share, as XenoNet's service, its
/// supervisor and Desktop don't: git, PowerShell and the apps' commands
/// they run in the background would each flash one on the screen.
/// CREATE_NO_WINDOW keeps them hidden.
pub const NO_WINDOW: u32 = 0x0800_0000;

/// A command XenoNet runs in the background, which shows no window of its
/// own on Windows (`NO_WINDOW`), and is unchanged elsewhere. Every command
/// XenoNet starts takes it, but one meant to open a window for you; a
/// tokio command takes it through `as_std_mut`.
pub trait Background {
    fn no_window(&mut self) -> &mut Self;
}

impl Background for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(NO_WINDOW);
        }
        self
    }
}

/// A program XenoNet started and every process it starts in turn, which
/// stop together. On Windows that's a job object, which also ends them all
/// when XenoNet's handle to it closes: a headless turn there can be cmd.exe
/// running a .cmd that runs node, and stopping cmd.exe alone would leave
/// node running. On unix it's the child's process group, when the child
/// leads one (started with `process_group(0)`); otherwise the program
/// itself.
pub struct Tree {
    #[cfg(windows)]
    job: Option<Job>,
    #[cfg(unix)]
    group: Option<libc::pid_t>,
}

impl Tree {
    /// The tree of a child just started. What it starts from now on is in
    /// it too.
    pub fn of(child: &std::process::Child) -> Tree {
        #[cfg(windows)]
        {
            Tree { job: Job::new().filter(|j| j.assign(child)) }
        }
        #[cfg(unix)]
        {
            Tree { group: led_group(child.id()) }
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = child;
            Tree {}
        }
    }

    /// Whether any process in it still runs besides the child itself, which
    /// its starter waits on: none outside Windows.
    pub fn others(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.as_ref().is_some_and(|j| j.active() > 0)
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Stops it all, at once: the job on Windows (or the child, if it has
    /// none), the child's whole process group on unix when it leads one
    /// (SIGKILL), else the child alone.
    pub fn kill(&self, child: &mut std::process::Child) {
        #[cfg(windows)]
        {
            if let Some(j) = &self.job {
                j.kill();
            }
        }
        #[cfg(unix)]
        if let Some(g) = self.group {
            kill_group(g);
        }
        let _ = child.kill();
    }
}

/// `pid`'s process group, if `pid` leads it: what a child started with
/// `process_group(0)` does. A child that shares its starter's group is
/// not one to signal as a group, which would reach the starter too.
#[cfg(unix)]
#[expect(unsafe_code, reason = "getpgid(2), which std doesn't wrap")]
fn led_group(pid: u32) -> Option<libc::pid_t> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: getpgid takes a plain pid and reads nothing of ours.
    (unsafe { libc::getpgid(pid) } == pid).then_some(pid)
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "killpg(2), which std doesn't wrap")]
fn kill_group(group: libc::pid_t) {
    // SAFETY: killpg takes a plain group id; one with no members left
    // answers ESRCH, which is ignored.
    unsafe {
        libc::killpg(group, libc::SIGKILL);
    }
}

/// A program started on unix with a descriptor of its own at a number it
/// is told (`Held::start`), which adytum-exec's `Spawned` has no way to
/// pass: the run's sign-in comes this way, so it is in no environment.
/// Like `Spawned` it leads its own process group, has a deadline that
/// stops the group, and is polled rather than waited on.
///
/// The descriptor is a pipe, which Claude Code can open as `/dev/fd/N` (a
/// socket can't be on Linux), and the program has descriptors 0 to 3 and no
/// other. The line goes into the pipe only after the program has started,
/// and XenoNet's end is closed right after, so what holds the read end
/// early holds an empty pipe. What is left: on macOS a pipe's two ends are
/// made close-on-exec by a second call (there is no `pipe2`), so a process
/// forked by another thread of the service in the microseconds between the
/// two calls holds the read end too and could read the line before the
/// program does. On Linux `pipe2` makes both ends close-on-exec at once, and
/// there is no such gap.
#[cfg(unix)]
pub struct Held {
    child: std::process::Child,
    group: libc::pid_t,
    started: std::time::Instant,
    deadline: Option<std::time::Duration>,
    ended: bool,
    code: Option<i32>,
    timed_out: bool,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    pub stderr: Option<std::process::ChildStderr>,
}

/// A line a `Held` is given on a descriptor of its own, at the number `at`.
#[cfg(unix)]
pub struct Give<'a> {
    pub line: &'a str,
    pub at: i32,
}

/// How a `Held` ended: its exit code (None if a signal ended it), and
/// whether its deadline stopped it.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeldExit {
    pub code: Option<i32>,
    pub timed_out: bool,
}

#[cfg(unix)]
impl Held {
    /// Starts `cmd` with its standard input, output and error as pipes, in
    /// a process group of its own. With `give`, the program has the read end
    /// of a pipe at `give.at`, and every other descriptor of ours that
    /// isn't close-on-exec is closed in it (std makes ours close-on-exec,
    /// but a library may not). The line and a newline go into the pipe once
    /// the program has started, and our end is closed.
    pub fn start(mut cmd: std::process::Command, deadline: Option<std::time::Duration>, give: Option<Give<'_>>) -> std::io::Result<Held> {
        use std::os::fd::{AsRawFd, OwnedFd};
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        // Looked at before the pipe is made, so its ends aren't in it.
        let mut ends: Option<(OwnedFd, OwnedFd)> = None;
        if let Some(g) = &give {
            let strays = left_open();
            let ceiling = fd_ceiling();
            let (read, write) = std::io::pipe()?;
            let (read, write): (OwnedFd, OwnedFd) = (read.into(), write.into());
            give_fd(&mut cmd, read.as_raw_fd(), g.at, strays, ceiling);
            ends = Some((read, write));
        }
        let mut child = cmd.spawn()?;
        let group = libc::pid_t::try_from(child.id()).map_err(|_| std::io::Error::other("a process id too large for a group"));
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let group = match group {
            Ok(g) => g,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let held = Held { child, group, started: std::time::Instant::now(), deadline, ended: false, code: None, timed_out: false, stdin, stdout, stderr };
        if let (Some((read, write)), Some(g)) = (ends, &give) {
            // The program has its own copy of the read end.
            drop(read);
            let mut write = std::fs::File::from(write);
            // A failure leaves the run to be stopped by its drop.
            std::io::Write::write_all(&mut write, g.line.as_bytes()).and_then(|()| std::io::Write::write_all(&mut write, b"\n"))?;
        }
        Ok(held)
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether anything in the child's group still runs: signal 0 sends
    /// nothing, and a member that has exited but isn't reaped counts.
    #[expect(unsafe_code, reason = "killpg(2) with signal 0, which std doesn't wrap")]
    pub fn group_running(&self) -> bool {
        // SAFETY: killpg takes a plain group id and a signal; 0 sends none.
        unsafe { libc::killpg(self.group, 0) == 0 }
    }

    /// Whether the child has ended, without waiting. Past the deadline the
    /// group is killed, as `Spawned::try_wait` does.
    pub fn try_wait(&mut self) -> std::io::Result<Option<HeldExit>> {
        if !self.ended {
            if let Some(status) = self.child.try_wait()? {
                self.ended = true;
                self.code = status.code();
            }
        }
        let late = self.deadline.is_some_and(|d| self.started.elapsed() >= d);
        if late && (!self.ended || self.group_running()) {
            self.kill_group();
            self.timed_out = true;
            self.ended = true;
        }
        Ok(self.ended.then_some(HeldExit { code: self.code, timed_out: self.timed_out }))
    }

    /// Kills the child and its group now, and reaps the child.
    pub fn kill_group(&mut self) {
        kill_group(self.group);
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
impl Drop for Held {
    fn drop(&mut self) {
        if !self.ended && !matches!(self.child.try_wait(), Ok(Some(_))) {
            self.kill_group();
        }
    }
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "pre_exec, which std marks unsafe")]
fn give_fd(cmd: &mut std::process::Command, from: libc::c_int, at: libc::c_int, strays: Vec<libc::c_int>, ceiling: libc::c_int) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure makes only dup2, fcntl and syscall calls,
    // which are async-signal-safe, and touches no memory of the parent's but
    // the list it owns.
    unsafe {
        cmd.pre_exec(move || {
            place(from, at)?;
            shut(&strays, at, ceiling);
            Ok(())
        });
    }
}

/// The descriptors of ours from 4 up that don't close on exec, by what the
/// process holds open now. Read before the pipe is made.
#[cfg(unix)]
#[expect(unsafe_code, reason = "fcntl(2), which std doesn't wrap")]
fn left_open() -> Vec<libc::c_int> {
    let Ok(dir) = std::fs::read_dir("/dev/fd") else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<libc::c_int>().ok())
        .filter(|&fd| {
            // SAFETY: F_GETFD on a number reads only the flags; one that isn't open answers -1.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            fd >= 4 && flags >= 0 && flags & libc::FD_CLOEXEC == 0
        })
        .collect()
}

/// How far up the child looks for descriptors to keep from the exec: the
/// soft limit on open files, read in the parent, capped so a huge limit
/// doesn't make every start slow.
#[cfg(unix)]
#[expect(unsafe_code, reason = "getrlimit(2), which std doesn't wrap")]
fn fd_ceiling() -> libc::c_int {
    const CAP: libc::c_int = 65_536;
    let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit writes only the struct it's given.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut lim) } != 0 {
        return CAP;
    }
    libc::c_int::try_from(lim.rlim_cur).map_or(CAP, |n| n.min(CAP))
}

/// In the child, after `place`: no descriptor but 0 to `at` goes through the
/// exec. On Linux every one from `at` + 1 up is marked close-on-exec with
/// close_range. Elsewhere (or on a Linux without it) each one from `at` + 1
/// up to `ceiling`, and any `left_open` found above that, is marked
/// close-on-exec where it isn't already. Marking rather than closing never
/// touches the pipe std reports a failed exec on (it is close-on-exec
/// already), even when another thread closed a listed number and std's pipe
/// took it, and it covers descriptors opened after `left_open` looked.
#[cfg(unix)]
#[expect(unsafe_code, reason = "close_range(2) and fcntl(2), which std doesn't wrap")]
fn shut(strays: &[libc::c_int], at: libc::c_int, ceiling: libc::c_int) {
    #[cfg(target_os = "linux")]
    {
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
        let first = libc::c_uint::try_from(at + 1).unwrap_or(4);
        // SAFETY: a plain syscall with numbers; it touches no memory.
        if unsafe { libc::syscall(libc::SYS_close_range, first, libc::c_uint::MAX, CLOSE_RANGE_CLOEXEC) } == 0 {
            return;
        }
    }
    let keep = |fd: libc::c_int| {
        // SAFETY: F_GETFD and F_SETFD on a number read and set only its
        // flags; one that isn't open answers -1, which is skipped.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    };
    for fd in (at + 1)..ceiling {
        keep(fd);
    }
    for &fd in strays {
        if fd >= ceiling {
            keep(fd);
        }
    }
}

/// In the child, between fork and exec: `from` at the number `at`, without
/// close-on-exec. When it already has that number it only loses the flag.
#[cfg(unix)]
#[expect(unsafe_code, reason = "dup2(2) and fcntl(2), which std doesn't wrap")]
fn place(from: libc::c_int, at: libc::c_int) -> std::io::Result<()> {
    // SAFETY: plain descriptor numbers; both calls are async-signal-safe.
    unsafe {
        if from == at {
            let flags = libc::fcntl(at, libc::F_GETFD);
            if flags < 0 || libc::fcntl(at, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        } else if libc::dup2(from, at) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A Windows job object whose processes end when its last handle closes
/// (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).
#[cfg(windows)]
struct Job(isize);

#[cfg(windows)]
#[expect(unsafe_code, reason = "job objects, which std doesn't wrap")]
impl Job {
    fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0 as windows_sys::Win32::Foundation::HANDLE
    }

    fn new() -> Option<Job> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: a job with no name and the default security; its limits
        // are read from a local that outlives the call, with its own size.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let job = Job(h as isize);
            let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            (SetInformationJobObject(h, JobObjectExtendedLimitInformation, (&raw const info).cast(), size) != 0).then_some(job)
        }
    }

    fn assign(&self, child: &std::process::Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: both handles are open: the job's is this Job's own, and the
        // child's is std's, which it holds until the child is dropped.
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.handle(), child.as_raw_handle().cast()) != 0 }
    }

    /// How many of its processes still run.
    fn active(&self) -> u32 {
        use windows_sys::Win32::System::JobObjects::{JobObjectBasicAccountingInformation, QueryInformationJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION};
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let size = size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32;
        // SAFETY: the job's handle is open, and the answer goes to a local
        // of the size given.
        let ok = unsafe { QueryInformationJobObject(self.handle(), JobObjectBasicAccountingInformation, (&raw mut info).cast(), size, std::ptr::null_mut()) != 0 };
        if ok {
            info.ActiveProcesses
        } else {
            0
        }
    }

    fn kill(&self) {
        // SAFETY: the job's handle is open.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle(), 1);
        }
    }
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "closing the job's handle, which std doesn't wrap")]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is this Job's own and closed once, here.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle());
        }
    }
}

/// Whether `pid` is a running process. On unix that's kill(pid, 0), which
/// sends nothing; on Windows, a process it can open that hasn't exited, as
/// libuv's uv_kill(pid, 0) checks. A process that's there but another
/// user's (EPERM, or access denied on Windows) counts only with `others`.
pub fn running(pid: u32, others: bool) -> bool {
    pid != 0 && check(pid, others)
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "kill(2) with signal 0, which std doesn't wrap")]
fn check(pid: u32, others: bool) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // SAFETY: kill with signal 0 sends nothing; it only checks that the
    // process exists and may be signalled.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    others && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "OpenProcess and GetExitCodeProcess, which std doesn't wrap")]
fn check(pid: u32, others: bool) -> bool {
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: i32 = 5;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }
    // SAFETY: OpenProcess takes plain values; the handle it gives is asked
    // one question, through a local that outlives the call, then closed.
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_INFORMATION | SYNCHRONIZE, 0, pid);
        if h == 0 {
            return others && std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED);
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &raw mut code) != 0 && code == STILL_ACTIVE;
        CloseHandle(h);
        ok
    }
}

#[cfg(not(any(unix, windows)))]
fn check(_pid: u32, _others: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::running;
    #[cfg(unix)]
    use super::Tree;

    #[test]
    fn this_process_runs_and_nothing_else_is_made_up() {
        assert!(running(std::process::id(), false));
        assert!(running(std::process::id(), true));
        assert!(!running(0, true));
        assert!(!running(u32::MAX - 1, true));
    }

    #[cfg(unix)]
    #[test]
    fn killing_a_group_leader_kills_what_it_started_too() {
        use std::os::unix::process::CommandExt as _;
        use std::process::{Command, Stdio};
        // The shell starts a grandchild that prints its pid, then both wait.
        let mut child = Command::new("sh").args(["-c", "sleep 300 & echo $!; wait"]).stdout(Stdio::piped()).process_group(0).spawn().expect("sh starts");
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(child.stdout.take().expect("piped")), &mut line).expect("the grandchild's pid");
        let grandchild: u32 = line.trim().parse().expect("a pid");
        assert!(running(grandchild, false));
        let tree = Tree::of(&child);
        tree.kill(&mut child);
        let _ = child.wait();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // A killed grandchild is reparented to init, which reaps it; wait for that.
        while running(grandchild, false) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!running(grandchild, false), "the grandchild is still running");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_in_the_starters_group_is_killed_alone() {
        use std::process::{Command, Stdio};
        let mut child = Command::new("sleep").arg("300").stdout(Stdio::null()).spawn().expect("sleep starts");
        let tree = Tree::of(&child);
        assert!(tree.group.is_none(), "the test's own group must never be signalled");
        tree.kill(&mut child);
        assert!(child.wait().is_ok());
    }

    /// Runs `script` under `sh -c` as a held run, with `line` on descriptor 3,
    /// and returns what it printed.
    #[cfg(unix)]
    fn held(script: &str, line: Option<&str>) -> String {
        use std::io::Read as _;
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args(["-c", script]);
        let give = line.map(|line| super::Give { line, at: 3 });
        let mut h = super::Held::start(cmd, Some(std::time::Duration::from_secs(20)), give).expect("sh starts");
        let mut out = String::new();
        h.stdout.take().expect("piped").read_to_string(&mut out).expect("output");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while h.try_wait().expect("waits").is_none() {
            assert!(std::time::Instant::now() < until, "the run ended");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        out
    }

    /// The numbers from 4 up that `sh` finds open (a path in /dev/fd exists
    /// for each descriptor the process holds).
    #[cfg(unix)]
    const OPEN_FROM_4: &str = "n=4; while [ $n -lt 120 ]; do [ -e /dev/fd/$n ] && echo \"open $n\"; n=$((n+1)); done; echo done";

    #[cfg(unix)]
    #[test]
    #[expect(unsafe_code, reason = "fcntl(2) on a descriptor the test owns")]
    fn a_held_run_has_descriptors_0_to_3_and_nothing_a_service_left_open() {
        use std::os::fd::AsRawFd;
        // A descriptor some code of the service made without close-on-exec,
        // and one it did make close-on-exec.
        let stray = std::fs::File::open("/dev/null").expect("opens");
        // SAFETY: fcntl on a descriptor this test owns, clearing its close-on-exec flag.
        unsafe {
            let flags = libc::fcntl(stray.as_raw_fd(), libc::F_GETFD);
            assert!(flags >= 0 && libc::fcntl(stray.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) == 0);
        }
        let careful = std::fs::File::open("/dev/null").expect("opens");
        let out = held(&format!("[ -e /dev/fd/3 ] && echo three; {OPEN_FROM_4}"), Some("made-up-token-nova"));
        drop((stray, careful));
        assert_eq!(out, "three\ndone\n", "descriptors 0-3 and no other: {out}");
        let out = held(OPEN_FROM_4, None);
        assert_eq!(out, "done\n", "a held run without a descriptor to give has none either: {out}");
    }

    #[cfg(unix)]
    #[test]
    fn a_held_run_reads_its_line_from_dev_fd_3_and_then_the_end() {
        // As Claude Code reads it on Linux: by opening /dev/fd/3, a pipe (a
        // socket can't be opened that way), one line and then the end, which
        // comes only because XenoNet closed its end after writing.
        let out = held("IFS= read -r tok < /dev/fd/3; rest=$(cat < /dev/fd/3); echo \"len: ${#tok} rest: ${#rest}\"", Some("made-up-token-nova"));
        assert_eq!(out, "len: 18 rest: 0\n");
    }
}
