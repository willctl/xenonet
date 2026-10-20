#![no_main]

// JSON as XenoNet reads and writes it (JSON.parse and JSON.stringify):
// whatever it reads, it writes back as JSON that reads as the same value.

use libfuzzer_sys::fuzz_target;
use xenonet_base::json;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    xenonet_fuzz::run(data, || {
        if let Ok(v) = json::parse(text) {
            let written = json::stringify(&v, 0);
            let again = json::parse(&written).expect("stringify writes JSON");
            assert_eq!(json::stringify(&again, 0), written);
            let _ = json::stringify(&v, 2);
        }
    });
});
