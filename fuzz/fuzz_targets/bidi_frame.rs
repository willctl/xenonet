#![no_main]

// WebSocket frames from a browser's BiDi endpoint: whatever the bytes are,
// the parser answers with a frame, a request for more, or an error, and
// what it says it used is inside the buffer.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // A small limit keeps the fuzzer from allocating what a frame claims.
    if let Ok(Some((frame, used))) = eidolon_bidi::frame::parse_with_limit(data, 1 << 20) {
        assert!(used <= data.len());
        assert!(frame.payload.len() < used);
    }
    let _ = eidolon_bidi::frame::parse(data);
});
