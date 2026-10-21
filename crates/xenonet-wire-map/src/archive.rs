//! Codex's archived rollouts (.jsonl.zst), read as the JS reads them with
//! zlib.zstdDecompressSync: stats, search and a thread's transcript read
//! one whole, decompressed. The live tailer doesn't read them (blocks.md).

use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd::decoding::StreamingDecoder;
use std::io::Read;

/// The most an archive may decompress to. A rollout runs to hundreds of
/// megabytes at most; a file that says more is refused rather than filling
/// the memory.
const MOST: u64 = 1 << 30;

/// Whether a transcript is a compressed archive: its name ends in .zst, as
/// the JS's endsWith tells it.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the JS's endsWith, which tells .zst from .ZST")]
pub fn is_archive(file: &str) -> bool {
    file.ends_with(".zst")
}

/// A transcript's bytes: a compressed archive's decompressed.
pub fn read(file: &str) -> std::io::Result<Vec<u8>> {
    let data = std::fs::read(file)?;
    if is_archive(file) {
        unzstd(&data, MOST)
    } else {
        Ok(data)
    }
}

/// Zstandard's frames, one after another, decompressed; a skippable frame
/// is passed over, and each frame's checksum checked when it has one.
/// Node reads only the first frame, where the format allows several (as
/// the zstd command reads them); Codex writes one.
pub fn unzstd(mut input: &[u8], most: u64) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    while !input.is_empty() {
        // How far a skippable frame reaches past its header, which the
        // decoder has read.
        let skip = match StreamingDecoder::new(&mut input) {
            Ok(mut frame) => {
                let room = (most + 1).saturating_sub(out.len() as u64);
                (&mut frame).take(room).read_to_end(&mut out)?;
                if out.len() as u64 > most {
                    return Err(std::io::Error::other("this archive is larger than XenoNet reads"));
                }
                // The decoder reads a frame's checksum but leaves checking
                // it to its caller.
                let d = &frame.decoder;
                if let (Some(said), Some(made)) = (d.get_checksum_from_data(), d.get_calculated_checksum()) {
                    if said != made {
                        return Err(std::io::Error::other("this archive's checksum doesn't match what it holds"));
                    }
                }
                continue;
            }
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame { length, .. })) => length as usize,
            Err(e) => return Err(std::io::Error::other(e)),
        };
        input = input.get(skip..).ok_or_else(|| std::io::Error::other("a skippable frame runs past the end of the archive"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // zlib.zstdCompressSync of two lines, and of a hundred x's with a
    // checksum.
    const LINES: &[u8] = &[40, 181, 47, 253, 32, 13, 105, 0, 0, 123, 34, 116, 121, 112, 101, 34, 58, 34, 97, 34, 125, 10];
    const LAST: &[u8] = &[40, 181, 47, 253, 32, 12, 97, 0, 0, 123, 34, 116, 121, 112, 101, 34, 58, 34, 98, 34, 125];
    const CHECKED: &[u8] = &[40, 181, 47, 253, 36, 100, 69, 0, 0, 16, 120, 120, 1, 0, 63, 1, 44, 148, 192, 163, 136];

    #[test]
    fn frames_decompress_in_turn() {
        assert_eq!(unzstd(LINES, MOST).unwrap(), b"{\"type\":\"a\"}\n");
        assert_eq!(unzstd(CHECKED, MOST).unwrap(), "x".repeat(100).as_bytes());
        assert_eq!(unzstd(&[LINES, LAST].concat(), MOST).unwrap(), b"{\"type\":\"a\"}\n{\"type\":\"b\"}");
        // A skippable frame (its magic, a length of 3, then 3 bytes) first.
        let skipped = [&[0x50, 0x2a, 0x4d, 0x18, 3, 0, 0, 0, 1, 2, 3][..], LAST].concat();
        assert_eq!(unzstd(&skipped, MOST).unwrap(), b"{\"type\":\"b\"}");
        assert_eq!(unzstd(&[], MOST).unwrap(), b"");
    }

    #[test]
    fn a_broken_or_oversized_archive_is_an_error_not_a_panic() {
        assert!(unzstd(b"{\"type\":\"a\"}\n", MOST).is_err(), "not zstd at all");
        assert!(unzstd(&LINES[..LINES.len() - 3], MOST).is_err(), "cut short");
        let mut bad = CHECKED.to_vec();
        let at = bad.len() - 1;
        bad[at] ^= 0xff;
        assert!(unzstd(&bad, MOST).is_err(), "a checksum that doesn't match");
        assert!(unzstd(&[0x50, 0x2a, 0x4d, 0x18, 9, 0, 0, 0, 1], MOST).is_err(), "a skippable frame past the end");
        assert!(unzstd(CHECKED, 99).is_err(), "larger than allowed");
        assert_eq!(unzstd(CHECKED, 100).unwrap().len(), 100);
    }

    #[test]
    fn a_file_is_read_whole_and_an_archive_decompressed() {
        let dir = std::env::temp_dir().join(format!("xenonet-archive-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("rollout.jsonl");
        let packed = dir.join("rollout.jsonl.zst");
        std::fs::write(&plain, LINES).unwrap();
        std::fs::write(&packed, LINES).unwrap();
        assert_eq!(read(&plain.to_string_lossy()).unwrap(), LINES, "a .jsonl is taken as it is");
        assert_eq!(read(&packed.to_string_lossy()).unwrap(), b"{\"type\":\"a\"}\n");
        assert!(!is_archive("rollout.jsonl.ZST"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
