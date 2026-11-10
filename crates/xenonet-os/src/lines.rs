//! Lines of another program's output, read as Node's utf8 streams read
//! them: a byte that isn't UTF-8 is U+FFFD, and the reading goes on.
//! tokio's `lines()` fails at such a byte, and a reader that stops there
//! loses every line after it, while the program's pipe fills and it
//! blocks on its next write.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// The next line, without its `\n` or `\r\n`; None at the end, or when
/// the read fails. `buf` is the reader's own, kept between lines.
pub async fn next_line<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> Option<String> {
    buf.clear();
    match r.read_until(b'\n', buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            Some(String::from_utf8_lossy(buf).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bytes_that_arent_utf8_are_read_past() {
        let mut r = tokio::io::BufReader::new(&b"one\n\xff\xfetwo\r\n\nthree"[..]);
        let mut buf = Vec::new();
        let mut got = Vec::new();
        while let Some(l) = next_line(&mut r, &mut buf).await {
            got.push(l);
        }
        assert_eq!(got, ["one", "\u{FFFD}\u{FFFD}two", "", "three"]);
    }
}
