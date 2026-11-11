//! The local API's credentials, each 32 random bytes made at each start of
//! the service and compared in constant time (routes/rules.rs takes them):
//!
//! - the window key (core/actors/key.js), the window's credential. It goes
//!   to a Desktop the service can tell is XenoNet's (the component
//!   service's hello) and nowhere else: never logged, written down or put
//!   in a URL. The page presents it as a cookie Desktop sets, or in
//!   x-xenonet-window.
//! - the API credential, for the command, the hooks and the MCP servers:
//!   written to `<runtime>/api-<port>.cred`, which only this account can
//!   read, so other accounts and anything that can't read the file are
//!   kept out. An agent on this account can read it, so it's never enough
//!   for a read that matters (rules.rs says which).
//! - the tray's key, for the tray app's routes (routes/tray.rs).
//! - the service's own key, for what it reads of its own API in-process
//!   (the hub serving this host's map to another machine): never written
//!   down or sent anywhere but this service's own port.

use std::fmt;
use std::path::{Path, PathBuf};

pub struct WindowKey {
    value: String,
}

impl WindowKey {
    pub fn new() -> WindowKey {
        WindowKey { value: random_hex(32) }
    }

    /// For the Desktop's hello; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-window is the key, compared in
    /// constant time.
    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }
}

impl Default for WindowKey {
    fn default() -> Self {
        WindowKey::new()
    }
}

// Logged by mistake, it says what it is and nothing more.
impl fmt::Debug for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the window key]")
    }
}

/// The tray's key: 32 random bytes, made at each start of the service, for
/// the tray app's routes (routes/tray.rs). It goes only to the tray app the
/// component socket can tell is XenoNet's (verify.tray), in its hello's
/// answer, and nowhere else: never written to a file, logged or put in a
/// URL. A file would be read by anything running as you, agents included.
pub struct TrayKey {
    value: String,
}

impl TrayKey {
    pub fn new() -> TrayKey {
        TrayKey { value: random_hex(32) }
    }

    /// For the tray's hello; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-tray is the key, in constant time.
    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }
}

impl Default for TrayKey {
    fn default() -> Self {
        TrayKey::new()
    }
}

impl fmt::Debug for TrayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the tray's key]")
    }
}

/// The API credential: made at each start and written to the runtime
/// folder for the programs of this account that talk to the API.
pub struct ApiKey {
    value: String,
}

impl ApiKey {
    pub fn new() -> ApiKey {
        ApiKey { value: random_hex(32) }
    }

    /// For the file, and for tests; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    /// Whether a request's x-xenonet-credential is the credential.
    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }

    /// Writes the credential to `file`, readable by this account alone,
    /// through a temporary file renamed into place, so no other account
    /// ever sees it half-written or with a wider mode.
    pub fn write(&self, file: &Path) -> std::io::Result<()> {
        write_private(file, self.value.as_bytes())
    }
}

impl Default for ApiKey {
    fn default() -> Self {
        ApiKey::new()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the API credential]")
    }
}

/// The service's own key, for its in-process reads of its own API.
pub struct SelfKey {
    value: String,
}

impl SelfKey {
    pub fn new() -> SelfKey {
        SelfKey { value: random_hex(32) }
    }

    /// For the service's own HTTP client; nowhere else.
    pub fn reveal(&self) -> &str {
        &self.value
    }

    pub fn matches(&self, given: Option<&str>) -> bool {
        given.is_some_and(|g| same(g.as_bytes(), self.value.as_bytes()))
    }
}

impl Default for SelfKey {
    fn default() -> Self {
        SelfKey::new()
    }
}

impl fmt::Debug for SelfKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[the service's own key]")
    }
}

/// Writes `bytes` to `file` for this account alone (0600, or on Windows an
/// ACL naming this account only), through a temporary file made that way
/// before anything is in it and renamed into place. Its folder is made
/// 0700 where it's new.
pub fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let tmp = PathBuf::from(format!("{}.{}.tmp", file.display(), std::process::id()));
    let wrote = (|| -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        drop(opts.open(&tmp)?);
        let mut run = |f: &str, a: &[&str]| xenonet_os::private_file::run_private(f, a);
        xenonet_os::private_file::protect_file(&tmp.to_string_lossy(), cfg!(windows), &mut run).map_err(|e| std::io::Error::other(e.message().to_string()))?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, file)
    })();
    if wrote.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    wrote
}

/// crypto.timingSafeEqual, with the lengths compared first as the JS does.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `crypto.randomBytes(n).toString("hex")`.
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the system's random numbers");
    xenonet_text::utf16::hex(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_matches_only_itself() {
        let k = WindowKey::new();
        assert_eq!(k.reveal().len(), 64);
        assert!(k.matches(Some(k.reveal())));
        assert!(!k.matches(Some("nope")));
        assert!(!k.matches(None));
        assert_eq!(format!("{k:?}"), "[the window key]");
        let t = TrayKey::new();
        assert!(t.matches(Some(t.reveal())) && !t.matches(Some(k.reveal())) && !t.matches(None));
        assert_eq!(format!("{t:?}"), "[the tray's key]");
        let a = ApiKey::new();
        assert!(a.matches(Some(a.reveal())) && !a.matches(Some(k.reveal())) && !a.matches(None));
        assert_eq!(format!("{a:?}"), "[the API credential]");
        let s = SelfKey::new();
        assert!(s.matches(Some(s.reveal())) && !s.matches(Some(a.reveal())));
    }

    #[test]
    fn the_credential_file_is_this_accounts_alone_and_new_each_time() {
        let dir = std::env::temp_dir().join(format!("xenonet-api-cred-{}", random_hex(4)));
        let file = dir.join("api-47999.cred");
        let (a, b) = (ApiKey::new(), ApiKey::new());
        a.write(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), a.reveal());
        b.write(&file).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), b.reveal(), "a new start writes over the last");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no temporary file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
