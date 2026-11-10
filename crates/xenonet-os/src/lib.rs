//! What XenoNet reads of the system it runs on, as Node's built-ins give it
//! (os.homedir(), process.platform, crypto.randomUUID()), and the system's
//! own I/O the service and the drivers share: fetch() over HTTPS and lines
//! of another program's output.

pub mod lines;
pub mod net;

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
    let h = xenonet_base::jsstr::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}
