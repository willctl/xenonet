//! Files only you can read (core/private-file.js): Unix modes, or Windows
//! ACLs, never Windows mode bits. On Windows XenoNet asks whoami and icacls,
//! through a `run` it's given (a command's output, or its error), so a test
//! can be a made-up Windows.
//!
//! `write` and `write_new` make such a file private from the moment it
//! exists instead (mode 0600, or a protected DACL), and let other accounts
//! read it by SID, as a Windows service's account reads what its user wrote.

use crate::fs::FsError;
use regex::Regex;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// Runs a command: its output, or its error's message.
pub type Run<'a> = dyn FnMut(&str, &[&str]) -> Result<String, String> + 'a;

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A command `run` was asked for failed: its message.
    Run(String),
    /// XenoNet's own words.
    Said(String),
    /// Reading or writing a file, as Node words it.
    Fs(FsError),
}

impl Error {
    pub fn message(&self) -> &str {
        match self {
            Error::Run(m) | Error::Said(m) => m,
            Error::Fs(e) => &e.message,
        }
    }
}

/// How long a private command may take. Endpoint inspection can delay even
/// whoami on a busy Windows workstation, so the wait is long, but bounded,
/// and a failure stops the work before a private file is touched.
pub const PRIVATE_COMMAND_MS: u64 = 60000;

/// Runs a command as execFileSync would (with no window on Windows): its
/// output, or an error when it can't start, exits with a failure or runs
/// past `timeout`.
pub fn run_command(file: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new(file);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::Background::no_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("spawnSync {file} {}", crate::fs::code_of(&e).map_or_else(|| e.to_string(), str::to_string)))?;
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = read(Box::new(child.stdout.take().expect("stdout is piped above")));
    let err = read(Box::new(child.stderr.take().expect("stderr is piped above")));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("spawnSync {file} ETIMEDOUT"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if status.success() {
        Ok(out)
    } else {
        Err(format!("Command failed: {}\n{err}", [file].iter().chain(args).copied().collect::<Vec<_>>().join(" ")))
    }
}

/// `run_command` with the private commands' timeout.
pub fn run_private(file: &str, args: &[&str]) -> Result<String, String> {
    run_command(file, args, Duration::from_millis(PRIVATE_COMMAND_MS))
}

static SID_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?mR)"(S-1-[0-9-]+)"\s*$"#).expect("a fixed pattern"));
static ACL_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"D:[A-Z]*(\(|NO_ACCESS_CONTROL)").expect("a fixed pattern"));
static NO_ACL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"D:[A-Z]*NO_ACCESS_CONTROL").expect("a fixed pattern"));
static ACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^)]*)\)").expect("a fixed pattern"));

/// Your account's SID, from whoami /user: the last field of its CSV line.
pub fn user_sid(run: &mut Run<'_>) -> Result<String, Error> {
    let out = run("whoami", &["/user", "/fo", "csv", "/nh"]).map_err(Error::Run)?;
    SID_LINE.captures(&out).map(|c| c[1].to_string()).ok_or_else(|| Error::Said("whoami didn't say which account this is".into()))
}

// A name no other look uses. It needn't be secret, only unlike the others.
fn random_hex() -> String {
    format!("{:012x}", RandomState::new().build_hasher().finish() & 0xffff_ffff_ffff)
}

fn read_text(raw: &[u8]) -> String {
    let utf16 = |b: &[u8]| String::from_utf16_lossy(&b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>());
    if raw.len() >= 2 && raw[0] == 0xff && raw[1] == 0xfe {
        utf16(&raw[2..])
    } else if raw.len() > 1 && raw[1] == 0 {
        utf16(raw)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    }
}

/// A file's ACL as SDDL, which names accounts by SID in any language:
/// icacls /save writes it to a file of its own (in UTF-16, after a line
/// with the file's name), which is read and taken away.
pub fn acl_of(file: &str, run: &mut Run<'_>) -> Result<String, Error> {
    let out = std::env::temp_dir().join(format!("xenonet-acl-{}-{}.txt", std::process::id(), random_hex()));
    let out_s = out.to_string_lossy().into_owned();
    let found = (|| {
        run("icacls", &[file, "/save", &out_s, "/q"]).map_err(Error::Run)?;
        let raw = std::fs::read(&out).map_err(|e| Error::Fs(FsError::of(e, "open", &out_s)))?;
        let text = read_text(&raw);
        let line =
            text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).find(|l| ACL_LINE.is_match(l)).ok_or_else(|| Error::Said(format!("icacls gave no ACL for {file}")))?;
        Ok(line[line.find("D:").expect("the line matched ACL_LINE, so it has a D:")..].to_string())
    })();
    let _ = std::fs::remove_file(&out);
    found
}

// Yours aside, SYSTEM and the Administrators, which OpenSSH for Windows
// allows on a key too. icacls can shorten the built-in Administrator
// account to LA, including when that account was renamed: it has the
// administrator access allowed here, and treating it as an unknown user
// refuses files XenoNet just protected.
const TRUSTED: [&str; 7] = ["SY", "S-1-5-18", "BA", "S-1-5-32-544", "LA", "OW", "S-1-3-4"];

/// The accounts an ACL (SDDL) lets read a file besides yours (`sid`) and
/// the trusted ones: those of allow entries that apply to the file itself,
/// and everyone when there's no ACL at all.
pub fn others_in(sddl: &str, sid: &str) -> Vec<String> {
    if NO_ACL.is_match(sddl) {
        return vec!["everyone".into()];
    }
    let mut others = Vec::new();
    for ace in ACE.captures_iter(sddl) {
        let fields: Vec<&str> = ace[1].split(';').collect();
        let (kind, flags, trustee) = (fields[0], fields.get(1).copied().unwrap_or(""), fields.get(5).copied().unwrap_or(""));
        if !["A", "OA", "XA", "ZA"].contains(&kind) || flags.contains("IO") || trustee == sid || TRUSTED.contains(&trustee) {
            continue;
        }
        others.push(trustee.to_string());
    }
    others
}

/// A file's mode bits, as Node's stat gives them: Windows has none, and
/// Node calls a file there 0o666.
pub fn mode_of(m: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.mode()
    }
    #[cfg(not(unix))]
    {
        if m.permissions().readonly() {
            0o444
        } else {
            0o666
        }
    }
}

/// Who besides you can read a file: the accounts its ACL names, on
/// Windows, and elsewhere "others" when its mode lets anyone else in.
pub fn other_readers(file: &str, windows: bool, run: &mut Run<'_>) -> Result<Vec<String>, Error> {
    if windows {
        let acl = acl_of(file, run)?;
        return Ok(others_in(&acl, &user_sid(run)?));
    }
    let m = std::fs::metadata(file).map_err(|e| Error::Fs(FsError::of(e, "stat", file)))?;
    Ok(if mode_of(&m) & 0o077 != 0 { vec!["others".into()] } else { vec![] })
}

/// chmod 600, as Node does it: on Windows only the read-only flag.
#[cfg_attr(not(unix), expect(clippy::permissions_set_readonly_false, reason = "Windows has no world-writable bit: clearing read-only is all chmod 600 does there, as in Node"))]
pub fn chmod_600(file: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let mut p = std::fs::metadata(file)?.permissions();
        p.set_readonly(false);
        std::fs::set_permissions(file, p)
    }
}

/// Makes a file yours alone: on Windows nothing inherited and your account
/// alone in its ACL, elsewhere mode 0600.
pub fn protect_file(file: &str, windows: bool, run: &mut Run<'_>) -> Result<(), Error> {
    if windows {
        let grant = format!("*{}:F", user_sid(run)?);
        run("icacls", &[file, "/inheritance:r", "/grant:r", &grant, "/q"]).map_err(Error::Run)?;
        return Ok(());
    }
    chmod_600(Path::new(file)).map_err(|e| Error::Fs(FsError::of(e, "chmod", file)))
}

/// Writes `data` to `file`, readable by this account alone (with SYSTEM and, on Windows, `readers`:
/// string SIDs of the accounts that may read it too, as a service's account reads what its user
/// wrote). It goes to a new file nobody else can open from the moment it exists (`create_new`),
/// flushed, then renamed over the old one: no other account ever sees it readable, and a reader
/// sees the old file or the new one, never part of either.
pub fn write(file: &Path, data: &[u8], readers: &[String]) -> std::io::Result<()> {
    staged(file, data, readers, |tmp| renaming(tmp, file))
}

/// `write`, for a file that must not be replaced (a key): linked into place, which fails with
/// AlreadyExists when the file is there, so of two writers at once one wins and the other can read
/// what it wrote.
pub fn write_new(file: &Path, data: &[u8], readers: &[String]) -> std::io::Result<()> {
    staged(file, data, readers, |tmp| std::fs::hard_link(tmp, file))
}

// `data` in a new private file beside `file`, then `place` puts it there; the staged file is gone
// afterwards either way.
fn staged(file: &Path, data: &[u8], readers: &[String], place: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    let dir = file.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), random_hex()));
    let placed = (|| {
        let mut f = create_new(&tmp, readers)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        place(&tmp)
    })();
    let _ = std::fs::remove_file(&tmp);
    placed?;
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// A rename Windows refuses while another process has the file open (a reader, a scan): again, for
// up to about four seconds.
fn renaming(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut i = 0u64;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if cfg!(windows) && i < 20 && (e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))) => {
                std::thread::sleep(Duration::from_millis(20 + i * 20));
                i += 1;
            }
            r => return r,
        }
    }
}

/// A new file (never one that's there) only this account may open from the moment it exists:
/// mode 0600 on Unix; on Windows a protected DACL (`sddl`) that names SYSTEM, the file's owner and
/// this account in full, and each of `readers` reading.
pub fn create_new(file: &Path, readers: &[String]) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = readers;
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(file)
    }
    #[cfg(windows)]
    {
        win::create_new(file, readers)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = readers;
        std::fs::OpenOptions::new().write(true).create_new(true).open(file)
    }
}

/// Whether a string is a SID as SDDL takes it (S-1-…, digits and dashes).
pub fn valid_sid(s: &str) -> bool {
    s.len() > 4 && s.starts_with("S-1-") && s[4..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

/// The SDDL `create_new` gives a file on Windows: protected (nothing inherited); SYSTEM, the file's
/// owner (OW) and `me` in full; each valid reader reading.
pub fn sddl(me: Option<&str>, readers: &[String]) -> String {
    use std::fmt::Write as _;
    let me = me.filter(|m| valid_sid(m));
    let mut s = String::from("D:P(A;;FA;;;SY)(A;;FA;;;OW)");
    if let Some(me) = me {
        let _ = write!(s, "(A;;FA;;;{me})");
    }
    for r in readers.iter().filter(|r| valid_sid(r) && Some(r.as_str()) != me) {
        let _ = write!(s, "(A;;FR;;;{r})");
    }
    s
}

/// An account's SID by name (`NT SERVICE\Xenolith`), or None when there's no such account, and
/// always None off Windows, where a service runs as its user.
pub fn account_sid(name: &str) -> Option<String> {
    #[cfg(windows)]
    {
        win::account_sid(name)
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

#[cfg(windows)]
#[expect(unsafe_code, reason = "the Win32 calls that make a file with its DACL from the start and name accounts by SID, which std doesn't wrap")]
mod win {
    use std::path::Path;
    use windows_sys::Win32::Security::PSID;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// # Safety
    /// `sid` must point to a valid SID.
    unsafe fn sid_string(sid: PSID) -> Option<String> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
        let mut s: windows_sys::core::PWSTR = std::ptr::null_mut();
        // SAFETY: the caller passes a valid SID; the string is read up to its NUL and freed with
        // LocalFree.
        unsafe {
            if ConvertSidToStringSidW(sid, &raw mut s) == 0 || s.is_null() {
                return None;
            }
            let mut len = 0;
            while *s.add(len) != 0 {
                len += 1;
            }
            let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
            LocalFree(s.cast());
            Some(out)
        }
    }

    pub fn account_sid(name: &str) -> Option<String> {
        use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};
        let w = wide(name);
        let mut sid = [0u8; 128];
        let mut sid_len = sid.len() as u32;
        let mut domain = [0u16; 256];
        let mut domain_len = domain.len() as u32;
        let mut kind: SID_NAME_USE = 0;
        // SAFETY: the buffers and their lengths are passed in; the SID is read from our own buffer.
        unsafe {
            if LookupAccountNameW(std::ptr::null(), w.as_ptr(), sid.as_mut_ptr().cast(), &raw mut sid_len, domain.as_mut_ptr(), &raw mut domain_len, &raw mut kind) == 0 {
                return None;
            }
            sid_string(sid.as_mut_ptr().cast())
        }
    }

    /// The SID this process runs as.
    fn current_sid() -> Option<String> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the token handle is closed below; the buffer is ours, aligned for TOKEN_USER, and
        // its size passed in.
        unsafe {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
                return None;
            }
            let mut buf = [0u64; 64];
            let mut len = 0u32;
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), size_of_val(&buf) as u32, &raw mut len) != 0;
            CloseHandle(token);
            if !ok {
                return None;
            }
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            sid_string(user.User.Sid)
        }
    }

    /// CreateFileW with CREATE_NEW and a protected DACL from the start (super::sddl).
    pub fn create_new(p: &Path, readers: &[String]) -> std::io::Result<std::fs::File> {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::Foundation::{LocalFree, GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
        use windows_sys::Win32::Storage::FileSystem::{CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL};
        let sddl = wide(&super::sddl(current_sid().as_deref(), readers));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a valid, NUL-terminated SDDL string; the descriptor is freed with LocalFree below.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &raw mut sd, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        let path: Vec<u16> = {
            use std::os::windows::ffi::OsStrExt;
            p.as_os_str().encode_wide().chain([0]).collect()
        };
        // SAFETY: a valid, NUL-terminated path and security attributes; the handle becomes the File's.
        let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, &raw const sa, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
        let err = std::io::Error::last_os_error();
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW above.
        unsafe { LocalFree(sd) };
        if h == INVALID_HANDLE_VALUE {
            return Err(err);
        }
        // SAFETY: a handle CreateFileW just opened, owned by nothing else.
        Ok(unsafe { std::fs::File::from_raw_handle(h) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

    #[test]
    fn who_else_an_acl_lets_in() {
        assert_eq!(others_in(&format!("D:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;FA;;;{SID})"), SID), Vec::<String>::new());
        assert_eq!(others_in(&format!("D:AI(A;OICIIO;GA;;;CO)(A;ID;FA;;;{SID})(D;;FA;;;WD)"), SID), Vec::<String>::new());
        assert_eq!(others_in(&format!("D:AI(A;ID;FA;;;{SID})(A;ID;0x1200a9;;;BU)(A;ID;FR;;;AU)"), SID), ["BU", "AU"]);
        assert_eq!(others_in("D:NO_ACCESS_CONTROL", SID), ["everyone"]);
    }

    #[test]
    fn the_account_and_the_acl_from_a_made_up_windows() {
        let mut run = |file: &str, args: &[&str]| -> Result<String, String> {
            match file {
                "whoami" => Ok(format!("\"pc\\\\someone\",\"{SID}\"\r\n")),
                "icacls" if args[1] == "/save" => {
                    let body: Vec<u8> = [0xff, 0xfe].into_iter().chain(format!("x.token\r\nD:P(A;;FA;;;{SID})\r\n").encode_utf16().flat_map(u16::to_le_bytes)).collect();
                    std::fs::write(args[2], body).unwrap();
                    Ok(String::new())
                }
                _ => Err(format!("no {file} here")),
            }
        };
        assert_eq!(user_sid(&mut run).unwrap(), SID);
        assert_eq!(acl_of("x.token", &mut run).unwrap(), format!("D:P(A;;FA;;;{SID})"));
        assert_eq!(other_readers("x.token", true, &mut run).unwrap(), Vec::<String>::new());
        let mut silent = |_: &str, _: &[&str]| Ok(String::new());
        assert_eq!(user_sid(&mut silent), Err(Error::Said("whoami didn't say which account this is".into())));
    }

    #[test]
    fn who_else_an_acl_lets_in_by_kind_flags_and_trustee() {
        // Allow entries of every kind count; inherit-only and deny ones don't.
        let acl = format!("D:PAI(OA;;FR;;;AU)(XA;;FR;;;WD)(ZA;;FR;;;S-1-5-21-9)(A;IO;FA;;;BU)(A;OICIIO;FA;;;IU)(D;;FA;;;AN)(AU;;FA;;;NU)()(A;;FA;;;{SID})");
        assert_eq!(others_in(&acl, SID), ["AU", "WD", "S-1-5-21-9"]);
        // SYSTEM, the Administrators (by name, SID or the renamed built-in
        // account), the owner rights and yours are trusted.
        assert_eq!(others_in(&format!("D:(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)(A;;FA;;;LA)(A;;FA;;;OW)(A;;FA;;;S-1-3-4)(A;;FA;;;{SID})"), SID), Vec::<String>::new());
        // A SID that only starts like yours is someone else's.
        assert_eq!(others_in(&format!("D:(A;;FA;;;{SID}0)"), SID), [format!("{SID}0")]);
        assert_eq!(others_in("D:AINO_ACCESS_CONTROL", SID), ["everyone"]);
        assert_eq!(others_in("D:", SID), Vec::<String>::new());
    }

    /// whoami and icacls on a made-up Windows: what each was asked, with
    /// icacls /save writing `saved` as the ACL's file.
    fn windows(asked: &std::cell::RefCell<Vec<String>>, saved: Option<Vec<u8>>) -> impl FnMut(&str, &[&str]) -> Result<String, String> + '_ {
        move |file: &str, args: &[&str]| {
            asked.borrow_mut().push(std::iter::once(file).chain(args.iter().copied()).collect::<Vec<_>>().join(" "));
            match (file, args.get(1).copied(), &saved) {
                ("whoami", ..) => Ok(format!("\"pc\\\\someone\",\"{SID}\"\r\n")),
                ("icacls", Some("/save"), Some(body)) => {
                    std::fs::write(args[2], body).unwrap();
                    Ok(String::new())
                }
                ("icacls", Some("/inheritance:r"), _) => Ok(String::new()),
                _ => Err(format!("{file} failed\nAccess is denied.")),
            }
        }
    }

    #[test]
    fn an_acl_saved_in_any_of_icacls_encodings_and_the_file_taken_away() {
        let line = format!("x.token\r\nD:P(A;;FA;;;{SID})\r\n");
        let utf16: Vec<u8> = line.encode_utf16().flat_map(u16::to_le_bytes).collect();
        for body in [utf16, line.into_bytes()] {
            let asked = std::cell::RefCell::new(Vec::new());
            assert_eq!(acl_of("x.token", &mut windows(&asked, Some(body))).unwrap(), format!("D:P(A;;FA;;;{SID})"));
            // icacls x.token /save <the file> /q, with the file's path as it is, spaces and all.
            let saved = asked.borrow()[0].strip_prefix("icacls x.token /save ").and_then(|s| s.strip_suffix(" /q")).unwrap().to_string();
            assert!(saved.starts_with(&std::env::temp_dir().join("xenonet-acl-").to_string_lossy().into_owned()));
            assert!(!Path::new(&saved).exists(), "the saved ACL is taken away");
        }
        let asked = std::cell::RefCell::new(Vec::new());
        assert_eq!(acl_of("x.token", &mut windows(&asked, Some(b"x.token\r\nno ACL here\r\n".to_vec()))), Err(Error::Said("icacls gave no ACL for x.token".into())));
        assert_eq!(acl_of("x.token", &mut windows(&asked, None)), Err(Error::Run("icacls failed\nAccess is denied.".into())));
        assert_eq!(other_readers("x.token", true, &mut windows(&asked, None)).unwrap_err().message(), "icacls failed\nAccess is denied.");
    }

    #[test]
    fn made_yours_alone_on_a_made_up_windows() {
        let asked = std::cell::RefCell::new(Vec::new());
        protect_file("C:\\Users\\someone\\x.token", true, &mut windows(&asked, None)).unwrap();
        assert_eq!(asked.borrow().clone(), ["whoami /user /fo csv /nh".to_string(), format!("icacls C:\\Users\\someone\\x.token /inheritance:r /grant:r *{SID}:F /q")]);
        // Without the account, nothing is granted.
        let mut no_whoami = |file: &str, _: &[&str]| Err(format!("{file} failed"));
        assert_eq!(protect_file("x.token", true, &mut no_whoami), Err(Error::Run("whoami failed".into())));
        assert_eq!(Error::Fs(FsError { code: Some("EPERM".into()), message: "EPERM: operation not permitted".into() }).message(), "EPERM: operation not permitted");
    }

    #[test]
    fn the_sddl_names_the_owner_system_and_each_reader() {
        assert_eq!(sddl(None, &[]), "D:P(A;;FA;;;SY)(A;;FA;;;OW)");
        let svc = "S-1-5-80-1-2-3-4-5".to_string();
        assert_eq!(sddl(Some(SID), &[svc.clone(), "not a sid".into(), SID.into()]), format!("D:P(A;;FA;;;SY)(A;;FA;;;OW)(A;;FA;;;{SID})(A;;FR;;;{svc})"));
        assert!(!valid_sid("S-1-") && !valid_sid("S-1-5)(A;;FA;;;WD") && valid_sid(SID));
        if !cfg!(windows) {
            assert_eq!(account_sid(r"NT SERVICE\Xenolith"), None, "off Windows a service is its user");
        }
    }

    #[test]
    fn written_private_from_the_start_and_a_new_file_never_replaces_one() {
        let dir = std::env::temp_dir().join(format!("xenonet-private-write-{}-{}", std::process::id(), random_hex()));
        let file = dir.join("sub").join("secret.json");
        write(&file, b"one\n", &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\n");
        write(&file, b"two\n", &[]).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two\n", "write replaces");
        assert_eq!(write_new(&file, b"three\n", &[]).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two\n", "write_new never replaces");
        let key = dir.join("sub").join("a.key");
        write_new(&key, b"k\n", &[]).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("sub")).unwrap().count(), 2, "nothing staged is left");
        let mut never = |_: &str, _: &[&str]| -> Result<String, String> { panic!("no commands needed off Windows") };
        if !cfg!(windows) {
            for f in [&file, &key] {
                assert_eq!(other_readers(&f.to_string_lossy(), false, &mut never).unwrap(), Vec::<String>::new(), "{}", f.display());
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Replacing a file others could read leaves it private.
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
            write(&file, b"four\n", &[]).unwrap();
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        #[cfg(windows)]
        {
            let mut run = |f: &str, a: &[&str]| run_private(f, a);
            assert_eq!(other_readers(&key.to_string_lossy(), true, &mut run).unwrap(), Vec::<String>::new(), "only SYSTEM, the owner and this account");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn elsewhere_who_can_read_a_file_is_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let file = std::env::temp_dir().join(format!("xenonet-private-file-{}", std::process::id()));
        std::fs::write(&file, "x").unwrap();
        let name = file.to_string_lossy().into_owned();
        let mut never = |_: &str, _: &[&str]| -> Result<String, String> { panic!("no commands on unix") };
        for (mode, others) in [(0o644, vec!["others".to_string()]), (0o640, vec!["others".into()]), (0o604, vec!["others".into()]), (0o600, vec![]), (0o400, vec![])] {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(other_readers(&name, false, &mut never).unwrap(), others, "{mode:o}");
        }
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666)).unwrap();
        protect_file(&name, false, &mut never).unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_file(&file).unwrap();
        assert!(other_readers(&name, false, &mut never).unwrap_err().message().starts_with("ENOENT"));
    }

    #[cfg(unix)]
    #[test]
    fn commands_and_their_timeout() {
        assert_eq!(run_command("echo", &["hi"], Duration::from_secs(5)).unwrap(), "hi\n");
        assert!(run_command("false", &[], Duration::from_secs(5)).unwrap_err().starts_with("Command failed: false"));
        assert_eq!(run_command("sleep", &["5"], Duration::from_millis(100)).unwrap_err(), "spawnSync sleep ETIMEDOUT");
        assert_eq!(run_command("no-such-command-here", &[], Duration::from_secs(5)).unwrap_err(), "spawnSync no-such-command-here ENOENT");
    }
}
