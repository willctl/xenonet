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
    use super::{running, Tree};

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
