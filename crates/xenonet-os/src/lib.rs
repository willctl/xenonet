//! The machine XenoNet runs on, and the system's own I/O every crate
//! shares:
//!
//! - `fs`: files as Node's fs reads and writes them; `files`: the file
//!   calls with their errors worded as Node words them; `private_file`: a
//!   file only its owner may read, on every system;
//! - `error`: why something didn't give its value, as its callers read it;
//! - `paths`: paths as XenoNet compares and shows them, and `nodepath`, as
//!   Node's `path` builds them;
//! - `process`: whether a process runs, and programs started in the
//!   background with no window;
//! - `sync`: locks that outlive a panic;
//! - `log`: the error log;
//! - `dirs`: XenoNet's own folders (state, cache); `clock`: a time in the
//!   machine's own time zone;
//! - `net`: fetch() over HTTPS (feature `fetch`); `lines`: lines of another
//!   program's output (feature `process`, with `fx` and `system`);
//! - `open`: the programs that open a URL, a file or a folder;
//! - `stats`: what the computer is doing (CPU, memory, GPU), for the
//!   console's system panel, and `system`, which reads it;
//! - `fx`: child processes run and started, the environment, and a thread
//!   for work whose futures aren't Send;
//! - and, as Node's built-ins give them, the home folder, the platform and
//!   random ids (below).

pub mod clock;
pub mod dirs;
pub mod error;
pub mod files;
#[cfg(feature = "follow")]
pub mod follow;
pub mod fs;
#[cfg(feature = "process")]
pub mod fx;
#[cfg(feature = "process")]
pub mod lines;
pub mod log;
#[cfg(feature = "fetch")]
pub mod net;
pub mod nodepath;
pub mod open;
pub mod paths;
pub mod private_file;
pub mod process;
pub mod stats;
pub mod sync;
#[cfg(feature = "process")]
pub mod system;

/// os.homedir(): HOME (USERPROFILE on Windows), as Node reads it.
pub fn home_dir(env: &dyn Fn(&str) -> Option<String>) -> String {
    let named = if cfg!(windows) { env("USERPROFILE") } else { env("HOME") };
    named.filter(|h| !h.is_empty()).unwrap_or_else(|| if cfg!(windows) { "C:\\".into() } else { "/".into() })
}

/// process.platform's name for this one.
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// crypto.randomUUID(): a version 4 UUID.
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = xenonet_text::utf16::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}
