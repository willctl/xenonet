//! Content-defined chunking: a gear hash cuts a file where its bytes say,
//! not at fixed offsets, so an insert changes only the chunks around it.
//!
//! This is the FastCDC scheme (gear hash, minimum size skipped, a stricter
//! mask before the average size and a looser one after it). The gear table
//! is made at compile time from a fixed seed, so every machine cuts the
//! same file the same way.

use crate::error::{PackError, Result};
use std::io::Read;

pub const MIN_CHUNK: usize = 16 * 1024;
pub const AVG_CHUNK: usize = 64 * 1024;
pub const MAX_CHUNK: usize = 256 * 1024;

// The hash shifts left, so its high bits hold the most recent bytes. 18 high
// bits must be zero before the average size, 14 after it (normalization 2).
const MASK_STRICT: u64 = ((1u64 << 18) - 1) << 46;
const MASK_LOOSE: u64 = ((1u64 << 14) - 1) << 50;

const fn make_gear() -> [u64; 256] {
    // splitmix64 from a fixed seed.
    let mut state: u64 = 0x6569_646f_6c6f_6e31;
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
}

static GEAR: [u64; 256] = make_gear();

/// How many bytes of `data` make the first chunk. `data` must hold either
/// `MAX_CHUNK` bytes or everything left of the file.
pub fn cut_point(data: &[u8]) -> usize {
    let n = data.len().min(MAX_CHUNK);
    if n <= MIN_CHUNK {
        return n;
    }
    let normal = n.min(AVG_CHUNK);
    let mut hash = 0u64;
    for (i, &b) in data.iter().enumerate().take(normal).skip(MIN_CHUNK) {
        hash = (hash << 1).wrapping_add(GEAR[usize::from(b)]);
        if hash & MASK_STRICT == 0 {
            return i + 1;
        }
    }
    for (i, &b) in data.iter().enumerate().take(n).skip(normal) {
        hash = (hash << 1).wrapping_add(GEAR[usize::from(b)]);
        if hash & MASK_LOOSE == 0 {
            return i + 1;
        }
    }
    n
}

/// Reads `reader` to its end and calls `each` with every chunk in order.
/// Returns the number of bytes read.
pub fn chunk_reader<R: Read>(mut reader: R, mut each: impl FnMut(&[u8]) -> Result<()>) -> Result<u64> {
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_CHUNK * 2);
    let mut total = 0u64;
    let mut eof = false;
    loop {
        while !eof && buf.len() < MAX_CHUNK {
            let have = buf.len();
            buf.resize(MAX_CHUNK, 0);
            let got = reader.read(&mut buf[have..]).map_err(PackError::io("read a file"));
            let got = match got {
                Ok(g) => g,
                Err(e) => {
                    buf.truncate(have);
                    return Err(e);
                }
            };
            buf.truncate(have + got);
            if got == 0 {
                eof = true;
            }
        }
        if buf.is_empty() {
            return Ok(total);
        }
        let cut = cut_point(&buf);
        each(&buf[..cut])?;
        total += cut as u64;
        buf.drain(..cut);
    }
}

/// The chunks of an in-memory file, as slices.
pub fn chunk_slice(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let cut = cut_point(rest);
        let (head, tail) = rest.split_at(cut);
        out.push(head);
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed pseudo-random byte stream (xorshift), so tests are repeatable.
    pub(crate) fn noise(len: usize, mut seed: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            out.extend_from_slice(&seed.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    #[test]
    fn sizes_stay_inside_the_bounds_and_average_near_the_target() {
        let data = noise(8 << 20, 7);
        let chunks = chunk_slice(&data);
        assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), data.len());
        let (last, body) = chunks.split_last().unwrap();
        assert!(body.iter().all(|c| (MIN_CHUNK..=MAX_CHUNK).contains(&c.len())));
        assert!(last.len() <= MAX_CHUNK);
        let avg = data.len() / chunks.len();
        assert!((32 * 1024..=128 * 1024).contains(&avg), "average {avg}");
    }

    #[test]
    fn the_cuts_do_not_depend_on_how_the_bytes_arrive() {
        let data = noise(1 << 20, 3);
        let whole: Vec<Vec<u8>> = chunk_slice(&data).into_iter().map(<[u8]>::to_vec).collect();
        // A reader that hands over 1000 bytes at a time.
        struct Dribble<'a>(&'a [u8]);
        impl Read for Dribble<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.0.len().min(out.len()).min(1000);
                out[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut streamed = Vec::new();
        let total = chunk_reader(Dribble(&data), |c| {
            streamed.push(c.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(total, data.len() as u64);
        assert_eq!(whole, streamed);
    }

    #[test]
    fn small_and_empty_inputs() {
        assert_eq!(chunk_slice(&[]).len(), 0);
        assert_eq!(chunk_slice(&[1, 2, 3]).len(), 1);
    }
}
