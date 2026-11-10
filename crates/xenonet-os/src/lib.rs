//! The machine XenoNet runs on, and the system's own I/O every crate
//! shares:
//!
//! - `fs`: files as Node's fs reads and writes them; `private_file`: a
//!   file only its owner may read, on every system;
//! - `paths`: paths as XenoNet compares and shows them, and `nodepath`, as
//!   Node's `path` builds them;
//! - `process`: whether a process runs, and programs started in the
//!   background with no window;
//! - `sync`: locks that outlive a panic;
//! - `log`: the error log;
//! - `dirs`: XenoNet's own folders (state, cache);
//! - `net`: fetch() over HTTPS; `lines`: lines of another program's output;
//! - and, as Node's built-ins give them, the home folder, the platform and
//!   random ids (below).

pub mod dirs;
pub mod fs;
pub mod lines;
pub mod log;
pub mod net;
pub mod nodepath;
pub mod paths;
pub mod private_file;
pub mod process;
pub mod sync;

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
