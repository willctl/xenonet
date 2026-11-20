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
    /// a process group of its own. `give` is a descriptor and the number it
    /// has in the child (every other descriptor of ours stays
    /// close-on-exec, as std makes them); ours is closed once the child
    /// has it.
    pub fn start(mut cmd: std::process::Command, deadline: Option<std::time::Duration>, give: Option<(std::os::fd::OwnedFd, i32)>) -> std::io::Result<Held> {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        if let Some((fd, at)) = &give {
            give_fd(&mut cmd, fd.as_raw_fd(), *at);
        }
        let mut child = cmd.spawn()?;
        drop(give);
        let group = libc::pid_t::try_from(child.id()).map_err(|_| std::io::Error::other("a process id too large for a group"))?;
        let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        Ok(Held { child, group, started: std::time::Instant::now(), deadline, ended: false, code: None, timed_out: false, stdin, stdout, stderr })
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
fn give_fd(cmd: &mut std::process::Command, from: libc::c_int, at: libc::c_int) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure makes only dup2 and fcntl calls, which are
    // async-signal-safe, and touches no memory of the parent's.
    unsafe {
        cmd.pre_exec(move || place(from, at));
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
}
