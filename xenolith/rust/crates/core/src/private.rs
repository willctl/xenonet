//! Files only their owner may read: the owner key, access.json, sync.json and the peers' tokens.
//! Each is written to a new file that nobody else can open from the moment it exists (created
//! exclusively, mode 0600 on Unix, a protected DACL on Windows), then renamed over the old one, so
//! no other account ever sees it readable.
//!
//! On Unix a file is its owner's alone: the service runs as the user there. On Windows the service
//! runs as NT SERVICE\Xenolith, so a file may name readers besides its owner (the service account
//! for access.json, the installing user for the owner key the service writes).

use std::fs;
use std::io::Write;
use std::path::Path;

/// Writes `text` to `file`, readable by its owner (and SYSTEM) and, on Windows, by `readers`
/// (string SIDs) too.
pub fn write(file: &Path, text: &str, readers: &[String]) -> std::io::Result<()> {
    let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), crate::text::random_hex(8)));
    let r = (|| {
        let mut f = create_new(&tmp, readers)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        let mut i = 0u64;
        loop {
            match fs::rename(&tmp, file) {
                Err(e) if i < 20 && crate::wiki::retryable(&e) => {
                    std::thread::sleep(std::time::Duration::from_millis(20 + i * 20));
                    i += 1;
                }
                r => return r,
            }
        }
    })();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r?;
    crate::wiki::fsync_dir(dir);
    crate::readcache::forget(file);
    Ok(())
}

/// The readers a file the service must read gets on Windows: NT SERVICE\Xenolith, when that
/// service is installed. Nobody on Unix, where the service is the user.
pub fn service_readers() -> Vec<String> {
    #[cfg(windows)]
    {
        win::account_sid(r"NT SERVICE\Xenolith").into_iter().collect()
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

#[cfg(unix)]
fn create_new(p: &Path, _readers: &[String]) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p)
}

#[cfg(windows)]
fn create_new(p: &Path, readers: &[String]) -> std::io::Result<fs::File> {
    win::create_new(p, readers)
}

#[cfg(not(any(unix, windows)))]
fn create_new(p: &Path, _readers: &[String]) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().write(true).create_new(true).open(p)
}

/// Whether a string is a SID as SDDL takes it (S-1-…, digits and dashes).
pub fn valid_sid(s: &str) -> bool {
    s.len() > 4 && s.starts_with("S-1-") && s[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// The SDDL for a private file: protected (nothing inherited); SYSTEM, the file's owner (OW) and
/// `me` in full; each valid reader may read.
pub fn sddl(me: Option<&str>, readers: &[String]) -> String {
    let mut s = String::from("D:P(A;;FA;;;SY)(A;;FA;;;OW)");
    if let Some(me) = me.filter(|m| valid_sid(m)) {
        s.push_str(&format!("(A;;FA;;;{me})"));
    }
    for r in readers.iter().filter(|r| valid_sid(r) && Some(r.as_str()) != me) {
        s.push_str(&format!("(A;;FR;;;{r})"));
    }
    s
}

#[cfg(windows)]
mod win {
    use std::path::Path;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// # Safety
    /// `sid` must point to a valid SID.
    unsafe fn sid_string(sid: windows_sys::Win32::Security::PSID) -> Option<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
        let mut s: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: the caller passes a valid SID; the string is freed with LocalFree.
        unsafe {
            if ConvertSidToStringSidW(sid, &mut s) == 0 || s.is_null() {
                return None;
            }
            let len = (0..).take_while(|&i| *s.add(i) != 0).count();
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
            LocalFree(s as _);
            Some(out)
        }
    }

    /// An account's SID by name ("NT SERVICE\Xenolith"), or None when there is no such account.
    pub fn account_sid(name: &str) -> Option<String> {
        use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};
        let w = wide(name);
        let mut sid = [0u8; 128];
        let mut sid_len = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: buffers and their lengths are passed in; the SID is read from our own buffer.
        unsafe {
            if LookupAccountNameW(std::ptr::null(), w.as_ptr(), sid.as_mut_ptr() as _, &mut sid_len, domain.as_mut_ptr(), &mut domain_len, &mut kind) == 0 {
                return None;
            }
            sid_string(sid.as_mut_ptr() as _)
        }
    }

    /// The SID this process runs as.
    pub fn current_sid() -> Option<String> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the token handle is closed below; the buffer is ours and its size passed in.
        unsafe {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut buf = [0u64; 64];
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), std::mem::size_of_val(&buf) as u32, &mut len) != 0;
            CloseHandle(token);
            if !ok {
                return None;
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            sid_string(user.User.Sid)
        }
    }

    /// CreateFileW with CREATE_NEW and a protected DACL from the start (super::sddl).
    pub fn create_new(p: &Path, readers: &[String]) -> std::io::Result<std::fs::File> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree};
        use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows_sys::Win32::Storage::FileSystem::{CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL};
        let sddl = wide(&super::sddl(current_sid().as_deref(), readers));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a valid SDDL string; the descriptor is freed with LocalFree below.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        let path: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            p.as_os_str().encode_wide().chain([0]).collect()
        };
        // SAFETY: a valid path and security attributes; the handle becomes the File's.
        let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, &sa, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
        let err = std::io::Error::last_os_error();
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW above.
        unsafe { LocalFree(sd as _) };
        if h == INVALID_HANDLE_VALUE {
            return Err(err);
        }
        // SAFETY: a handle CreateFileW just opened, owned by nothing else.
        Ok(unsafe { std::fs::File::from_raw_handle(h as _) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sddl_names_the_owner_system_and_each_reader() {
        assert_eq!(sddl(None, &[]), "D:P(A;;FA;;;SY)(A;;FA;;;OW)");
        let me = "S-1-5-21-1-2-3-1001";
        let svc = "S-1-5-80-1-2-3-4-5".to_string();
        assert_eq!(sddl(Some(me), &[svc.clone(), "not a sid".into(), me.into()]), format!("D:P(A;;FA;;;SY)(A;;FA;;;OW)(A;;FA;;;{me})(A;;FR;;;{svc})"));
        assert!(!valid_sid("S-1-") && !valid_sid("S-1-5)(A;;FA;;;WD") && valid_sid(me));
    }

    #[cfg(unix)]
    #[test]
    fn a_private_file_is_the_owners_alone_from_the_start() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xenolith-private-{}", crate::text::random_hex(8)));
        let file = dir.join("sub").join("secret.json");
        write(&file, "one\n", &[]).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        // Replacing a file that was readable by others leaves it private, with nothing left over.
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        write(&file, "two\n", &[]).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two\n");
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1, "no temporary file left");
        fs::remove_dir_all(dir).unwrap();
    }
}
