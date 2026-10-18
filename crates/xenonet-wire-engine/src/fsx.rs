//! node:fs, for the cores that were given it ("fs", "files"): the few
//! synchronous calls they make, on the disk, with what they throw worded
//! and coded as Node words and codes it ("ENOENT: no such file or
//! directory, open '/x'").

use crate::js::{at, no_method, num, obj, text};
use std::io::Write;
use std::path::Path;
use xenonet_base::fs::{mtime_ms_of, FsError};
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::sql::Arg;

fn thrown(e: FsError) -> Thrown {
    Thrown { message: e.message, code: e.code.map(|c| Box::new(serde_json::Value::String(c))), handle: None, stdout: None }
}

fn of(e: std::io::Error, syscall: &str, path: &str) -> Thrown {
    thrown(FsError::of(e, syscall, path))
}

/// A two-path error, as Node words a rename's or a copy's.
fn of2(e: std::io::Error, syscall: &str, from: &str, to: &str) -> Thrown {
    let mut t = of(e, syscall, from);
    if t.code.is_some() {
        t.message = format!("{} -> '{to}'", t.message);
    }
    t
}

fn mode_of(o: &Arg) -> Option<u32> {
    match o.field("mode") {
        Arg::Num(n) if n.is_finite() && n >= 0.0 => Some(n as u32),
        _ => None,
    }
}

/// What writeFileSync and appendFileSync were given, as bytes.
fn bytes_of(a: &Arg) -> Vec<u8> {
    match a {
        Arg::Bytes(b) => b.clone(),
        other => text(other).into_bytes(),
    }
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

fn stats(m: &std::fs::Metadata) -> Arg {
    let mtime = mtime_ms_of(m).unwrap_or(f64::NAN);
    #[cfg(unix)]
    let mode = Arg::Num(f64::from(std::os::unix::fs::PermissionsExt::mode(&m.permissions())));
    #[cfg(not(unix))]
    let mode = Arg::Undef;
    obj(vec![("size", Arg::Num(m.len() as f64)), ("mtimeMs", Arg::Num(mtime)), ("isFile", Arg::Bool(m.is_file())), ("isDirectory", Arg::Bool(m.is_dir())), ("mode", mode)])
}

/// `fs.<method>(...args)`.
pub fn call(method: &str, args: &[Arg]) -> Result<Arg> {
    let p = text(&at(args, 0));
    match method {
        "existsSync" => Ok(Arg::Bool(!at(args, 0).is_nullish() && Path::new(&p).exists())),
        "readFileSync" => {
            let bytes = std::fs::read(&p).map_err(|e| of(e, "open", &p))?;
            // With an encoding, text; without, a Buffer's bytes.
            Ok(match at(args, 1) {
                Arg::Undef | Arg::Null => Arg::Bytes(bytes),
                _ => Arg::Text(String::from_utf8_lossy(&bytes).into_owned()),
            })
        }
        "writeFileSync" | "appendFileSync" => {
            let append = method == "appendFileSync";
            let mut f = open_with(&p, append, mode_of(&at(args, 2))).map_err(|e| of(e, "open", &p))?;
            f.write_all(&bytes_of(&at(args, 1))).map_err(|e| of(e, "write", &p))?;
            Ok(Arg::Undef)
        }
        "mkdirSync" => {
            let recursive = at(args, 1).field("recursive").truthy();
            let r = if recursive { std::fs::create_dir_all(&p) } else { std::fs::create_dir(&p) };
            r.map_err(|e| of(e, "mkdir", &p))?;
            Ok(Arg::Undef)
        }
        "renameSync" => {
            let to = text(&at(args, 1));
            std::fs::rename(&p, &to).map_err(|e| of2(e, "rename", &p, &to))?;
            Ok(Arg::Undef)
        }
        "copyFileSync" => {
            let to = text(&at(args, 1));
            std::fs::copy(&p, &to).map_err(|e| of2(e, "copyfile", &p, &to))?;
            Ok(Arg::Undef)
        }
        "unlinkSync" | "rmSync" => {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if method == "rmSync" && at(args, 1).field("force").truthy() && e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(of(e, if method == "rmSync" { "rm" } else { "unlink" }, &p)),
            }
            Ok(Arg::Undef)
        }
        "statSync" => std::fs::metadata(&p).map(|m| stats(&m)).map_err(|e| of(e, "stat", &p)),
        "lstatSync" => std::fs::symlink_metadata(&p).map(|m| stats(&m)).map_err(|e| of(e, "lstat", &p)),
        "readdirSync" => {
            let ents = xenonet_base::paths::read_dir_sorted(Path::new(&p)).map_err(|e| of(e, "scandir", &p))?;
            let types = at(args, 1).field("withFileTypes").truthy();
            Ok(Arg::List(
                ents.into_iter()
                    .map(|e| {
                        let name = Arg::Text(e.file_name().to_string_lossy().into_owned());
                        if !types {
                            return name;
                        }
                        let t = e.file_type().ok();
                        obj(vec![("name", name), ("isDirectory", Arg::Bool(t.is_some_and(|t| t.is_dir()))), ("isFile", Arg::Bool(t.is_some_and(|t| t.is_file())))])
                    })
                    .collect(),
            ))
        }
        "readlinkSync" => std::fs::read_link(&p).map(|t| Arg::Text(t.to_string_lossy().into_owned())).map_err(|e| of(e, "readlink", &p)),
        "realpathSync" => std::fs::canonicalize(&p).map(|t| Arg::Text(t.to_string_lossy().into_owned())).map_err(|e| of(e, "realpath", &p)),
        "accessSync" => std::fs::metadata(&p).map(|_| Arg::Undef).map_err(|e| of(e, "access", &p)),
        "chmodSync" => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(num(&at(args, 1)) as u32)).map_err(|e| of(e, "chmod", &p))?;
            }
            Ok(Arg::Undef)
        }
        _ => Err(no_method("fs", method)),
    }
}

/// The names of the folders in `dir` (a readdir with file types, folders
/// kept), as the observers' "dirs" reads them.
pub fn dirs(dir: &str) -> Result<Arg> {
    let ents = xenonet_base::paths::read_dir_sorted(Path::new(dir)).map_err(|e| of(e, "scandir", dir))?;
    Ok(Arg::List(ents.into_iter().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| Arg::Text(e.file_name().to_string_lossy().into_owned())).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-wire-fsx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.to_string_lossy().into_owned()
    }

    #[test]
    fn files_are_read_and_written_and_errors_are_worded_as_node_words_them() {
        let d = tmp("rw");
        call("mkdirSync", &[Arg::text(format!("{d}/a/b")), obj(vec![("recursive", Arg::Bool(true))])]).unwrap();
        let f = format!("{d}/a/b/x.json");
        call("writeFileSync", &[Arg::text(&f), Arg::text("{}"), obj(vec![("mode", Arg::Num(f64::from(0o600)))])]).unwrap();
        call("appendFileSync", &[Arg::text(&f), Arg::text("\n")]).unwrap();
        assert_eq!(call("readFileSync", &[Arg::text(&f), Arg::text("utf8")]).unwrap(), Arg::text("{}\n"));
        assert_eq!(call("existsSync", &[Arg::text(&f)]).unwrap(), Arg::Bool(true));
        assert_eq!(call("statSync", &[Arg::text(&f)]).unwrap().field("size"), Arg::Num(3.0));
        assert_eq!(call("readdirSync", &[Arg::text(format!("{d}/a/b"))]).unwrap(), Arg::List(vec![Arg::text("x.json")]));
        let missing = format!("{d}/nope");
        let e = call("readFileSync", &[Arg::text(&missing), Arg::text("utf8")]).unwrap_err();
        assert_eq!(e.message, format!("ENOENT: no such file or directory, open '{missing}'"));
        assert_eq!(e.code.as_deref(), Some(&serde_json::json!("ENOENT")));
        call("renameSync", &[Arg::text(&f), Arg::text(format!("{d}/y.json"))]).unwrap();
        assert_eq!(dirs(&d).unwrap(), Arg::List(vec![Arg::text("a")]));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
