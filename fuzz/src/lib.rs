//! What the fuzz targets share: turning libFuzzer's bytes into the inputs
//! XenoNet's readers take, and running a reader on the stack it gets in
//! the service.

use serde_json::Value;
use xenonet_base::json;

pub mod net;

/// The service runs its readers on threads with Rust's and tokio's default
/// stack, 2 MiB. libFuzzer runs on the main thread, which has more, so a
/// deeply nested input is read again on a thread like the service's, where
/// a reader that recurses too far overflows.
pub const SERVICE_STACK: usize = 2 * 1024 * 1024;

/// How deeply `text` nests brackets, counted the way a JSON reader would
/// (strings aren't skipped: it only decides when to use a second thread).
pub fn nesting(text: &[u8]) -> usize {
    let (mut depth, mut most) = (0usize, 0usize);
    for &b in text {
        match b {
            b'[' | b'{' => {
                depth += 1;
                most = most.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    most
}

/// Runs `f` as the service would: on the main thread, and, when the input
/// nests deeply, again on a thread with the service's stack.
pub fn run(data: &[u8], f: impl Fn() + Send + Sync) {
    f();
    if nesting(data) > 64 {
        std::thread::scope(|s| {
            let t = std::thread::Builder::new().stack_size(SERVICE_STACK).spawn_scoped(s, &f).expect("a thread");
            if let Err(e) = t.join() {
                std::panic::resume_unwind(e);
            }
        });
    }
}

/// The bytes as text, the way XenoNet reads a file it expects to be UTF-8:
/// lossily, so a cut character is U+FFFD rather than a skipped input.
pub fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

/// The bytes read as JSON, as XenoNet's readers read it.
pub fn json(data: &[u8]) -> Option<Value> {
    json::parse(&String::from_utf8_lossy(data)).ok()
}

/// Splits the input in two at the first `sep`: a target that takes two
/// inputs (a schema id and a record, a pattern and a subject) reads them
/// this way, so a seed stays readable.
pub fn split(data: &[u8], sep: u8) -> (&[u8], &[u8]) {
    match data.iter().position(|&b| b == sep) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[]),
    }
}
