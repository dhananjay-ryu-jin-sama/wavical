#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(utf8_str) = std::str::from_utf8(data) {
        let _ = wavical_ai::catalog::parse_catalog_json(utf8_str);
    }
});

