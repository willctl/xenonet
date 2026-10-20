//! Unpacking a component's .tar.gz, as the JS did with node-tar
//! (`tar.x({ strict: true, preservePaths: false, filter, chmod: true })`),
//! with the same filter in front of every entry: only files and folders,
//! no name with a backslash or a colon, none from the root or with a ".."
//! in it, at most 100,000 entries and 1 GiB in all. An entry the filter
//! refuses is never written, and the whole archive counts as unsafe.
//!
//! The archive is read the way node-tar reads one: ustar headers with
//! their prefix, pax extended headers (x, and g for every entry after),
//! GNU long names (L, K), checksums checked, octal and base-256 numbers.

use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub const MAX_ENTRIES: usize = 100_000;
pub const MAX_BYTES: f64 = 1073741824.0;

/// Why an archive couldn't be unpacked.
#[derive(Debug, PartialEq)]
pub enum Unpack {
    /// An entry the filter refused: the archive is unsafe or too big.
    Refused,
    /// The archive itself is broken (node-tar's code and words).
    Bad(String),
    /// Writing what it holds failed.
    Write(xenonet_app::fsx::FsError),
}

/// node-tar's name for a header's type flag.
fn type_name(flag: u8) -> &'static str {
    match flag {
        b'0' => "File",
        0 => "OldFile",
        b'1' => "Link",
        b'2' => "SymbolicLink",
        b'3' => "CharacterDevice",
        b'4' => "BlockDevice",
        b'5' => "Directory",
        b'6' => "FIFO",
        b'7' => "ContiguousFile",
        b'g' => "GlobalExtendedHeader",
        b'x' => "ExtendedHeader",
        b'A' => "SolarisACL",
        b'D' => "GNUDumpDir",
        b'I' => "Inode",
        b'K' => "NextFileHasLongLinkpath",
        b'L' => "NextFileHasLongPath",
        b'M' => "ContinuationFile",
        b'N' => "OldGnuLongPath",
        b'S' => "SparseFile",
        b'V' => "TapeVolumeHeader",
        b'X' => "OldExtendedHeader",
        _ => "Unsupported",
    }
}

// A header's text field: up to the first NUL.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

// A header's number: octal, or base-256 when its first byte's top bit is set.
fn number(b: &[u8]) -> Option<f64> {
    if b.first().is_some_and(|c| c & 0x80 != 0) {
        let negative = b[0] & 0x40 != 0;
        let mut n = 0.0f64;
        for (i, &c) in b.iter().enumerate() {
            let c = if i == 0 { c & 0x7f } else { c };
            let c = if negative { !c } else { c };
            n = n * 256.0 + f64::from(c);
        }
        return Some(if negative { -(n + 1.0) } else { n });
    }
    let t = text(b);
    let t = t.trim_matches(|c: char| c == ' ' || c == '\0');
    if t.is_empty() {
        return None;
    }
    u64::from_str_radix(t, 8).ok().map(|n| n as f64)
}

fn checksum_ok(h: &[u8]) -> bool {
    let want = number(&h[148..156]);
    let sum: u32 = h.iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { 0x20 } else { u32::from(c) }).sum();
    want == Some(f64::from(sum))
}

// pax records: "<length> <key>=<value>\n", one after another.
fn pax(body: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = body;
    while !rest.is_empty() {
        let Some(sp) = rest.iter().position(|&c| c == b' ') else { break };
        let Ok(len) = std::str::from_utf8(&rest[..sp]).unwrap_or("").parse::<usize>() else { break };
        if len == 0 || len > rest.len() || len <= sp + 1 {
            break;
        }
        let record = &rest[sp + 1..len];
        let record = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(eq) = record.iter().position(|&c| c == b'=') {
            out.insert(String::from_utf8_lossy(&record[..eq]).into_owned(), String::from_utf8_lossy(&record[eq + 1..]).into_owned());
        }
        rest = &rest[len..];
    }
    out
}

/// One entry, as the filter sees it.
#[derive(Debug)]
pub struct Entry {
    pub path: String,
    pub kind: &'static str,
    pub size: f64,
    pub mode: Option<u32>,
    pub mtime: Option<f64>,
    data: (usize, usize),
}

/// The entries of an archive's tar, read in full (and their contents'
/// places in `tar`).
fn entries(tar: &[u8]) -> Result<Vec<Entry>, Unpack> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut global: HashMap<String, String> = HashMap::new();
    let mut extended: Option<HashMap<String, String>> = None;
    let mut long_path: Option<String> = None;
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|&c| c == 0) {
            break;
        }
        if !checksum_ok(h) {
            return Err(Unpack::Bad("TAR_ENTRY_INVALID: checksum failure".into()));
        }
        let flag = h[156];
        let mut size = number(&h[124..136]).unwrap_or(0.0);
        let ustar = &h[257..263] == b"ustar\0";
        let mut path = text(&h[0..100]);
        if ustar {
            let prefix = text(&h[345..500]);
            if !prefix.is_empty() {
                path = format!("{prefix}/{path}");
            }
        }
        let meta = |k: &str, ex: &Option<HashMap<String, String>>| ex.as_ref().and_then(|e| e.get(k)).or_else(|| global.get(k)).cloned();
        if let Some(s) = meta("size", &extended).and_then(|s| s.parse::<f64>().ok()) {
            if !matches!(flag, b'x' | b'g') {
                size = s;
            }
        }
        if size < 0.0 || !size.is_finite() {
            return Err(Unpack::Bad("TAR_ENTRY_INVALID: invalid size".into()));
        }
        let body_from = at + 512;
        let body_len = size as usize;
        let body_to = body_from.checked_add(body_len).filter(|t| *t <= tar.len()).ok_or_else(|| Unpack::Bad("TAR_BAD_ARCHIVE: Truncated input (needed more data)".into()))?;
        let body = &tar[body_from..body_to];
        at = body_from + body_len.div_ceil(512) * 512;
        match flag {
            b'x' => {
                extended = Some(pax(body));
                continue;
            }
            b'g' => {
                for (k, v) in pax(body) {
                    global.insert(k, v);
                }
                continue;
            }
            b'L' | b'N' => {
                long_path = Some(text(body));
                continue;
            }
            b'K' => continue,
            _ => {}
        }
        if let Some(p) = meta("path", &extended) {
            path = p;
        }
        if let Some(p) = long_path.take() {
            path = p;
        }
        let mtime = meta("mtime", &extended).and_then(|m| m.parse::<f64>().ok()).or_else(|| number(&h[136..148]));
        extended = None;
        out.push(Entry { path, kind: type_name(flag), size, mode: number(&h[100..108]).map(|m| m as u32), mtime, data: (body_from, body_to) });
    }
    Ok(out)
}

/// The filter the JS gave node-tar: whether an entry may be unpacked.
pub struct Filter {
    count: usize,
    size: f64,
    pub invalid: bool,
}

impl Filter {
    pub fn new() -> Self {
        Filter { count: 0, size: 0.0, invalid: false }
    }

    pub fn allows(&mut self, name: &str, kind: &str, size: f64) -> bool {
        self.count += 1;
        let unsafe_name = name.contains('\\') || name.contains(':') || name.starts_with('/') || name.split('/').any(|p| p == "..");
        if self.count > MAX_ENTRIES || !matches!(kind, "File" | "Directory") || unsafe_name || {
            self.size += size;
            self.size > MAX_BYTES
        } {
            self.invalid = true;
            return false;
        }
        true
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, Unpack> {
    // A plain tar is unpacked as node-tar would, without gunzipping.
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Ok(bytes.to_vec());
    }
    let mut out = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut out).map_err(|e| Unpack::Bad(format!("zlib: {e}")))?;
    Ok(out)
}

/// Unpacks `bytes` (a .tar.gz, or a .tar) into `cwd`, which exists. The
/// entries the filter refuses are skipped, and say so as Refused once the
/// rest are in.
pub fn unpack(bytes: &[u8], cwd: &Path) -> Result<(), Unpack> {
    let tar = gunzip(bytes)?;
    let mut filter = Filter::new();
    for e in entries(&tar)? {
        if !filter.allows(&e.path, e.kind, e.size) {
            continue;
        }
        let rel = e.path.trim_end_matches('/');
        if rel.is_empty() || rel == "." {
            continue;
        }
        let target = cwd.join(rel);
        let fs_err = |err: std::io::Error, call: &str| Unpack::Write(xenonet_app::fsx::FsError::of(err, call, &target.to_string_lossy()));
        if e.kind == "Directory" {
            make_dir(&target, e.mode.map_or(0o755, |m| m & 0o7777)).map_err(|err| fs_err(err, "mkdir"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| fs_err(err, "mkdir"))?;
        }
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(e.mode.map_or(0o644, |m| m & 0o7777));
        }
        let mut f = o.open(&target).map_err(|err| fs_err(err, "open"))?;
        std::io::Write::write_all(&mut f, &tar[e.data.0..e.data.1]).map_err(|err| fs_err(err, "write"))?;
        // A time no file can have (a pax mtime of 1e30 or inf, a base-256
        // one past what the system counts to) leaves the file with the
        // time it was written: Duration and SystemTime would panic on it.
        let mtime = e.mtime.filter(|m| *m >= 0.0).and_then(|m| std::time::Duration::try_from_secs_f64(m).ok()).and_then(|d| std::time::UNIX_EPOCH.checked_add(d));
        if let Some(t) = mtime {
            let _ = f.set_modified(t);
        }
    }
    if filter.invalid {
        return Err(Unpack::Refused);
    }
    Ok(())
}

fn make_dir(p: &Path, mode: u32) -> std::io::Result<()> {
    if p.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    b.create(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(h: &mut [u8]) {
        let sum: u32 = h.iter().enumerate().map(|(i, &c)| if (148..156).contains(&i) { 0x20 } else { u32::from(c) }).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h[155] = b' ';
    }

    fn header(name: &str, flag: u8, size: usize, mode: u32) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        h[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        h[136..147].copy_from_slice(b"14670000000");
        h[156] = flag;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        checksum(&mut h);
        h
    }

    /// A pax record, "<length> <key>=<value>\n", its length counting itself.
    fn pax_record(key: &str, value: &str) -> String {
        let rest = format!(" {key}={value}\n");
        let mut len = rest.len();
        while len != rest.len() + len.to_string().len() {
            len = rest.len() + len.to_string().len();
        }
        format!("{len}{rest}")
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("xenonet-tar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        dir
    }

    fn archive(entries: &[(&str, u8, &[u8], u32)]) -> Vec<u8> {
        let mut t = Vec::new();
        for (name, flag, data, mode) in entries {
            t.extend(header(name, *flag, data.len(), *mode));
            t.extend_from_slice(data);
            t.resize(t.len().div_ceil(512) * 512, 0);
        }
        t.extend(vec![0u8; 1024]);
        t
    }

    #[test]
    fn unpacks_files_and_folders_and_refuses_the_rest() {
        let dir = std::env::temp_dir().join(format!("xenonet-tar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = archive(&[("cli/", b'5', b"", 0o755), ("cli/xenonet.js", b'0', b"// tools", 0o644), ("node", b'0', b"#!/bin/sh\n", 0o755)]);
        unpack(&ok, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("cli/xenonet.js")).unwrap(), "// tools");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(std::fs::metadata(dir.join("node")).unwrap().permissions().mode() & 0o100 != 0);
        }
        for bad in
            [archive(&[("../escape", b'0', b"x", 0o644)]), archive(&[("link", b'1', b"", 0o644)]), archive(&[("/abs", b'0', b"x", 0o644)]), archive(&[("a\\b", b'0', b"x", 0o644)])]
        {
            assert_eq!(unpack(&bad, &dir), Err(Unpack::Refused));
        }
        assert!(!dir.parent().unwrap().join("escape").exists());
        let mut broken = ok.clone();
        broken[10] ^= 1;
        assert!(matches!(unpack(&broken, &dir), Err(Unpack::Bad(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_names_from_pax_and_gnu() {
        let long = "a/".repeat(60) + "file.txt";
        let record = format!(" path={long}\n");
        let rec = format!("{}{record}", record.len() + (record.len() + 3).to_string().len());
        let t = archive(&[
            ("PaxHeader", b'x', rec.as_bytes(), 0o644),
            ("short", b'0', b"pax", 0o644),
            ("././@LongLink", b'L', format!("{long}2\0").as_bytes(), 0o644),
            ("short2", b'0', b"gnu", 0o644),
        ]);
        let es = entries(&t).unwrap();
        assert_eq!(es.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), [long.as_str(), &format!("{long}2")]);
    }

    #[test]
    fn a_time_no_file_can_have_leaves_the_files_own() {
        let dir = temp("mtime");
        let into = dir.join("in");
        for mtime in ["1e30", "inf", "18446744073709551616"] {
            let t = archive(&[("PaxHeader", b'x', pax_record("mtime", mtime).as_bytes(), 0o644), ("pax", b'0', b"x", 0o644)]);
            assert_eq!(unpack(&t, &into), Ok(()), "{mtime}");
        }
        // A base-256 mtime of 2^88 - 1 seconds.
        let mut big = header("b256", b'0', 1, 0o644);
        big[136] = 0x80;
        big[137..148].fill(0xff);
        checksum(&mut big);
        let mut t = big;
        t.push(b'y');
        t.resize(t.len().div_ceil(512) * 512 + 1024, 0);
        assert_eq!(unpack(&t, &into), Ok(()));
        assert_eq!(std::fs::read_to_string(into.join("b256")).unwrap(), "y");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
