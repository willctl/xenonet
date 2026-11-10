//! The few file calls the drivers and the command make on the disk, with
//! their errors worded and coded as Node words and codes them ("ENOENT: no
//! such file or directory, open '/x'"), since those words reach the page,
//! the command and the doctor as they are.

use std::io::Write;
use std::path::Path;
pub use xenonet_base::fs::FsError;

fn of(e: std::io::Error, syscall: &str, path: &str) -> FsError {
    FsError::of(e, syscall, path)
}

/// A two-path error, as Node words a rename's or a copy's.
fn of2(e: std::io::Error, syscall: &str, from: &str, to: &str) -> FsError {
    let mut err = of(e, syscall, from);
    if err.code.is_some() {
        err.message = format!("{} -> '{to}'", err.message);
    }
    err
}

/// Whether anything is at `path`.
pub fn exists(path: &str) -> bool {
    !path.is_empty() && Path::new(path).exists()
}

/// A file as text, with U+FFFD for bytes that aren't UTF-8.
pub fn read_text(path: &str) -> Result<String, FsError> {
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| of(e, "open", path))
}

fn open_with(path: &str, append: bool, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true);
    if append {
        o.append(true);
    } else {
        o.truncate(true);
    }
    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(m);
    }
    #[cfg(not(unix))]
    let _ = mode;
    o.open(path)
}

fn put(path: &str, data: &[u8], append: bool, mode: Option<u32>) -> Result<(), FsError> {
    let mut f = open_with(path, append, mode).map_err(|e| of(e, "open", path))?;
    f.write_all(data).map_err(|e| of(e, "write", path))
}

/// A file written whole; `mode` only for one it makes.
pub fn write_file(path: &str, data: &[u8], mode: Option<u32>) -> Result<(), FsError> {
    put(path, data, false, mode)
}

/// Added to the end of a file, made if it isn't there (with `mode`).
pub fn append_file(path: &str, data: &[u8], mode: Option<u32>) -> Result<(), FsError> {
    put(path, data, true, mode)
}

/// A folder and the folders above it that aren't there yet.
pub fn mkdir_all(dir: &str) -> Result<(), FsError> {
    std::fs::create_dir_all(dir).map_err(|e| of(e, "mkdir", dir))
}

pub fn rename(from: &str, to: &str) -> Result<(), FsError> {
    std::fs::rename(from, to).map_err(|e| of2(e, "rename", from, to))
}

pub fn copy(from: &str, to: &str) -> Result<(), FsError> {
    std::fs::copy(from, to).map(drop).map_err(|e| of2(e, "copyfile", from, to))
}

/// A file taken away; one that isn't there is no error.
pub fn remove_if_there(path: &str) -> Result<(), FsError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(of(e, "rm", path)),
        _ => Ok(()),
    }
}

/// The names in a folder, in Node's order.
pub fn read_dir(dir: &str) -> Result<Vec<String>, FsError> {
    let ents = xenonet_base::paths::read_dir_sorted(Path::new(dir)).map_err(|e| of(e, "scandir", dir))?;
    Ok(ents.into_iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
}

/// Where a link points.
pub fn read_link(path: &str) -> Result<String, FsError> {
    std::fs::read_link(path).map(|t| t.to_string_lossy().into_owned()).map_err(|e| of(e, "readlink", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-wire-fs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn files_are_read_and_written_and_errors_are_worded_as_node_words_them() {
        let d = tmp("rw");
        mkdir_all(&format!("{d}/a/b")).unwrap();
        let f = format!("{d}/a/b/x.json");
        write_file(&f, b"{}", Some(0o600)).unwrap();
        append_file(&f, b"\n", None).unwrap();
        assert_eq!(read_text(&f).unwrap(), "{}\n");
        assert!(exists(&f));
        assert_eq!(std::fs::metadata(&f).unwrap().len(), 3);
        assert_eq!(read_dir(&format!("{d}/a/b")).unwrap(), ["x.json"]);
        let missing = format!("{d}/nope");
        let e = read_text(&missing).unwrap_err();
        assert_eq!(e.message, format!("ENOENT: no such file or directory, open '{missing}'"));
        assert_eq!(e.code.as_deref(), Some("ENOENT"));
        let e = rename(&missing, &format!("{d}/z")).unwrap_err();
        assert_eq!(e.message, format!("ENOENT: no such file or directory, rename '{missing}' -> '{d}/z'"));
        rename(&f, &format!("{d}/y.json")).unwrap();
        assert_eq!(read_dir(&d).unwrap(), ["a", "y.json"]);
        remove_if_there(&format!("{d}/y.json")).unwrap();
        remove_if_there(&format!("{d}/y.json")).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}
