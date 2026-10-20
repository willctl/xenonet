//! The file calls the modules here make, with their errors worded as
//! Node's fs words them ("EACCES: permission denied, open '/x'"), so a
//! message the JS let through reads the same from Rust.

use std::io::Write;
use std::path::Path;
pub use xenonet_base::fs::FsError;

/// An error that isn't the system's: a message, with a code when the JS
/// gave its Error one.
pub fn said(message: impl Into<String>) -> FsError {
    FsError { code: None, message: message.into() }
}

pub fn coded(code: &str, message: impl Into<String>) -> FsError {
    FsError { code: Some(code.into()), message: message.into() }
}

/// fs.existsSync.
pub fn exists(p: &str) -> bool {
    !p.is_empty() && std::fs::metadata(p).is_ok()
}

/// fs.readFileSync(p, "utf8"), with U+FFFD for bytes that aren't UTF-8.
pub fn read_text(p: &str) -> Result<String, FsError> {
    std::fs::read(p).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| FsError::of(e, "open", p))
}

/// fs.mkdirSync(p, { recursive: true }, mode).
pub fn mkdir_p(p: &str, mode: Option<u32>) -> Result<(), FsError> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(m);
    }
    #[cfg(not(unix))]
    let _ = mode;
    b.create(p).map_err(|e| FsError::of(e, "mkdir", p))
}

/// fs.mkdirSync(p, { mode }): one folder, which mustn't be there yet.
pub fn mkdir(p: &str, mode: Option<u32>) -> Result<(), FsError> {
    #[cfg_attr(not(unix), expect(unused_mut, reason = "only unix gives the folder a mode"))]
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(m);
    }
    #[cfg(not(unix))]
    let _ = mode;
    b.create(p).map_err(|e| FsError::of(e, "mkdir", p))
}

/// fs.writeFileSync(p, data, { mode }): the mode only for a file it makes.
pub fn write_file(p: &str, data: &[u8], mode: Option<u32>) -> Result<(), FsError> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode.unwrap_or(0o666));
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = o.open(p).map_err(|e| FsError::of(e, "open", p))?;
    f.write_all(data).map_err(|e| FsError::of(e, "write", p))
}

/// fs.renameSync.
pub fn rename(from: &str, to: &str) -> Result<(), FsError> {
    std::fs::rename(from, to).map_err(|e| {
        let mut err = FsError::of(e, "rename", from);
        if err.code.is_some() {
            err.message = format!("{} -> '{to}'", err.message);
        }
        err
    })
}

/// fs.unlinkSync.
pub fn unlink(p: &str) -> Result<(), FsError> {
    std::fs::remove_file(p).map_err(|e| FsError::of(e, "unlink", p))
}

/// fs.rmSync(p, { recursive: true, force: true }).
pub fn rm_rf(p: &str) -> Result<(), FsError> {
    match std::fs::symlink_metadata(p) {
        Err(_) => Ok(()),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p).map_err(|e| FsError::of(e, "rm", p)),
        Ok(_) => std::fs::remove_file(p).map_err(|e| FsError::of(e, "rm", p)),
    }
}

/// fs.realpathSync.
pub fn realpath(p: &str) -> Result<String, FsError> {
    xenonet_base::fs::realpath(p).map_err(|e| FsError::of(e, "realpath", p))
}

/// fs.realpathSync.native.
pub fn realpath_native(p: &str) -> Result<String, FsError> {
    xenonet_base::fs::realpath_native(p).map_err(|e| FsError::of(e, "realpath", p))
}

/// fs.readdirSync(p): names in Node's order.
pub fn read_dir_names(p: &str) -> Result<Vec<String>, FsError> {
    xenonet_base::paths::read_dir_sorted(Path::new(p))
        .map(|ents| ents.into_iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .map_err(|e| FsError::of(e, "scandir", p))
}

/// n random bytes from the system, as hex (crypto.randomBytes(n).toString("hex")).
pub fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the system's random numbers");
    xenonet_base::jsstr::hex(&b)
}

/// fs.mkdtempSync(prefix): the prefix and six random letters and digits.
pub fn mkdtemp(prefix: &str) -> Result<String, FsError> {
    const SET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut last = None;
    for _ in 0..100 {
        let mut b = [0u8; 6];
        getrandom::fill(&mut b).expect("the system's random numbers");
        let dir = format!("{prefix}{}", b.iter().map(|x| SET[*x as usize % 62] as char).collect::<String>());
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(FsError::of(e, "mkdtemp", &format!("{prefix}XXXXXX"))),
            Err(e) => return Err(FsError::of(e, "mkdtemp", &format!("{prefix}XXXXXX"))),
        }
    }
    Err(last.unwrap_or_else(|| said("couldn't make a temporary folder")))
}
