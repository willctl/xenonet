//! tar.c({ file, cwd, portable: true, gzip: true }, entries), as
//! cli/commands/setup.js packs a downloaded component: each entry under
//! `cwd`, folders with what's in them, as ustar headers (a pax header for a
//! name too long for one), without the owner's ids or names, gzipped. The
//! component's own reader (xenonet-components' tar) checks it as it does a
//! release's package.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

const BLOCK: usize = 512;

fn octal(field: &mut [u8], n: u64) {
    // The digits, then a NUL, in the field's width.
    let digits = format!("{n:0width$o}", width = field.len() - 1);
    let bytes = digits.as_bytes();
    let width = field.len() - 1;
    let from = bytes.len().saturating_sub(width);
    field[..width].copy_from_slice(&bytes[from..]);
    field[width] = 0;
}

fn put(field: &mut [u8], text: &[u8]) {
    let n = text.len().min(field.len());
    field[..n].copy_from_slice(&text[..n]);
}

/// A name in ustar's name and prefix, when it fits them.
fn split(name: &str) -> Option<(&str, &str)> {
    if name.len() <= 100 {
        return Some(("", name));
    }
    let bytes = name.as_bytes();
    (0..bytes.len()).rev().filter(|&i| bytes[i] == b'/').find(|&i| i <= 155 && bytes.len() - i - 1 <= 100 && i > 0).map(|i| (&name[..i], &name[i + 1..]))
}

fn header(name: &str, prefix: &str, flag: u8, mode: u32, size: u64, mtime: u64, link: &str) -> [u8; BLOCK] {
    let mut h = [0u8; BLOCK];
    put(&mut h[0..100], name.as_bytes());
    octal(&mut h[100..108], u64::from(mode & 0o7777));
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], mtime);
    h[156] = flag;
    put(&mut h[157..257], link.as_bytes());
    put(&mut h[257..263], b"ustar\0");
    put(&mut h[263..265], b"00");
    put(&mut h[345..500], prefix.as_bytes());
    // The checksum, counted with its own field as spaces.
    h[148..156].fill(b' ');
    let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
    octal(&mut h[148..155], sum);
    h[155] = b' ';
    h
}

fn pad(out: &mut impl Write, n: usize) -> io::Result<()> {
    let rest = (BLOCK - n % BLOCK) % BLOCK;
    out.write_all(&vec![0u8; rest])
}

/// A pax record: `"<length> path=<name>\n"`, its length counting itself.
fn pax_path(name: &str) -> Vec<u8> {
    let body = format!(" path={name}\n");
    let mut n = body.len() + 1;
    while format!("{n}{body}").len() != n {
        n += 1;
    }
    format!("{n}{body}").into_bytes()
}

#[cfg(unix)]
fn mode_of(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(m: &fs::Metadata) -> u32 {
    if m.is_dir() {
        0o755
    } else if m.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

fn entry(out: &mut impl Write, root: &Path, rel: &str) -> io::Result<()> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path)?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let mode = mode_of(&meta);
    let kind = meta.file_type();
    let name = if kind.is_dir() { format!("{rel}/") } else { rel.to_string() };
    let link = if kind.is_symlink() { fs::read_link(&path)?.to_string_lossy().into_owned() } else { String::new() };
    let (flag, size) = if kind.is_dir() {
        (b'5', 0)
    } else if kind.is_symlink() {
        (b'2', 0)
    } else {
        (b'0', meta.len())
    };
    let (prefix, short) = match split(&name) {
        Some(p) => p,
        None => {
            let record = pax_path(&name);
            out.write_all(&header("PaxHeader", "", b'x', 0o644, record.len() as u64, mtime, ""))?;
            out.write_all(&record)?;
            pad(out, record.len())?;
            ("", &name[name.len().saturating_sub(100)..])
        }
    };
    out.write_all(&header(short, prefix, flag, mode, size, mtime, &link))?;
    if flag == b'0' {
        let data = fs::read(&path)?;
        out.write_all(&data)?;
        pad(out, data.len())?;
    }
    if kind.is_dir() {
        for e in xenonet_base::paths::read_dir_sorted(&path)? {
            entry(out, root, &format!("{rel}/{}", e.file_name().to_string_lossy()))?;
        }
    }
    Ok(())
}

/// Packs `entries` (names under `cwd`) into `file`, gzipped.
pub fn create(file: &str, cwd: &str, entries: &[String]) -> io::Result<()> {
    let mut gz = GzEncoder::new(fs::File::create(file)?, Compression::default());
    for e in entries {
        entry(&mut gz, Path::new(cwd), e.trim_end_matches('/'))?;
    }
    // Two empty blocks end the archive.
    gz.write_all(&[0u8; BLOCK * 2])?;
    gz.finish()?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A component's folder, packed and unpacked again by the components'
    // own reader, which takes only what node-tar would have written.
    #[test]
    fn packs_what_the_components_reader_unpacks() {
        let root = std::env::temp_dir().join(format!("xenonet-cli-tar-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        let deep = format!("cli/{}", "d".repeat(120));
        fs::create_dir_all(src.join(&deep)).expect("folders");
        fs::write(src.join("cli/xenonet.js"), "// tools\n").expect("a file");
        fs::write(src.join(&deep).join("long.js"), "x").expect("a deep file");
        fs::write(src.join("component.json"), "{}").expect("a manifest");
        let file = root.join("c.tar.gz").to_string_lossy().into_owned();
        create(&file, &src.to_string_lossy(), &["cli".into(), "component.json".into()]).expect("packed");
        let out = root.join("out");
        fs::create_dir_all(&out).expect("a folder");
        xenonet_components::tar::unpack(&fs::read(&file).expect("the archive"), &out).expect("unpacked");
        assert_eq!(fs::read_to_string(out.join("cli/xenonet.js")).expect("the file"), "// tools\n");
        assert_eq!(fs::read_to_string(out.join(&deep).join("long.js")).expect("the deep file"), "x");
        let _ = fs::remove_dir_all(&root);
    }
}
